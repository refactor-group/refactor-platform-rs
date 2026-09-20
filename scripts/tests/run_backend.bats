#!/usr/bin/env bats
#
# Regression tests for scripts/run_backend.sh.
#
# Unit tests source the script (its main guard keeps it from launching) and
# exercise the parsing and URL helpers. Launch tests copy the script into a
# throwaway repo layout with stub `cargo`, `psql`, and binaries on PATH, so
# mode selection, configuration precedence, exit-status propagation, and
# cleanup are checked without building or running anything real.

SCRIPT="$BATS_TEST_DIRNAME/../run_backend.sh"

setup() {
    # shellcheck disable=SC1090
    source "$SCRIPT"

    ROOT="$BATS_TEST_TMPDIR/repo"
    mkdir -p "$ROOT/scripts" "$ROOT/target/debug" "$ROOT/bin"
    cp "$SCRIPT" "$ROOT/scripts/run_backend.sh"
    LAUNCHER="$ROOT/scripts/run_backend.sh"
    CALLS="$ROOT/calls.log"

    # Stubs record their invocation and, for the binaries, their environment.
    cat > "$ROOT/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo "cargo $*" >> "$CALLS"
EOF
    cat > "$ROOT/bin/psql" <<'EOF'
#!/usr/bin/env bash
echo "psql $*" >> "$CALLS"
# Report the collab database as absent so the create path runs.
[[ "$*" == *pg_database* ]] && echo "" && exit 0
exit 0
EOF
    for bin in docs-collab-server refactor_platform_rs; do
        cat > "$ROOT/target/debug/$bin" <<'EOF'
#!/usr/bin/env bash
me="$(basename "$0")"
env | grep -E '^(DATABASE_URL|DATABASE_SCHEMA|JWT_SIGNING_KEY|MANAGEMENT_AUTH_KEY|BIND_ADDR|RUST_LOG)=' \
    | sed "s/^/$me env: /" >> "$CALLS"
echo "$me started"
# STUB_EXIT_<NAME>=<code> makes this binary exit with that code; else it idles.
var="STUB_EXIT_$(echo "$me" | tr 'a-z-' 'A-Z_')"
if [[ -n "${!var:-}" ]]; then exit "${!var}"; fi
sleep 30
EOF
    done
    chmod +x "$ROOT/bin/"* "$ROOT/target/debug/"*
    export CALLS
    export PATH="$ROOT/bin:$PATH"

    cat > "$ROOT/.env" <<'EOF'
POSTGRES_USER=refactor  # trailing comment
POSTGRES_PASSWORD="p@ss/w:rd#1"
POSTGRES_HOST=localhost
POSTGRES_PORT=5432
POSTGRES_SCHEMA=refactor_platform
TIPTAP_URL="https://old.example"
TIPTAP_URL="http://localhost:1234"
TIPTAP_AUTH_KEY='auth-secret'
TIPTAP_JWT_SIGNING_KEY="jwt-secret"
DATABASE_URL=postgres://x:y@localhost:5432/refactor?sslmode=require&sslrootcert=/tmp/ca.crt
EOF
}

teardown() {
    # Nothing the launcher started may outlive the test.
    pkill -TERM -f "$ROOT/target/debug/" 2>/dev/null || true
}

# --- unit: env parsing --------------------------------------------------------

@test "env_value strips trailing comments and surrounding quotes" {
    [ "$(env_value "$ROOT/.env" POSTGRES_USER)" = "refactor" ]
    [ "$(env_value "$ROOT/.env" TIPTAP_AUTH_KEY)" = "auth-secret" ]
    [ "$(env_value "$ROOT/.env" TIPTAP_JWT_SIGNING_KEY)" = "jwt-secret" ]
}

@test "env_value takes the last definition of a repeated key" {
    [ "$(env_value "$ROOT/.env" TIPTAP_URL)" = "http://localhost:1234" ]
}

@test "env_value returns empty for a missing key without failing" {
    run env_value "$ROOT/.env" NOT_PRESENT
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "env_value never executes the file (a value with & is returned verbatim)" {
    [ "$(env_value "$ROOT/.env" DATABASE_URL)" = "postgres://x:y@localhost:5432/refactor?sslmode=require&sslrootcert=/tmp/ca.crt" ]
}

@test "load_key prefers the shell environment over the file" {
    POSTGRES_USER=from-shell
    load_key "$ROOT/.env" POSTGRES_USER
    [ "$POSTGRES_USER" = "from-shell" ]
    unset POSTGRES_HOST
    load_key "$ROOT/.env" POSTGRES_HOST
    [ "$POSTGRES_HOST" = "localhost" ]
}

# --- unit: URL helpers --------------------------------------------------------

@test "urlencode percent-encodes URL-reserved characters and keeps unreserved ones" {
    [ "$(urlencode 'p@ss/w:rd#1?x%y')" = "p%40ss%2Fw%3Ard%231%3Fx%25y" ]
    [ "$(urlencode 'plain-user_1.~')" = "plain-user_1.~" ]
}

@test "collab_database_url encodes credentials but not host, port, or database" {
    [ "$(collab_database_url 'us@er' 'p@ss/w' 'db.local' 5433 refactor_collab)" = \
      "postgres://us%40er:p%40ss%2Fw@db.local:5433/refactor_collab" ]
}

@test "client_url_from_bind maps wildcard and loopback binds to localhost, keeps the port" {
    [ "$(client_url_from_bind 0.0.0.0:1234)" = "http://localhost:1234" ]
    [ "$(client_url_from_bind 127.0.0.1:4321)" = "http://localhost:4321" ]
    [ "$(client_url_from_bind 192.168.1.5:1234)" = "http://192.168.1.5:1234" ]
}

# --- launch: mode selection and configuration ---------------------------------

@test "--app-only builds only the app and never touches psql" {
    STUB_EXIT_REFACTOR_PLATFORM_RS=0 run "$LAUNCHER" --app-only
    [ "$status" -eq 0 ]
    grep -q '^cargo build -p refactor_platform_rs$' "$CALLS"
    ! grep -q 'docs-collab-server' "$CALLS"
    ! grep -q '^psql' "$CALLS"
}

@test "--collab-only creates the local database and passes derived config to the binary" {
    STUB_EXIT_DOCS_COLLAB_SERVER=0 run "$LAUNCHER" --collab-only
    [ "$status" -eq 0 ]
    grep -q '^cargo build -p docs-collab-server$' "$CALLS"
    grep -q 'psql .*CREATE DATABASE refactor_collab OWNER refactor' "$CALLS"
    grep -q 'env: DATABASE_URL=postgres://refactor:p%40ss%2Fw%3Ard%231@localhost:5432/refactor_collab$' "$CALLS"
    grep -q 'env: DATABASE_SCHEMA=refactor_platform$' "$CALLS"
    grep -q 'env: JWT_SIGNING_KEY=jwt-secret$' "$CALLS"
    grep -q 'env: MANAGEMENT_AUTH_KEY=auth-secret$' "$CALLS"
    grep -q 'env: BIND_ADDR=127.0.0.1:1234$' "$CALLS"
    ! grep -q 'refactor_platform_rs' "$CALLS"
}

@test "default mode builds both packages and starts both binaries" {
    STUB_EXIT_REFACTOR_PLATFORM_RS=0 run "$LAUNCHER"
    grep -q '^cargo build -p refactor_platform_rs -p docs-collab-server$' "$CALLS"
    [[ "$output" == *"[collab] docs-collab-server started"* ]]
    [[ "$output" == *"[app] refactor_platform_rs started"* ]]
}

@test "COLLAB_DATABASE_URL override skips database creation and POSTGRES_* requirements" {
    sed -i.bak '/^POSTGRES_/d' "$ROOT/.env"
    COLLAB_DATABASE_URL="postgres://u:p@elsewhere:6000/other" STUB_EXIT_DOCS_COLLAB_SERVER=0 \
        run "$LAUNCHER" --collab-only
    [ "$status" -eq 0 ]
    ! grep -q '^psql' "$CALLS"
    grep -q 'env: DATABASE_URL=postgres://u:p@elsewhere:6000/other$' "$CALLS"
}

@test "COLLAB_DATABASE_URL is also honoured from .env" {
    echo 'COLLAB_DATABASE_URL=postgres://u:p@fromfile:6000/other' >> "$ROOT/.env"
    STUB_EXIT_DOCS_COLLAB_SERVER=0 run "$LAUNCHER" --collab-only
    [ "$status" -eq 0 ]
    ! grep -q '^psql' "$CALLS"
    grep -q 'env: DATABASE_URL=postgres://u:p@fromfile:6000/other$' "$CALLS"
}

@test "missing collab secrets fail before anything is built" {
    sed -i.bak '/^TIPTAP_JWT_SIGNING_KEY/d' "$ROOT/.env"
    run "$LAUNCHER" --collab-only
    [ "$status" -ne 0 ]
    [[ "$output" == *"TIPTAP_JWT_SIGNING_KEY is not set in .env"* ]]
    [ ! -f "$CALLS" ] || ! grep -q '^cargo' "$CALLS"
}

@test "warns when TIPTAP_URL does not match the collab URL, and stays quiet when it does" {
    sed -i.bak 's#^TIPTAP_URL="http://localhost:1234"#TIPTAP_URL="https://v91x.collab.tiptap.cloud"#' "$ROOT/.env"
    STUB_EXIT_REFACTOR_PLATFORM_RS=0 run "$LAUNCHER"
    [[ "$output" == *"warning: TIPTAP_URL in .env is 'https://v91x.collab.tiptap.cloud', not 'http://localhost:1234'"* ]]

    rm -f "$CALLS"
    sed -i.bak 's#^TIPTAP_URL=.*#TIPTAP_URL="http://localhost:1234"#' "$ROOT/.env"
    STUB_EXIT_REFACTOR_PLATFORM_RS=0 run "$LAUNCHER"
    [[ "$output" != *"warning: TIPTAP_URL"* ]]
}

@test "a custom bind address changes the expected TIPTAP_URL and the binary's BIND_ADDR" {
    COLLAB_BIND_ADDR=0.0.0.0:4321 STUB_EXIT_REFACTOR_PLATFORM_RS=0 run "$LAUNCHER"
    [[ "$output" == *"not 'http://localhost:4321'"* ]]
    grep -q 'env: BIND_ADDR=0.0.0.0:4321$' "$CALLS"
}

# --- unit: signal handling ----------------------------------------------------

@test "on_signal exits 128 plus the signal number even before any child was started" {
    run bash -c "source '$SCRIPT'; on_signal TERM"
    [ "$status" -eq 143 ]
    run bash -c "source '$SCRIPT'; on_signal INT"
    [ "$status" -eq 130 ]
}

@test "stop_children returns promptly even for children it never recorded" {
    # Reproduce a signal landing mid-launch: a reader blocked on a FIFO with no
    # writer, and a running binary whose PID was never added to pids. The
    # worker carries its own watchdog so a hang can never outlive the test.
    bash -c "
        source '$SCRIPT'
        fifo_dir=\$(mktemp -d); mkfifo \"\$fifo_dir/x\"
        ( cat < \"\$fifo_dir/x\" ) &
        ( exec sleep 30 ) &
        ( sleep 5; pkill -9 -P \$\$; kill -9 \$\$ ) &
        stop_children
        echo cleaned
    " > "$ROOT/stop.out" 2>&1 3>&-
    grep -q '^cleaned$' "$ROOT/stop.out"
}

# --- launch: supervision ------------------------------------------------------

@test "a binary that exits non-zero fails the launcher with that status" {
    STUB_EXIT_DOCS_COLLAB_SERVER=3 run "$LAUNCHER" --collab-only
    [ "$status" -eq 3 ]
}

@test "when one binary dies the other is stopped and the launcher returns promptly" {
    STUB_EXIT_REFACTOR_PLATFORM_RS=7 run "$LAUNCHER"
    [ "$status" -eq 7 ]
    sleep 1
    ! pgrep -f "$ROOT/target/debug/docs-collab-server" >/dev/null
}

@test "SIGTERM stops both binaries and exits 143, not a wait error" {
    "$LAUNCHER" > "$ROOT/launch.out" 2>&1 3>&- &
    launcher=$!
    for _ in $(seq 1 50); do
        grep -q 'refactor_platform_rs started' "$ROOT/launch.out" 2>/dev/null && break
        sleep 0.2
    done
    kill -TERM "$launcher"
    wait "$launcher" && status=0 || status=$?
    [ "$status" -eq 143 ]
    sleep 1
    ! pgrep -f "$ROOT/target/debug/" >/dev/null
}

@test "unknown flags are rejected with usage" {
    run "$LAUNCHER" --bogus
    [ "$status" -eq 2 ]
    [[ "$output" == *"unknown argument '--bogus'"* ]]
}
