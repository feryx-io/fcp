#!/bin/bash
# Puts the Let's Encrypt certificate where the services that face phones,
# apps, browsers and carriers read theirs: the proxy (HTTPS 8443-8445),
# the call manager (SIPS 5061, WSS 5081), the WebRTC gateway (8095) and
# the SBC (5079), in place of the compose CA's. Nothing in the stack
# checks those four against the compose CA. Also certbot's deploy hook,
# which sets RENEWED_LINEAGE.
set -euo pipefail
LIVE=${RENEWED_LINEAGE:?}
VOLUME=docker-compose_fcp-certs
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp -L "$LIVE/fullchain.pem" "$LIVE/privkey.pem" "$tmp/"
docker run --rm -v "$VOLUME":/certs -v "$tmp":/le:ro alpine sh -c '
    for svc in fcp-proxy fcp-call-manager fcp-webrtc-gw fcp-sbc; do
        owner=$(stat -c %u:%g /certs/$svc.key)
        cp /le/fullchain.pem /certs/$svc.crt
        cp /le/privkey.pem /certs/$svc.key
        chown "$owner" /certs/$svc.crt /certs/$svc.key
        chmod 644 /certs/$svc.crt; chmod 600 /certs/$svc.key
    done'
cd /opt/d8/docker-compose
for svc in fcp-proxy fcp-call-manager fcp-webrtc-gw fcp-sbc; do
    [ -n "$(docker compose ps -q $svc 2>/dev/null)" ] && docker compose restart $svc
done
true
