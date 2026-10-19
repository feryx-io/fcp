#!/bin/bash
# Run on the D8 server as root by up.sh, with a GHCR token on stdin:
#   setup.sh <domain> <email, or empty> <public address>
# Idempotent: the secrets are made once into /opt/d8/secrets.env and kept,
# the configurations are edited again from the copy up.sh just unpacked.
set -euo pipefail
DOMAIN=$1 EMAIL=$2 IP=$3
D8=/opt/d8
COMPOSE=$D8/docker-compose
# Text to speech is part of the stack from v0.25.425; an image before it
# still has it under the `tts` profile.
export COMPOSE_PROFILES=tts
cd "$COMPOSE"

# The public address on the loopback: GCE maps it at the edge, so without
# this a service's own public address (each one's Contact and SDP) is
# unreachable from the host itself, and every in-dialog request between
# two services on it (a BYE to voicemail, a transfer) is lost.
ip addr show dev lo | grep -q " $IP/32" || ip addr add "$IP/32" dev lo
cat > /etc/systemd/system/fcp-d8-loopback.service <<UNIT
[Unit]
Description=The D8 server's public address on the loopback
Before=docker.service
[Service]
Type=oneshot
ExecStart=/bin/sh -c 'ip addr show dev lo | grep -q " $IP/32" || ip addr add $IP/32 dev lo'
RemainAfterExit=yes
[Install]
WantedBy=multi-user.target
UNIT
systemctl enable fcp-d8-loopback.service >/dev/null 2>&1

# Secrets: every `change-me` in the stack's .env made random, once.
if [ ! -f $D8/secrets.env ]; then
    umask 077
    grep -E '^[A-Z_]+=.*change-me' .env | while IFS='=' read -r key _; do
        echo "$key=$(openssl rand -hex 24)"
    done > $D8/secrets.env
fi
while IFS='=' read -r key value; do
    sed -i "s|^$key=.*|$key=$value|" .env
done < $D8/secrets.env
sed -i "s|^HOST_IP=.*|HOST_IP=$IP|; s|^FCP_IMAGE_TAG=.*|FCP_IMAGE_TAG=${FCP_IMAGE_TAG:-nightly}|" .env
chmod 600 .env

# What phones, apps and browsers are told: the name the certificate is for.
sed -i "s|^provisioning_url = .*|provisioning_url = \"https://$DOMAIN:8444\"|" configs/fcp-call-manager.toml
sed -i "s|^gateway_url = .*|gateway_url = \"https://$DOMAIN:8095\"|" configs/fcp-admin.toml
# Phones register to the name (the default tenant claims it, below) and
# fetch from it.
sed -i "s|^sip_server = .*|sip_server = \"$DOMAIN\"|; s|^public_url = \"https://\${HOST_IP}:8444\"|public_url = \"https://$DOMAIN:8444\"|" configs/fcp-admin.toml

# Push: the publisher's keys in /opt/d8/push (push.toml naming the files
# under /var/lib/fcp/push), added to the call manager when present.
mkdir -p $D8/push && chmod 700 $D8/push
if [ -f $D8/push/push.toml ]; then
    printf '\n' >> configs/fcp-call-manager.toml
    cat $D8/push/push.toml >> configs/fcp-call-manager.toml
fi
cat > docker-compose.override.yml <<YML
services:
  fcp-call-manager:
    volumes:
      - $D8/push:/var/lib/fcp/push:ro
YML

# The certificate phones, apps and browsers trust.
if [ ! -d /etc/letsencrypt/live/$DOMAIN ]; then
    if [ -n "$EMAIL" ]; then contact=(-m "$EMAIL"); else contact=(--register-unsafely-without-email); fi
    # RSA: older phones (a Poly VVX) take an RSA chain to ISRG Root X1
    # more surely than an ECDSA one.
    certbot certonly --standalone -n --agree-tos --key-type rsa "${contact[@]}" -d "$DOMAIN"
fi
mkdir -p /etc/letsencrypt/renewal-hooks/deploy
ln -sf $D8/gcp-d8/install-certs.sh /etc/letsencrypt/renewal-hooks/deploy/fcp-d8.sh

# The images, then nothing of the token left behind.
docker login ghcr.io -u fcp-d8 --password-stdin
docker compose pull -q
docker logout ghcr.io
rm -f /root/.docker/config.json

docker compose up -d certs
docker compose wait certs >/dev/null || true
RENEWED_LINEAGE=/etc/letsencrypt/live/$DOMAIN bash $D8/gcp-d8/install-certs.sh
docker compose up -d --no-build
# The files edited above are bind-mounted: `up` recreates a container for a
# new image, not for a changed file.
docker compose restart fcp-admin fcp-call-manager
bash scripts/wait-healthy.sh || { docker compose ps; exit 1; }

# Phones register by the name: the default tenant claims it.
token=$(grep ^FCP_ADMIN_AUTH_TOKEN= .env | cut -d= -f2)
curl -fsS -o /dev/null -X PUT -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    "http://127.0.0.1:8080/api/v1/tenants/tenant-00000000-0000-0000-0000-000000000000" \
    -d "{\"sip_domains\":[\"$DOMAIN\"]}" || echo "the default tenant did not take $DOMAIN (an image before v0.25.415?)"
docker compose ps --format '{{.Name}} {{.Status}}'
