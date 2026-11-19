# shellcheck shell=bash
# Shared by deploy_to_do.yml and rollback_do.yml, which send it ahead of their
# own script to the production host. It defines functions only. Callers set
# SITE_HOST (the site's host name) and CHECK_COLLAB (true to health-check
# docs-collab), and run from the directory holding docker-compose.yaml.

# Print what each container runs: its tag, the git commit it was built from
# (the image's org.opencontainers.image.revision label), image ID, and
# registry digest.
print_images() {
  local name tag image_id revision digest
  for name in "$@"; do
    read -r tag image_id < <(docker inspect -f '{{.Config.Image}} {{.Image}}' "$name" 2>/dev/null) \
      || { echo "INFO: $name is not running"; continue; }
    revision=$(docker inspect -f '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$name" 2>/dev/null || true)
    [ -n "$revision" ] && [ "$revision" != "<no value>" ] || revision=unknown
    digest=$(docker image inspect -f '{{index .RepoDigests 0}}' "$image_id" 2>/dev/null || true)
    echo "INFO: $name runs $tag commit=$revision image=$image_id digest=${digest#*@}"
  done
}

# Static nginx upstreams resolve container IPs only at config load, so reload
# after any container is recreated or restarted.
reload_nginx() {
  local _
  for _ in $(seq 1 10); do
    if docker exec nginx-reverse-proxy nginx -t >/dev/null 2>&1 \
      && docker exec nginx-reverse-proxy nginx -s reload; then
      echo "INFO: nginx reloaded"
      return 0
    fi
    sleep 2
  done
  docker exec nginx-reverse-proxy nginx -t || true
  return 1
}

check_url() {
  curl -fsS -o /dev/null --max-time 5 --resolve "$SITE_HOST:443:127.0.0.1" "https://$SITE_HOST$1"
}

wait_for() {
  local _
  for _ in $(seq 1 45); do
    if "$@" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

# The backend and docs-collab readiness checks also confirm each can reach its
# database.
healthy() {
  wait_for check_url /api/health/ready || { echo "ERROR: backend is not healthy"; return 1; }
  wait_for check_url / || { echo "ERROR: frontend is not healthy"; return 1; }
  if [ "$CHECK_COLLAB" = "true" ]; then
    wait_for check_url /collab/health/ready || { echo "ERROR: docs-collab is not healthy"; return 1; }
  fi
  echo "INFO: ✅ backend, frontend$([ "$CHECK_COLLAB" = "true" ] && echo ", docs-collab") healthy"
}

# Succeeds only when the given backend image knows every applied migration.
migrations_known_by() {
  local status=0
  docker tag "$1" refactor-rollback/gate:candidate
  # </dev/null: stdin is the rest of the caller's script, which `compose run`
  # would consume.
  BACKEND_IMAGE_NAME=refactor-rollback/gate:candidate \
    docker compose run --rm --no-deps -T --name "migration-gate-$$" migrator migrationctl status </dev/null || status=$?
  docker rmi refactor-rollback/gate:candidate >/dev/null || true
  return $status
}
