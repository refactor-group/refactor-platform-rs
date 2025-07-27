#!/bin/bash
  # /usr/local/bin/renew-certs.sh
  # Let's Encrypt certificate renewal script for containerized nginx

  # Set strict error handling
  set -euo pipefail

  # Define paths
  WEBROOT_PATH="./nginx/html"
  CONTAINER_NAME="nginx-reverse-proxy"
  CERTBOT_TIMEOUT="300"

  # Function to log messages
  log() {
      echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
  }

  # Validate sudo access before proceeding
  log "Validating sudo access..."
  if ! sudo -n true 2>/dev/null; then
      log "ERROR: sudo access required for certbot operations"
      exit 1
  fi

  # Check if timeout command is available
  if ! command -v timeout &> /dev/null; then
      log "WARNING: timeout command not available, proceeding without timeout protection"
      CERTBOT_TIMEOUT=""
  fi

  # Check if container is running
  if ! docker ps --filter "name=${CONTAINER_NAME}" --filter "status=running" --quiet | grep -q .; then
      log "ERROR: Container ${CONTAINER_NAME} is not running"
      exit 1
  fi

  log "Starting certificate renewal process"

  # Attempt to renew certificates with timeout protection (if available)
  if [ -n "${CERTBOT_TIMEOUT}" ]; then
      CERTBOT_CMD="timeout ${CERTBOT_TIMEOUT} sudo certbot renew --webroot -w ${WEBROOT_PATH} --quiet"
  else
      CERTBOT_CMD="sudo certbot renew --webroot -w ${WEBROOT_PATH} --quiet"
  fi

  if eval "${CERTBOT_CMD}"; then
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