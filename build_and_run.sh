#!/bin/bash
# filepath: build_and_run.sh
# This script builds and runs the Docker images using docker-compose.
# It uses the .env file (default: .env.local) for environment variables.
# Usage: ./build_and_run.sh [optional_env_file]
# If you don't pass an env file, it defaults to .env.local.

set -e

# Use first argument as env file if provided, else default to .env.local
ENV_FILE=${1:-.env.local}

if [ ! -f "$ENV_FILE" ]; then
  echo "ERROR: ${ENV_FILE} not found. Please create it with the required environment variables."
  exit 1
fi

echo "Using environment file: ${ENV_FILE}"

# Export ENV_FILE variable so docker-compose can use it
export ENV_FILE

echo "Building and running Docker images via docker-compose using ${ENV_FILE}..."
docker-compose --env-file="${ENV_FILE}" up --build -d

echo "Docker containers are up and running on your localhost."
echo "To view logs, run: docker-compose logs -f"
echo "To stop the containers, run: docker-compose down"
