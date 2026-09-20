#!/usr/bin/env bash
#
# Build and run the local backend: the app server (refactor_platform_rs) and
# the collaborative-notes server (docs-collab-server), together by default.
#
#   scripts/run_backend.sh              # both
#   scripts/run_backend.sh --app-only   # app server only
#   scripts/run_backend.sh --collab-only
#
# Both processes derive their configuration from the repo's .env. The app loads
# it itself; the collab server reads only process env, so this script passes it
# the shared secrets and a connection string built from the POSTGRES_* values.
# The two secrets therefore match by construction, which is the one thing the
# hand-run setup kept getting wrong.
#
# Ctrl-C stops both.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

COLLAB_DB_NAME="${COLLAB_DB_NAME:-refactor_collab}"
COLLAB_BIND_ADDR="${COLLAB_BIND_ADDR:-127.0.0.1:1234}"
LOCAL_COLLAB_URL="http://${COLLAB_BIND_ADDR/127.0.0.1/localhost}"

run_app=true
run_collab=true

usage() {
    sed -n '2,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
    case "$arg" in
        --app-only) run_collab=false ;;
        --collab-only) run_app=false ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown argument '$arg'" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ ! -f .env ]]; then
    echo "error: no .env in $REPO_ROOT; see docs/setup.md" >&2
    exit 1
fi

# Read one KEY from .env without executing the file as shell (values may hold
# `&` or `$`). Last definition wins, trailing comments and surrounding quotes
# are stripped, matching how the app's dotenv loader reads it.
env_value() {
    local line
    line="$(grep -E "^$1=" .env | tail -1)" || return 0
    line="${line#*=}"
    line="$(printf '%s' "$line" | sed -E 's/[[:space:]]+#.*$//')"
    line="${line%\"}"; line="${line#\"}"
    line="${line%\'}"; line="${line#\'}"
    printf '%s' "$line"
}

for key in TIPTAP_JWT_SIGNING_KEY TIPTAP_AUTH_KEY TIPTAP_URL \
           POSTGRES_USER POSTGRES_PASSWORD POSTGRES_HOST POSTGRES_PORT POSTGRES_SCHEMA; do
    printf -v "$key" '%s' "$(env_value "$key")"
done

require() {
    local name="$1"
    if [[ -z "${!name:-}" ]]; then
        echo "error: $name is not set in .env" >&2
        exit 1
    fi
}

# The collab server refuses to start without these, but failing here gives a
# message that names the .env key rather than the collab-side flag.
if $run_collab; then
    require TIPTAP_JWT_SIGNING_KEY
    require TIPTAP_AUTH_KEY
    require POSTGRES_USER
    require POSTGRES_PASSWORD
    require POSTGRES_HOST
    require POSTGRES_PORT
fi

# A backend pointed at TipTap Cloud mints tokens the local collab server never
# sees. The editor then opens local-only with no error, so catch it up front.
if $run_app && $run_collab && [[ "${TIPTAP_URL:-}" != "$LOCAL_COLLAB_URL" ]]; then
    echo "warning: TIPTAP_URL in .env is '${TIPTAP_URL:-<unset>}', not '$LOCAL_COLLAB_URL'." >&2
    echo "         The app will not talk to the local collab server. Set it to '$LOCAL_COLLAB_URL'." >&2
fi

# Separate database, mirroring production and PR preview. Created as the
# postgres superuser like scripts/rebuild_db.sh does, since the app role is
# not granted CREATEDB.
ensure_collab_db() {
    if ! command -v psql >/dev/null; then
        echo "warning: psql not found; assuming database '$COLLAB_DB_NAME' already exists" >&2
        return
    fi
    local exists
    exists="$(psql -U postgres -h "$POSTGRES_HOST" -p "$POSTGRES_PORT" -tAc \
        "SELECT 1 FROM pg_database WHERE datname='$COLLAB_DB_NAME'" 2>/dev/null || true)"
    if [[ "$exists" != "1" ]]; then
        echo "==> creating database '$COLLAB_DB_NAME' owned by '$POSTGRES_USER'"
        psql -U postgres -h "$POSTGRES_HOST" -p "$POSTGRES_PORT" -c \
            "CREATE DATABASE $COLLAB_DB_NAME OWNER $POSTGRES_USER" \
            || { echo "error: could not create database '$COLLAB_DB_NAME'" >&2; exit 1; }
    fi
}

# Build first so compile errors surface cleanly instead of interleaved with
# the other process's logs.
packages=()
$run_app && packages+=(-p refactor_platform_rs)
$run_collab && packages+=(-p docs-collab-server)
echo "==> cargo build ${packages[*]}"
cargo build "${packages[@]}"

pids=()

# Stop the tracked children (each is a subshell whose direct children are the
# binary and its log-prefix loop) when this script exits, whether by Ctrl-C or
# because one binary died. Targets PIDs rather than the process group so a
# caller that shares the group (make, another script, CI) is never signalled.
stop_children() {
    for pid in "${pids[@]}"; do
        pkill -TERM -P "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap 'trap - INT TERM EXIT; stop_children; exit' INT TERM EXIT

# Run a command in the background with each output line tagged. Works on the
# bash 3.2 that macOS ships, which lacks `wait -n`.
prefixed() {
    local tag="$1"; shift
    ( "$@" 2>&1 | while IFS= read -r line; do printf '[%s] %s\n' "$tag" "$line"; done ) &
    pids+=($!)
}

if $run_collab; then
    ensure_collab_db
    prefixed collab env \
        DATABASE_URL="${COLLAB_DATABASE_URL:-postgres://$POSTGRES_USER:$POSTGRES_PASSWORD@$POSTGRES_HOST:$POSTGRES_PORT/$COLLAB_DB_NAME}" \
        DATABASE_SCHEMA="${POSTGRES_SCHEMA:-refactor_platform}" \
        JWT_SIGNING_KEY="$TIPTAP_JWT_SIGNING_KEY" \
        MANAGEMENT_AUTH_KEY="$TIPTAP_AUTH_KEY" \
        BIND_ADDR="$COLLAB_BIND_ADDR" \
        RUST_LOG="${COLLAB_RUST_LOG:-info,docs_collab_server=debug}" \
        target/debug/docs-collab-server
fi

if $run_app; then
    prefixed app env RUST_LOG="${RUST_LOG:-info}" target/debug/refactor_platform_rs
fi

# Return when the first child exits; the EXIT trap then stops the rest.
while :; do
    for pid in "${pids[@]}"; do
        kill -0 "$pid" 2>/dev/null || exit 0
    done
    sleep 1
done
