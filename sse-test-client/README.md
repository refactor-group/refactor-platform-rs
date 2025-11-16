# SSE Test Client

A standalone Rust binary for testing Server-Sent Events (SSE) functionality without requiring a frontend client. The tool authenticates as two users, establishes SSE connections, triggers events via API calls, and validates that events are received correctly.

## Overview

This tool validates the SSE infrastructure by:
1. Authenticating two users (typically a coach and coachee)
2. Establishing SSE connections for both users
3. Creating a test coaching relationship and session
4. Triggering events (create/update/delete actions, force logout)
5. Verifying that the correct SSE events are received by the appropriate users

## Prerequisites

- Backend server running (default: `http://localhost:4000`)
- Two valid user accounts with credentials
- Users must have permission to create coaching relationships

## Usage

### Run Individual Test Scenarios

```bash
# Test action creation
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "coach@example.com:password123" \
  --user2 "coachee@example.com:password456" \
  --scenario action-create

# Test action update
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "coach@example.com:password123" \
  --user2 "coachee@example.com:password456" \
  --scenario action-update

# Test action delete
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "coach@example.com:password123" \
  --user2 "coachee@example.com:password456" \
  --scenario action-delete

# Test force logout
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "admin@example.com:adminpass" \
  --user2 "user@example.com:userpass" \
  --scenario force-logout
```

### Run All Tests

```bash
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "coach@example.com:password123" \
  --user2 "coachee@example.com:password456" \
  --scenario all
```

### Enable Verbose Logging

```bash
cargo run -p sse-test-client -- \
  --base-url http://localhost:4000 \
  --user1 "coach@example.com:password123" \
  --user2 "coachee@example.com:password456" \
  --scenario all \
  --verbose
```

## Available Scenarios

- `action-create` - Tests SSE events for action creation
- `action-update` - Tests SSE events for action updates
- `action-delete` - Tests SSE events for action deletion
- `force-logout` - Tests SSE events for force logout
- `all` - Runs all test scenarios sequentially

## Command-Line Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `--base-url` | Yes | Base URL of the backend (e.g., `http://localhost:000`) |
| `--user1` | Yes | User 1 credentials in format `email:password` |
| `--user2` | Yes | User 2 credentials in format `email:password` |
| `--scenario` | Yes | Test scenario to run (see Available Scenarios) |
| `--verbose` or `-v` | No | Enable verbose output with debug logging |

## How It Works

### Setup Phase
1. Authenticates both users and obtains session cookies
2. Creates a coaching relationship between the two users
3. Creates a coaching session within that relationship
4. Establishes SSE connections for both users

### Test Phase
For each scenario:
1. User 1 triggers an action (e.g., creates an action)
2. The tool waits for User 2 to receive the corresponding SSE event
3. Validates that the event data matches expectations
4. Records the test result (pass/fail) and duration

### Results Phase
- Displays a summary of all test results
- Shows pass/fail status with durations
- Exits with code 0 if all tests pass, 1 if any fail

## Example Output

```
=== SETUP PHASE ===
→ Authenticating users...
✓ User 1 authenticated (ID: 123e4567-e89b-12d3-a456-426614174000)
✓ User 2 authenticated (ID: 234e5678-e89b-12d3-a456-426614174001)

→ Creating test coaching relationship and session...
✓ Coaching relationship created (ID: 345e6789-e89b-12d3-a456-426614174002)
✓ Coaching session created (ID: 456e789a-e89b-12d3-a456-426614174003)

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

## Module Structure

- `main.rs` - CLI entry point and scenario orchestration
- `auth.rs` - User authentication and session management
- `sse_client.rs` - SSE connection handling and event listening
- `api_client.rs` - API calls to create test data and trigger events
- `scenarios.rs` - Test scenario implementations
- `output.rs` - Color-coded console output formatting
