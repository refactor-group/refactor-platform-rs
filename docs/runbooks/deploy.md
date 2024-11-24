<h3 style="color: darkolivegreen; text-align: center; text-decoration: underline;">Refactor Platform Deployment Runbook</h3>

<details>
    <summary>Ticket Contents</summary><br>

> **Create the automation code and Terraform code to deploy the frontend and backend Rust app to a running Ubuntu 22.04 server, using local Terraform state, try deploying it with a container and without using Docker containers or something “native”. A Container that can run on the ContainerD Container Runtime, which should include Docker Containers.**

**Idea:** *use Ansible for performing initial setup on a server target.*

### Definition of done

* Able to build and start the backend and frontend applications using Docker containers and docker-compose
* All needed Ansible entities (roles, resource groupings) and scripts exist that can deploy to any target Ubuntu 22.04 instance running Ansible

</details>

<details>
    <summary>Summary</summary><br>

*This runbook provides detailed steps to deploy Rust frontend and backend applications on an Ubuntu 22.04 server using Ansible for initial setup and Terraform for automation. It covers both containerized (using Docker) and native deployment methods.*

</details>

<details>
    <summary>I. Initial Server Setup with Ansible</summary><br>

1. **Define Ansible Inventory:** Create an Ansible inventory file containing the IP address or hostname of your target Ubuntu 22.04 server.

```ini
[ubuntu_server]
192.168.1.10 ansible_user=ubuntu ansible_ssh_private_key_file=~/.ssh/id_rsa
```

*Explanation*: The inventory file specifies the target servers for Ansible to manage. Here, we define the target server with its IP address and SSH details.

1. **Create Ansible Playbook:** Develop an Ansible playbook to perform the following tasks on the server:
    * **Install Required Packages:** Install necessary packages like `curl`, `git`, `unzip`, `sudo`, `python3`, `python3-pip`, `jq`, `make`, `gcc`, `g++`, and others required for Rust compilation and container runtime.
    * **Install Containerd:** Install and configure Containerd as the container runtime.
    * **Install Docker (Optional):** Install Docker if you plan to use it alongside Containerd.
    * **Install Ansible (Optional):** Install Ansible on the server for future deployments and automation.
    * **Configure Firewall:** Open necessary ports for your applications (e.g., HTTP/HTTPS).

```yaml
---
- hosts: ubuntu_server
  become: yes
  tasks:
    - name: Install required packages
      apt:
        name:
          - curl
          - git
          - unzip
          - sudo
          - python3
          - python3-pip
          - jq
          - make
          - gcc
          - g++
        state: present
        update_cache: yes

    - name: Install Containerd
      shell: |
        sudo apt-get install -y containerd

    - name: Install Docker (Optional)
      shell: |
        sudo apt-get install -y docker.io

    - name: Install Ansible (Optional)
      apt:
        name: ansible
        state: present

    - name: Configure firewall
      ufw:
        rule: allow
        port: "{{ item }}"
        proto: tcp
      with_items:
        - 80
        - 443
```

*Explanation*: The playbook installs necessary packages, container runtime, and configures the firewall.

1. **Run Ansible Playbook:** Execute the playbook against the server.

```bash
ansible-playbook -i inventory.ini setup.yml
```

*Gotcha*: Ensure Ansible is installed on your local machine and accessible via `ansible-playbook` command.

</details>

<details>
    <summary>II. Frontend and Backend Application Deployment</summary><br>

### A. Containerized Approach (Docker-Compose)

1. **Create Dockerfiles:** Write Dockerfiles for both the frontend and backend applications, defining the necessary dependencies, build commands, and entry points.

```Dockerfile
# Backend Dockerfile
FROM rust:latest

WORKDIR /app
COPY . .

RUN cargo build --release

CMD ["./target/release/backend_app"]
```

```Dockerfile
# Frontend Dockerfile
FROM rust:latest

WORKDIR /app
COPY . .

RUN cargo build --release

CMD ["./target/release/frontend_app"]
```

*Gotcha*: Ensure the Dockerfile paths and commands match your application structure.

1. **Create Docker-Compose.yml:** Define the service dependencies and configurations for both frontend and backend applications within a docker-compose.yml file.

```yaml
version: '3'
services:
  backend:
    build: ./backend
    ports:
      - "8000:8000"
  frontend:
    build: ./frontend
    ports:
      - "3000:3000"
```

*Context*: This configuration binds the container ports to the host machine.

1. **Build Images:** Build the Docker images for both applications.

```bash
docker-compose build
```

*Gotcha*: Ensure Docker is running on your local machine.

1. **Deploy with Docker-Compose:** Use `docker-compose up -d` to start the containers on the server based on the defined docker-compose configuration.

```bash
docker-compose up -d
```

*Context*: The `-d` flag runs containers in the background.

## B. Native Approach (Without Containers)

1. **Build Frontend and Backend Applications:** Build the frontend and backend applications using Rust's build tools (e.g., `cargo build`).

```bash
cd backend
cargo build --release

cd ../frontend
cargo build --release
```

*Gotcha*: Ensure Rust and Cargo are installed on your local machine.

1. **Copy Compiled Binaries to Server:** Transfer the compiled binaries of the frontend and backend applications to the server.

```bash
scp backend/target/release/backend_app ubuntu@192.168.1.10:~/backend_app
scp frontend/target/release/frontend_app ubuntu@192.168.1.10:~/frontend_app
```

*Context*: Use `scp` to securely copy files over SSH.

1. **Configure and Start Applications:** Configure and start the applications on the server using systemd services or other suitable methods.

```ini
# backend_app.service
[Unit]
Description=Backend Application

[Service]
ExecStart=/home/ubuntu/backend_app

[Install]
WantedBy=multi-user.target
```

```ini
# frontend_app.service
[Unit]
Description=Frontend Application

[Service]
ExecStart=/home/ubuntu/frontend_app

[Install]
WantedBy=multi-user.target
```

Start and enable services:

```bash
sudo systemctl daemon-reload
sudo systemctl start backend_app
sudo systemctl enable backend_app

sudo systemctl start frontend_app
sudo systemctl enable frontend_app
```

*Gotcha*: Ensure the service files are correctly placed in `/etc/systemd/system/`.

</details>

<details>
    <summary>III. Terraform Deployment (Local State)</summary><br>

1. **Create Terraform Code:** Write Terraform code to manage the deployment process, including:
    * **Create Remote-Exec Provider:** Configure the remote-exec provider for running commands on the server.
    * **Provision Ansible:** If not already installed, use the Terraform provider to install Ansible on the server (optional).
    * **Run Ansible Playbook:** Use the remote-exec provider to execute the Ansible playbook on the server for initial setup.
    * **Manage Application Files:** Transfer the Dockerfiles, docker-compose.yml, compiled binaries (for the native approach), or application configuration files to the server.
    * **Run Docker-Compose (Containerized Approach):** Use the remote-exec provider to run `docker-compose up -d` command on the server to start the containers.
    * **Start Applications (Native Approach):** Use the remote-exec provider to start the application processes on the server.

```hcl
provider "local" {}

provider "null" {}

resource "null_resource" "setup" {
  connection {
    type        = "ssh"
    user        = "ubuntu"
    private_key = file("~/.ssh/id_rsa")
    host        = "192.168.1.10"
  }

  provisioner "remote-exec" {
    inline = [
      "sudo apt-get update",
      "sudo apt-get install -y ansible",
      "ansible-playbook -i inventory.ini setup.yml"
    ]
  }
}
```

*Context*: This config uses the `null` provider to run remote commands via SSH.

1. **Initialize Terraform:** Initialize Terraform in the working directory.

```bash
terraform init
```

*Gotcha*: Ensure you are in the correct directory containing `main.tf`.

1. **Apply Terraform Changes:** Use `terraform apply` to apply the infrastructure and configuration changes.

```bash
terraform apply
```

*Context*: Review the changes before applying by typing `yes` when prompted.

</details>

<details>
    <summary>IV. Definition of Done</summary><br>

* Both the frontend and backend applications are successfully built and started.
* The applications are accessible and function as expected.
* The Ansible playbook, Dockerfiles, and Terraform code are written and documented.
* All necessary steps are automated and can be deployed to any Ubuntu 22.04 instance running Ansible.

</details>

<details>
    <summary>Additional Considerations</summary><br>

* **Logging and Monitoring:** Implement logging and monitoring solutions like Prometheus, Grafana, or ELK stack for application performance and health checks.
* **Resilience and Elasticity:** Consider using load balancers and auto-scaling groups to handle traffic spikes and ensure high availability.

</details>
