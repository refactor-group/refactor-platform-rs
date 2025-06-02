# Refactor Platform: Docker Quickstart

*This project uses Docker & Docker Compose for local development. It deploys a PostgreSQL database, a Rust back-end, and a Next.js front-end (all pre-built images from GitHub Container Registry).*

## Prerequisites

- Docker (v20+)
- Docker Compose (v1.29+)
- A configured .env file (see below)

## Example .env File

Below is an example of a complete and correct .env file. Copy this content (or adjust values as needed) and save it as .env in the project root.

```bash
# ==============================
#   PostgreSQL Configuration
# ==============================
POSTGRES_USER=refactor                       # PostgreSQL username
POSTGRES_PASSWORD=password                   # PostgreSQL password
POSTGRES_DB=refactor                         # PostgreSQL database name
POSTGRES_HOST=postgres                       # Hostname for the PostgreSQL container (set in docker-compose)
POSTGRES_PORT=5432                           # Internal PostgreSQL port
POSTGRES_SCHEMA=refactor_platform            # Database schema
POSTGRES_OPTIONS="sslmode=require"           # Set connection string options like sslmode
# DATABASE_URL used by the Rust back-end to connect to Postgres
DATABASE_URL=postgres://refactor:password@postgres:5432/refactor

# ==============================
#   Rust Back-end Configuration
# ==============================
BACKEND_CONTAINER_NAME=refactor-platform     # Name for the Rust back-end container
BACKEND_IMAGE_NAME=ghcr.io/refactor-group/refactor-platform-rs/<branch-name>:latest
                                             # Pre-built image for the Rust back-end from GHCR
BACKEND_BUILD_CONTEXT="<refactor_platform_rs_source_dir>" # Optional, set to build locally and shorten $BACKEND_IMAGE_NAME
BACKEND_ALLOWED_ORIGINS=*                    # Allowed CORS origins
BACKEND_LOG_FILTER_LEVEL=DEBUG               # Logging level for the back-end
BACKEND_PORT=4000                            # Port on which the Rust back-end listens
BACKEND_INTERFACE=0.0.0.0                    # Interface for the Rust back-end
BACKEND_SERVICE_PROTOCOL=http                # Protocol (usually http)
BACKEND_SERVICE_PORT=4000                    # Derived service port
BACKEND_SERVICE_HOST=localhost               # Hostname used by the service
BACKEND_API_VERSION=0.0.1                    # API version
RUST_ENV=development                         # development, staging, production

# ==============================
#   Next.js Front-end Configuration
# ==============================
FRONTEND_IMAGE_NAME=ghcr.io/refactor-group/refactor-platform-fe/<branch-name>:latest
                                             # Pre-built image for the Next.js front-end from GHCR
FRONTEND_CONTAINER_NAME=refactor-platform-frontend  # Name for the front-end container
FRONTEND_BUILD_CONTEXT="<refactor_platform_fe_source_dir>" # Optional, set to build locally and shorten $FRONTEND_IMAGE_NAME
FRONTEND_SERVICE_INTERFACE=0.0.0.0           # Interface for the front-end service
FRONTEND_SERVICE_PORT=3000                   # Port for the front-end service

PLATFORM="linux/arm64/v8"                    # Or linux/amd64

# ==============================
#   TipTap Service Configuration
# ==============================
TIPTAP_URL=""                                # URL for the TipTap service
TIPTAP_AUTH_KEY=""                           # Authentication key for TipTap
TIPTAP_JWT_SIGNING_KEY=""                    # JWT signing key for TipTap
```

## Steps & Commands

1. **Clone the repository & set up the environment:**

   ```bash
   # Clone the repository and change into the project directory
   git clone <repository-url> && cd <repository-directory>

   # Copy the example .env file and adjust values as needed
   cp .env.example .env
   ```

2. **Build and Start the Containers with Docker Compose:**

   ```bash
   docker compose --env-file .env up --build
   # This command starts:
   # - PostgreSQL (development and staging environments only)
   # - Rust back-end
   # - Next.js front-end
   ```

3. **Basic Management Commands:**

   ```bash
   docker compose ps                          # List running containers
   docker compose logs -f                     # Follow live logs (press Ctrl+C to exit)
   docker compose restart rust-app            # Restart the Rust back-end container
   docker compose down                        # Stop and remove all containers and networks
   docker compose down -v                     # Stop containers and remove volumes for a fresh start
   docker compose exec rust-app cargo check   # Run 'cargo check' inside the Rust back-end container
   docker compose exec rust-app cargo run     # Run the Rust back-end application
   docker compose ps                          # List running containers
   docker compose logs -f                     # Follow live logs (press Ctrl+C to exit)
   docker compose restart rust-app            # Restart the Rust back-end container
   docker compose exec rust-app cargo check   # Run 'cargo check' inside the Rust back-end container
   docker compose exec rust-app cargo run     # Run the Rust back-end application
   ```

4. **Direct Docker Commands (Optional):**

   ```bash
   # Pull the Rust back-end image from GHCR (if not built locally)
   docker pull ghcr.io/refactor-group/refactor-platform-rs/your-branch-tag:latest  # Replace 'your-branch-tag' as needed

   # Run the Rust back-end image directly
   docker run -p 4000:4000 --env-file .env --name refactor-platform-backend ghcr.io/refactor-group/refactor-platform-rs/your-tag:latest
   ```

   **Note:** *By default, Docker Compose uses locally cached images. The remote image is pulled only once unless you force a new pull using commands like `docker compose pull` or by passing the `--no-cache` flag.*

   ```bash
   # Directly run the migrationctl binary, checking the migration status, in the migrator docker compose service passing it an explicit DB connection string
   docker compose run migrator migrationctl status

   # Or do the same thing but override the environment variable for the DATABASE_URL
   docker compose run migrator migrationctl -u "postgresql://<$POSTGRES_USER>:<$POSTGRES_PASSWORD>@dbserver:5432/refactor" status
   ```

5. **Debugging & Troubleshooting:**

   ```bash
   docker compose exec rust-app bash         # Access a shell in the Rust back-end container
   docker compose exec rust-app env          # View environment variables in the Rust back-end container
   docker compose exec postgres bash         # Access a shell in the PostgreSQL container
   docker compose exec postgres pg_isready -U $POSTGRES_USER -d $POSTGRES_DB  
                                             # Verify PostgreSQL is ready
   docker compose exec rust-app bash         # Access a shell in the Rust back-end container
   docker compose exec rust-app env          # Check environment variables inside the rust-app container
   docker compose exec postgres bash         # Access a shell in the PostgreSQL container for troubleshooting
   docker compose exec postgres pg_isready -U $POSTGRES_USER -d $POSTGRES_DB  # Verify PostgreSQL is ready
   docker compose exec rust-app cargo test   # Run tests inside the Rust back-end container
   ```

**Final Notes:**

- Ensure your `.env` file includes required variables such as `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `POSTGRES_OPTIONS` `DATABASE_URL`, `BACKEND_PORT`, `BACKEND_INTERFACE`, `BACKEND_ALLOWED_ORIGINS`, `BACKEND_LOG_FILTER_LEVEL`, `RUST_ENV`, etc.
- Docker Compose automatically loads the `.env` file located in the project root.
- The pre-built images from GHCR for both the Rust back-end and the Next.js front-end are used by default. These remote images are only pulled if not already available locally, unless a pull is forced.
- The commands above follow best practices and help ensure a reliable setup every time you run the project.
