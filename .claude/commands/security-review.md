Conduct a security-focused review of this Rust/Axum/SeaORM code. Check for:

**INJECTION VULNERABILITIES**
- SQL injection via raw queries or string interpolation
- Command injection in system calls
- Path traversal in file operations
- Log injection attacks

**AUTHENTICATION & AUTHORIZATION**
- Insecure session/token management
- Missing authorization checks on endpoints
- Token exposure or leakage in logs/responses
- Privilege escalation risks
- JWT validation completeness (exp, iss, aud)

**DATA EXPOSURE**
- Hardcoded secrets, API keys, or credentials
- Sensitive data in error messages
- PII in logs or debug output
- Overly permissive response data (returning full entities)

**INPUT VALIDATION**
- Missing or incomplete input validation
- Type coercion vulnerabilities
- Boundary condition handling
- Malformed data handling

**CRYPTOGRAPHY**
- Weak hashing algorithms
- Insecure random number generation
- Missing encryption for sensitive data
- Improper key management

**AXUM/WEB SPECIFIC**
- CORS misconfiguration
- Missing security headers
- Cookie security attributes (HttpOnly, Secure, SameSite)
- Rate limiting gaps
- Request size limits

**RUST SPECIFIC**
- Unsafe code blocks and their justification
- Integer overflow in security contexts
- Time-of-check to time-of-use (TOCTOU) issues
- Panic paths that could cause DoS

**DATABASE SECURITY**
- Connection string exposure
- Excessive database permissions
- Missing row-level security considerations

Provide specific recommendations for each finding with code examples.
