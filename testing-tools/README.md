# Testing Tools

A collection of testing utilities and tools for the Refactor Platform.

## SSE Test Client

A standalone Rust binary for testing Server-Sent Events (SSE) functionality without requiring a frontend client. The tool authenticates as two users, establishes SSE connections, triggers events via API calls, and validates that events are received correctly.

### Overview

This tool validates the SSE infrastructure by:
1. Authenticating two users (typically a coach and coachee)
2. Establishing SSE connections for both users
3. Using existing coaching relationships/sessions or creating them if needed (for action tests)
4. Triggering events (create/update/delete actions, force logout)
5. Verifying that the correct SSE events are received by the appropriate users

### Prerequisites

- Backend server running (default: `http://localhost:4000`)
- Two valid user accounts with credentials (seeded users recommended)
- **For action tests**: An existing coaching relationship between the users (will be created if it doesn't exist)
- **For connection test**: No special permissions or relationships required

### Usage

### Run Individual Test Scenarios

```bash
# Test basic SSE connection (no admin permissions required)
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario connection-test

# Test action creation (requires admin permissions)
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario action-create

# Test action update (requires admin permissions)
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario action-update

# Test action delete (requires admin permissions)
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario action-delete

# Test force logout (requires admin permissions - NOT YET IMPLEMENTED)
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario force-logout-test
```

### Run All Tests

```bash
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario all
```

### Enable Verbose Logging

```bash
cargo run -p testing-tools --bin sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "james.hodapp@gmail.com:password" \
  --user2 "calebbourg2@gmail.com:password" \
  --scenario all \
  --verbose
```

### Available Scenarios

- `connection-test` - Tests basic SSE connectivity without creating any data
- `action-create` - Tests SSE events for action creation (uses existing coaching relationship or creates one)
- `action-update` - Tests SSE events for action updates (uses existing coaching relationship or creates one)
- `action-delete` - Tests SSE events for action deletion (uses existing coaching relationship or creates one)
- `force-logout-test` - Tests SSE events for force logout (NOT YET IMPLEMENTED)
- `all` - Runs all test scenarios sequentially

### Command-Line Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `--base-url` | Yes | Base URL of the backend (e.g., `http://localhost:000`) |
| `--user1` | Yes | User 1 credentials in format `email:password` |
| `--user2` | Yes | User 2 credentials in format `email:password` |
| `--scenario` | Yes | Test scenario to run (see Available Scenarios) |
| `--verbose` or `-v` | No | Enable verbose output with debug logging |

### How It Works

### Setup Phase
1. Authenticates both users and obtains session cookies
2. For action tests: Finds existing coaching relationship/session or creates new ones if needed
3. For connection test: Skips coaching data setup
4. Establishes SSE connections for both users

### Test Phase
For each scenario:
1. **Connection Test**: Verifies SSE connections are established and remain stable
2. **Action Tests**: User 1 triggers an action (e.g., creates an action), the tool waits for User 2 to receive the corresponding SSE event, and validates event data
3. Records the test result (pass/fail) and duration

### Results Phase
- Displays a summary of all test results
- Shows pass/fail status with durations
- Exits with code 0 if all tests pass, 1 if any fail

### Example Output

### Connection Test (No Admin Required)
```
=== SETUP PHASE ===
→ Authenticating users...
✓ User 1 authenticated (ID: 123e4567-e89b-12d3-a456-426614174000)
✓ User 2 authenticated (ID: 234e5678-e89b-12d3-a456-426614174001)

→ Skipping test environment setup (not needed for this test)

→ Establishing SSE connections...
✓ User 1 SSE connection established
✓ User 2 SSE connection established

=== TEST PHASE ===

=== TEST: Connection Test ===
Testing basic SSE connectivity without creating any data
✓ User 1 (123e4567-e89b-12d3-a456-426614174000) SSE connection: established
✓ User 2 (234e5678-e89b-12d3-a456-426614174001) SSE connection: established
→ Waiting 2 seconds to verify connections stay alive...
✓ Connections remain stable
✓ SSE infrastructure is working correctly

=== RESULTS ===
=== TEST SUMMARY ===
[PASS] connection_test (2.002086s)
      SSE connections established and maintained successfully

Results: 1 passed, 0 failed

All tests passed! ✓
```

### Action Test
```
=== SETUP PHASE ===
→ Authenticating users...
✓ User 1 authenticated (ID: 123e4567-e89b-12d3-a456-426614174000)
✓ User 2 authenticated (ID: 234e5678-e89b-12d3-a456-426614174001)

→ Setting up test coaching relationship and session...
✓ Using coaching relationship (ID: 345e6789-e89b-12d3-a456-426614174002)
✓ Using coaching session (ID: 456e789a-e89b-12d3-a456-426614174003)

→ Establishing SSE connections...
✓ User 1 SSE connection established
✓ User 2 SSE connection established

=== TEST PHASE ===

=== TEST: Action Create ===
→ User 1 creating action...
✓ Action created (ID: 567e89ab-e89b-12d3-a456-426614174004)
→ Waiting for User 2 to receive action_created event...

[User 2 (Coachee)] action_created event received
   {
     "type": "action_created",
     "data": {
       "coaching_session_id": "456e789a-e89b-12d3-a456-426614174003",
       "action": { ... }
     }
   }
✓ Event data verified correctly

=== RESULTS ===
=== TEST SUMMARY ===
[PASS] action_create (234ms)

Results: 1 passed, 0 failed

All tests passed! ✓
```

### Module Structure

- `src/bin/sse-test-client.rs` - CLI entry point and scenario orchestration
- `src/auth.rs` - User authentication and session management
- `src/sse_client.rs` - SSE connection handling and event listening
- `src/api_client.rs` - API calls to create test data and trigger events
- `src/scenarios.rs` - Test scenario implementations
- `src/output.rs` - Color-coded console output formatting
- `src/lib.rs` - Library exports for testing-tools crate
