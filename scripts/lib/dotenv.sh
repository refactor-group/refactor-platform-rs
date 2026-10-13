# shellcheck shell=bash
# Shared .env readers for the local dev scripts. Source it; it defines functions only.

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
