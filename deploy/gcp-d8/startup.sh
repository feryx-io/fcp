#!/bin/bash
# First boot of the D8 server: Docker (from Docker's repository, for the
# compose plugin) and certbot.
set -euo pipefail
[ -f /var/lib/fcp-d8-ready ] && exit 0
apt-get update
apt-get install -y ca-certificates curl gnupg certbot openssl jq
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
. /etc/os-release
echo "deb [signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian $VERSION_CODENAME stable" \
    > /etc/apt/sources.list.d/docker.list
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
systemctl enable --now docker
touch /var/lib/fcp-d8-ready
