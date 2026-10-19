#!/bin/bash
# Removes the D8 server: the VM and its disk, its firewall rules and its
# address. Everything on it goes, the database included. The domain's A
# record is yours to remove.
set -euo pipefail
PROJECT=${PROJECT:-brayz-dev}
REGION=${REGION:-us-central1}
ZONE=${ZONE:-us-central1-b}
NAME=${NAME:-fcp-d8}
g() { gcloud --project "$PROJECT" --quiet "$@"; }
g compute instances delete "$NAME" --zone "$ZONE" || true
for r in iap-ssh signalling rtp; do g compute firewall-rules delete "$NAME-$r" || true; done
g compute addresses delete "$NAME" --region "$REGION" || true
