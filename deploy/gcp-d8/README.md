# The D8 server

The compose stack from the nightly images on one GCE VM with a static
address and a Let's Encrypt certificate, so that real phones, a real
carrier and real push can reach it (`docs/DEEP_TESTING.md` D8). It is up
for the D8 sessions and removed after them.

```bash
bash up.sh d8.example.com          # the A record must already name the address
bash down.sh                       # VM, firewall rules and address, everything on it
```

`up.sh` reserves the address `fcp-d8` (and stops if the name does not
resolve to it yet), opens the firewall, makes the VM, copies
`deploy/docker-compose` and this directory to `/opt/d8`, and runs
`setup.sh` there with a GHCR token from `gh` on its standard input.
`setup.sh`:
- makes every `change-me` secret in `.env` random, once, into
  `/opt/d8/secrets.env`, sets `HOST_IP` to the address and runs the
  `nightly` images (`FCP_IMAGE_TAG=<sha> bash up.sh …` for another);
- points the provisioning URL and the WebRTC join URL at the name;
- adds `/opt/d8/push/push.toml` to the call manager's configuration when
  it exists, with `/opt/d8/push` mounted at `/var/lib/fcp/push`;
- gets the certificate with certbot and installs it (`install-certs.sh`,
  also certbot's renewal hook) for the proxy, the call manager's SIPS and
  WSS, the WebRTC gateway and the SBC's TLS, in place of the compose CA's;
- pulls, logs out of GHCR, and starts the stack.

What is open to the world: HTTPS 8443–8445 (gateway, admin, events)
and 8095 (WebRTC), SIP on 5060/5061/5081 (call manager) and 5078/5079
(SBC), the RTP ranges, and 80 for Let's Encrypt. Postgres, Redis and the
operator ports are not; SSH is through IAP.

## Push

To wake Anvil, put the publisher's keys on the VM and run `up.sh` again:

```bash
sudo install -m 600 AuthKey_XXXXXXXXXX.p8 service-account.json /opt/d8/push/
sudo tee /opt/d8/push/push.toml <<'EOF'
[push]
[[push.apns]]
team_id = "TEAMID"
key_id = "XXXXXXXXXX"
key_file = "/var/lib/fcp/push/AuthKey_XXXXXXXXXX.p8"
topics = ["com.example.anvil.voip"]
sandbox = true            # a development build

[[push.fcm]]
project_id = "my-project"
service_account_file = "/var/lib/fcp/push/service-account.json"
EOF
```
