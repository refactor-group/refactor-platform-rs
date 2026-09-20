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
# The two secrets therefore match by construction.
#
# Optional overrides (shell env or .env):
#   COLLAB_DATABASE_URL  use this database instead of a local refactor_collab
#   COLLAB_BIND_ADDR     listen address for the collab server (127.0.0.1:1234)
#   COLLAB_URL           app-facing collab URL when it differs from the bind
#                        address (defaults to http://localhost:<port>)
#
# Ctrl-C stops both. Exits with the status of the first binary that dies.

set -euo pipefail

usage() {
    sed -n '2,22p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# Read one KEY from an env file without executing it as shell (values may hold
# `&` or `$`). Last definition wins; trailing comments and surrounding quotes
# are stripped, matching how the app's dotenv loader reads it.
env_value() {
    local file="$1" key="$2" line
    line="$(grep -E "^${key}=" "$file" | tail -1)" || return 0
    line="${line#*=}"
    line="$(printf '%s' "$line" | sed -E 's/[[:space:]]+#.*$//')"
    line="${line%\"}"; line="${line#\"}"
    line="${line%\'}"; line="${line#\'}"
    printf '%s' "$line"
}

# Populate a variable from the shell environment first, then the env file.
load_key() {
    local file="$1" key="$2"
    if [[ -z "${!key:-}" ]]; then
        printf -v "$key" '%s' "$(env_value "$file" "$key")"
    fi
}

# Percent-encode a URL component so credentials with reserved characters
# (`@ / ? # % :`) survive inside a connection string.
urlencode() {
    local s="$1" i c out=""
    for (( i = 0; i < ${#s}; i++ )); do
        c="${s:i:1}"
        case "$c" in
            [A-Za-z0-9._~-]) out+="$c" ;;
            *) out+="$(printf '%%%02X' "'$c")" ;;
        esac
    done
    printf '%s' "$out"
}

collab_database_url() {
    local user="$1" pass="$2" host="$3" port="$4" db="$5"
    printf 'postgres://%s:%s@%s:%s/%s' "$(urlencode "$user")" "$(urlencode "$pass")" "$host" "$port" "$db"
}

# The URL the app should use to reach a server bound at `bind`. A wildcard or
# loopback bind is reachable as localhost; anything else is used verbatim.
client_url_from_bind() {
    local bind="$1" host="${1%:*}" port="${1##*:}"
    case "$host" in
        0.0.0.0|127.0.0.1|localhost|"") host=localhost ;;
    esac
    printf 'http://%s:%s' "$host" "$port"
}

require() {
    local name="$1"
    if [[ -z "${!name:-}" ]]; then
        echo "error: $name is not set in .env" >&2
        return 1
    fi
}

# Separate database, mirroring production and PR preview. Created as the
# postgres superuser like scripts/rebuild_db.sh does, since the app role is
# not granted CREATEDB.
ensure_collab_db() {
    local host="$1" port="$2" owner="$3" db="$4" exists
    if ! command -v psql >/dev/null; then
        echo "warning: psql not found; assuming database '$db' already exists" >&2
        return 0
    fi
    exists="$(psql -U postgres -h "$host" -p "$port" -tAc \
        "SELECT 1 FROM pg_database WHERE datname='$db'" 2>/dev/null || true)"
    if [[ "$exists" != "1" ]]; then
        echo "==> creating database '$db' owned by '$owner'"
        psql -U postgres -h "$host" -p "$port" -c "CREATE DATABASE $db OWNER $owner" \
            || { echo "error: could not create database '$db'" >&2; return 1; }
    fi
}

pids=()
readers=()
fifo_dir=""

# Run a binary in the background with each output line tagged. The binary is
# `exec`ed so the tracked PID is the binary itself (a direct child, with its
# own argv), fed through a FIFO to a reader subshell that prefixes lines.
# Works on the bash 3.2 that macOS ships, which lacks `wait -n`.
prefixed() {
    local tag="$1"; shift
    [[ -n "$fifo_dir" ]] || fifo_dir="$(mktemp -d)"
    local fifo="$fifo_dir/$tag"
    mkfifo "$fifo"
    ( while IFS= read -r line; do printf '[%s] %s\n' "$tag" "$line"; done < "$fifo" ) &
    readers+=($!)
    ( exec "$@" > "$fifo" 2>&1 ) &
    pids+=($!)
}

# Stop the tracked binaries by PID (never the process group, so a caller that
# shares it is not signalled), then reap them and their log readers.
stop_children() {
    local pid
    for pid in "${pids[@]:-}"; do
        [[ -n "$pid" ]] && kill -TERM "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    if [[ -n "$fifo_dir" ]]; then
        rm -rf "$fifo_dir"
    fi
}

# On Ctrl-C or an external TERM, stop the children and exit with the
# conventional 128 + signal status (130 / 143) so callers can tell intentional
# termination from a launcher error.
on_signal() {
    local signal="$1"
    trap - INT TERM EXIT
    stop_children
    exit $(( 128 + $(kill -l "$signal") ))
}

# Block until the first child exits and return its status, so a binary that
# fails at startup (bad DB, port in use) fails the launcher too.
supervise() {
    local pid status
    while :; do
        for pid in "${pids[@]}"; do
            if ! kill -0 "$pid" 2>/dev/null; then
                wait "$pid" && status=0 || status=$?
                return "$status"
            fi
        done
        sleep 1
    done
}

main() {
    local repo_root env_file run_app=true run_collab=true arg key
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    cd "$repo_root"
    env_file=".env"

    for arg in "$@"; do
        case "$arg" in
            --app-only) run_collab=false ;;
            --collab-only) run_app=false ;;
            -h|--help) usage; return 0 ;;
            *) echo "error: unknown argument '$arg'" >&2; usage >&2; return 2 ;;
        esac
    done

    if [[ ! -f "$env_file" ]]; then
        echo "error: no .env in $repo_root; see docs/setup.md" >&2
        return 1
    fi

    for key in TIPTAP_JWT_SIGNING_KEY TIPTAP_AUTH_KEY TIPTAP_URL \
               POSTGRES_USER POSTGRES_PASSWORD POSTGRES_HOST POSTGRES_PORT POSTGRES_SCHEMA \
               COLLAB_DATABASE_URL COLLAB_BIND_ADDR COLLAB_URL; do
        load_key "$env_file" "$key"
    done
    COLLAB_BIND_ADDR="${COLLAB_BIND_ADDR:-127.0.0.1:1234}"
    COLLAB_URL="${COLLAB_URL:-$(client_url_from_bind "$COLLAB_BIND_ADDR")}"
    local collab_db_name="${COLLAB_DB_NAME:-refactor_collab}"

    if $run_collab; then
        # The collab server refuses to start without the secrets, but failing
        # here names the .env key rather than the collab-side flag.
        require TIPTAP_JWT_SIGNING_KEY
        require TIPTAP_AUTH_KEY
        if [[ -z "${COLLAB_DATABASE_URL:-}" ]]; then
            require POSTGRES_USER
            require POSTGRES_PASSWORD
            require POSTGRES_HOST
            require POSTGRES_PORT
        fi
    fi

    # A backend pointed elsewhere mints tokens the local collab server never
    # sees. The editor then opens local-only with no error, so catch it up front.
    if $run_app && $run_collab && [[ "${TIPTAP_URL:-}" != "$COLLAB_URL" ]]; then
        echo "warning: TIPTAP_URL in .env is '${TIPTAP_URL:-<unset>}', not '$COLLAB_URL'." >&2
        echo "         The app will not talk to the local collab server. Set it to '$COLLAB_URL'," >&2
        echo "         and keep the frontend's NEXT_PUBLIC_DOCS_COLLAB_URL on the same host and port (ws://)." >&2
    fi

    # Build first so compile errors surface cleanly instead of interleaved with
    # the other process's logs.
    local packages=()
    $run_app && packages+=(-p refactor_platform_rs)
    $run_collab && packages+=(-p docs-collab-server)
    echo "==> cargo build ${packages[*]}"
    cargo build "${packages[@]}"

    trap 'on_signal INT' INT
    trap 'on_signal TERM' TERM
    trap 'trap - INT TERM EXIT; stop_children' EXIT

    if $run_collab; then
        local database_url="${COLLAB_DATABASE_URL:-}"
        if [[ -z "$database_url" ]]; then
            ensure_collab_db "$POSTGRES_HOST" "$POSTGRES_PORT" "$POSTGRES_USER" "$collab_db_name"
            database_url="$(collab_database_url "$POSTGRES_USER" "$POSTGRES_PASSWORD" \
                "$POSTGRES_HOST" "$POSTGRES_PORT" "$collab_db_name")"
        fi
        prefixed collab env \
            DATABASE_URL="$database_url" \
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

    local status=0
    supervise || status=$?
    stop_children
    trap - INT TERM EXIT
    return "$status"
}

# Guard so tests can source the functions without launching anything.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
