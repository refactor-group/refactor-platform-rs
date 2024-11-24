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

#### **For Local PostgreSQL (Docker-based)**:
- Create a `.env.local` file based on the template below and specify `POSTGRES_HOST=postgres`.

#### **For Remote PostgreSQL**:
- Create a `.env.remote-db` file and set `POSTGRES_HOST` to the external IP or hostname of the remote PostgreSQL instance.

Example `.env.local`:
```env
POSTGRES_USER=refactor
POSTGRES_PASSWORD=password
POSTGRES_DB=refactor
POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_SCHEMA=refactor_platform
DATABASE_URL=postgres://refactor:password@postgres:5432/refactor_platform
SERVICE_PORT=4000
SERVICE_INTERFACE=0.0.0.0
USERNAME=appuser
USER_UID=1000
USER_GID=1000
PLATFORM=linux/arm64
```

Example `.env.remote-db`:
```env
POSTGRES_USER=remote_refactor
POSTGRES_PASSWORD=remote_password
POSTGRES_DB=refactor
POSTGRES_HOST=postgres.example.com
POSTGRES_SCHEMA=refactor_platform
DATABASE_URL=postgres://remote_refactor:remote_password@postgres.example.com:5432/refactor_platform
POSTGRES_PORT=5432
SERVICE_PORT=4000
SERVICE_INTERFACE=0.0.0.0
USERNAME=remote_appuser
USER_UID=1001
USER_GID=1001
PLATFORM=linux/arm64
```

### 3. **Review `docker-compose.yaml`**

The `docker-compose.yaml` file uses environment variables defined in your `.env` files.

```yaml
services:
  postgres:
    image: postgres:17
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
    ports:
      - "${POSTGRES_PORT}:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  rust-app:
    build:
      context: .
      dockerfile: Dockerfile
      target: runtime
    platform: ${PLATFORM}
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_HOST: ${POSTGRES_HOST}
      POSTGRES_PORT: ${POSTGRES_PORT}
      DATABASE_URL: ${POSTGRES_HOST}://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}
      SERVICE_PORT: ${SERVICE_PORT}
    ports:
      - "${SERVIE_PORT}:4000"
    depends_on:
      - postgres
volumes:
  postgres_data:
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

The web API will be accessible at `http://localhost:<SERVICE_PORT>` .

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
