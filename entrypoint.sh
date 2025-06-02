#!/bin/bash
set -euo pipefail

# =============================================================================
# Refactor Platform Entrypoint Script
# =============================================================================

# Logging functions
log_info() { echo "ℹ️  $*"; }
log_success() { echo "✅ $*"; }
log_error() { echo "❌ $*" >&2; }
log_debug() { echo "🐛 $*"; }

# Validate required binaries exist
validate_binary() {
    local binary="$1"
    if [[ ! -x "/app/$binary" ]]; then
        log_error "Required binary not found or not executable: /app/$binary"
        exit 1
    fi
}

# Validate required environment variables
validate_env() {
    local var_name="$1"
    local var_value="${!var_name:-}"
    if [[ -z "$var_value" ]]; then
        log_error "Required environment variable not set: $var_name"
        exit 1
    fi
}

# Main execution
main() {
    log_info "Starting Refactor Platform entrypoint..."
    
    # Set default role
    ROLE="${ROLE:-app}"
    
    # Handle direct CLI commands first
    if [[ $# -gt 0 ]]; then
        log_info "Processing CLI arguments: $*"
        case "$1" in
            migrationctl)
                validate_binary "migrationctl"
                log_info "Executing migration command directly"
                exec "/app/$@"
                ;;
            seed_db)
                validate_binary "seed_db"
                log_info "Executing seed command directly"
                exec "/app/$@"
                ;;
            *)
                log_info "Unknown command '$1', proceeding with role-based execution"
                ;;
        esac
    fi
    
    # Role-based execution
    case "$ROLE" in
        migrator)
            log_info "Running in MIGRATOR mode"
            validate_binary "migrationctl"
            validate_env "DATABASE_URL"
            validate_env "RUST_ENV"

            log_info "Running in $RUST_ENV environment"

            log_success "Running SeaORM migrations..."
            exec /app/migrationctl up
            ;;
            
        app)
            log_info "Running in APP mode"
            validate_binary "refactor_platform_rs"
            validate_env "DATABASE_URL"
            validate_env "RUST_ENV"

            log_info "Running in $RUST_ENV environment"
            
            # Set application defaults
            local log_level="${BACKEND_LOG_FILTER_LEVEL:-INFO}"
            local interface="${BACKEND_INTERFACE:-0.0.0.0}"
            local port="${BACKEND_PORT:-4000}"
            local origins="${BACKEND_ALLOWED_ORIGINS:-*}"
            
            log_info "Starting Refactor Platform API server..."
            log_debug "Log level: $log_level, Interface: $interface, Port: $port"
            
            exec /app/refactor_platform_rs \
                -l "$log_level" \
                -i "$interface" \
                -p "$port" \
                --allowed-origins="$origins" \
                "$@"
            ;;
            
        *)
            log_error "Unknown ROLE: '$ROLE'. Valid roles are: migrator, app"
            log_error "Set ROLE environment variable to one of the valid values"
            exit 1
            ;;
    esac
}

# Run main function with all arguments
main "$@"