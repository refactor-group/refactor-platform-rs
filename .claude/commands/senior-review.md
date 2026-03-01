You are reviewing this PR as a senior Rust engineer who values readability,
test confidence, and minimal duplication. Be direct and specific — no generic
advice. If you make a claim, support it with evidence from the diff.

First, get the full diff for this PR using `git diff main...HEAD`.
Reference the project's coding standards in `.claude/coding-standards.md`
throughout your review.

Scrutinize the code in the following ways. Think hard about these, and rank
your findings by severity — things that could cause bugs or regressions first,
stylistic concerns last.

## 1. Entity & Domain Type Redundancy

Are any new types (structs, enums, error variants, newtypes) structurally or
semantically redundant with types already defined in the codebase?

Check against the canonical type locations:
- **`entity/src/`** — SeaORM models: `actions.rs`, `coaching_sessions.rs`,
  `coaching_relationships.rs`, `users.rs`, `organizations.rs`, `notes.rs`,
  `agreements.rs`, `goals.rs`, `status.rs`, `roles.rs`, etc.
- **`entity_api/src/`** — Query/mutation types and error types
- **`domain/src/`** — Domain models, business logic types, and error hierarchy

If a new type duplicates an existing one, show the existing type that should be
reused and explain what would need to change. Pay special attention to:
- New structs that mirror existing SeaORM `Model` types
- New enums that duplicate `entity/src/status.rs` or `entity/src/roles.rs` variants
- Error variants that already exist in the `entity_api → domain → web` chain

## 2. Trait, Module & Interface Proliferation

When the diff introduces a new trait, module, or public API surface, determine
whether an existing equivalent could be extended instead of creating something
new. Unnecessary proliferation fragments the codebase and makes it harder to
discover what's already available.

Specifically check:
- **New traits**: Is there an existing trait with overlapping semantics that
  could gain a method or associated type instead of a whole new trait?
- **New modules**: Does the new module's responsibility overlap with an
  existing module in the same layer? Could it be a submodule or extension
  of what's already there?
- **New public functions/methods**: Is a similar function already exposed at
  the same layer, differing only in filter criteria, sort order, or minor
  parameters? Could the existing function be generalized (e.g., accept an
  optional parameter) instead?
- **New error types or variants**: Could an existing error kind cover the
  same failure mode with a more descriptive message, rather than introducing
  a new variant?

For each case, show both the new code and the existing code that could be
extended, and explain what the extension would look like. Acknowledge when
a new abstraction is genuinely justified (different concern, different
lifecycle, would make the existing interface incoherent).

## 3. Layered Architecture Compliance

This is the most important architectural constraint. Verify that every change
respects the strict layer hierarchy:

```
entity/  →  entity_api/  →  domain/  →  web/
(models)    (CRUD/queries)  (business    (HTTP handlers,
                             logic)       routing)
```

Flag violations in order of severity:
1. **`web/` importing from `entity_api/`** — This is always wrong. The web
   layer must only depend on `domain/` types.
2. **`domain/` re-exporting `entity_api` functions directly** — Must be wrapped
   in a thin domain function so callers receive `domain::Error`, not
   `entity_api::Error`.
3. **Business logic in `web/` handlers** — Logic beyond request parsing and
   response formatting belongs in `domain/` or `entity_api/`.
4. **HTTP concerns leaking into `domain/`** — Status codes, headers, or
   Axum extractors referenced in domain code.
5. **Error propagation skipping layers** — Errors must flow through
   `entity_api::Error → domain::Error → web::Error` without shortcuts.

For each violation, show the offending import or function call and where it
should live instead.

## 4. Test Coverage of Critical Paths

Do the tests cover the critical paths for this feature? Identify specific
scenarios that are untested and would make you nervous about refactoring later.

Pay special attention to:
- Error/failure paths (DB errors, not-found, unauthorized)
- Authorization boundary conditions (protect middleware behavior)
- Edge cases in database operations (empty results, constraint violations)
- Any new `domain/` business logic that lacks unit tests

## 5. Test Redundancy

Are any of the tests redundant with each other? Prove this by showing what
each test actually asserts and where the overlap is.

## 6. Function Complexity

For any new function or method over ~20 lines, show a simplified version that
reads more like a story, or explain why the current complexity is justified.
Consider whether iterator chains, early returns, `let-else`, or extracting
a helper would reduce nesting without hurting clarity.

## 7. Coupling & Interface Generality

Is anything too tightly coupled or overly specific in its interface?
- Does a handler reach deep into entity internals instead of calling through
  the domain layer?
- Are concrete types used where a trait or generic would allow reuse?
- Are function signatures accepting more than they need (full `Model` when
  only an `Id` is required)?

Show what a more general version would look like and explain the tradeoff.

## 8. Surprise Detection

Flag anything else that concerns you about this diff that wasn't asked about
above. This includes: subtle bugs, missing error handling, security concerns
(injection, auth bypass, data exposure), performance traps (N+1 queries,
unnecessary clones, blocking in async), or patterns that diverge from the
rest of the codebase.

## Output Format

For each finding:
1. **Severity**: Critical / High / Medium / Low
2. **Category**: Which section above (1-8)
3. **Evidence**: Quote the specific code from the diff
4. **Recommendation**: Concrete fix with code example if applicable

For deeper dives into specific areas, follow up with:
- `/security-review` for security-focused analysis
- `/performance-review` for performance profiling
- `/migration-review` for database migration scrutiny
- `/rust-review` for idiomatic Rust patterns
