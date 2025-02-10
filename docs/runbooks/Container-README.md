# Refactor Coaching & Mentoring Platform with Docker & Docker Compose

*This project is a Rust-based backend/web API that connects to a PostgreSQL database. It uses Docker and Docker Compose for easy local development and deployment, and includes utilities for database management, migrations, and more. You can choose to run PostgreSQL either locally (via Docker) or remotely by configuring the environment variables.*

---

## Prerequisites

Before you begin, ensure you have the following installed:

- [Docker](https://www.docker.com/products/docker-desktop) (version 20+)
- [Docker Compose](https://docs.docker.com/compose/install/) (version 1.29+)

---

## Project Setup

### 1. **Clone the Repository**

```bash
git clone <repository-url>
cd <repository-directory>
```

### 2. **Environment Configuration**

Decide whether you're connecting to a **local PostgreSQL container** (using Docker) or a **remote PostgreSQL instance**. Configure this using `.env` files.

#### **For Local PostgreSQL (Docker-based)**

- Create a `.env.local` file based on the template below and specify `POSTGRES_HOST=postgres`.

**Example** `.env.local`:

```env
POSTGRES_USER=refactor
POSTGRES_PASSWORD=password
POSTGRES_DB=refactor
POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_SCHEMA=refactor_platform
DATABASE_URL=postgres://$POSTGRES_USER:$POSTGRES_PASSWORD@$POSTGRES_HOST:$POSTGRES_PORT/$POSTGRES_DB

BACKEND_LOG_FILTER_LEVEL="DEBUG"
BACKEND_PORT=4000
BACKEND_INTERFACE=0.0.0.0
BACKEND_ALLOWED_ORIGINS="http://localhost:3000,https://localhost:3000"

BACKEND_SERVICE_PROTOCOL="http"
BACKEND_SERVICE_PORT=${BACKEND_PORT}
BACKEND_SERVICE_HOST="localhost"
BACKEND_API_VERSION="0.0.1"
FRONTEND_SERVICE_INTERFACE=0.0.0.0
FRONTEND_SERVICE_PORT=3000

USERNAME=appuser
USER_UID=1000
USER_GID=1000
CONTAINER_NAME=refactor-platform
PLATFORM=linux/arm64

TIPTAP_URL=https://{Tiptap API Key}.collab.tiptap.cloud/
TIPTAP_AUTH_KEY=tiptap-auth-key
TIPTAP_JWT_SIGNING_KEY=tiptap-jwt-signing-key
```

#### **For Remote PostgreSQL**

- Create a `.env.remote-db` file and set `POSTGRES_HOST` to the external IP or hostname of the remote PostgreSQL instance.

**Example** `.env.remote-db`:

```env
# PostgreSQL environment variables for local development
POSTGRES_USER=refactor  # Default PostgreSQL user for local development
POSTGRES_PASSWORD=password  # Default PostgreSQL password for local development
POSTGRES_DB=refactor  # Default PostgreSQL database for local development
POSTGRES_HOST=postgres  # The local Docker Compose PostgreSQL container hostname
POSTGRES_PORT=5432  # PostgreSQL default port for local development
POSTGRES_SCHEMA=refactor_platform  # PostgreSQL schema for the application
# Database connection URL for the Rust application
DATABASE_URL=postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}

# Rust application environment variables
BACKEND_LOG_FILTER_LEVEL="DEBUG"
BACKEND_ALLOWED_ORIGINS="http://localhost:3000,https://localhost:3000"
BACKEND_PORT=4000
BACKEND_INTERFACE=0.0.0.0

# Next.js application build & environment variables
BACKEND_SERVICE_PROTOCOL="http"
BACKEND_SERVICE_PORT=${BACKEND_PORT}
BACKEND_SERVICE_HOST="localhost"
BACKEND_API_VERSION="0.0.1"
FRONTEND_SERVICE_INTERFACE=0.0.0.0
FRONTEND_SERVICE_PORT=3000

PLATFORM=linux/arm64 # For Raspberry Pi 5 or Apple Silicon
CONTAINER_NAME="refactor-platform"

# App user configuration
USERNAME=appuser  # Username for the non-root user in the container
USER_UID=1000  # User ID for the appuser
USER_GID=1000  # Group ID for the appuser

TIPTAP_URL=https://{Tiptap API Key}.collab.tiptap.cloud/
TIPTAP_AUTH_KEY=tiptap-auth-key
TIPTAP_JWT_SIGNING_KEY=tiptap-jwt-signing-key
```

### 3. **Review `docker-compose.yaml`**

The `docker-compose.yaml` file uses environment variables defined in your `.env` file setting important
configuration variables for both the Rust backend and the Next.js frontend applications.

```yaml
services:
  postgres:
    image: postgres:17
    container_name: postgres
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
    ports:
      - "${POSTGRES_PORT}:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data
      - ./migration/src/setup.sql:/docker-entrypoint-initdb.d/0-setup.sql
      - ./migration/src/refactor_platform_rs.sql:/docker-entrypoint-initdb.d/1-refactor_plaform_rs.sql
      - ./migration/src/setup_default_user.sql:/docker-entrypoint-initdb.d/2-setup_default_user.sql
    networks:
      - backend_network

  rust-app:
    image: rust-backend
    build:
      context: .
      dockerfile: Dockerfile
      target: runtime
    platform: ${PLATFORM}
    container_name: ${CONTAINER_NAME}
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_SCHEMA: ${POSTGRES_SCHEMA}
      POSTGRES_HOST: postgres
      POSTGRES_PORT: ${POSTGRES_PORT}
      DATABASE_URL: postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@postgres:${POSTGRES_PORT}/${POSTGRES_DB}
      BACKEND_PORT: ${BACKEND_PORT}
      BACKEND_INTERFACE: ${BACKEND_INTERFACE}
      BACKEND_ALLOWED_ORIGINS: ${BACKEND_ALLOWED_ORIGINS}
      BACKEND_LOG_FILTER_LEVEL: ${BACKEND_LOG_FILTER_LEVEL}
      TIPTAP_URL: ${TIPTAP_URL}
      TIPTAP_AUTH_KEY: ${TIPTAP_AUTH_KEY}
      TIPTAP_JWT_SIGNING_KEY: ${TIPTAP_JWT_SIGNING_KEY}
    ports:
      - "${BACKEND_PORT}:${BACKEND_PORT}"
    depends_on:
      - postgres
    networks:
      - backend_network
    command: ["sh", "-c", "sleep 5 && /usr/local/bin/refactor_platform_rs"]
  
  nextjs-app:
    build:
      context: https://github.com/refactor-group/refactor-platform-fe.git#main
      dockerfile: Dockerfile
      target: runner
      args:
        NEXT_PUBLIC_BACKEND_SERVICE_PROTOCOL: ${BACKEND_SERVICE_PROTOCOL}
        NEXT_PUBLIC_BACKEND_SERVICE_PORT: ${BACKEND_PORT}
        NEXT_PUBLIC_BACKEND_SERVICE_HOST: ${BACKEND_SERVICE_HOST}
        NEXT_PUBLIC_BACKEND_API_VERSION: ${BACKEND_API_VERSION}
        FRONTEND_SERVICE_PORT: ${FRONTEND_SERVICE_PORT}
        FRONTEND_SERVICE_INTERFACE: ${FRONTEND_SERVICE_INTERFACE}
    environment:
      NEXT_PUBLIC_BACKEND_SERVICE_PROTOCOL: ${BACKEND_SERVICE_PROTOCOL}
      NEXT_PUBLIC_BACKEND_SERVICE_PORT: ${BACKEND_PORT}
      NEXT_PUBLIC_BACKEND_SERVICE_HOST: ${BACKEND_SERVICE_HOST}
      NEXT_PUBLIC_BACKEND_API_VERSION: ${BACKEND_API_VERSION}
    ports:
      - "${FRONTEND_SERVICE_PORT}:${FRONTEND_SERVICE_PORT}"
    depends_on:
      - rust-app

networks:
  backend_network:
    driver: bridge

volumes:
  postgres_data
```

---

## Building and Running the Application

### **1. Build the Rust Backend Image**

```bash
docker buildx build --platform linux/amd64,linux/arm64 -t rust-backend .
```

This builds the image for both `amd64` and `arm64` architectures. Use the `--platform` flag to build for a specific architecture.

### **2. Build and Run with Docker Compose**

#### For Local PostgreSQL:

```bash
docker-compose --env-file .env.local up --build
```

#### For Remote PostgreSQL:

```bash
docker-compose --env-file .env.remote-db up --build
```

The web API will be accessible at `http://localhost:<SERVICE_PORT>`

---

## Database Utilities

### **Rebuild the Database**

```bash
docker-compose run rust-app rebuild-db
```

### **Seed the Database**

```bash
docker-compose run rust-app seed-db
```

### **Convert DBML to SQL**

If you have a DBML file (`schema.dbml`), convert it to SQL:

```bash
docker-compose run -v $(pwd)/sql:/app/sql -v $(pwd)/schema.dbml:/app/schema.dbml rust-app dbml2sql
```

```bash
docker-compose run -v $(pwd)/sql:/app/sql -v $(pwd)/schema.dbml:/app/schema.dbml rust-app dbml2sql
```

---

## Managing Containers

### **Stop Containers**

```bash
docker-compose down
```

### **Remove Containers, Networks, and Volumes**

```bash
docker-compose down -v
```

---

## Troubleshooting

### **Cannot Connect to PostgreSQL**

1. Verify PostgreSQL is running:

   ```bash
   docker-compose ps
   ```

2. Check logs for PostgreSQL:

   ```bash
   docker-compose logs postgres
   ```

### **Web API Not Accessible**

1. Verify the container is running:

   ```bash
   docker-compose ps
   ```

2. Check logs for the Rust app:

   ```bash
   docker-compose logs rust-app
   ```

3. Confirm the correct port in `.env`:

   ```bash
   SERVICE_PORT=4000
   ```

### **Port Conflicts**

Change the ports in `.env` or `docker-compose.yaml`:

```yaml
services:
  postgres:
    ports:
      - "5433:5432"

  rust-app:
    ports:
      - "9090:8080"
```

### **Rebuild After Changes**

```bash
docker-compose build
docker-compose up
```

### **Database Persistence**

Ensure volumes are configured in `docker-compose.yaml`:

```yaml
volumes:
  postgres_data:
```

---

## Development Tips

- Run containers in detached mode:

  ```bash
  docker-compose up -d
  ```

- Access a running container:
  
  ```bash
  docker exec -it <container_name> bash
  ```

- Restart a single service:
  
  ```bash
  docker-compose restart rust-app
  ```

---

## Interactive Testing

- Test interactively:
  
  ```bash
  docker run -it rust-backend:latest
  ```

- Debug inside the container:
  
  ```bash
  docker run -it --entrypoint /bin/bash rust-backend:latest
  ```

---

# GitHub Actions Workflow for Container Deployment

### 🚀 Workflow Overview: Build, Test, and Deploy with Containers

This workflow automates the process of building, testing, and deploying the Refactor Coaching & Mentoring Platform using Docker containers. It's triggered on pushes to branches other than `main`, pull requests to `main`, and can also be manually triggered.

### ⚙️ Key Components

1.  **Environment Setup**:
    *   Defines environment variables like `REGISTRY` (ghcr.io), `IMAGE_NAME`, `BACKEND_IMAGE_NAME`, and `FRONTEND_IMAGE_NAME`.
    *   Sets up secrets for PostgreSQL credentials, ports, and other configurations. These secrets are stored securely in GitHub.

2.  **Build and Test Job (`build_test_run`)**:
    *   Runs on Ubuntu.
    *   Checks out the code using `actions/checkout@v4`.
    *   Sets environment variables from GitHub secrets.
    *   Installs the Rust toolchain using `dtolnay/rust-toolchain@stable`.
    *   Caches dependencies using `Swatinem/rust-cache@v2` to speed up subsequent builds.
    *   Installs `sea-orm-cli`.
    *   Builds the Rust project using `cargo build --all-targets`.
    *   Runs tests using `cargo test`.

3.  **Build and Push Docker Images Job (`build_and_push_docker`)**:
    *   Depends on the `build_test_run` job to ensure tests pass before building images.
    *   Logs into the GitHub Container Registry (ghcr.io) using `docker/login-action@v2`.
    *   Sets up Docker Buildx using `docker/setup-buildx-action@v3` for multi-platform builds (amd64 and arm64).
    *   Caches Docker layers using `actions/cache@v3` to speed up image builds.
    *   Extracts metadata for Docker images using `docker/metadata-action@v4`.
    *   Builds and pushes the Rust backend image using `docker/build-push-action@v6`.
        *   Context: The root directory (`.`).
        *   Dockerfile: Uses the Dockerfile in the root.
        *   Tags: Creates tags for the image, including `latest` and a tag based on the Git SHA.
    *   Builds and pushes the Next.js frontend image using `docker/build-push-action@v6`.
        *   Context: The web directory.
        *   Dockerfile: Uses the Dockerfile.
        *   Tags: Creates tags for the image, similar to the backend.
    *   Generates artifact attestation for both images using `actions/attest-build-provenance@v2`.

### 🛠️ Rust Workspace and Build Process

*   **Rust Workspace**: The project is structured as a Rust workspace, defined by the main Cargo.toml file. This allows managing multiple related crates (e.g., entity, entity_api, migration, service, web) in a single repository.
*   **Build Targets**: The `cargo build --all-targets` command builds all binaries, examples, and tests defined in the workspace.
*   **Release Build**: The Dockerfile uses `cargo build --release` to create optimized release builds.

### 🐳 Docker and Docker Compose

*   **Docker**: Docker is used to containerize the Rust backend and Next.js frontend applications. Each application has its own Dockerfile that specifies the build environment, dependencies, and entry point.
*   **Docker Compose**: While the workflow doesn't directly use `docker-compose`, the `docker-compose.yaml` file defines how the different services (e.g., backend, frontend, database) are orchestrated and linked together for local development.

### 📦 GitHub Container Registry (GHCR)

*   The workflow pushes the built Docker images to the GitHub Container Registry (GHCR). GHCR is a container registry provided by GitHub that allows storing and managing Docker images alongside the code.
*   Images are tagged with `latest` and the Git SHA for versioning.

### ✅ Improvements and Optimizations

1.  **Multi-Arch Builds**: The workflow already supports multi-architecture builds (amd64 and arm64), which is great for deploying to different platforms.
2.  **Cache**: Docker layer caching is implemented to speed up builds.
3.  **Secrets**: Secrets are used to securely manage sensitive information.

### 📝 Summary for Newcomers

This GitHub Actions workflow automates building, testing, and deploying our Rust-based platform using Docker containers. Here's the gist:

1  **Code Changes**: When code is pushed (excluding `main` branch) or a pull request is made to `main`, the workflow kicks off.
2.  **Build & Test**: It builds the Rust code and runs tests to ensure everything works.
3.  **Containerize**: It creates Docker images for the backend and frontend.
4.  **Push to GHCR**: It pushes these images to GitHub's container registry (GHCR).

This setup ensures that our application is automatically built, tested, and containerized whenever we make changes, making deployment a breeze! 🌬️

### ⚠️ Potential Corrections

1.  **Workflow Triggers**: Consider adding a trigger for the `main` branch to rebuild and deploy on merges to main.
2.  **Image Tagging**: Implement a more robust tagging strategy (e.g., semantic versioning) for production releases.
3.  **Deployment**: The workflow currently builds and pushes images but doesn't deploy them. Add a deployment step to deploy the images to a staging or production environment.
4.  **Error Handling**: Implement error handling and logging to provide better insights into workflow failures.