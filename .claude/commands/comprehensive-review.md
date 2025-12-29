Please perform a comprehensive code review of this Rust/Axum/SeaORM pull request.

## Review Areas

Apply the criteria from each of the following specialized reviews:

1. **Security** → See `.claude/commands/security-review.md`
2. **Performance** → See `.claude/commands/performance-review.md`
3. **API Design** → See `.claude/commands/api-review.md`
4. **Architecture** → See `.claude/commands/architecture-review.md`
5. **Rust Idioms** → See `.claude/commands/rust-review.md`
6. **Bug Detection** → See `.claude/commands/bug-hunt.md`
7. **Database Migrations** → See `.claude/commands/migration-review.md` (if applicable)

## Priority Levels

Categorize findings by severity:
- **Critical**: Security vulnerabilities, data loss risks, panics in production paths
- **High**: Performance bottlenecks, incorrect behavior, missing error handling
- **Medium**: Code quality issues, maintainability concerns, missing tests
- **Low**: Style inconsistencies, minor optimizations, documentation gaps

## Output Format

For each issue found:
1. Specify the severity (Critical/High/Medium/Low)
2. Reference which review area it falls under
3. Explain the issue clearly
4. Provide specific fix recommendations
5. Include code examples where helpful

Only comment on significant issues. Be concise but thorough.
