//! Keyword search: the `Searcher` trait, its shared request/response types, and
//! the per-entity implementations (one submodule per searched type).
//!
//! Design contract (see docs/implementation-plans/search-capability-backend.md):
//! searchers never write scoping SQL — visibility arrives as an already-resolved
//! relationship id set, and a searcher's only relationship-flavored SQL is the FK
//! path to that id plus a *data* join for `organization_id`. Searchers return
//! hits with `snippet: None` and placeholder display titles; the domain hydrates
//! both for the returned page only.

use std::cmp::Ordering;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::sea_query::{Condition, Expr, IntoCondition, SimpleExpr};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, JoinType, Order, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, RelationDef, Select, Value,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::Error;
use entity::coaching_relationships;
use entity::coaching_relationships::Visibility;
use entity::status::Status;
use entity::topic_priority::Priority;
use entity::topic_status::Status as TopicStatus;
use entity::transcript_suggestion_target::Target;
use entity::Id;

pub mod action;
pub mod agreement;
pub mod coaching_session;
pub mod goal;
mod session_title;
mod snippet;
pub mod topic;
pub mod transcript_suggestion;
mod visibility;

pub use session_title::resolve_session_titles;
pub use snippet::hydrate_snippets;
pub use visibility::{resolve_participant_relationship_ids, resolve_visible_relationship_ids};

/// Discriminates hit variants for the `types` filter, cursor tie-breaking, and
/// merge ordering. The derived `Ord` (declaration order, alphabetical by wire
/// name) is the "type ASC" of the response sort — both the per-searcher cursor
/// predicate fold and the domain merge must use it and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HitType {
    Action,
    Agreement,
    CoachingSession,
    Goal,
    Topic,
    TranscriptSuggestion,
}

impl HitType {
    /// Every searchable type, in merge (type-ascending) order. The single list
    /// that fan-out, snippet hydration, and the default `types` filter draw from.
    pub const ALL: [HitType; 6] = [
        HitType::Action,
        HitType::Agreement,
        HitType::CoachingSession,
        HitType::Goal,
        HitType::Topic,
        HitType::TranscriptSuggestion,
    ];

    /// Stable wire code for cursor payloads. Never renumber: new types take the
    /// next free code regardless of where they sort, so old cursors stay valid.
    fn code(self) -> u8 {
        match self {
            HitType::CoachingSession => 1,
            HitType::Goal => 2,
            HitType::Action => 3,
            HitType::Agreement => 4,
            HitType::Topic => 5,
            HitType::TranscriptSuggestion => 6,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.code() == code)
    }
}

/// Fields shared by every hit variant, flattened into each hit struct.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::Core)]
pub struct Core {
    pub id: Id,
    /// `ts_rank`; higher is better; comparable only within one response.
    pub score: f32,
    /// One-line label for the result row; never empty once the domain's
    /// post-merge hydration has run (sessions fall back to
    /// "Coaching session — YYYY-MM-DD").
    pub title: String,
    /// HTML-escaped excerpt whose only tags are `<mark>…</mark>` around
    /// matches (stored text can't forge them), hydrated post-merge for the
    /// returned page only — searchers always produce `None`.
    pub snippet: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<DateTimeWithTimeZone>,
    pub organization_id: Id,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::SessionHit)]
pub struct SessionHit {
    #[serde(flatten)]
    pub core: Core,
    pub coaching_relationship_id: Id,
    #[schema(value_type = String, format = DateTime)]
    pub date: DateTime,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::GoalHit)]
pub struct GoalHit {
    #[serde(flatten)]
    pub core: Core,
    pub coaching_relationship_id: Id,
    pub status: Status,
    pub created_in_session_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::ActionHit)]
pub struct ActionHit {
    #[serde(flatten)]
    pub core: Core,
    pub coaching_session_id: Id,
    pub coaching_relationship_id: Id,
    pub goal_id: Option<Id>,
    pub status: Status,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub due_by: Option<DateTimeWithTimeZone>,
    #[schema(value_type = String, format = DateTime)]
    pub session_date: DateTime,
    /// Hydrated post-merge (dated fallback when the session composes no title).
    pub session_title: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::AgreementHit)]
pub struct AgreementHit {
    #[serde(flatten)]
    pub core: Core,
    pub coaching_session_id: Id,
    pub coaching_relationship_id: Id,
    #[schema(value_type = String, format = DateTime)]
    pub session_date: DateTime,
    /// Hydrated post-merge (dated fallback when the session composes no title).
    pub session_title: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::TopicHit)]
pub struct TopicHit {
    #[serde(flatten)]
    pub core: Core,
    pub coaching_session_id: Id,
    pub coaching_relationship_id: Id,
    pub status: TopicStatus,
    pub priority: Option<Priority>,
}

/// A pending AI-suggested item; no review status, since every hit is pending.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = domain::search::TranscriptSuggestionHit)]
pub struct TranscriptSuggestionHit {
    #[serde(flatten)]
    pub core: Core,
    /// What accepting the suggestion would create.
    pub target: Target,
    pub coaching_session_id: Id,
    pub coaching_relationship_id: Id,
    #[schema(value_type = String, format = DateTime)]
    pub session_date: DateTime,
    /// Hydrated post-merge (dated fallback when the session composes no title).
    pub session_title: String,
}

/// One search result. Additional variants ship additively in later PRs
/// (notes, transcripts, users, organizations); clients must ignore unknown
/// `type` values.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = domain::search::Hit)]
pub enum Hit {
    Action(ActionHit),
    Agreement(AgreementHit),
    CoachingSession(SessionHit),
    Goal(GoalHit),
    Topic(TopicHit),
    TranscriptSuggestion(TranscriptSuggestionHit),
}

impl Hit {
    /// The entity type of this hit; matches the serialized `type` tag.
    pub fn hit_type(&self) -> HitType {
        match self {
            Hit::Action(_) => HitType::Action,
            Hit::Agreement(_) => HitType::Agreement,
            Hit::CoachingSession(_) => HitType::CoachingSession,
            Hit::Goal(_) => HitType::Goal,
            Hit::Topic(_) => HitType::Topic,
            Hit::TranscriptSuggestion(_) => HitType::TranscriptSuggestion,
        }
    }

    /// The fields every hit carries, whatever its variant.
    pub fn core(&self) -> &Core {
        match self {
            Hit::Action(h) => &h.core,
            Hit::Agreement(h) => &h.core,
            Hit::CoachingSession(h) => &h.core,
            Hit::Goal(h) => &h.core,
            Hit::Topic(h) => &h.core,
            Hit::TranscriptSuggestion(h) => &h.core,
        }
    }

    /// Mutable access to the shared fields, so post-processing (e.g. snippet
    /// hydration) can fill them in.
    pub fn core_mut(&mut self) -> &mut Core {
        match self {
            Hit::Action(h) => &mut h.core,
            Hit::Agreement(h) => &mut h.core,
            Hit::CoachingSession(h) => &mut h.core,
            Hit::Goal(h) => &mut h.core,
            Hit::Topic(h) => &mut h.core,
            Hit::TranscriptSuggestion(h) => &mut h.core,
        }
    }
}

/// Actions-by-goal-linkage filter (`goal_filter` param). Mirrors the
/// `assignee_filter` vocabulary precedent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalFilter {
    #[default]
    All,
    Linked,
    Unlinked,
}

/// Half-open `[from, to_exclusive)` window over a timestamptz column. Timezone
/// interpretation happens upstream (web) — these bounds are already absolute.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeRange {
    pub from: Option<DateTimeWithTimeZone>,
    pub to_exclusive: Option<DateTimeWithTimeZone>,
}

impl TimeRange {
    /// True when neither bound is set, so the filter adds no predicate.
    pub fn is_unbounded(&self) -> bool {
        self.from.is_none() && self.to_exclusive.is_none()
    }
}

/// The compiled, validated form of the request's filter params. One shared
/// union struct: each searcher reads only its relevant fields, so adding a
/// param touches this struct and the concerned searchers, never every
/// signature. `organization_id` and the `coaching_relationship_id` param are
/// folded into the resolved relationship id set by the domain before fan-out;
/// `organization_id` stays here for the PR 3 users searcher, which scopes by
/// org membership rather than relationships.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub organization_id: Option<Id>,
    /// Creator (`user_id` column) for goals/actions/agreements/topics; for
    /// sessions and transcript suggestions (which have no creator) the domain
    /// compiles it into `participant_relationship_ids`.
    pub user_id: Option<Id>,
    pub coaching_session_id: Option<Id>,
    pub goal_id: Option<Id>,
    pub goal_filter: GoalFilter,
    pub status: Option<Status>,
    pub topic_status: Option<TopicStatus>,
    pub created: TimeRange,
    pub updated: TimeRange,
    /// When `user_id` is set: the relationships where that user is coach or
    /// coachee, already intersected with the caller's visible set. The session
    /// searcher's effective relationship set ("by user" means participant for
    /// sessions, not creator). Compiled by the domain, never by callers.
    pub participant_relationship_ids: Option<Vec<Id>>,
}

/// Everything a searcher needs, resolved once per request by the domain.
pub struct Request<'a> {
    /// Length-clamped but *untrimmed* query text: the final-token prefix rule
    /// inspects the raw tail (trailing whitespace opts out of prefixing).
    pub q: &'a str,
    /// Reserved for searchers that scope by something other than relationships
    /// (users/organizations, later PRs). Relationship-anchored searchers use
    /// `visible_relationship_ids` only.
    pub visibility: &'a Visibility,
    /// `None` = super admin, unrestricted. `Some` may be empty (no visible
    /// relationships), which every searcher must treat as "no rows".
    pub visible_relationship_ids: Option<&'a [Id]>,
    pub filters: &'a Filters,
    pub cursor: Option<&'a Cursor>,
    /// `limit + 1` — must track page size so no row becomes unreachable
    /// behind a cursor (see the pagination decision in the plan).
    pub fetch: u64,
}

impl Request<'_> {
    /// The relationship id set a session-anchored searcher scopes on:
    /// the participant-narrowed set when the `user_id` filter targets
    /// participation (sessions, transcript suggestions), otherwise the
    /// caller's visible set.
    pub(crate) fn effective_relationship_ids(&self, participant_semantics: bool) -> Option<&[Id]> {
        if participant_semantics {
            if let Some(ids) = &self.filters.participant_relationship_ids {
                return Some(ids.as_slice());
            }
        }
        self.visible_relationship_ids
    }
}

/// One per searchable entity type. Implementations own their table's FTS
/// expression and FK path; they never re-express the visibility rule.
#[async_trait]
pub trait Searcher: Send + Sync {
    fn hit_type(&self) -> HitType;
    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error>;
}

/// Every active searcher, in merge (type-ascending) order.
pub fn searchers() -> Vec<Box<dyn Searcher>> {
    HitType::ALL.into_iter().map(searcher).collect()
}

fn searcher(hit_type: HitType) -> Box<dyn Searcher> {
    match hit_type {
        HitType::Action => Box::new(action::Searcher),
        HitType::Agreement => Box::new(agreement::Searcher),
        HitType::CoachingSession => Box::new(coaching_session::Searcher),
        HitType::Goal => Box::new(goal::Searcher),
        HitType::Topic => Box::new(topic::Searcher),
        HitType::TranscriptSuggestion => Box::new(transcript_suggestion::Searcher),
    }
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// Keyset cursor over the response sort `(score DESC, type ASC, id ASC)`.
///
/// `score` round-trips bit-exactly (`f32::to_bits`) — a decimal rendering would
/// break the equality arm of the continuation predicate. The wire form is
/// opaque base64 led by a one-byte mode/version discriminator so later modes
/// can carry a different payload shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub score: f32,
    pub hit_type: HitType,
    pub id: Id,
}

/// Keyword-mode cursor payload version.
const CURSOR_VERSION_KEYWORD: u8 = 1;

/// The `cursor` param failed to decode — the documented 400 for a malformed
/// cursor. Carries no detail: the encoding is opaque to clients by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorDecodeError;

impl Cursor {
    /// The opaque, URL-safe wire form handed to clients as `next_cursor`.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(22);
        bytes.push(CURSOR_VERSION_KEYWORD);
        bytes.extend_from_slice(&self.score.to_bits().to_be_bytes());
        bytes.push(self.hit_type.code());
        bytes.extend_from_slice(self.id.as_bytes());
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Parses a client-supplied `cursor` param.
    ///
    /// # Errors
    ///
    /// Returns `CursorDecodeError` if the input is not valid base64, has the
    /// wrong length or version, or names an unknown hit type.
    pub fn decode(raw: &str) -> Result<Self, CursorDecodeError> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| CursorDecodeError)?;
        if bytes.len() != 22 || bytes[0] != CURSOR_VERSION_KEYWORD {
            return Err(CursorDecodeError);
        }
        let score_bits = u32::from_be_bytes(bytes[1..5].try_into().map_err(|_| CursorDecodeError)?);
        let hit_type = HitType::from_code(bytes[5]).ok_or(CursorDecodeError)?;
        let id = Id::from_slice(&bytes[6..22]).map_err(|_| CursorDecodeError)?;
        Ok(Self {
            score: f32::from_bits(score_bits),
            hit_type,
            id,
        })
    }
}

// ---------------------------------------------------------------------------
// Query compilation (final-token prefix matching)
// ---------------------------------------------------------------------------

/// The tsquery a searcher runs: `websearch_to_tsquery` over the whole input,
/// optionally AND-ed with a `<lexeme>:*` prefix branch for the final plain word
/// (search-as-you-type).
///
/// The prefix branch ORs the `english` and `simple` parses of the lexeme.
/// `english` stems (`running:*` → `run:*`) but drops stop words, so a partial
/// word that happens to be one (`for` on the way to "forecast") would compile to
/// an empty query and match nothing; `simple` keeps it verbatim. Both match the
/// same `english` tsvector, so the GIN index still serves the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompiledQuery {
    /// `websearch_to_tsquery('english', $1)`
    Plain(String),
    /// The prefix branch over `$1` — a single alphanumeric `<lexeme>:*` term.
    Prefix(String),
    /// `websearch_to_tsquery('english', $1) &&` the prefix branch over `$2`.
    HeadAndPrefix(String, String),
}

/// Compile the raw (length-clamped, untrimmed) query text.
///
/// The final token is prefix-matched only when it is a plain alphanumeric word,
/// the caller didn't opt out with trailing whitespace, it isn't inside an open
/// quoted phrase, and the query contains no `or` operator. Punctuation
/// disqualifies: the parser indexes `check-in` as `check-in` and `check`, so
/// any lexeme we derived (`checkin:*`) would miss. The alphanumeric rule also
/// preserves the never-throws guarantee — `websearch_to_tsquery` never sees
/// anything but raw user text (safe), and `to_tsquery` only ever sees a bare
/// alphanumeric word.
pub(crate) fn compile_query(raw: &str) -> CompiledQuery {
    let trimmed = raw.trim();
    let opted_out = raw.ends_with(char::is_whitespace);
    let mut tokens = trimmed.split_whitespace().rev();
    let last = tokens.next().unwrap_or("");
    // Splitting off the final word regroups any `or` in the head: `a or b c`
    // means `a | (b & c)`, but the AND-ed prefix makes it `(a | b) & c:*`.
    // A trailing `or` is worse — websearch_to_tsquery drops it outright. A
    // lone `or` has no left operand, so it stays a prefix.
    let head = trimmed[..trimmed.len() - last.len()].trim_end();
    let has_or = tokens.any(|t| t.eq_ignore_ascii_case("or"))
        || (!head.is_empty() && last.eq_ignore_ascii_case("or"));
    let unbalanced_quotes = trimmed.matches('"').count() % 2 == 1;
    let prefixable = !opted_out
        && !unbalanced_quotes
        && !has_or
        && !last.is_empty()
        && last.chars().all(char::is_alphanumeric);

    if !prefixable {
        return CompiledQuery::Plain(trimmed.to_string());
    }
    if head.is_empty() {
        CompiledQuery::Prefix(format!("{last}:*"))
    } else {
        CompiledQuery::HeadAndPrefix(head.to_string(), format!("{last}:*"))
    }
}

impl CompiledQuery {
    /// The tsquery as SQL text with `$1`(/`$2`) placeholders plus its binds,
    /// for embedding into a larger `Expr::cust_with_values` expression.
    fn sql_and_binds(&self) -> (&'static str, Vec<Value>) {
        match self {
            CompiledQuery::Plain(q) => (
                "websearch_to_tsquery('english', $1)",
                vec![q.clone().into()],
            ),
            // SeaQuery numbers placeholders by position, so each parse of the
            // lexeme gets its own bind rather than sharing one `$n`.
            CompiledQuery::Prefix(lexeme) => (
                "(to_tsquery('english', $1) || to_tsquery('simple', $2))",
                vec![lexeme.clone().into(), lexeme.clone().into()],
            ),
            CompiledQuery::HeadAndPrefix(head, lexeme) => (
                "(websearch_to_tsquery('english', $1) \
                 && (to_tsquery('english', $2) || to_tsquery('simple', $3)))",
                vec![
                    head.clone().into(),
                    lexeme.clone().into(),
                    lexeme.clone().into(),
                ],
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared SQL builders
// ---------------------------------------------------------------------------

/// The FTS expressions for one searcher's table, built over its raw-text SQL
/// expression (each searcher's `TEXT_EXPR`). The derived tsvector expression
/// must stay semantically identical to the matching GIN index expression in
/// `migration/src/m20260922_000000_add_search_fts_indexes.rs` (same functions,
/// casts and coalesces — table qualification is fine), or the planner stops
/// using the index.
pub(crate) struct FtsExpressions {
    tsv: String,
}

impl FtsExpressions {
    pub(crate) fn over(text_expr: &str) -> Self {
        Self {
            tsv: format!("to_tsvector('english', {text_expr})"),
        }
    }

    /// `tsv @@ query` — the index-served match condition.
    pub(crate) fn match_condition(&self, cq: &CompiledQuery) -> SimpleExpr {
        let (query_sql, binds) = cq.sql_and_binds();
        Expr::cust_with_values(format!("{} @@ {}", self.tsv, query_sql), binds)
    }

    /// `ts_rank(tsv, query)` — recomputed per matching row (see the plan's
    /// expression-index tradeoff note).
    pub(crate) fn score(&self, cq: &CompiledQuery) -> SimpleExpr {
        let (query_sql, binds) = cq.sql_and_binds();
        Expr::cust_with_values(format!("ts_rank({}, {})", self.tsv, query_sql), binds)
    }
}

/// The keyset continuation predicate for one searcher, with the mixed-direction
/// tie-breakers folded per the searcher's constant type:
/// `score < s OR (score = s AND (type > t OR (type = t AND id > i)))`.
pub(crate) fn cursor_condition<C: ColumnTrait>(
    score: impl Fn() -> SimpleExpr,
    id_column: C,
    my_type: HitType,
    cursor: &Cursor,
) -> Condition {
    match my_type.cmp(&cursor.hit_type) {
        // This type sorts after the cursor's type: equal-score rows are still due.
        Ordering::Greater => Expr::expr(score()).lte(cursor.score).into_condition(),
        // Same type: break ties by id.
        Ordering::Equal => Condition::any()
            .add(Expr::expr(score()).lt(cursor.score))
            .add(
                Condition::all()
                    .add(Expr::expr(score()).eq(cursor.score))
                    .add(id_column.gt(cursor.id)),
            ),
        // This type sorts before the cursor's type: equal scores were already served.
        Ordering::Less => Expr::expr(score()).lt(cursor.score).into_condition(),
    }
}

/// Where one searcher's hits come from: what the shared query skeleton needs
/// to know about its table.
pub(crate) struct Source<C, R> {
    pub hit_type: HitType,
    /// The searcher's raw-text SQL expression (its `TEXT_EXPR`).
    pub text_expr: &'static str,
    pub id: C,
    pub created_at: C,
    pub updated_at: C,
    /// The relationship id column the visibility scope tests: the table's own
    /// FK, or `coaching_sessions`' for session-anchored types.
    pub relationship_id: R,
    /// FK path from this table to `coaching_relationships` — a data join for
    /// `organization_id`, never a scope test.
    pub path: Vec<RelationDef>,
}

/// The query skeleton every FTS searcher shares, so a searcher only adds its
/// own columns and type-specific filters between [`select`](Self::select) and
/// [`finish`](Self::finish).
pub(crate) struct FtsQuery<'r, 'a, C, R> {
    req: &'r Request<'a>,
    source: Source<C, R>,
    cq: CompiledQuery,
    fts: FtsExpressions,
    participant_scoped: bool,
}

impl<'r, 'a, C, R> FtsQuery<'r, 'a, C, R>
where
    C: ColumnTrait + Copy,
    R: ColumnTrait + Copy,
{
    pub(crate) fn new(req: &'r Request<'a>, source: Source<C, R>) -> Self {
        Self {
            req,
            cq: compile_query(req.q),
            fts: FtsExpressions::over(source.text_expr),
            source,
            participant_scoped: false,
        }
    }

    /// Scope on the domain's participant-narrowed relationship set when the
    /// `user_id` filter is present (it means participant, not creator).
    pub(crate) fn participant_scoped(mut self) -> Self {
        self.participant_scoped = true;
        self
    }

    /// The matching rows with the columns every hit's `Core` reads: `id`,
    /// `score`, `created_at`, `updated_at`, `coaching_relationship_id`, and
    /// `organization_id` via the FK path.
    pub(crate) fn select<E: EntityTrait<Column = C>>(&self) -> Select<E> {
        let src = &self.source;
        let select = E::find()
            .select_only()
            .column(src.id)
            .column_as(self.fts.score(&self.cq), "score")
            .column(src.created_at)
            .column(src.updated_at)
            .column_as(src.relationship_id, "coaching_relationship_id");
        src.path
            .iter()
            .cloned()
            .fold(select, |s, rel| s.join(JoinType::InnerJoin, rel))
            .column(coaching_relationships::Column::OrganizationId)
            .filter(self.fts.match_condition(&self.cq))
    }

    /// Relationship scope, created/updated windows, cursor keyset, response
    /// ordering, and the `limit + 1` fetch.
    pub(crate) fn finish<E: EntityTrait<Column = C>>(&self, select: Select<E>) -> Select<E> {
        let src = &self.source;
        let f = self.req.filters;
        select
            .apply_if(
                self.req.effective_relationship_ids(self.participant_scoped),
                |q, ids| q.filter(src.relationship_id.is_in(ids.iter().copied())),
            )
            .apply_if(f.created.from, |q, v| q.filter(src.created_at.gte(v)))
            .apply_if(f.created.to_exclusive, |q, v| {
                q.filter(src.created_at.lt(v))
            })
            .apply_if(f.updated.from, |q, v| q.filter(src.updated_at.gte(v)))
            .apply_if(f.updated.to_exclusive, |q, v| {
                q.filter(src.updated_at.lt(v))
            })
            .apply_if(self.req.cursor, |q, c| {
                q.filter(cursor_condition(
                    || self.fts.score(&self.cq),
                    src.id,
                    src.hit_type,
                    c,
                ))
            })
            .order_by(Expr::cust(r#""score""#), Order::Desc)
            .order_by(src.id, Order::Asc)
            .limit(self.req.fetch)
    }
}

/// Word-boundary excerpt used as the one-line `title` for body-only entities
/// (actions, agreements, topics, and goals without a title). Never empty:
/// falls back to the entity noun when the body is blank.
pub(crate) fn excerpt_title(body: Option<&str>, fallback: &str) -> String {
    const MAX_CHARS: usize = 80;
    let text = body.map(str::trim).unwrap_or("");
    if text.is_empty() {
        return fallback.to_string();
    }
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_CHARS).collect();
    let at_boundary = cut.rfind(char::is_whitespace).unwrap_or(cut.len());
    format!("{}…", cut[..at_boundary].trim_end())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
