# Review Risk Tiers

How much human attention a pull request needs before it merges. Most changes here are written with a coding agent and directed by one person, so "the author" below means the person who directed the work and will answer for it.

## Rules

- **The change sets the tier, not the author.** A one-line edit to a workflow file is high; a large docs rewrite is low.
- **The highest tier touched wins.** One high-tier file makes the whole PR high.
- **An automated review never counts as a person.** It reduces what a person has to read; it does not replace the person a tier requires.
- **A read covers one commit.** Commits pushed after the read need their own read, of the new diff at least.
- **Start strict, loosen with evidence.** Move a path down a tier only in a PR that says what evidence justifies it.

## Tiers

| Tier | What it covers | What it takes to merge |
|---|---|---|
| **Low** | Docs, comments, release notes, tests and test tooling with no production code. | CI, AI review, and security review green with every thread resolved. The author confirms that and need not read every line. |
| **Medium** | Ordinary product changes: controllers, domain logic, queries, additive migrations. | Low, plus the author reads the whole diff and the evidence (test output, review findings) and can say what the checks would not catch. |
| **High** | Any path in `.github/CODEOWNERS`, anything listed under "Always high" below. | Medium, plus a deep local review (`/code-review high` and `/security-review`), a filled-in "How to Revert", and a second person: the code owner, or another maintainer when the author is the code owner. |

When a high-tier change has no second person available, say so in the PR's Risk section and attach a manual test result. That is an exception to record, not a routine path.

## Always high

`.github/CODEOWNERS` lists the paths. The reasons:

- **Authentication and authorization**: `web/src/extractors/`, `web/src/protect/`, `web/src/middleware/`, `web/src/router.rs`, `domain/src/jwt/`, OAuth and password reset code, `meeting-auth/`. A mistake here exposes private coaching data.
- **Migrations**: `migration/`. A revert does not undo a schema change.
- **CI, deploy, and runtime**: `.github/`, `Dockerfile`, `entrypoint.sh`, compose files, `nginx/`. These run with production secrets or decide what reaches production.
- **Agent and reviewer instructions**: `.claude/`, `AGENTS.md`, `REVIEW.md`. Weakening these weakens every later review.

Also high, wherever the code lives:

- A change to who receives an email, calendar invite, or recording bot, or to when one is sent.
- A backfill, data repair, or migration that drops or rewrites existing rows.
- An upgrade of a dependency that handles auth, crypto, sessions, or the database.

## After merge

The author watches the deploy and owns the fix. When something gets through, name the layer that should have caught it in the fix PR's description, then fix that layer: a test, a lint, a line in `REVIEW.md`, or a tier change here.
