#!/bin/bash

# Define default variables
DB_NAME=${1:-"refactor_platform"}
DB_USER=${2:-"refactor"}
SCHEMA_NAME=${3:-"refactor_platform"}

echo "Using the following configuration:"
echo "Database Name: $DB_NAME"
echo "Database User: $DB_USER"
echo "Schema Name: $SCHEMA_NAME"

# Check if postgres is installed with its client CLI
[ -f $(which postgres) ] &&
[ -f $(which pg_ctl ) ] &&
[ -f $(which psql) ] > /dev/null 2>&1 ||
{ echo "Postgres and psql are not completely installed. Please install postgres with your package manager or Postgres.app and try again"; exit 1; }

# if [[ -z "${PGDATA}" ]]; then
#     echo 'Environment variable PGDATA unset. See `pg_ctl --help for more information.'
# fi

# Ensure postgres is running and start postgres with homebrew if it is not running
pg_isready -h $POSTGRES_HOST -p $POSTGRES_PORT -U $POSTGRES_USER -d $POSTGRES_DB
#pg_ctl status > /dev/null 2>&1 || { echo "Starting Postgres..."; pg_ctl -w -t 15 start; }

# Check if the postgres database exists and create it if it doesn't
POSTGRES_DB_EXISTS=$(psql $POSTGRES_URL -tAc "SELECT 1 FROM pg_database WHERE datname='postgres'")
if [ "$POSTGRES_DB_EXISTS" != "1" ]; then
    echo "Creating 'postgres' database..."
    createdb -U postgres postgres || { echo "Failed to create 'postgres' database"; exit 1; }
fi

# Check if the user exists and create it if it doesn't
USER_EXISTS=$(psql $POSTGRES_URL -tAc "SELECT 1 FROM pg_roles WHERE rolname='$DB_USER'")
if [ "$USER_EXISTS" != "1" ]; then
    echo "Creating user $DB_USER..."
    psql -U postgres -c "CREATE USER $DB_USER;" || { echo "Failed to create user $DB_USER"; exit 1; }
fi

# Check if the database exists
DB_EXISTS=$(psql $POSTGRES_URL -tAc "SELECT 1 FROM pg_database WHERE datname='$DB_NAME'")

# If the database exists, drop it
if [ "$DB_EXISTS" = "1" ]; then
    echo "Database $DB_NAME exists. Dropping the database..."
    psql $POSTGRES_URL -c "DROP DATABASE IF EXISTS $DB_NAME;" || { echo "Failed to drop database $DB_NAME"; exit 1; }
fi

echo "Creating the database $DB_NAME..."
psql $POSTGRES_URL -c "CREATE DATABASE $DB_NAME;" || { echo "Failed to create database $DB_NAME"; exit 1; }

echo "Granting privileges to $DB_USER on $DB_NAME..."
psql $POSTGRES_URL -c "GRANT ALL PRIVILEGES ON DATABASE $DB_NAME TO $DB_USER;"

# Check if the schema exists
SCHEMA_EXISTS=$(psql $POSTGRES_URL -d $DB_NAME -tAc "SELECT 1 FROM information_schema.schemata WHERE schema_name = '$SCHEMA_NAME'")

# If the schema does not exist, create it
if [ "$SCHEMA_EXISTS" != "1" ]; then
    echo "Creating schema $SCHEMA_NAME..."
    psql $POSTGRES_URL -d $DB_NAME -c "CREATE SCHEMA $SCHEMA_NAME;" || { echo "Failed to create schema $SCHEMA_NAME"; exit 1; }
fi

# Generating SQL for the migrations using dbml2sql
echo "Generating SQL for the migrations..."
dbml2sql docs/db/refactor_platform_rs.dbml -o migration/src/refactor_platform_rs.sql || { echo "Error generating SQL file"; exit 1; }

# Remove the line to create a schema from the generated SQL file
echo "Modifying the generated SQL file..."
sed -i '' '/CREATE SCHEMA/d' migration/src/refactor_platform_rs.sql

echo "Running the migrations..."
DATABASE_URL=$POSTGRES_URL sea-orm-cli migrate up -s $SCHEMA_NAME || { echo "Failed to run migrations"; exit 1; }

echo "Database setup and migrations completed successfully"