#!/bin/bash
  # /usr/local/bin/renew-certs.sh
  # Let's Encrypt certificate renewal script for containerized nginx

  # Set strict error handling
  set -euo pipefail

  # Define paths
  WEBROOT_PATH="./nginx/html"
  CONTAINER_NAME="nginx-reverse-proxy"

  # Function to log messages
  log() {
      echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
  }

  # Check if container is running
  if ! docker ps --filter "name=${CONTAINER_NAME}" --filter "status=running" --quiet | grep -q .; then
      log "ERROR: Container ${CONTAINER_NAME} is not running"
      exit 1
  fi

  log "Starting certificate renewal process"

  # Attempt to renew certificates (requires sudo for system directories)
  if sudo certbot renew --webroot -w "${WEBROOT_PATH}" --quiet; then
      log "Certificate renewal successful"

      # Reload nginx configuration
      if docker exec "${CONTAINER_NAME}" nginx -s reload; then
          log "nginx configuration reloaded successfully"
      else
          log "ERROR: Failed to reload nginx configuration"
          exit 1
      fi

      log "Certificate renewal process completed successfully"
  else
      log "ERROR: Certificate renewal failed"
      exit 1
  fi