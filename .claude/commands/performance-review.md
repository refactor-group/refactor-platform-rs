Review this Rust/Axum/SeaORM code for performance and efficiency:

**MEMORY & ALLOCATIONS**
- Unnecessary clones and allocations
- String vs &str usage optimization
- Vec pre-allocation with `with_capacity`
- Avoiding temporary allocations in hot paths
- Box vs inline storage decisions

**DATABASE PERFORMANCE**
- N+1 query detection and prevention
- Missing database indexes
- Inefficient query patterns
- Connection pool sizing and usage
- Transaction scope optimization
- Batch operations vs individual queries

**ASYNC RUNTIME**
- Blocking operations in async context
- Task spawning appropriateness
- Future combinators efficiency
- Channel buffer sizing
- Proper use of `tokio::spawn` vs direct await

**SERIALIZATION**
- Serde optimization (skip_serializing_if, etc.)
- Response payload size
- Unnecessary serialization/deserialization cycles
- Zero-copy patterns where applicable

**CACHING OPPORTUNITIES**
- Repeated expensive computations
- Database query caching potential
- Static data that could be cached
- Cache invalidation considerations

**API EFFICIENCY**
- Pagination implementation
- Partial response support (field selection)
- Compression (gzip/brotli)
- Connection keep-alive settings

**RUST-SPECIFIC OPTIMIZATIONS**
- Iterator vs loop performance
- Cow<str> for conditional ownership
- SmallVec for small collections
- Lazy initialization patterns

Rate each area and suggest specific improvements with benchmarking recommendations.
