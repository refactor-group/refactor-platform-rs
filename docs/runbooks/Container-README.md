# Refactor Platform – Docker Quickstart

This project uses Docker & Docker Compose for local development. It deploys a PostgreSQL database, a Rust back-end, and a Next.js front-end (all pre-built images from GitHub Container Registry).

## Prerequisites

- Docker (v20+)
- Docker Compose (v1.29+)
- A configured .env file (see examples)

## Steps & Commands

1. Clone the repository & set up the environment:

```bash
   git clone <repository-url> && cd <repository-directory>
  ```
  
## Copy the example .env file and adjust values as needed

  ```bash
    cp .env.example .env
  ```

2. Use Docker Compose to build and start services:
   docker-compose --env-file .env up --build

   ## This starts PostgreSQL (local), the Rust back-end, and the Next.js front-end

3. Basic Management Commands:

```bash
   docker-compose ps          # List running containers
   docker-compose logs -f     # Follow logs; press Ctrl+C to exit
   docker-compose restart rust-app  # Restart the Rust back-end service
   docker-compose down       # Stop and remove all containers and networks
   docker-compose down -v    # Also remove volumes for a fresh start
   docker-compose exec rust-app cargo check  # Run a command inside the Rust back-end container
   docker-compose exec rust-app cargo run    # Run the Rust back-end application
   ```

4. Direct Docker Commands (Optional):

   <!-- Pull the Rust back-end image from GHCR (if not built locally) -->
   ```bash
   docker pull ghcr.io/refactor-group/refactor-platform-rs/your-tag:latest  # Replace 'your-tag' accordingly
   ```

   ## Run the Rust back-end image directly

   ```bash
   docker run -p 4000:4000 --env-file .env --name refactor-backend ghcr.io/refactor-group/refactor-platform-rs/your-tag:latest
   ```
   
4. **Debugging / Troubleshooting:**

   ```bash

   docker-compose exec rust-app bash       # Access a shell in the Rust back-end container
   docker-compose exec rust-app env            # Check environment variables inside the rust-app container
   docker-compose exec postgres bash           # Access a shell in the PostgreSQL container for troubleshooting
   docker-compose exec postgres pg_isready -U $POSTGRES_USER -d $POSTGRES_DB  # Verify PostgreSQL is ready
   ```

**Notes:**

- Ensure your `.env` file includes required variables such as `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `DATABASE_URL`, `BACKEND_PORT`, `BACKEND_INTERFACE`, `BACKEND_ALLOWED_ORIGINS`, `BACKEND_LOG_FILTER_LEVEL`, etc.
- The nextjs-app service uses the pre-built image from GHCR (update the image name if necessary).
- If using docker-compose, the `.env` file located in the project root is automatically loaded.

*This guide provides all essential commands to safely work with the containers using Docker and Docker Compose.*
