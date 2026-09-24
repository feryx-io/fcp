# Installing Docker on Debian

This guide walks you through installing Docker and Docker Compose on a fresh Debian instance.

---

## Quick Install (Recommended)

```bash
# Update package index
sudo apt-get update

# Install prerequisites
sudo apt-get install -y \
    ca-certificates \
    curl \
    gnupg \
    lsb-release

# Add Docker's official GPG key
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
sudo chmod a+r /etc/apt/keyrings/docker.gpg

# Set up Docker repository
echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian \
  $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null

# Update package index again
sudo apt-get update

# Install Docker Engine, CLI, and Docker Compose
sudo apt-get install -y \
    docker-ce \
    docker-ce-cli \
    containerd.io \
    docker-buildx-plugin \
    docker-compose-plugin

# Verify installation
sudo docker --version
sudo docker compose version
```

---

## Post-Installation Setup

### 1. Add Your User to Docker Group (Optional but Recommended)

This allows you to run Docker commands without `sudo`:

```bash
# Add your user to the docker group
sudo usermod -aG docker $USER

# Apply the group change (or log out and back in)
newgrp docker

# Verify you can run docker without sudo
docker run hello-world
```

### 2. Enable Docker to Start on Boot

```bash
sudo systemctl enable docker.service
sudo systemctl enable containerd.service
```

### 3. Configure Docker (Optional)

Create or edit `/etc/docker/daemon.json`:

```bash
sudo mkdir -p /etc/docker
sudo tee /etc/docker/daemon.json > /dev/null <<EOF
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "storage-driver": "overlay2"
}
EOF

# Restart Docker to apply changes
sudo systemctl restart docker
```

---

## Verify Installation

```bash
# Check Docker version
docker --version
# Expected output: Docker version 24.0.x or higher

# Check Docker Compose version
docker compose version
# Expected output: Docker Compose version v2.x.x or higher

# Test Docker with hello-world
docker run hello-world
# Should download and run successfully

# Check Docker service status
sudo systemctl status docker
# Should show "active (running)"
```

---

## Deploy FCP with Docker

Now that Docker is installed, you can deploy FCP:

### 1. Clone FCP Repository

```bash
# Install git if not already installed
sudo apt-get install -y git

# Clone with submodules
git clone --recursive https://github.com/thevoiceguy/fcp.git
cd fcp
```

### 2. Configure FCP

```bash
# Review and edit configuration if needed
nano config/fcp-docker.toml

# Key settings to check:
# - external_ip: Set to your server's public IP
# - domain: Set to your domain name
# - database password: Change from default
```

### 3. Start FCP Services

```bash
# Start all services in detached mode
docker compose up -d

# View logs
docker compose logs -f

# Check service status
docker compose ps
```

### 4. Verify FCP is Running

```bash
# Health check
curl http://localhost:8080/api/v1/health

# Expected response:
# {"status":"healthy","version":"0.9.0"}
```

### 5. Access Services

- **FCP API**: http://your-server-ip:8080
- **Grafana**: http://your-server-ip:3000 (admin/admin)
- **Prometheus**: http://your-server-ip:9090

### 6. Create First Tenant

```bash
curl -X POST http://localhost:8080/api/v1/tenants \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Company",
    "domain": "mycompany.local",
    "max_extensions": 100
  }'
```

### 7. Create First User

```bash
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "display_name": "Alice Smith",
    "email": "alice@mycompany.com",
    "password": "secure_password_here"
  }'
```

---

## Firewall Configuration

If you have a firewall enabled (UFW), open the necessary ports:

```bash
# Install UFW if not already installed
sudo apt-get install -y ufw

# Allow SSH (important - do this first!)
sudo ufw allow 22/tcp

# Allow SIP ports
sudo ufw allow 5060/udp
sudo ufw allow 5060/tcp
sudo ufw allow 5061/tcp  # SIP TLS

# Allow RTP media ports
sudo ufw allow 10000:20000/udp

# Allow API access (restrict to specific IPs in production)
sudo ufw allow 8080/tcp

# Allow Grafana (optional, restrict in production)
sudo ufw allow 3000/tcp

# Enable firewall
sudo ufw enable

# Check status
sudo ufw status
```

---

## Useful Docker Commands

### Managing Services

```bash
# Start services
docker compose up -d

# Stop services
docker compose stop

# Restart services
docker compose restart

# Stop and remove containers
docker compose down

# View logs
docker compose logs -f fcp-server

# View all logs
docker compose logs -f

# Check service status
docker compose ps
```

### Managing Containers

```bash
# List running containers
docker ps

# List all containers (including stopped)
docker ps -a

# Execute command in running container
docker compose exec fcp-server /bin/bash

# View container resource usage
docker stats
```

### Cleaning Up

```bash
# Remove stopped containers
docker container prune

# Remove unused images
docker image prune

# Remove unused volumes
docker volume prune

# Remove everything unused (be careful!)
docker system prune -a
```

---

## Troubleshooting

### Docker daemon not starting

```bash
# Check Docker service status
sudo systemctl status docker

# View Docker daemon logs
sudo journalctl -u docker.service -n 50 --no-pager

# Restart Docker service
sudo systemctl restart docker
```

### Permission denied error

```bash
# If you get "permission denied" when running docker commands:

# Make sure you added your user to docker group
sudo usermod -aG docker $USER

# Log out and log back in, or run:
newgrp docker
```

### Port conflicts

```bash
# If ports are already in use, check what's using them:
sudo netstat -tulpn | grep 5060
sudo netstat -tulpn | grep 8080

# Stop conflicting services or change ports in docker-compose.yml
```

### Container fails to start

```bash
# View container logs
docker compose logs fcp-server

# Check for error messages in logs
docker compose logs fcp-server | grep -i error

# Remove and recreate containers
docker compose down
docker compose up -d
```

### Database connection issues

```bash
# Check if PostgreSQL container is running
docker compose ps postgres

# View PostgreSQL logs
docker compose logs postgres

# Connect to PostgreSQL to test
docker compose exec postgres psql -U fcp -d fcp -c "SELECT 1;"
```

---

## Updating FCP

```bash
# Pull latest changes
cd fcp
git pull origin main
git submodule update --recursive

# Rebuild containers
docker compose build --no-cache

# Restart with new images
docker compose down
docker compose up -d

# Verify health
curl http://localhost:8080/api/v1/health
```

---

## Complete Installation Script

Here's a complete script to install Docker and deploy FCP on fresh Debian:

```bash
#!/bin/bash
set -e

echo "=== Installing Docker on Debian ==="

# Update system
sudo apt-get update
sudo apt-get upgrade -y

# Install prerequisites
sudo apt-get install -y \
    ca-certificates \
    curl \
    gnupg \
    lsb-release \
    git

# Add Docker's official GPG key
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
sudo chmod a+r /etc/apt/keyrings/docker.gpg

# Set up Docker repository
echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian \
  $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null

# Install Docker
sudo apt-get update
sudo apt-get install -y \
    docker-ce \
    docker-ce-cli \
    containerd.io \
    docker-buildx-plugin \
    docker-compose-plugin

# Add user to docker group
sudo usermod -aG docker $USER

# Enable Docker service
sudo systemctl enable docker.service
sudo systemctl enable containerd.service

echo "=== Docker installed successfully! ==="
docker --version
docker compose version

echo ""
echo "=== Cloning FCP Repository ==="
git clone --recursive https://github.com/thevoiceguy/fcp.git
cd fcp

echo ""
echo "=== Starting FCP Services ==="
docker compose up -d

echo ""
echo "=== Waiting for services to start (30 seconds) ==="
sleep 30

echo ""
echo "=== Checking FCP Health ==="
curl http://localhost:8080/api/v1/health || echo "FCP not ready yet, check logs with: docker compose logs -f"

echo ""
echo "=== Installation Complete! ==="
echo ""
echo "Access FCP services at:"
echo "  - API: http://localhost:8080"
echo "  - Grafana: http://localhost:3000 (admin/admin)"
echo "  - Prometheus: http://localhost:9090"
echo ""
echo "View logs: docker compose logs -f"
echo "Stop services: docker compose down"
echo ""
echo "IMPORTANT: Log out and log back in for docker group changes to take effect!"
```

Save this as `install-fcp.sh`, make it executable, and run:

```bash
chmod +x install-fcp.sh
./install-fcp.sh
```

---

## Next Steps

After installation:

1. ✅ Configure your domain/IP in `config/fcp-docker.toml`
2. ✅ Set up TLS certificates for production
3. ✅ Create tenants and users via REST API
4. ✅ Configure SIP phones to register
5. ✅ Test call routing
6. ✅ Access Grafana dashboards
7. ✅ Review security settings

See [DEPLOYMENT.md](../DEPLOYMENT.md) for detailed production deployment instructions.

---

**Need help?** Check the [troubleshooting section](#troubleshooting) or open an issue on GitHub.
