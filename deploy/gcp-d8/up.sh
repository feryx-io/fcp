#!/bin/bash
# The D8 server (docs/DEEP_TESTING.md D8): the compose stack from the
# nightly images on one GCE VM with a static address, reached by real
# phones, a real carrier and real push. Run from this directory:
#
#   bash up.sh <domain> [email]         # e.g. d8.feryx.io; the email is
#                                       # Let's Encrypt's, for expiry notices
#
# The domain's A record must already name the address `fcp-d8` holds
# (made here if missing; printed at the end). Needs gcloud and gh (a token
# that can read the GHCR packages). Nothing secret is left on this machine:
# the stack's secrets are made on the VM, the GHCR token is piped to
# `docker login` there and logged out after the pull.
set -euo pipefail

DOMAIN=${1:?usage: up.sh <domain> [email]}
EMAIL=${2:-}
PROJECT=${PROJECT:-brayz-dev}
REGION=${REGION:-us-central1}
ZONE=${ZONE:-us-central1-b}
NAME=${NAME:-fcp-d8}
MACHINE=${MACHINE:-e2-standard-4}
HERE=$(cd "$(dirname "$0")" && pwd)
g() { gcloud --project "$PROJECT" "$@"; }

if ! g compute addresses describe "$NAME" --region "$REGION" >/dev/null 2>&1; then
    g compute addresses create "$NAME" --region "$REGION"
fi
IP=$(g compute addresses describe "$NAME" --region "$REGION" --format='value(address)')
echo "address: $IP"

# Asked of the domain's own nameservers, as Let's Encrypt asks: a public
# resolver may still hold the "no such name" it cached before the record.
zone=${DOMAIN#*.}
ns=$(dig +short NS "$zone" | head -1)
resolved=$(dig +short "$DOMAIN" A ${ns:+@"$ns"} | tail -1)
if [ "$resolved" != "$IP" ]; then
    echo "$DOMAIN resolves to '${resolved:-nothing}', not $IP: add the A record first" >&2
    exit 1
fi

# What the world may reach. Postgres, Redis and every operator port
# (8080-8094, 9090, 3000) stay closed; SSH comes only through IAP.
rule() {
    local name=$1; shift
    g compute firewall-rules describe "$name" >/dev/null 2>&1 ||
        g compute firewall-rules create "$name" --network default --target-tags "$NAME" "$@"
}
rule "$NAME-iap-ssh" --allow tcp:22 --source-ranges 35.235.240.0/20
# 80 for Let's Encrypt; 8443-8445 the proxy (gateway, admin, events);
# 8095 the WebRTC gateway; SIP on the call manager (5060/5061, WSS 5081)
# and the SBC (5078, TLS 5079).
rule "$NAME-signalling" --source-ranges 0.0.0.0/0 \
    --allow tcp:80,tcp:8443-8445,tcp:8095,tcp:5060-5061,tcp:5081,udp:5060,tcp:5078-5079,udp:5078
# RTP: call manager, conference, IVR, voicemail, SBC, media, WebRTC gateway.
rule "$NAME-rtp" --source-ranges 0.0.0.0/0 \
    --allow udp:10000-10999,udp:20000-20999,udp:22000-22999,udp:24000-24999,udp:28000-28999,udp:30000-30999,udp:34000-35999

if ! g compute instances describe "$NAME" --zone "$ZONE" >/dev/null 2>&1; then
    g compute instances create "$NAME" --zone "$ZONE" --machine-type "$MACHINE" \
        --image-family debian-13 --image-project debian-cloud \
        --boot-disk-size 50GB --boot-disk-type pd-balanced \
        --address "$IP" --tags "$NAME" \
        --metadata enable-oslogin=TRUE \
        --metadata-from-file startup-script="$HERE/startup.sh"
fi

ssh_() { g compute ssh "$NAME" --zone "$ZONE" --tunnel-through-iap "$@"; }
echo "waiting for the VM's packages"
until ssh_ --command 'test -f /var/lib/fcp-d8-ready' 2>/dev/null; do sleep 15; done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar czf "$tmp/d8.tgz" -C "$HERE/.." docker-compose gcp-d8
g compute scp --zone "$ZONE" --tunnel-through-iap "$tmp/d8.tgz" "$NAME:d8.tgz"
ssh_ --command 'sudo mkdir -p /opt/d8 && sudo tar xzf d8.tgz -C /opt/d8 && rm d8.tgz'
gh auth token | ssh_ --command "sudo FCP_IMAGE_TAG=${FCP_IMAGE_TAG:-nightly} bash /opt/d8/gcp-d8/setup.sh '$DOMAIN' '$EMAIL' '$IP'"

cat <<DONE

The D8 server is up.
  admin:   https://$DOMAIN:8444/admin/
  SIP:     $DOMAIN:5060 (UDP/TCP), sips:$DOMAIN:5061, wss://$DOMAIN:5081
  carrier: $IP:5078 (the SBC) or the call manager on 5060
  token:   sudo grep FCP_ADMIN_AUTH_TOKEN /opt/d8/docker-compose/.env  (on the VM)
  ssh:     gcloud compute ssh $NAME --project $PROJECT --zone $ZONE --tunnel-through-iap
Tear it down with: bash down.sh
DONE
