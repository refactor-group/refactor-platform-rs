#!/usr/bin/env bash
#
# Expose the local backend to Recall.ai so recording and transcript webhooks
# reach it, through ngrok on your reserved domain.
#
#   scripts/recall_webhook_tunnel.sh          # check, open the tunnel, keep it up
#   scripts/recall_webhook_tunnel.sh --check  # check, open the tunnel, verify, stop
#
# Start the backend first (scripts/run_backend.sh). Before opening the tunnel
# this checks the .env values the recording flow depends on and that your
# Recall key works in its region. Once the tunnel is up it confirms that
# POST /webhooks/recall_ai reaches the backend and prints the one-time Recall
# dashboard setup for your own development workspace.
#
# Reads from the shell environment first, then .env:
#   NGROK_DOMAIN               your reserved ngrok domain, e.g. jim-dev.ngrok.app
#   RECALL_AI_API_KEY          key from your own Recall development workspace
#   RECALL_AI_REGION           that workspace's region, e.g. us-west-2
#   RECALL_AI_WEBHOOK_SECRET   that workspace's signing secret (whsec_...)
#   PORT                       backend port (4000)
#   GOOGLE_REDIRECT_URI        checked to point at the local backend
#
# Ctrl-C stops the tunnel.

set -euo pipefail

usage() {
    sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# shellcheck source=SCRIPTDIR/lib/dotenv.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/dotenv.sh"

RECALL_REGIONS="us-east-1 us-west-2 eu-central-1 ap-northeast-1"

# Every event the backend handles; the endpoint must subscribe to each one.
RECALL_EVENTS=(
    bot.joining_call bot.in_waiting_room bot.in_call_not_recording bot.in_call_recording
    bot.done bot.fatal recording.done recording.failed
    transcript.done transcript.failed transcript.processing
)

ngrok_pid=""
ngrok_log=""

# A bare host from whatever form NGROK_DOMAIN was written in.
normalize_domain() {
    local value="${1#http://}"
    value="${value#https://}"
    printf '%s' "${value%%/*}"
}

recall_base_url() {
    printf 'https://%s.recall.ai/api/v1' "$1"
}

valid_region() {
    [[ -n "$1" && " $RECALL_REGIONS " == *" $1 "* ]]
}

valid_webhook_secret() {
    local body="${1#whsec_}"
    [[ "$1" == whsec_* && -n "$body" ]] \
        && [[ "$body" =~ ^[A-Za-z0-9+/]+={0,2}$ ]] \
        && (( ${#body} % 4 == 0 ))
}

redirect_is_local() {
    local uri="$1" port="$2"
    [[ "$uri" == "http://localhost:$port/"* || "$uri" == "http://127.0.0.1:$port/"* ]]
}

fail() {
    echo "error: $*" >&2
    return 1
}

# HTTP status of a request, or 000 when nothing answered.
http_code() {
    curl -s -o /dev/null -w '%{http_code}' --max-time 10 "$@" || true
}

# The key goes in a header read from stdin so it never appears in a process list.
check_recall_key() {
    local key="$1" region="$2" code
    code="$(printf 'Authorization: Token %s\n' "$key" \
        | http_code -H @- "$(recall_base_url "$region")/bot/?page_size=1")"
    case "$code" in
        200) echo "==> Recall key works in $region" ;;
        401|403) fail "Recall rejected RECALL_AI_API_KEY in region $region ($code). Use a key from your own development workspace, in that workspace's region." ;;
        *) echo "warning: could not confirm the Recall key ($code); continuing" >&2 ;;
    esac
}

check_backend() {
    local port="$1" code
    code="$(http_code "http://localhost:$port/health")"
    [[ "$code" == 200 ]] \
        || fail "the backend is not answering on http://localhost:$port/health ($code). Start it with scripts/run_backend.sh."
    echo "==> backend is up on port $port"
}

stop_tunnel() {
    if [[ -n "$ngrok_pid" ]]; then
        kill -TERM "$ngrok_pid" 2>/dev/null || true
        wait "$ngrok_pid" 2>/dev/null || true
        ngrok_pid=""
    fi
    if [[ -n "$ngrok_log" ]]; then
        rm -f "$ngrok_log"
    fi
}

on_signal() {
    local signal="$1"
    trap - INT TERM EXIT
    stop_tunnel
    exit $(( 128 + $(kill -l "$signal") ))
}

ngrok_failed() {
    cat "$ngrok_log" >&2
    fail "ngrok exited (output above). If the domain is already online, stop the other ngrok first."
}

# Wait for the tunnel to deliver a POST to the webhook route. The handler
# answers an unsigned request with 401, which proves the route is reached.
# Our ngrok must still be running afterwards: a stale tunnel from an earlier
# run would answer the probe while this one fails to claim the domain.
verify_route() {
    local url="$1" deadline code="000"
    deadline=$(( SECONDS + ${TUNNEL_WAIT_SECS:-20} ))
    while (( SECONDS < deadline )); do
        sleep 0.5
        kill -0 "$ngrok_pid" 2>/dev/null || { ngrok_failed; return 1; }
        code="$(http_code -X POST "$url")"
        case "$code" in
            401)
                sleep 1
                kill -0 "$ngrok_pid" 2>/dev/null || { ngrok_failed; return 1; }
                echo "==> $url reaches the backend"
                echo "    (the backend logs one \"Missing svix-id/webhook-id header\" warning for this unsigned check; that is expected)"
                return 0
                ;;
            405) fail "$url answered 405: that port is not serving POST /webhooks/recall_ai. Is PORT the backend's port?"; return 1 ;;
        esac
    done
    fail "$url never reached the backend (last status $code). Check that ngrok forwards to the backend's port."
}

print_dashboard_setup() {
    local url="$1"
    cat <<EOF

Recall dashboard, in your own development workspace (one time; the URL never changes):
  1. Webhooks > Add Endpoint
     URL: $url
  2. Subscribe to these events:
$(printf '       %s\n' "${RECALL_EVENTS[@]}")
  3. Developers > API Keys & Secrets: put the workspace signing secret in .env as
     RECALL_AI_WEBHOOK_SECRET, then restart the backend.
EOF
}

main() {
    local repo_root env_file check_only=false arg key domain url
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    cd "$repo_root"
    env_file=".env"

    for arg in "$@"; do
        case "$arg" in
            --check) check_only=true ;;
            -h|--help) usage; return 0 ;;
            *) echo "error: unknown argument '$arg'" >&2; usage >&2; return 2 ;;
        esac
    done

    [[ -f "$env_file" ]] || fail "no .env in $repo_root; see docs/setup.md"

    for key in NGROK_DOMAIN RECALL_AI_API_KEY RECALL_AI_REGION RECALL_AI_WEBHOOK_SECRET \
               PORT GOOGLE_REDIRECT_URI; do
        load_key "$env_file" "$key"
    done
    PORT="${PORT:-4000}"

    [[ -n "$NGROK_DOMAIN" ]] \
        || fail "NGROK_DOMAIN is not set; add your reserved ngrok domain to .env (see docs/setup.md)."
    [[ -n "$RECALL_AI_API_KEY" ]] || fail "RECALL_AI_API_KEY is not set in .env."
    valid_region "$RECALL_AI_REGION" \
        || fail "RECALL_AI_REGION '$RECALL_AI_REGION' is not a Recall region ($RECALL_REGIONS)."
    valid_webhook_secret "$RECALL_AI_WEBHOOK_SECRET" \
        || fail "RECALL_AI_WEBHOOK_SECRET must be a Recall signing secret of the form whsec_<base64>."
    if [[ -z "$GOOGLE_REDIRECT_URI" ]]; then
        echo "warning: GOOGLE_REDIRECT_URI is not set; connecting Google will fail." >&2
    elif ! redirect_is_local "$GOOGLE_REDIRECT_URI" "$PORT"; then
        echo "warning: GOOGLE_REDIRECT_URI is '$GOOGLE_REDIRECT_URI', not the local backend on port $PORT; connecting Google will leave this machine." >&2
    fi

    check_recall_key "$RECALL_AI_API_KEY" "$RECALL_AI_REGION"
    check_backend "$PORT"

    domain="$(normalize_domain "$NGROK_DOMAIN")"
    url="https://$domain/webhooks/recall_ai"

    trap 'on_signal INT' INT
    trap 'on_signal TERM' TERM
    trap 'trap - INT TERM EXIT; stop_tunnel' EXIT

    echo "==> opening https://$domain -> localhost:$PORT"
    ngrok_log="$(mktemp)"
    ngrok http "$PORT" --url="https://$domain" --log=stdout > "$ngrok_log" 2>&1 &
    ngrok_pid=$!

    verify_route "$url"
    print_dashboard_setup "$url"

    if $check_only; then
        stop_tunnel
        return 0
    fi

    echo
    echo "Tunnel is up. Press Ctrl-C to stop it."
    local status=0
    wait "$ngrok_pid" || status=$?
    ngrok_pid=""
    cat "$ngrok_log" >&2
    fail "ngrok stopped unexpectedly ($status; output above)."
}

# Guard so tests can source the functions without running anything.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
