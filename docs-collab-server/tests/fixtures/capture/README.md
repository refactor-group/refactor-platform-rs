# Hocuspocus wire fixture harness

This script captures the **actual bytes** emitted by `@hocuspocus/provider`
v2.15.3 for each message type, plus the server-to-client frames the provider
*receives* (modeled here with the same `lib0` and `@hocuspocus/common`
primitives the provider itself uses).

The committed `../*.bin` files and `../manifest.json` are the source of truth
for the Rust protocol-conformance tests. The script is here so anyone can
re-derive them; you should not need to run it during normal development.

## Regenerate

```
cd docs-collab-server/tests/fixtures/capture
npm install
npm run capture
```

This rewrites `../*.bin` and `../manifest.json`. The `tests/fixtures/`
directory is normally read-only (chmod a-w from the test freeze); lift that
with `chmod -R u+w docs-collab-server/tests/fixtures` before regenerating,
then re-freeze with `chmod -R a-w docs-collab-server/tests/fixtures` after.

## What each fixture covers

| File                  | Direction | Outer / sub tag      |
|-----------------------|-----------|----------------------|
| `sync_step1.bin`      | C->S      | Sync(0) / Step1(0)   |
| `sync_step2.bin`      | C->S      | Sync(0) / Step2(1)   |
| `update.bin`          | C->S      | Sync(0) / Update(2)  |
| `awareness.bin`       | C->S      | Awareness(1)         |
| `awareness_query.bin` | C->S      | QueryAwareness(3)    |
| `auth_token.bin`      | C->S      | Auth(2) / Token(0)   |
| `permission_denied.bin` | S->C    | Auth(2) / Denied(1)  |
| `authenticated.bin`   | S->C      | Auth(2) / Auth'd(2)  |
| `stateless.bin`       | both      | Stateless(5)         |
| `close.bin`           | C->S      | CLOSE(7)             |
| `sync_status.bin`     | S->C      | SyncStatus(8)        |
