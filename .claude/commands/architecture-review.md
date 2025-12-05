Review the Rust code architecture and design patterns:

**MODULE DESIGN**
- Single responsibility principle adherence
- Module size and complexity
- Proper visibility (pub, pub(crate), private)
- Clear module boundaries and dependencies

**LAYER SEPARATION**
- entity/ - SeaORM entities only
- entity_api/ - CRUD and entity queries
- domain/ - Business logic and validation
- service/ - Orchestration and complex operations
- web/ - HTTP concerns only

**TRAIT USAGE**
- Trait abstraction appropriateness
- Trait object vs generics decisions
- Default implementations where sensible
- Trait bounds clarity

**ERROR DESIGN**
- Custom error type hierarchy
- Error conversion implementations (From, Into)
- Error propagation patterns
- Context preservation in errors

**STATE MANAGEMENT**
- AppState design in Axum
- Shared state via Arc where needed
- Database connection handling
- Configuration management

**DEPENDENCY PATTERNS**
- Constructor injection vs State extractor
- Testability of components
- Mock-friendly interfaces
- Circular dependency prevention

**TESTING CONSIDERATIONS**
- Unit testability of business logic
- Integration test patterns
- Database test isolation
- Mock/stub patterns in Rust

Identify refactoring opportunities and architectural improvements.
