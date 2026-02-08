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
            validate_env "DATABASE_SCHEMA"
            validate_env "RUST_ENV"

            log_info "Running in $RUST_ENV environment"
            log_info "Using schema $DATABASE_SCHEMA to apply the migrations in"

            # Ensure schema exists before running migrations
            # This makes the migrator idempotent and independent of external setup
            log_info "Ensuring schema '$DATABASE_SCHEMA' exists..."

            # Extract connection parameters from DATABASE_URL
            # Format: postgres://user:password@host:port/database
            DB_HOST=$(echo "$DATABASE_URL" | sed -E 's|postgres://[^@]+@([^:/]+).*|\1|')
            DB_PORT=$(echo "$DATABASE_URL" | sed -E 's|postgres://[^@]+@[^:]+:([0-9]+)/.*|\1|')
            DB_NAME=$(echo "$DATABASE_URL" | sed -E 's|postgres://[^@]+@[^/]+/([^?]+).*|\1|')
            DB_USER=$(echo "$DATABASE_URL" | sed -E 's|postgres://([^:]+):.*|\1|')
            DB_PASS=$(echo "$DATABASE_URL" | sed -E 's|postgres://[^:]+:([^@]+)@.*|\1|')

            # Wait for PostgreSQL to be ready
            log_info "Waiting for PostgreSQL to be ready..."
            for i in $(seq 1 30); do
                if PGPASSWORD="$DB_PASS" psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" -c "SELECT 1" >/dev/null 2>&1; then
                    log_success "PostgreSQL is ready"
                    break
                fi
                if [ "$i" -eq 30 ]; then
                    log_error "PostgreSQL did not become ready in time"
                    exit 1
                fi
                sleep 1
            done

            # Create schema if it doesn't exist
            log_info "Creating schema '$DATABASE_SCHEMA' if it doesn't exist..."
            if ! PGPASSWORD="$DB_PASS" psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" -c "CREATE SCHEMA IF NOT EXISTS \"$DATABASE_SCHEMA\";" >/dev/null 2>&1; then
                log_error "Failed to create schema '$DATABASE_SCHEMA'"
                exit 1
            fi

            log_success "Schema '$DATABASE_SCHEMA' is ready"

            # Set search_path in DATABASE_URL so all connections use the correct schema
            # Append options parameter to DATABASE_URL if not already present
            if echo "$DATABASE_URL" | grep -q '?'; then
                # URL already has query parameters
                export DATABASE_URL="${DATABASE_URL}&options=-csearch_path%3D${DATABASE_SCHEMA}"
            else
                # No query parameters yet
                export DATABASE_URL="${DATABASE_URL}?options=-csearch_path%3D${DATABASE_SCHEMA}"
            fi

            log_info "Set search_path to '$DATABASE_SCHEMA' in DATABASE_URL"
            log_success "Running SeaORM migrations..."
            exec /app/migrationctl up -s $DATABASE_SCHEMA
            ;;
            
        app)
            log_info "Running in APP mode"
            validate_binary "refactor_platform_rs"
            validate_env "DATABASE_URL"
            validate_env "RUST_ENV"

            log_info "Running in $RUST_ENV environment"
            
            # Set application defaults
            local rust_env="${RUST_ENV:-development}"
            local log_level="${BACKEND_LOG_FILTER_LEVEL:-INFO}"
            local interface="${BACKEND_INTERFACE:-0.0.0.0}"
            local port="${BACKEND_PORT:-4000}"
            local origins="${BACKEND_ALLOWED_ORIGINS:-*}"
            local session_expiry="${BACKEND_SESSION_EXPIRY_SECONDS:-86400}"
            
            log_info "Starting Refactor Platform API server..."
            log_debug "Log level: $log_level, Interface: $interface, Port: $port"
            
            exec /app/refactor_platform_rs \
                -r "$rust_env" \
                -l "$log_level" \
                -i "$interface" \
                -p "$port" \
                --allowed-origins="$origins" \
                --backend-session-expiry-seconds="$session_expiry" \
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