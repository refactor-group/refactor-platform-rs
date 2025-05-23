[![Build & Tests (backend)](https://github.com/Jim-Hodapp-Coaching/refactor-platform-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Jim-Hodapp-Coaching/refactor-platform-rs/actions/workflows/ci.yml) [![Build and Deploy Containers](https://github.com/refactor-group/refactor-platform-rs/actions/workflows/build_and_deploy_containers.yml/badge.svg)](https://github.com/refactor-group/refactor-platform-rs/actions/workflows/build_and_deploy_containers.yml)

# Refactor Coaching & Mentoring Platform

## Backend

## Intro

A Rust-based backend that provides a web API for various client applications (e.g. a web frontend) that facilitate the coaching and mentoring of software engineers.

The platform itself is useful for professional independent coaches, informal mentors and engineering leaders who work with individual software engineers and/or teams by providing a single application that facilitates and enhances your coaching practice.

## Basic Local DB Setup and Management

## Running the Database Setup Script

1. Ensure you have PostgreSQL installed and running on your machine. If you're using macOS, you can use
[Postgres.app](https://postgresapp.com/) or install it with Homebrew:

    ```shell
    brew install postgresql
    ```

2. Make sure you have the `dbml2sql` and SeaORM CLI tools installed. You can install them with:

    ```shell
    npm install -g @dbml/cli
    ```

    ```shell
    cargo install sea-orm-cli
    ```

3. Run the script with default settings:

    ```shell
    ./scripts/rebuild_db.sh
    ```

    This will create a database named `refactor_platform`, a user named `refactor`, and a schema named `refactor_platform`.

4. If you want to use different settings, you can provide them as arguments to the script:

    ```shell
    ./scripts/rebuild_db.sh my_database my_user my_schema
    ```

    This will create a database named `my_database`, a user named `my_user`, and a schema named `my_schema`.

5. If you want seeded test data in your database, run:

   ```shell
   cargo run --bin seed_db
   ```

Please note that the script assumes that the password for the new PostgreSQL user is `password`. If you want to use a different password, you'll need to modify the script accordingly.

## Starting the Backend

To run the backend directly outside of a container:

The first example will start the backend with log level DEBUG and attempt to connect to a Postgres DB server on the same machine with user `refactor` and password `password` on port `5432` and selecting the database named `refactor_platform`.

```bash
cargo run --  --tiptap-url https://<TIPTAP_APP_ID>.collab.tiptap.cloud --tiptap-auth-key=<TIPTAP_API_SECRET> --tiptap-jwt-signing-key=<TIPTAP_CLOUD_APP_SECRET> --tiptap-app-id=<TIPTAP_APP_ID>
```

To run with a custom Postgresql connection string:

```bash
cargo run -- -d postgres://refactor:my_password@localhost:5432/refactor_platform --tiptap-url https://<TIPTAP_APP_ID>.collab.tiptap.cloud --tiptap-auth-key=<TIPTAP_API_SECRET> --tiptap-jwt-signing-key=<TIPTAP_CLOUD_APP_SECRET> --tiptap-app-id=<TIPTAP_APP_ID>
```

To run with an additional list of allowed cross-site network origins:

```bash
cargo run -- --allowed-origins="http://192.168.1.2:3000,https://192.168.1.2:3000" --tiptap-url https://<TIPTAP_APP_ID>.collab.tiptap.cloud --tiptap-auth-key=<TIPTAP_API_SECRET> --tiptap-jwt-signing-key=<TIPTAP_CLOUD_APP_SECRET> --tiptap-app-id=<TIPTAP_APP_ID>
```

---

## Basic Container DB Setup and Management

_This Rust-based backend/web API connects to a PostgreSQL database. It uses Docker and Docker Compose for local development and deployment, including utilities for database management and migrations. You can run PostgreSQL locally (via Docker) or remotely by configuring environment variables._

---

### Building and Running Locally or Remotely in Containers

1. **Install Prerequisites**:
   - [Docker](https://www.docker.com/products/docker-desktop) (20+)
   - [Docker Compose](https://docs.docker.com/compose/install/) (1.29+)

2. **Clone the Repository**:

   ```bash
   git clone <repository-url>
   cd <repository-directory>
   ```

3. **Set Environment Variables**:
   - For **local PostgreSQL**, create a `.env.local` file and set `POSTGRES_HOST=postgres`.
   - For **remote PostgreSQL**, use a `.env.remote-db` file with `POSTGRES_HOST` pointing to the external database.

4. **Build and Start the Platform**:
   - Local PostgreSQL:

     ```bash
     docker-compose --env-file .env.local up --build
     ```

   - Remote PostgreSQL:

     ```bash
     docker-compose --env-file .env.remote-db up --build
     ```

5. **Access the API**:
   - Visit `http://localhost:<SERVICE_PORT>` in your browser or API client.

### Key Commands

- **Stop all containers**:

  ```bash
  docker-compose down
  ```
  
   **Note**: This will stop all containers, including the database.
  
- **Rebuild and restart**:

  ```bash
  docker-compose up --build
  ```

- **View logs**:

  ```bash
  docker-compose logs <service>
  ```

_For additional commands, database utilities, and debugging tips, check the [Container README](docs/runbooks/Container-README.md)._

---

## Project Directory Structure

`docs` - project documentation including architectural records, DB schema, API docs, etc

`domain` - Layer of abstraction above `entity_api` and intended to encapsulate most business logic. Ex. interactions between `entity_api` and network calls to the outside world.

`entity_api` - data operations on the various `Entity` models

`entity` - shape of the data models and the relationships to each other

`migration` - relational DB SQL migrations

`scripts` - contains handy developer-related scripts that make working with this codebase more straightforward

`service` - CLI flags, environment variables, config handling and backend daemon setup

`src` - contains a main function that initializes logging and calls all sub-services

`web` - API endpoint definition, routing, handling of request/responses, controllers

---

## Advanced / Manual DB operations

### Set Up Database Manually

Note: these are commands meant to run against a real Postgresql server with an admin level user.

```sql
--create new database `refactor_platform`
CREATE DATABASE refactor_platform;
```

Change to the refactor_platform DB visually if using app like Postico, otherwise change using the
Postgresql CLI:

```sh
\c refactor_platform
```

```sql
--create new database user `refactor`
CREATE USER refactor WITH PASSWORD 'password';
--create a new schema owned by user `refactor`
CREATE SCHEMA IF NOT EXISTS refactor_platform AUTHORIZATION refactor;
--Check to see that the schema `refactor_platform` exists in the results
SELECT schema_name FROM information_schema.schemata;
--Grant all privileges on schema `refactor_platform` to user `refactor`
GRANT ALL PRIVILEGES ON SCHEMA refactor_platform TO refactor;
```

### Run Migrations

Note: this assumes a database name of `refactor_platform`

```bash
DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform sea-orm-cli migrate up -s refactor_platform
```

### Generate a new Entity from Database

Note that to generate a new Entity using the CLI you must ignore all other tables using the `--ignore-tables` option. You must add the option for _each_ table you are ignoring.

```bash
 DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform sea-orm-cli generate entity  -s refactor_platform -o entity/src -v --with-serde both --serde-skip-deserializing-primary-key --ignore-tables {table to ignore} --ignore-tables {other table to ignore}
```
