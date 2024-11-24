[![Build & Tests (backend)](https://github.com/Jim-Hodapp-Coaching/refactor-platform-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Jim-Hodapp-Coaching/refactor-platform-rs/actions/workflows/ci.yml)

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

## Starting the Backend

To run the backend directly outside of a container:

```bash
cargo run  -- -l DEBUG -d postgres://refactor:password@localhost:5432/refactor_platform
```

This will start the backend with log level DEBUG and attempt to connect to a Postgres DB server on the same machine with user `refactor` and password `password` on port `5432` and selecting the database named `refactor_platform`.

## Project Directory Structure

`docs` - project documentation including architectural records, DB schema, API docs, etc

`entity_api` - data operations on the various `Entity` models

`entity` - shape of the data models and the relationships to each other

`migration` - relational DB SQL migrations

`scripts` - contains handy developer-related scripts that make working with this codebase more straightforward

`service` - CLI flags, environment variables, config handling and backend daemon setup

`src` - contains a main function that initializes logging and calls all sub-services

`web` - API endpoint definition, routing, handling of request/responses, controllers
