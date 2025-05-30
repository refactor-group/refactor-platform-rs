# Refactor Platform Deployment Runbook

## Platform Container Service Management

Currently, we use a systemd service definition to manage starting, stopping and restarting the container instances. This
is located at `/etc/systemd/system/refactor-platform.service` and is manually put into place and updated when necessary (very rare).

Once installed and enabled, systemd will manage starting the right service dependencies (e.g. docker, network) before this gets started.
If the service crashes, it should automatically get restarted.

Do not start/stop the production instance manually using docker compose. This service definition uses docker compose to start/stop the
platform containers.

**To start the containers (as deploy user):** `systemctl start refactor-platform.service`

**To stop the containers (as deploy user):** `systemctl stop refactor-platform.service`

**To restart the containers (as deploy user):** `systemctl restart refactor-platform.service`

**To view the containers' systemd logs (as deploy user):** `journalctl -xeu refactor-platform.service`
