#!/bin/sh
# The compose stack's certificates (docs/PRODUCTION_STACK.md phase 3), made
# once by the `certs` service into the `fcp-certs` volume: a CA, and a
# certificate per service that terminates TLS, each naming the service on
# the bridge network, the host's address and the loopback. Idempotent: a
# file that exists is kept, so a restart keeps the CA the phones trust.
# Regenerate everything with `docker compose down -v` (which also drops
# the database) or `docker volume rm docker-compose_fcp-certs`.
set -eu

DIR="${CERTS_DIR:-/certs}"
HOST_IP="${HOST_IP:-127.0.0.1}"
DAYS="${CERT_DAYS:-3650}"
SERVICES="${CERT_SERVICES:-fcp-call-manager fcp-registrar fcp-conference fcp-sbc fcp-media fcp-webrtc-gw fcp-proxy}"

mkdir -p "$DIR"
cd "$DIR"

if [ ! -f ca.crt ]; then
    echo "certs: making the CA"
    openssl req -x509 -newkey rsa:3072 -nodes -sha256 -days "$DAYS" \
        -subj "/CN=FCP compose CA" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign" \
        -keyout ca.key -out ca.crt >/dev/null 2>&1
    chmod 600 ca.key
else
    echo "certs: keeping the CA"
fi

for svc in $SERVICES; do
    if [ -f "$svc.crt" ] && [ -f "$svc.key" ]; then
        echo "certs: keeping $svc"
        continue
    fi
    echo "certs: making $svc"
    ext="subjectAltName=DNS:$svc,DNS:localhost,DNS:host.docker.internal,IP:127.0.0.1,IP:$HOST_IP"
    openssl req -new -newkey rsa:2048 -nodes -sha256 \
        -subj "/CN=$svc" -keyout "$svc.key" -out "$svc.csr" >/dev/null 2>&1
    printf '%s\nextendedKeyUsage=serverAuth,clientAuth\n' "$ext" > "$svc.ext"
    openssl x509 -req -in "$svc.csr" -CA ca.crt -CAkey ca.key -CAcreateserial \
        -days "$DAYS" -sha256 -extfile "$svc.ext" -out "$svc.crt" >/dev/null 2>&1
    rm -f "$svc.csr" "$svc.ext"
    chmod 600 "$svc.key"
done
chmod 644 ca.crt ./*.crt
echo "certs: ready in $DIR"
ls -1 "$DIR"
