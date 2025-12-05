Perform a Rust-focused review of this code for idiomatic patterns and language best practices:

**OWNERSHIP & BORROWING**
- Unnecessary clones that could be borrows
- Missing borrows that cause moves
- Lifetime elision opportunities
- Proper use of `&self` vs `self` vs `&mut self`

**LIFETIME ANNOTATIONS**
- Correct lifetime bounds
- Lifetime elision where possible
- Static lifetime usage appropriateness
- Higher-ranked trait bounds (HRTB) when needed

**ERROR HANDLING**
- 📚 **Use Context7 MCP to check latest thiserror/anyhow patterns**
- `Result<T, E>` vs `Option<T>` appropriateness
- Error type design (thiserror, anyhow patterns)
- `?` operator usage and propagation
- Custom error context with `.context()` or `.map_err()`
- Avoiding `.unwrap()` and `.expect()` in library code

**TYPE SYSTEM**
- Newtype patterns for type safety
- Generic type parameter bounds
- Associated types vs generic parameters
- Phantom types for compile-time guarantees
- Type aliases for readability

**TRAIT DESIGN**
- Trait coherence and orphan rules
- Default implementations
- Supertraits and trait bounds
- Object safety considerations
- Blanket implementations

**PATTERN MATCHING**
- Exhaustive matching
- Guard clauses
- Destructuring patterns
- `if let` vs `match` appropriateness
- `matches!` macro usage

**ITERATORS & CLOSURES**
- Iterator adapter chains
- `collect()` type inference
- Closure capture modes (move, borrow)
- `impl Fn` vs `dyn Fn` vs generics

**SERDE PATTERNS**
- 📚 **Use Context7 MCP to check latest serde API usage**
- Derive macro attributes
- Custom serialization/deserialization
- `skip_serializing_if` and other field attributes
- Rename strategies and case conventions

**ASYNC PATTERNS**
- 📚 **Use Context7 MCP to check latest tokio API usage**
- Future combinators and async traits
- Spawn vs direct await decisions
- Cancellation safety
- Async closure patterns

**MACROS**
- Macro hygiene
- Declarative vs procedural appropriateness
- Derive macro usage
- Avoiding macro overuse

**UNSAFE CODE**
- Justification for unsafe blocks
- Invariant documentation
- Minimizing unsafe scope
- Safe abstractions over unsafe

**CLIPPY & STYLE**
- Clippy lint compliance
- Naming conventions (snake_case, PascalCase)
- Module organization
- Documentation completeness

Suggest idiomatic improvements with code examples.
