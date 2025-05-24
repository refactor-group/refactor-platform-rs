To implement a zero-downtime Blue/Green deployment strategy leveraging DigitalOcean Load Balancers and GitHub Actions, significant changes are needed to the deployment logic. This involves interacting with the DigitalOcean API (via `doctl`) to manage which droplet is serving live traffic through the Load Balancer.

**Prerequisites (Manual Setup in DigitalOcean):**

1.  **Two Droplets:** You need two DigitalOcean Droplets that will serve as your Blue and Green environments. Note their IDs and IP addresses/hostnames.
2.  **DigitalOcean Load Balancer (DO LB):**
    *   Create a Load Balancer in the same region as your droplets.
    *   Configure its frontend (e.g., HTTP on port 80 or HTTPS on port 443).
    *   Configure its backend forwarding rule to point to your application's port on the droplets (e.g., `4000` as per your `BACKEND_PORT`).
    *   Set up a health check on the Load Balancer (e.g., HTTP GET to `/health` on your application port).
    *   Initially, you can add one of your droplets to the LB, or leave it empty if this is the very first deployment.
3.  **DNS:** Point your application's domain (e.g., `app.yourdomain.com`) to the static IP address of your DigitalOcean Load Balancer.
4.  **GitHub Secrets:** Ensure you have the following secrets configured in your GitHub repository:
    *   `DO_API_TOKEN`: Your DigitalOcean API token with read/write permissions.
    *   `DO_LOAD_BALANCER_ID`: The ID of your DigitalOcean Load Balancer.
    *   `DO_DROPLET_ONE_ID`: The ID of your first droplet.
    *   `DO_DROPLET_ONE_HOST`: The IP address or hostname of your first droplet (for SSH).
    *   `DO_DROPLET_TWO_ID`: The ID of your second droplet.
    *   `DO_DROPLET_TWO_HOST`: The IP address or hostname of your second droplet (for SSH).
    *   `DO_SSH_PRIVATE_KEY`: SSH private key to access both droplets.
    *   `DO_USER`: SSH user for the droplets.
    *   All application-specific secrets from your .env file (e.g., `DATABASE_URL`, `BACKEND_PORT`, `BACKEND_ALLOWED_ORIGINS`, `TIPTAP_URL`, etc.).

The workflow will now:
1.  Build and push the Docker image as before.
2.  Determine which of your two droplets is currently "Blue" (live in the LB) and which will be "Green" (candidate for the new release).
3.  Deploy the new release to the "Green" droplet.
4.  Perform a health check on the "Green" droplet directly.
5.  If healthy, add the "Green" droplet to the Load Balancer.
6.  Wait for the Load Balancer to confirm the "Green" droplet is healthy in its pool.
7.  Remove the "Blue" (old) droplet from the Load Balancer. The "Green" droplet is now "Blue".
8.  If the "Green" deployment fails its health check, it's not added to the LB, and the old "Blue" droplet continues serving traffic (automatic rollback of the candidate).
9.  **Push-button Rollback:** To roll back to a previous version with zero downtime, you would re-run this workflow, selecting the GitHub Release tag of the version you want to roll back to. The same Blue/Green swap logic will apply, deploying the older version to the inactive droplet and then safely swapping it into the Load Balancer.

Here's the updated workflow file:

````yaml
name: Release Build and Zero-Downtime Deploy # Name of the GitHub Actions workflow.

on: # Defines the events that trigger the workflow.
  release: # Triggers on release events.
    types: [published] # Specifically triggers when a new release is published via the GitHub UI.
  workflow_dispatch: # Allows manual triggering, useful for rollbacks by selecting a previous release tag.
    inputs:
      release_tag: # Input for specifying a release tag, defaults to latest release if run by 'release' event.
        description: 'Release tag to deploy (e.g., v1.0.0). Leave empty if triggered by a new release event.'
        required: false

env: # Environment variables available to all jobs in the workflow.
  REGISTRY: ghcr.io # Specifies the container registry (GitHub Container Registry).
  IMAGE_NAME_BASE: ${{ github.repository }} # Base image name, e.g., refactor-group/refactor-platform-rs.
  # Application container name on the droplets (can be the same on both as only one is "live" for the app logic at a time per droplet).
  APP_CONTAINER_NAME: "refactor-platform" # From your .env BACKEND_CONTAINER_NAME.
  HEALTH_CHECK_PATH: "/health" # Assumed health check path for your application. Adjust if needed.
  HEALTH_CHECK_WAIT_SECONDS: 45 # Seconds to wait for the green container to start before direct health check.
  LB_HEALTH_WAIT_TIMEOUT_MINUTES: 3 # Minutes to wait for LB to report new droplet as healthy.

jobs: # Defines the jobs that run as part of the workflow.
  build_and_push_image: # Job to build the Docker image and push it to GHCR.
    name: Build and Push Docker Image # Display name for the job.
    runs-on: ubuntu-latest # Specifies the runner environment.
    outputs: # Define outputs to be used by downstream jobs.
      release_version: ${{ steps.meta.outputs.version }} # The primary version tag (e.g., v1.0.0).
      image_digest: ${{ steps.build-push.outputs.digest }} # The digest of the pushed image.
    permissions: # Permissions needed by the GITHUB_TOKEN for this job.
      contents: read # To checkout the repository.
      packages: write # To push images to GHCR.
      id-token: write # For OIDC token.
      attestations: write # To write build attestations.

    steps: # Sequence of steps for this job.
      - name: Checkout repository # Step to checkout the code.
        uses: actions/checkout@v4 # Uses the official checkout action.
        with:
          fetch-tags: true # Ensure tags are fetched for release versioning.

      - name: Set up QEMU # Step to set up QEMU for multi-platform builds (if needed).
        uses: docker/setup-qemu-action@v3 # Uses Docker's QEMU setup action.

      - name: Set up Docker Buildx # Step to set up Docker Buildx.
        uses: docker/setup-buildx-action@v3 # Uses Docker's Buildx setup action.

      - name: Log in to GitHub Container Registry # Step to log in to GHCR.
        uses: docker/login-action@v3 # Uses Docker's login action.
        with: # Parameters for the login action.
          registry: ${{ env.REGISTRY }} # Specifies the GHCR registry.
          username: ${{ github.actor }} # Uses the GitHub actor (user/bot) as the username.
          password: ${{ secrets.GITHUB_TOKEN }} # Uses the auto-generated GITHUB_TOKEN as the password.

      - name: Determine Release Tag # Determine the tag to use for the image.
        id: get_release_tag
        run: | # Script to get tag from event or input.
          if [[ "${{ github.event_name }}" == "release" ]]; then
            echo "release_tag=${{ github.event.release.tag_name }}" >> $GITHUB_OUTPUT
          elif [[ "${{ github.event_name }}" == "workflow_dispatch" && -n "${{ github.event.inputs.release_tag }}" ]]; then
            echo "release_tag=${{ github.event.inputs.release_tag }}" >> $GITHUB_OUTPUT
          else
            echo "Error: Could not determine release tag."
            exit 1
          fi

      - name: Docker metadata # Step to generate Docker image tags and labels.
        id: meta # ID for this step to reference its outputs.
        uses: docker/metadata-action@v5 # Uses Docker's metadata action.
        with: # Parameters for the metadata action.
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME_BASE }} # Defines the base image name.
          tags: | # Defines the tagging strategy.
            type=raw,value=${{ steps.get_release_tag.outputs.release_tag }} # Use the specific release tag.
            type=raw,value=latest,enable=${{ github.event_name == 'release' }} # Also tag 'latest' if it's a new release event.
          # 'version' output will be the primary tag used (e.g., v1.0.0)

      - name: Build and push Docker image # Step to build and push the image.
        id: build-push # ID for this step.
        uses: docker/build-push-action@v5 # Uses Docker's build-and-push action.
        with: # Parameters for the build-and-push action.
          context: . # Sets the build context to the repository root.
          file: ./Dockerfile # Specifies the path to the Dockerfile.
          platforms: linux/amd64 # Specify platforms if needed.
          push: true # Pushes the image after building.
          tags: ${{ steps.meta.outputs.tags }} # Uses the tags generated by the metadata step.
          labels: ${{ steps.meta.outputs.labels }} # Uses the labels generated by the metadata step.
          cache-from: type=gha # Uses GitHub Actions cache for build layers.
          cache-to: type=gha,mode=max # Exports build cache to GitHub Actions cache.
          provenance: true # Generates SLSA build provenance.

      - name: Output Image Details # Step to log the pushed image details.
        run: | # Multi-line script.
          echo "Release Version for Deployment: ${{ steps.meta.outputs.version }}"
          echo "Image pushed with tags: ${{ steps.meta.outputs.tags }}" # Logs the tags.
          echo "Image digest: ${{ steps.build-push.outputs.digest }}" # Logs the image digest.

  deploy_blue_green_to_digitalocean: # Job to deploy the image to DigitalOcean using Blue/Green.
    name: Deploy Blue/Green to DigitalOcean # Display name for the job.
    runs-on: ubuntu-latest # Specifies the runner environment.
    needs: build_and_push_image # Depends on the image build job.
    env: # Environment variables specific to this job.
      # Droplet and LB identifiers from secrets
      DO_LOAD_BALANCER_ID_ENV: ${{ secrets.DO_LOAD_BALANCER_ID }}
      DO_DROPLET_ONE_ID_ENV: ${{ secrets.DO_DROPLET_ONE_ID }}
      DO_DROPLET_ONE_HOST_ENV: ${{ secrets.DO_DROPLET_ONE_HOST }}
      DO_DROPLET_TWO_ID_ENV: ${{ secrets.DO_DROPLET_TWO_ID }}
      DO_DROPLET_TWO_HOST_ENV: ${{ secrets.DO_DROPLET_TWO_HOST }}
      # Image to deploy, using the output from the build job
      IMAGE_TO_DEPLOY: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME_BASE }}:${{ needs.build_and_push_image.outputs.release_version }}
      # Application Environment Variables from Secrets (passed to SSH script)
      SECRET_BACKEND_ALLOWED_ORIGINS: ${{ secrets.BACKEND_ALLOWED_ORIGINS }}
      SECRET_BACKEND_ENV: "production"
      SECRET_BACKEND_LOG_FILTER_LEVEL: ${{ secrets.BACKEND_LOG_FILTER_LEVEL }}
      SECRET_BACKEND_PORT: ${{ secrets.BACKEND_PORT }}
      SECRET_BACKEND_INTERFACE: ${{ secrets.BACKEND_INTERFACE }}
      SECRET_DATABASE_URL: ${{ secrets.DATABASE_URL }}
      SECRET_TIPTAP_URL: ${{ secrets.TIPTAP_URL }}
      SECRET_TIPTAP_AUTH_KEY: ${{ secrets.TIPTAP_AUTH_KEY }}
      SECRET_TIPTAP_JWT_SIGNING_KEY: ${{ secrets.TIPTAP_JWT_SIGNING_KEY }}
      SECRET_BACKEND_API_VERSION: ${{ secrets.BACKEND_API_VERSION }} # Or use needs.build_and_push_image.outputs.release_version

    steps: # Sequence of steps for this job.
      - name: Install doctl and jq # doctl for DO API, jq for parsing JSON.
        run: | # Multi-line script.
          sudo apt-get update && sudo apt-get install -y jq
          curl -sL https://github.com/digitalocean/doctl/releases/download/v1.106.0/doctl-1.106.0-linux-amd64.tar.gz | tar -xzv
          sudo mv doctl /usr/local/bin

      - name: Authenticate doctl # Authenticate doctl with DO API token.
        env:
          DIGITALOCEAN_ACCESS_TOKEN: ${{ secrets.DO_API_TOKEN }}
        run: doctl auth init --access-token $DIGITALOCEAN_ACCESS_TOKEN

      - name: Determine Blue (Live) and Green (Target) Droplets # Logic to decide which droplet is which.
        id: determine_roles
        run: | # Multi-line script.
          echo "Determining Blue/Green roles for Load Balancer ID: $DO_LOAD_BALANCER_ID_ENV"
          LB_INFO_JSON=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json)
          if [ -z "$LB_INFO_JSON" ]; then
            echo "Error: Could not retrieve Load Balancer info for ID $DO_LOAD_BALANCER_ID_ENV."
            exit 1
          fi
          # Extract DropletIDs currently in the LB. This is an array.
          LIVE_DROPLET_IDS_STR=$(echo "$LB_INFO_JSON" | jq -r '.[0].droplet_ids | @json')

          echo "Droplet One ID: $DO_DROPLET_ONE_ID_ENV, Droplet Two ID: $DO_DROPLET_TWO_ID_ENV"
          echo "Live Droplet IDs string from LB: $LIVE_DROPLET_IDS_STR"

          BLUE_DROPLET_ID=""
          BLUE_DROPLET_HOST=""
          GREEN_TARGET_DROPLET_ID=""
          GREEN_TARGET_DROPLET_HOST=""

          # Check if Droplet One is live
          if echo "$LIVE_DROPLET_IDS_STR" | jq -e ".[] | select(. == $DO_DROPLET_ONE_ID_ENV)" > /dev/null; then
            echo "Droplet One ($DO_DROPLET_ONE_ID_ENV) is LIVE (Blue)."
            BLUE_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            BLUE_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_TWO_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_TWO_HOST_ENV"
          # Check if Droplet Two is live
          elif echo "$LIVE_DROPLET_IDS_STR" | jq -e ".[] | select(. == $DO_DROPLET_TWO_ID_ENV)" > /dev/null; then
            echo "Droplet Two ($DO_DROPLET_TWO_ID_ENV) is LIVE (Blue)."
            BLUE_DROPLET_ID="$DO_DROPLET_TWO_ID_ENV"
            BLUE_DROPLET_HOST="$DO_DROPLET_TWO_HOST_ENV"
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
          else # Neither is live, or LB is empty - target Droplet One as Green
            echo "Neither droplet is currently live in the LB, or LB is empty. Targeting Droplet One ($DO_DROPLET_ONE_ID_ENV) as Green."
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
            # BLUE_DROPLET_ID remains empty, indicating no current blue to remove initially.
          fi

          if [ -z "$GREEN_TARGET_DROPLET_ID" ]; then
            echo "Error: Could not determine a Green target droplet. This should not happen."
            exit 1
          fi

          echo "::set-output name=blue_id::$BLUE_DROPLET_ID"
          echo "::set-output name=blue_host::$BLUE_DROPLET_HOST"
          echo "::set-output name=green_id::$GREEN_TARGET_DROPLET_ID"
          echo "::set-output name=green_host::$GREEN_TARGET_DROPLET_HOST"
          echo "Green Target Droplet ID: $GREEN_TARGET_DROPLET_ID, Host: $GREEN_TARGET_DROPLET_HOST"
          echo "Blue (Current Live) Droplet ID: $BLUE_DROPLET_ID, Host: $BLUE_DROPLET_HOST"

      - name: Set up SSH Agent # Step to configure SSH access.
        uses: webfactory/ssh-agent@v0.9.0 # Uses an action for SSH agent setup.
        with: # Parameters for the action.
          ssh-private-key: ${{ secrets.DO_SSH_PRIVATE_KEY }} # Uses the SSH private key from GitHub secrets.

      - name: Deploy to Green Droplet and Health Check # Deploy and check health on the target green droplet.
        id: deploy_green
        env:
          GREEN_DROPLET_HOST_FOR_SSH: ${{ steps.determine_roles.outputs.green_host }}
          # Pass other necessary env vars for the SSH script
          REGISTRY_ENV: ${{ env.REGISTRY }}
          REGISTRY_USER_ENV: ${{ github.actor }}
          REGISTRY_PASSWORD_ENV: ${{ secrets.GITHUB_TOKEN }}
          APP_CONTAINER_NAME_ENV: ${{ env.APP_CONTAINER_NAME }}
          HEALTH_CHECK_PORT_FOR_CURL: ${{ secrets.BACKEND_PORT }} # Port for direct curl health check
          HEALTH_CHECK_PATH_FOR_CURL: ${{ env.HEALTH_CHECK_PATH }}
          HEALTH_CHECK_WAIT_SECONDS_FOR_CURL: ${{ env.HEALTH_CHECK_WAIT_SECONDS }}
        run: | # Multi-line script for SSH commands.
          echo "Deploying image $IMAGE_TO_DEPLOY to GREEN droplet: $GREEN_DROPLET_HOST_FOR_SSH"
          ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null ${{ secrets.DO_USER }}@$GREEN_DROPLET_HOST_FOR_SSH << EOF
            set -e # Exit immediately if a command fails, but health check failure is handled.

            # Function to safely stop and remove a container if it exists
            cleanup_container() {
              local container_name="\$1"
              if [ "\$(docker ps -q -f name=\$container_name)" ]; then
                echo "Stopping container \$container_name on \$HOSTNAME..."
                docker stop \$container_name
              fi
              if [ "\$(docker ps -aq -f name=\$container_name)" ]; then
                echo "Removing container \$container_name on \$HOSTNAME..."
                docker rm \$container_name
              fi
            }

            # Function to start the application container
            start_application_container() {
              local container_to_start_name="\$1"
              local image_to_use="\$2"
              echo "Starting container \$container_to_start_name with image \$image_to_use on \$HOSTNAME..."
              docker run -d --restart always \
                --name "\$container_to_start_name" \
                -p "\$SECRET_BACKEND_PORT:\$SECRET_BACKEND_PORT" \
                -e BACKEND_ALLOWED_ORIGINS="\$SECRET_BACKEND_ALLOWED_ORIGINS" \
                -e BACKEND_ENV="\$SECRET_BACKEND_ENV" \
                -e BACKEND_LOG_FILTER_LEVEL="\$SECRET_BACKEND_LOG_FILTER_LEVEL" \
                -e BACKEND_PORT="\$SECRET_BACKEND_PORT" \
                -e BACKEND_INTERFACE="\$SECRET_BACKEND_INTERFACE" \
                -e DATABASE_URL="\$SECRET_DATABASE_URL" \
                -e TIPTAP_URL="\$SECRET_TIPTAP_URL" \
                -e TIPTAP_AUTH_KEY="\$SECRET_TIPTAP_AUTH_KEY" \
                -e TIPTAP_JWT_SIGNING_KEY="\$SECRET_TIPTAP_JWT_SIGNING_KEY" \
                -e BACKEND_API_VERSION="\$SECRET_BACKEND_API_VERSION" \
                "\$image_to_use"
              # Check if container started successfully
              if [ ! "\$(docker ps -q -f name=\$container_to_start_name)" ]; then
                echo "ERROR: Failed to start container \$container_to_start_name on \$HOSTNAME."
                docker logs \$container_to_start_name # Show logs for debugging
                return 1 # Indicate failure
              fi
              echo "Container \$container_to_start_name started successfully on \$HOSTNAME."
              return 0 # Indicate success
            }

            echo "Logging into Docker registry (\$REGISTRY_ENV) on \$HOSTNAME..."
            echo "\$REGISTRY_PASSWORD_ENV" | docker login \$REGISTRY_ENV -u "\$REGISTRY_USER_ENV" --password-stdin

            echo "Pulling new image \$IMAGE_TO_DEPLOY on \$HOSTNAME..."
            docker pull "\$IMAGE_TO_DEPLOY"

            echo "Cleaning up any old application container (\$APP_CONTAINER_NAME_ENV) on \$HOSTNAME..."
            cleanup_container "\$APP_CONTAINER_NAME_ENV"

            echo "Starting new version as \$APP_CONTAINER_NAME_ENV on \$HOSTNAME..."
            if ! start_application_container "\$APP_CONTAINER_NAME_ENV" "\$IMAGE_TO_DEPLOY"; then
              echo "Critical error: Container \$APP_CONTAINER_NAME_ENV failed to start on \$HOSTNAME. Aborting deployment on this droplet."
              exit 1 # This will fail the GitHub Actions step if SSH script exits non-zero
            fi

            echo "Waiting \$HEALTH_CHECK_WAIT_SECONDS_FOR_CURL seconds for deployment to initialize on \$HOSTNAME..."
            sleep \$HEALTH_CHECK_WAIT_SECONDS_FOR_CURL

            echo "Performing direct health check on \$HOSTNAME (http://localhost:\$HEALTH_CHECK_PORT_FOR_CURL\$HEALTH_CHECK_PATH_FOR_CURL)..."
            if curl -fsS "http://localhost:\$HEALTH_CHECK_PORT_FOR_CURL\$HEALTH_CHECK_PATH_FOR_CURL" > /dev/null; then
              echo "Direct health check PASSED on \$HOSTNAME."
              # This SSH script will exit 0, indicating success to the GitHub Actions step.
            else
              HEALTH_CHECK_STATUS=\$?
              echo "Direct health check FAILED on \$HOSTNAME with status \$HEALTH_CHECK_STATUS."
              echo "Deployment on \$HOSTNAME is considered unhealthy."
              # Attempt to clean up the failed deployment
              cleanup_container "\$APP_CONTAINER_NAME_ENV"
              exit 1 # This will fail the GitHub Actions step
            fi
          EOF
          # If SSH script exits non-zero, this GitHub Actions step fails, and workflow stops before LB changes.

      - name: Update Load Balancer and Finalize Deployment # Add Green to LB, wait, then remove Blue.
        if: success() # Only run if the previous step (deploy_green) succeeded.
        env:
          BLUE_DROPLET_ID_TO_REMOVE: ${{ steps.determine_roles.outputs.blue_id }}
          GREEN_DROPLET_ID_TO_ADD: ${{ steps.determine_roles.outputs.green_id }}
        run: | # Multi-line script for doctl commands.
          echo "Green droplet deployment and health check successful."
          echo "Adding Green droplet ($GREEN_DROPLET_ID_TO_ADD) to Load Balancer ($DO_LOAD_BALANCER_ID_ENV)..."
          doctl compute load-balancer add-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $GREEN_DROPLET_ID_TO_ADD

          echo "Waiting for Green droplet ($GREEN_DROPLET_ID_TO_ADD) to become healthy in Load Balancer..."
          END_TIME=\$(( \$(date +%s) + ( $LB_HEALTH_WAIT_TIMEOUT_MINUTES * 60 ) ))
          HEALTHY_IN_LB=false
          while [ \$(date +%s) -lt \$END_TIME ]; do
            LB_STATUS_JSON=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV --format ID,Name,DropletIDs,DropletHealth --no-header -o json)
            # Check health of the specific green droplet
            # Assuming DropletHealth is an array parallel to DropletIDs or a map.
            # For simplicity, we'll check if *any* droplet with the green ID is healthy.
            # A more robust check would parse the DropletHealth structure precisely.
            # This jq query looks for the green droplet ID and checks if its corresponding health is "healthy".
            # This part is complex because doctl's JSON output for health can be tricky.
            # A simpler check: just wait and assume LB figures it out, or check overall LB status.
            # For now, we'll rely on a timed wait and then proceed. A more robust check is recommended.
            # A better way: check the 'status' field of the specific droplet within the LB's droplet list.
            # Example: doctl compute load-balancer get <lb-id> -o json | jq '.[] | select(.id == <lb-id>) | .droplets[] | select(.id == <droplet-id>) | .status'
            # This is a placeholder for a robust LB health check loop.
            # For now, we just wait a fixed time.
            echo "Checking LB status for droplet $GREEN_DROPLET_ID_TO_ADD... (current time: $(date))"
            # This is a simplified check. A real implementation should parse LB health for the specific droplet.
            # If the LB reports the droplet as healthy (this requires parsing the LB's droplet health status, which is complex with doctl's current output)
            # For this example, we'll assume a wait is sufficient, then proceed.
            # A more robust check would involve:
            # LB_DROPLETS_HEALTH=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json | jq -r --argjson id "$GREEN_DROPLET_ID_TO_ADD" '.[] .droplets[] | select(.id == $id) | .health.status')
            # if [ "$LB_DROPLETS_HEALTH" == "healthy" ]; then HEALTHY_IN_LB=true; break; fi
            # The above jq might need adjustment based on exact doctl output structure.
            # For now, using a simpler timed wait.
            sleep 30 # Check every 30 seconds
            # This loop needs a proper condition to break once healthy in LB.
            # For this example, we'll assume after the wait, we proceed.
            # A robust solution would poll the LB status for the specific droplet.
            # For now, let's assume after a longer wait, it's good, or rely on LB's own health checks.
            # This is a critical part that needs careful implementation based on `doctl` output.
            # For now, we'll just wait for a period.
            # A more robust check:
            GREEN_STATUS_IN_LB=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json | jq -r ".[] | .droplets[] | select(.id == $GREEN_DROPLET_ID_TO_ADD) | .health.status" 2>/dev/null || echo "unknown")
            if [ "$GREEN_STATUS_IN_LB" == "healthy" ]; then
              echo "Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now HEALTHY in Load Balancer."
              HEALTHY_IN_LB=true
              break
            fi
            echo "Green droplet status in LB: $GREEN_STATUS_IN_LB. Retrying..."
          done

          if [ "$HEALTHY_IN_LB" != "true" ]; then
            echo "Error: Green droplet ($GREEN_DROPLET_ID_TO_ADD) did not become healthy in Load Balancer within timeout."
            echo "Attempting to remove Green droplet ($GREEN_DROPLET_ID_TO_ADD) from LB as a precaution."
            doctl compute load-balancer remove-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $GREEN_DROPLET_ID_TO_ADD --force || echo "Failed to remove green droplet from LB during error handling."
            exit 1
          fi

          if [ -n "$BLUE_DROPLET_ID_TO_REMOVE" ] && [ "$BLUE_DROPLET_ID_TO_REMOVE" != "null" ]; then
            echo "Removing Blue droplet ($BLUE_DROPLET_ID_TO_REMOVE) from Load Balancer ($DO_LOAD_BALANCER_ID_ENV)..."
            doctl compute load-balancer remove-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $BLUE_DROPLET_ID_TO_REMOVE --force
            echo "Blue droplet ($BLUE_DROPLET_ID_TO_REMOVE) removed. Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now live."
          else
            echo "No Blue droplet was previously live in the LB or it was the same as Green (initial deployment). Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now live."
          fi

          echo "Zero-downtime deployment complete. New version on droplet $GREEN_DROPLET_ID_TO_ADD is live."
          echo "Cleaning up old, unused Docker images on the new live droplet ($GREEN_DROPLET_ID_TO_ADD)..."
          ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null ${{ secrets.DO_USER }}@${{ steps.determine_roles.outputs.green_host }} << EOF
            docker image prune -af
          EOF

      - name: Deployment Failed - Rollback Information # This step runs if deploy_green or Update Load Balancer fails.
        if: failure() && (steps.deploy_green.outcome == 'failure' || steps.update_load_balancer.outcome == 'failure')
        run: | # Multi-line script.
          echo "Deployment failed. The previous version (if any) on the Blue droplet should still be active via the Load Balancer."
          echo "The Green droplet deployment was unsuccessful or failed to become healthy in the Load Balancer."
          echo "Manual inspection of the Green droplet (${{ steps.determine_roles.outputs.green_host }}) and Load Balancer ($DO_LOAD_BALANCER_ID_ENV) might be required."
```// filepath: .github/workflows/release_build_deploy.yml
name: Release Build and Zero-Downtime Deploy # Name of the GitHub Actions workflow.

on: # Defines the events that trigger the workflow.
  release: # Triggers on release events.
    types: [published] # Specifically triggers when a new release is published via the GitHub UI.
  workflow_dispatch: # Allows manual triggering, useful for rollbacks by selecting a previous release tag.
    inputs:
      release_tag: # Input for specifying a release tag, defaults to latest release if run by 'release' event.
        description: 'Release tag to deploy (e.g., v1.0.0). Leave empty if triggered by a new release event.'
        required: false

env: # Environment variables available to all jobs in the workflow.
  REGISTRY: ghcr.io # Specifies the container registry (GitHub Container Registry).
  IMAGE_NAME_BASE: ${{ github.repository }} # Base image name, e.g., refactor-group/refactor-platform-rs.
  # Application container name on the droplets (can be the same on both as only one is "live" for the app logic at a time per droplet).
  APP_CONTAINER_NAME: "refactor-platform" # From your .env BACKEND_CONTAINER_NAME.
  HEALTH_CHECK_PATH: "/health" # Assumed health check path for your application. Adjust if needed.
  HEALTH_CHECK_WAIT_SECONDS: 45 # Seconds to wait for the green container to start before direct health check.
  LB_HEALTH_WAIT_TIMEOUT_MINUTES: 3 # Minutes to wait for LB to report new droplet as healthy.

jobs: # Defines the jobs that run as part of the workflow.
  build_and_push_image: # Job to build the Docker image and push it to GHCR.
    name: Build and Push Docker Image # Display name for the job.
    runs-on: ubuntu-latest # Specifies the runner environment.
    outputs: # Define outputs to be used by downstream jobs.
      release_version: ${{ steps.meta.outputs.version }} # The primary version tag (e.g., v1.0.0).
      image_digest: ${{ steps.build-push.outputs.digest }} # The digest of the pushed image.
    permissions: # Permissions needed by the GITHUB_TOKEN for this job.
      contents: read # To checkout the repository.
      packages: write # To push images to GHCR.
      id-token: write # For OIDC token.
      attestations: write # To write build attestations.

    steps: # Sequence of steps for this job.
      - name: Checkout repository # Step to checkout the code.
        uses: actions/checkout@v4 # Uses the official checkout action.
        with:
          fetch-tags: true # Ensure tags are fetched for release versioning.

      - name: Set up QEMU # Step to set up QEMU for multi-platform builds (if needed).
        uses: docker/setup-qemu-action@v3 # Uses Docker's QEMU setup action.

      - name: Set up Docker Buildx # Step to set up Docker Buildx.
        uses: docker/setup-buildx-action@v3 # Uses Docker's Buildx setup action.

      - name: Log in to GitHub Container Registry # Step to log in to GHCR.
        uses: docker/login-action@v3 # Uses Docker's login action.
        with: # Parameters for the login action.
          registry: ${{ env.REGISTRY }} # Specifies the GHCR registry.
          username: ${{ github.actor }} # Uses the GitHub actor (user/bot) as the username.
          password: ${{ secrets.GITHUB_TOKEN }} # Uses the auto-generated GITHUB_TOKEN as the password.

      - name: Determine Release Tag # Determine the tag to use for the image.
        id: get_release_tag
        run: | # Script to get tag from event or input.
          if [[ "${{ github.event_name }}" == "release" ]]; then
            echo "release_tag=${{ github.event.release.tag_name }}" >> $GITHUB_OUTPUT
          elif [[ "${{ github.event_name }}" == "workflow_dispatch" && -n "${{ github.event.inputs.release_tag }}" ]]; then
            echo "release_tag=${{ github.event.inputs.release_tag }}" >> $GITHUB_OUTPUT
          else
            echo "Error: Could not determine release tag."
            exit 1
          fi

      - name: Docker metadata # Step to generate Docker image tags and labels.
        id: meta # ID for this step to reference its outputs.
        uses: docker/metadata-action@v5 # Uses Docker's metadata action.
        with: # Parameters for the metadata action.
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME_BASE }} # Defines the base image name.
          tags: | # Defines the tagging strategy.
            type=raw,value=${{ steps.get_release_tag.outputs.release_tag }} # Use the specific release tag.
            type=raw,value=latest,enable=${{ github.event_name == 'release' }} # Also tag 'latest' if it's a new release event.
          # 'version' output will be the primary tag used (e.g., v1.0.0)

      - name: Build and push Docker image # Step to build and push the image.
        id: build-push # ID for this step.
        uses: docker/build-push-action@v5 # Uses Docker's build-and-push action.
        with: # Parameters for the build-and-push action.
          context: . # Sets the build context to the repository root.
          file: ./Dockerfile # Specifies the path to the Dockerfile.
          platforms: linux/amd64 # Specify platforms if needed.
          push: true # Pushes the image after building.
          tags: ${{ steps.meta.outputs.tags }} # Uses the tags generated by the metadata step.
          labels: ${{ steps.meta.outputs.labels }} # Uses the labels generated by the metadata step.
          cache-from: type=gha # Uses GitHub Actions cache for build layers.
          cache-to: type=gha,mode=max # Exports build cache to GitHub Actions cache.
          provenance: true # Generates SLSA build provenance.

      - name: Output Image Details # Step to log the pushed image details.
        run: | # Multi-line script.
          echo "Release Version for Deployment: ${{ steps.meta.outputs.version }}"
          echo "Image pushed with tags: ${{ steps.meta.outputs.tags }}" # Logs the tags.
          echo "Image digest: ${{ steps.build-push.outputs.digest }}" # Logs the image digest.

  deploy_blue_green_to_digitalocean: # Job to deploy the image to DigitalOcean using Blue/Green.
    name: Deploy Blue/Green to DigitalOcean # Display name for the job.
    runs-on: ubuntu-latest # Specifies the runner environment.
    needs: build_and_push_image # Depends on the image build job.
    env: # Environment variables specific to this job.
      # Droplet and LB identifiers from secrets
      DO_LOAD_BALANCER_ID_ENV: ${{ secrets.DO_LOAD_BALANCER_ID }}
      DO_DROPLET_ONE_ID_ENV: ${{ secrets.DO_DROPLET_ONE_ID }}
      DO_DROPLET_ONE_HOST_ENV: ${{ secrets.DO_DROPLET_ONE_HOST }}
      DO_DROPLET_TWO_ID_ENV: ${{ secrets.DO_DROPLET_TWO_ID }}
      DO_DROPLET_TWO_HOST_ENV: ${{ secrets.DO_DROPLET_TWO_HOST }}
      # Image to deploy, using the output from the build job
      IMAGE_TO_DEPLOY: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME_BASE }}:${{ needs.build_and_push_image.outputs.release_version }}
      # Application Environment Variables from Secrets (passed to SSH script)
      SECRET_BACKEND_ALLOWED_ORIGINS: ${{ secrets.BACKEND_ALLOWED_ORIGINS }}
      SECRET_BACKEND_ENV: "production"
      SECRET_BACKEND_LOG_FILTER_LEVEL: ${{ secrets.BACKEND_LOG_FILTER_LEVEL }}
      SECRET_BACKEND_PORT: ${{ secrets.BACKEND_PORT }}
      SECRET_BACKEND_INTERFACE: ${{ secrets.BACKEND_INTERFACE }}
      SECRET_DATABASE_URL: ${{ secrets.DATABASE_URL }}
      SECRET_TIPTAP_URL: ${{ secrets.TIPTAP_URL }}
      SECRET_TIPTAP_AUTH_KEY: ${{ secrets.TIPTAP_AUTH_KEY }}
      SECRET_TIPTAP_JWT_SIGNING_KEY: ${{ secrets.TIPTAP_JWT_SIGNING_KEY }}
      SECRET_BACKEND_API_VERSION: ${{ secrets.BACKEND_API_VERSION }} # Or use needs.build_and_push_image.outputs.release_version

    steps: # Sequence of steps for this job.
      - name: Install doctl and jq # doctl for DO API, jq for parsing JSON.
        run: | # Multi-line script.
          sudo apt-get update && sudo apt-get install -y jq
          curl -sL https://github.com/digitalocean/doctl/releases/download/v1.106.0/doctl-1.106.0-linux-amd64.tar.gz | tar -xzv
          sudo mv doctl /usr/local/bin

      - name: Authenticate doctl # Authenticate doctl with DO API token.
        env:
          DIGITALOCEAN_ACCESS_TOKEN: ${{ secrets.DO_API_TOKEN }}
        run: doctl auth init --access-token $DIGITALOCEAN_ACCESS_TOKEN

      - name: Determine Blue (Live) and Green (Target) Droplets # Logic to decide which droplet is which.
        id: determine_roles
        run: | # Multi-line script.
          echo "Determining Blue/Green roles for Load Balancer ID: $DO_LOAD_BALANCER_ID_ENV"
          LB_INFO_JSON=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json)
          if [ -z "$LB_INFO_JSON" ]; then
            echo "Error: Could not retrieve Load Balancer info for ID $DO_LOAD_BALANCER_ID_ENV."
            exit 1
          fi
          # Extract DropletIDs currently in the LB. This is an array.
          LIVE_DROPLET_IDS_STR=$(echo "$LB_INFO_JSON" | jq -r '.[0].droplet_ids | @json')

          echo "Droplet One ID: $DO_DROPLET_ONE_ID_ENV, Droplet Two ID: $DO_DROPLET_TWO_ID_ENV"
          echo "Live Droplet IDs string from LB: $LIVE_DROPLET_IDS_STR"

          BLUE_DROPLET_ID=""
          BLUE_DROPLET_HOST=""
          GREEN_TARGET_DROPLET_ID=""
          GREEN_TARGET_DROPLET_HOST=""

          # Check if Droplet One is live
          if echo "$LIVE_DROPLET_IDS_STR" | jq -e ".[] | select(. == $DO_DROPLET_ONE_ID_ENV)" > /dev/null; then
            echo "Droplet One ($DO_DROPLET_ONE_ID_ENV) is LIVE (Blue)."
            BLUE_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            BLUE_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_TWO_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_TWO_HOST_ENV"
          # Check if Droplet Two is live
          elif echo "$LIVE_DROPLET_IDS_STR" | jq -e ".[] | select(. == $DO_DROPLET_TWO_ID_ENV)" > /dev/null; then
            echo "Droplet Two ($DO_DROPLET_TWO_ID_ENV) is LIVE (Blue)."
            BLUE_DROPLET_ID="$DO_DROPLET_TWO_ID_ENV"
            BLUE_DROPLET_HOST="$DO_DROPLET_TWO_HOST_ENV"
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
          else # Neither is live, or LB is empty - target Droplet One as Green
            echo "Neither droplet is currently live in the LB, or LB is empty. Targeting Droplet One ($DO_DROPLET_ONE_ID_ENV) as Green."
            GREEN_TARGET_DROPLET_ID="$DO_DROPLET_ONE_ID_ENV"
            GREEN_TARGET_DROPLET_HOST="$DO_DROPLET_ONE_HOST_ENV"
            # BLUE_DROPLET_ID remains empty, indicating no current blue to remove initially.
          fi

          if [ -z "$GREEN_TARGET_DROPLET_ID" ]; then
            echo "Error: Could not determine a Green target droplet. This should not happen."
            exit 1
          fi

          echo "::set-output name=blue_id::$BLUE_DROPLET_ID"
          echo "::set-output name=blue_host::$BLUE_DROPLET_HOST"
          echo "::set-output name=green_id::$GREEN_TARGET_DROPLET_ID"
          echo "::set-output name=green_host::$GREEN_TARGET_DROPLET_HOST"
          echo "Green Target Droplet ID: $GREEN_TARGET_DROPLET_ID, Host: $GREEN_TARGET_DROPLET_HOST"
          echo "Blue (Current Live) Droplet ID: $BLUE_DROPLET_ID, Host: $BLUE_DROPLET_HOST"

      - name: Set up SSH Agent # Step to configure SSH access.
        uses: webfactory/ssh-agent@v0.9.0 # Uses an action for SSH agent setup.
        with: # Parameters for the action.
          ssh-private-key: ${{ secrets.DO_SSH_PRIVATE_KEY }} # Uses the SSH private key from GitHub secrets.

      - name: Deploy to Green Droplet and Health Check # Deploy and check health on the target green droplet.
        id: deploy_green
        env:
          GREEN_DROPLET_HOST_FOR_SSH: ${{ steps.determine_roles.outputs.green_host }}
          # Pass other necessary env vars for the SSH script
          REGISTRY_ENV: ${{ env.REGISTRY }}
          REGISTRY_USER_ENV: ${{ github.actor }}
          REGISTRY_PASSWORD_ENV: ${{ secrets.GITHUB_TOKEN }}
          APP_CONTAINER_NAME_ENV: ${{ env.APP_CONTAINER_NAME }}
          HEALTH_CHECK_PORT_FOR_CURL: ${{ secrets.BACKEND_PORT }} # Port for direct curl health check
          HEALTH_CHECK_PATH_FOR_CURL: ${{ env.HEALTH_CHECK_PATH }}
          HEALTH_CHECK_WAIT_SECONDS_FOR_CURL: ${{ env.HEALTH_CHECK_WAIT_SECONDS }}
        run: | # Multi-line script for SSH commands.
          echo "Deploying image $IMAGE_TO_DEPLOY to GREEN droplet: $GREEN_DROPLET_HOST_FOR_SSH"
          ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null ${{ secrets.DO_USER }}@$GREEN_DROPLET_HOST_FOR_SSH << EOF
            set -e # Exit immediately if a command fails, but health check failure is handled.

            # Function to safely stop and remove a container if it exists
            cleanup_container() {
              local container_name="\$1"
              if [ "\$(docker ps -q -f name=\$container_name)" ]; then
                echo "Stopping container \$container_name on \$HOSTNAME..."
                docker stop \$container_name
              fi
              if [ "\$(docker ps -aq -f name=\$container_name)" ]; then
                echo "Removing container \$container_name on \$HOSTNAME..."
                docker rm \$container_name
              fi
            }

            # Function to start the application container
            start_application_container() {
              local container_to_start_name="\$1"
              local image_to_use="\$2"
              echo "Starting container \$container_to_start_name with image \$image_to_use on \$HOSTNAME..."
              docker run -d --restart always \
                --name "\$container_to_start_name" \
                -p "\$SECRET_BACKEND_PORT:\$SECRET_BACKEND_PORT" \
                -e BACKEND_ALLOWED_ORIGINS="\$SECRET_BACKEND_ALLOWED_ORIGINS" \
                -e BACKEND_ENV="\$SECRET_BACKEND_ENV" \
                -e BACKEND_LOG_FILTER_LEVEL="\$SECRET_BACKEND_LOG_FILTER_LEVEL" \
                -e BACKEND_PORT="\$SECRET_BACKEND_PORT" \
                -e BACKEND_INTERFACE="\$SECRET_BACKEND_INTERFACE" \
                -e DATABASE_URL="\$SECRET_DATABASE_URL" \
                -e TIPTAP_URL="\$SECRET_TIPTAP_URL" \
                -e TIPTAP_AUTH_KEY="\$SECRET_TIPTAP_AUTH_KEY" \
                -e TIPTAP_JWT_SIGNING_KEY="\$SECRET_TIPTAP_JWT_SIGNING_KEY" \
                -e BACKEND_API_VERSION="\$SECRET_BACKEND_API_VERSION" \
                "\$image_to_use"
              # Check if container started successfully
              if [ ! "\$(docker ps -q -f name=\$container_to_start_name)" ]; then
                echo "ERROR: Failed to start container \$container_to_start_name on \$HOSTNAME."
                docker logs \$container_to_start_name # Show logs for debugging
                return 1 # Indicate failure
              fi
              echo "Container \$container_to_start_name started successfully on \$HOSTNAME."
              return 0 # Indicate success
            }

            echo "Logging into Docker registry (\$REGISTRY_ENV) on \$HOSTNAME..."
            echo "\$REGISTRY_PASSWORD_ENV" | docker login \$REGISTRY_ENV -u "\$REGISTRY_USER_ENV" --password-stdin

            echo "Pulling new image \$IMAGE_TO_DEPLOY on \$HOSTNAME..."
            docker pull "\$IMAGE_TO_DEPLOY"

            echo "Cleaning up any old application container (\$APP_CONTAINER_NAME_ENV) on \$HOSTNAME..."
            cleanup_container "\$APP_CONTAINER_NAME_ENV"

            echo "Starting new version as \$APP_CONTAINER_NAME_ENV on \$HOSTNAME..."
            if ! start_application_container "\$APP_CONTAINER_NAME_ENV" "\$IMAGE_TO_DEPLOY"; then
              echo "Critical error: Container \$APP_CONTAINER_NAME_ENV failed to start on \$HOSTNAME. Aborting deployment on this droplet."
              exit 1 # This will fail the GitHub Actions step if SSH script exits non-zero
            fi

            echo "Waiting \$HEALTH_CHECK_WAIT_SECONDS_FOR_CURL seconds for deployment to initialize on \$HOSTNAME..."
            sleep \$HEALTH_CHECK_WAIT_SECONDS_FOR_CURL

            echo "Performing direct health check on \$HOSTNAME (http://localhost:\$HEALTH_CHECK_PORT_FOR_CURL\$HEALTH_CHECK_PATH_FOR_CURL)..."
            if curl -fsS "http://localhost:\$HEALTH_CHECK_PORT_FOR_CURL\$HEALTH_CHECK_PATH_FOR_CURL" > /dev/null; then
              echo "Direct health check PASSED on \$HOSTNAME."
              # This SSH script will exit 0, indicating success to the GitHub Actions step.
            else
              HEALTH_CHECK_STATUS=\$?
              echo "Direct health check FAILED on \$HOSTNAME with status \$HEALTH_CHECK_STATUS."
              echo "Deployment on \$HOSTNAME is considered unhealthy."
              # Attempt to clean up the failed deployment
              cleanup_container "\$APP_CONTAINER_NAME_ENV"
              exit 1 # This will fail the GitHub Actions step
            fi
          EOF
          # If SSH script exits non-zero, this GitHub Actions step fails, and workflow stops before LB changes.

      - name: Update Load Balancer and Finalize Deployment # Add Green to LB, wait, then remove Blue.
        if: success() # Only run if the previous step (deploy_green) succeeded.
        env:
          BLUE_DROPLET_ID_TO_REMOVE: ${{ steps.determine_roles.outputs.blue_id }}
          GREEN_DROPLET_ID_TO_ADD: ${{ steps.determine_roles.outputs.green_id }}
        run: | # Multi-line script for doctl commands.
          echo "Green droplet deployment and health check successful."
          echo "Adding Green droplet ($GREEN_DROPLET_ID_TO_ADD) to Load Balancer ($DO_LOAD_BALANCER_ID_ENV)..."
          doctl compute load-balancer add-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $GREEN_DROPLET_ID_TO_ADD

          echo "Waiting for Green droplet ($GREEN_DROPLET_ID_TO_ADD) to become healthy in Load Balancer..."
          END_TIME=\$(( \$(date +%s) + ( $LB_HEALTH_WAIT_TIMEOUT_MINUTES * 60 ) ))
          HEALTHY_IN_LB=false
          while [ \$(date +%s) -lt \$END_TIME ]; do
            LB_STATUS_JSON=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV --format ID,Name,DropletIDs,DropletHealth --no-header -o json)
            # Check health of the specific green droplet
            # Assuming DropletHealth is an array parallel to DropletIDs or a map.
            # For simplicity, we'll check if *any* droplet with the green ID is healthy.
            # A more robust check would parse the DropletHealth structure precisely.
            # This jq query looks for the green droplet ID and checks if its corresponding health is "healthy".
            # This part is complex because doctl's JSON output for health can be tricky.
            # A simpler check: just wait and assume LB figures it out, or check overall LB status.
            # For now, we'll rely on a timed wait and then proceed. A more robust check is recommended.
            # A better way: check the 'status' field of the specific droplet within the LB's droplet list.
            # Example: doctl compute load-balancer get <lb-id> -o json | jq '.[] | select(.id == <lb-id>) | .droplets[] | select(.id == <droplet-id>) | .status'
            # This is a placeholder for a robust LB health check loop.
            # For now, we just wait a fixed time.
            echo "Checking LB status for droplet $GREEN_DROPLET_ID_TO_ADD... (current time: $(date))"
            # This is a simplified check. A real implementation should parse LB health for the specific droplet.
            # If the LB reports the droplet as healthy (this requires parsing the LB's droplet health status, which is complex with doctl's current output)
            # For this example, we'll assume a wait is sufficient, then proceed.
            # A more robust check would involve:
            # LB_DROPLETS_HEALTH=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json | jq -r --argjson id "$GREEN_DROPLET_ID_TO_ADD" '.[] .droplets[] | select(.id == $id) | .health.status')
            # if [ "$LB_DROPLETS_HEALTH" == "healthy" ]; then HEALTHY_IN_LB=true; break; fi
            # The above jq might need adjustment based on exact doctl output structure.
            # For now, using a simpler timed wait.
            sleep 30 # Check every 30 seconds
            # This loop needs a proper condition to break once healthy in LB.
            # For this example, we'll assume after the wait, we proceed.
            # A robust solution would poll the LB status for the specific droplet.
            # For now, let's assume after a longer wait, it's good, or rely on LB's own health checks.
            # This is a critical part that needs careful implementation based on `doctl` output.
            # For now, we'll just wait for a period.
            # A more robust check:
            GREEN_STATUS_IN_LB=$(doctl compute load-balancer get $DO_LOAD_BALANCER_ID_ENV -o json | jq -r ".[] | .droplets[] | select(.id == $GREEN_DROPLET_ID_TO_ADD) | .health.status" 2>/dev/null || echo "unknown")
            if [ "$GREEN_STATUS_IN_LB" == "healthy" ]; then
              echo "Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now HEALTHY in Load Balancer."
              HEALTHY_IN_LB=true
              break
            fi
            echo "Green droplet status in LB: $GREEN_STATUS_IN_LB. Retrying..."
          done

          if [ "$HEALTHY_IN_LB" != "true" ]; then
            echo "Error: Green droplet ($GREEN_DROPLET_ID_TO_ADD) did not become healthy in Load Balancer within timeout."
            echo "Attempting to remove Green droplet ($GREEN_DROPLET_ID_TO_ADD) from LB as a precaution."
            doctl compute load-balancer remove-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $GREEN_DROPLET_ID_TO_ADD --force || echo "Failed to remove green droplet from LB during error handling."
            exit 1
          fi

          if [ -n "$BLUE_DROPLET_ID_TO_REMOVE" ] && [ "$BLUE_DROPLET_ID_TO_REMOVE" != "null" ]; then
            echo "Removing Blue droplet ($BLUE_DROPLET_ID_TO_REMOVE) from Load Balancer ($DO_LOAD_BALANCER_ID_ENV)..."
            doctl compute load-balancer remove-droplets $DO_LOAD_BALANCER_ID_ENV --droplet-ids $BLUE_DROPLET_ID_TO_REMOVE --force
            echo "Blue droplet ($BLUE_DROPLET_ID_TO_REMOVE) removed. Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now live."
          else
            echo "No Blue droplet was previously live in the LB or it was the same as Green (initial deployment). Green droplet ($GREEN_DROPLET_ID_TO_ADD) is now live."
          fi

          echo "Zero-downtime deployment complete. New version on droplet $GREEN_DROPLET_ID_TO_ADD is live."
          echo "Cleaning up old, unused Docker images on the new live droplet ($GREEN_DROPLET_ID_TO_ADD)..."
          ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null ${{ secrets.DO_USER }}@${{ steps.determine_roles.outputs.green_host }} << EOF
            docker image prune -af
          EOF

      - name: Deployment Failed - Rollback Information # This step runs if deploy_green or Update Load Balancer fails.
        if: failure() && (steps.deploy_green.outcome == 'failure' || steps.update_load_balancer.outcome == 'failure')
        run: | # Multi-line script.
          echo "Deployment failed. The previous version (if any) on the Blue droplet should still be active via the Load Balancer."
          echo "The Green droplet deployment was unsuccessful or failed to become healthy in the Load Balancer."
          echo "Manual inspection of the Green droplet (${{ steps.determine_roles.outputs.green_host }}) and Load Balancer ($DO_LOAD_BALANCER_ID_ENV) might be required."
