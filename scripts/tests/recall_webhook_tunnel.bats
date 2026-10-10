#!/usr/bin/env bats
#
# Tests for scripts/recall_webhook_tunnel.sh.
#
# Unit tests source the script (its main guard keeps it from running) and check
# the validation helpers. Run tests copy it into a throwaway repo layout with
# stub `curl` and `ngrok` on PATH, so the checks, the tunnel lifecycle, and the
# printed setup are verified without any network or a real tunnel.

SCRIPT="$BATS_TEST_DIRNAME/../recall_webhook_tunnel.sh"
SECRET="whsec_$(printf 'local-dev-signing-secret' | base64)"

setup() {
    # shellcheck disable=SC1090
    source "$SCRIPT"

    ROOT="$BATS_TEST_TMPDIR/repo"
    mkdir -p "$ROOT/scripts/lib" "$ROOT/bin"
    cp "$SCRIPT" "$ROOT/scripts/recall_webhook_tunnel.sh"
    cp "$BATS_TEST_DIRNAME/../lib/dotenv.sh" "$ROOT/scripts/lib/dotenv.sh"
    TUNNEL="$ROOT/scripts/recall_webhook_tunnel.sh"
    CALLS="$ROOT/calls.log"

    # curl answers by URL with the code a test sets, recording its arguments.
    # Recall answers only when the key arrives as a Token header read from
    # stdin (-H @-); the stub logs whether it matched, never the key itself.
    cat > "$ROOT/bin/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >> "$CALLS"
case "$*" in
    *recall.ai*)
        if [[ " $* " == *" -H @- "* && "$(cat)" == "Authorization: Token $STUB_RECALL_KEY" ]]; then
            echo "recall auth header ok" >> "$CALLS"
            echo "${STUB_RECALL_CODE:-200}"
        else
            echo "recall auth header missing or wrong" >> "$CALLS"
            echo 401
        fi
        ;;
    */health*) echo "${STUB_HEALTH_CODE:-200}" ;;
    */webhooks/recall_ai*) echo "${STUB_WEBHOOK_CODE:-401}" ;;
    *) echo 000 ;;
esac
EOF
    # ngrok records its arguments, then idles until stopped (or exits early).
    cat > "$ROOT/bin/ngrok" <<'EOF'
#!/usr/bin/env bash
echo "ngrok $*" >> "$CALLS"
if [[ -n "${STUB_NGROK_EXIT:-}" ]]; then
    echo "ERROR: failed to start tunnel: ERR_NGROK_334"
    exit "$STUB_NGROK_EXIT"
fi
exec -a "$NGROK_STUB_TAG" sleep 30
EOF
    chmod +x "$ROOT/bin/"*
    export CALLS
    export STUB_RECALL_KEY=key-that-must-stay-secret
    export NGROK_STUB_TAG="ngrok-stub-$BATS_TEST_NUMBER-$$"
    export PATH="$ROOT/bin:$PATH"
    export TUNNEL_WAIT_SECS=2

    cat > "$ROOT/.env" <<EOF
NGROK_DOMAIN=jim-dev.ngrok.app
RECALL_AI_API_KEY=key-that-must-stay-secret
RECALL_AI_REGION=us-west-2
RECALL_AI_WEBHOOK_SECRET=$SECRET
GOOGLE_REDIRECT_URI=http://localhost:4000/oauth/google/callback
EOF
}

teardown() {
    # The tunnel the script starts may never outlive the test.
    pkill -TERM -f "$NGROK_STUB_TAG" 2>/dev/null || true
}

ngrok_is_running() {
    pgrep -f "$NGROK_STUB_TAG" >/dev/null
}

# --- unit: validation helpers ------------------------------------------------

@test "normalize_domain keeps only the host" {
    [ "$(normalize_domain jim-dev.ngrok.app)" = "jim-dev.ngrok.app" ]
    [ "$(normalize_domain https://jim-dev.ngrok.app/)" = "jim-dev.ngrok.app" ]
    [ "$(normalize_domain https://jim-dev.ngrok.app/webhooks/recall_ai)" = "jim-dev.ngrok.app" ]
}

@test "recall_base_url builds the regional API root" {
    [ "$(recall_base_url us-west-2)" = "https://us-west-2.recall.ai/api/v1" ]
}

@test "valid_region accepts Recall's regions and rejects anything else" {
    for region in us-east-1 us-west-2 eu-central-1 ap-northeast-1; do
        valid_region "$region"
    done
    ! valid_region eu-west-2
    ! valid_region ""
}

@test "valid_webhook_secret requires the whsec_ prefix and a base64 body" {
    valid_webhook_secret "$SECRET"
    ! valid_webhook_secret "local-dev-signing-secret"
    ! valid_webhook_secret "whsec_"
    ! valid_webhook_secret "whsec_not base64!"
}

@test "redirect_is_local accepts localhost or 127.0.0.1 on the backend port only" {
    redirect_is_local http://localhost:4000/oauth/google/callback 4000
    redirect_is_local http://127.0.0.1:4000/x 4000
    ! redirect_is_local http://localhost:4100/x 4000
    ! redirect_is_local https://myrefactor.com/oauth/google/callback 4000
}

# --- run: checks that stop before any tunnel ---------------------------------

@test "a missing NGROK_DOMAIN is named and no tunnel starts" {
    sed -i.bak '/^NGROK_DOMAIN=/d' "$ROOT/.env"
    run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"NGROK_DOMAIN"* ]]
    ! grep -q '^ngrok' "$CALLS"
}

@test "a webhook secret that is not whsec_ form is rejected" {
    RECALL_AI_WEBHOOK_SECRET=plain-secret run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"RECALL_AI_WEBHOOK_SECRET"* ]]
    ! grep -q '^ngrok' "$CALLS"
}

@test "an unknown Recall region is rejected" {
    RECALL_AI_REGION=eu-west-2 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"eu-west-2"* ]]
    ! grep -q '^ngrok' "$CALLS"
}

@test "a Recall key rejected in its region stops before the tunnel" {
    STUB_RECALL_CODE=401 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"us-west-2"* ]]
    grep -q 'https://us-west-2.recall.ai/api/v1/bot/' "$CALLS"
    ! grep -q '^ngrok' "$CALLS"
}

@test "the Recall check sends the key as a Token header on stdin" {
    run "$TUNNEL" --check
    [ "$status" -eq 0 ]
    grep -q '^recall auth header ok$' "$CALLS"
}

@test "the API key never appears in any command line" {
    run "$TUNNEL" --check
    [ "$status" -eq 0 ]
    ! grep -q 'key-that-must-stay-secret' "$CALLS"
    [[ "$output" != *"key-that-must-stay-secret"* ]]
}

@test "a backend that is not running stops before the tunnel" {
    STUB_HEALTH_CODE=000 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"run_backend.sh"* ]]
    ! grep -q '^ngrok' "$CALLS"
}

@test "a non-local Google redirect is a warning, not a failure" {
    GOOGLE_REDIRECT_URI=https://myrefactor.com/oauth/google/callback run "$TUNNEL" --check
    [ "$status" -eq 0 ]
    [[ "$output" == *"GOOGLE_REDIRECT_URI"* ]]
}

# --- run: the tunnel ---------------------------------------------------------

@test "--check opens the tunnel on the reserved domain, verifies the route, prints the setup, and stops" {
    run "$TUNNEL" --check
    [ "$status" -eq 0 ]
    grep -q '^ngrok http 4000 --url=https://jim-dev.ngrok.app' "$CALLS"
    grep -q 'POST.*https://jim-dev.ngrok.app/webhooks/recall_ai' "$CALLS"
    [[ "$output" == *"https://jim-dev.ngrok.app/webhooks/recall_ai"* ]]
    for event in bot.joining_call bot.in_waiting_room bot.in_call_not_recording \
                 bot.in_call_recording bot.done bot.fatal recording.done recording.failed \
                 transcript.done transcript.failed transcript.processing; do
        [[ "$output" == *"$event"* ]]
    done
    ! ngrok_is_running
}

@test "PORT moves both the health check and the tunnel" {
    PORT=4100 GOOGLE_REDIRECT_URI=http://localhost:4100/cb run "$TUNNEL" --check
    [ "$status" -eq 0 ]
    grep -q 'http://localhost:4100/health' "$CALLS"
    grep -q '^ngrok http 4100 ' "$CALLS"
}

@test "a 405 from the webhook route explains the path and stops the tunnel" {
    STUB_WEBHOOK_CODE=405 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"/webhooks/recall_ai"* ]]
    ! ngrok_is_running
}

@test "a tunnel that never reaches the backend fails after the wait, naming the code" {
    STUB_WEBHOOK_CODE=502 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"502"* ]]
    ! ngrok_is_running
}

@test "ngrok exiting at startup reports its output" {
    STUB_NGROK_EXIT=1 run "$TUNNEL" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"ERR_NGROK_334"* ]]
}

@test "without --check the tunnel stays up until SIGTERM, then stops with 143" {
    "$TUNNEL" > "$ROOT/out.log" 2>&1 &
    local launcher=$!
    for _ in $(seq 1 40); do
        grep -q 'Ctrl-C' "$ROOT/out.log" && break
        sleep 0.25
    done
    ngrok_is_running
    kill -TERM "$launcher"
    local status=0
    wait "$launcher" || status=$?
    [ "$status" -eq 143 ]
    ! ngrok_is_running
}

@test "unknown flags are rejected with usage" {
    run "$TUNNEL" --bogus
    [ "$status" -eq 2 ]
    [[ "$output" == *"--check"* ]]
}
