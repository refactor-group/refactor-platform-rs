# Review Risk Tiers

How much human attention a pull request needs before it merges. Most changes here are written with a coding agent and directed by one person, so "the author" below means the person who directed the work and will answer for it.

## Rules

- **The change sets the tier, not the author.** A one-line edit to a workflow file is high; a large docs rewrite is low.
- **The highest tier touched wins.** One high-tier file makes the whole PR high.
- **An automated review never counts as a person.** It reduces what a person has to read; it does not replace the person a tier requires.
- **A read covers one commit.** Commits pushed after the read need their own read, of the new diff at least.
- **The tiers say what a read must cover, not who may approve.** Branch protection separately requires one approval on every PR from anyone who cannot bypass it, whatever the tier.
- **Start strict, loosen with evidence.** Move a path down a tier only in a PR that says what evidence justifies it.

## Tiers

| Tier | What it covers | What it takes to merge |
|---|---|---|
| **Low** | Docs, comments, release notes, tests and test tooling with no production code. | CI, AI review, and security review green with every thread resolved. The author confirms that and need not read every line. |
| **Medium** | Ordinary product changes: controllers, domain logic, queries, additive migrations. | Low, plus the author reads the whole diff and the evidence (test output, review findings) and can say what the checks would not catch. |
| **High** | Any path in `.github/CODEOWNERS`, anything listed under "Always high" below. | Medium, plus a deep local review (`/code-review high` and `/security-review`) and a filled-in "How to Revert". The author reads every line, not just the diff summary. When the author is not the code owner, the code owner also reads and approves it. |

A code owner's own high-tier change needs no second person. The deep review and the revert plan carry that weight, so do not skip either.

## Always high

`.github/CODEOWNERS` lists the paths. The reasons:

- **Authentication and authorization**: `web/src/extractors/`, `web/src/protect/`, `web/src/middleware/`, `web/src/router.rs`, `web/src/lib.rs` (CORS and sessions), the login, magic-link, OAuth, password reset, password change, JWT, and webhook controllers, `domain/src/jwt/`, `domain/src/magic_link_token.rs`, `meeting-auth/`, and the collab server's `auth.rs` and `rest.rs`. A mistake here exposes private coaching data.
- **Real-time delivery**: `web/src/sse/`, `sse/`. These decide which user receives which event.
- **Migrations**: `migration/`. A revert does not undo a schema change.
- **CI, deploy, and runtime**: `.github/`, `Dockerfile`, `entrypoint.sh`, compose files, `nginx/`. These run with production secrets or decide what reaches production.
- **Agent and reviewer instructions**: `.claude/`, `AGENTS.md`, `REVIEW.md`, and this document. Weakening these weakens every later review.

Also high, wherever the code lives:

- A change to who receives an email, calendar invite, or recording bot, or to when one is sent.
- A backfill, data repair, or migration that drops or rewrites existing rows.
- An upgrade of a dependency that handles auth, crypto, sessions, or the database.

## After merge

The author watches the deploy and owns the fix. When something gets through, name the layer that should have caught it in the fix PR's description, then fix that layer: a test, a lint, a line in `REVIEW.md`, or a tier change here.
