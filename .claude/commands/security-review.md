---
description: Security review of a pull request or branch, the same review the CI Security Review job runs
argument-hint: "<PR number | branch> (default: the current branch)"
model: claude-opus-5-5
---

Run the security review of `$ARGUMENTS` that the CI Security Review job
(`.github/workflows/claude-review.yml`) runs on a pull request, following the same procedure.

**Set up, like CI does.**
- The base is `origin/main`. Run `git fetch origin` first.
- Read `.github/security-review-procedure.md`, `.github/security-review-instructions.txt` and
  `AGENTS.md` from the base with `git show origin/main:<path>`, never from the code under
  review, so the review rules can't come from the change being reviewed. If the procedure
  is not on `origin/main` yet, read it from the working tree.
- Check the code under review out into a fresh detached worktree in your scratch directory
  (outside the repository); it is the `pr-head` the procedure refers to. Never switch the
  branch of an existing checkout.
  - A pull request number: `git fetch origin pull/<number>/head`, then
    `git worktree add --detach <scratch>/pr-head FETCH_HEAD`. A merged or closed pull request
    is reviewed the same way.
  - A branch, or no argument (the current branch): `git worktree add --detach
    <scratch>/pr-head <branch>`, and review `git -C <scratch>/pr-head diff
    origin/main...HEAD`. Use its pull request's context too if `gh pr view <branch>` finds
    one.

**Review.** Follow `.github/security-review-procedure.md` step by step, with `pr-head` meaning
that worktree. Do not edit files, do not run project code, and post nothing to GitHub.

**Report in this conversation** what CI would post: each confirmed finding as its inline
comment would read, with its file and line relative to the repository root, then the summary
line. Then remove the worktree with `git worktree remove <scratch>/pr-head`.
