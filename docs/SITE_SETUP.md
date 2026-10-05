# Setting up a site

A site is your own FCP stack, on your own network, that belongs to a
tenant in a cloud: the cloud is where the tenant is administered and
the system of record for its configuration; the site carries the
tenant's phones and calls where they are, and goes on carrying them
when the cloud cannot be reached (`docs/HYBRID_CONTROL_PLANE.md`). This
is the document for the person who runs the site: what to get from the
cloud, how to bring the site up and enroll it, what it does from then
on, how to keep it, upgrade it and take it down. Written against the
code (v0.25.274, hybrid phase 7): every key, port, route and log line
here is the binary's.

The site ships as a compose project, `deploy/docker-compose/site`: the
site's own Postgres and Redis, a certificates service, the site's admin,
the site's call manager, and `fcp-link` — the one process that talks to
the cloud. Nothing in the cloud connects to the site; every connection
is the link's, outward, over TLS.

## 1. What you need

- **A Docker host** on the site's network with `docker compose`, a LAN
  address the site's phones can reach (`HOST_IP`), and outbound TCP to
  the cloud on three ports. The call manager runs on the host's network
  and opens SIP 5090 (UDP and TCP), SIPS 5091, WSS 5092, HTTP 18084 and
  RTP 12000–12999; the admin publishes 18080, Postgres 15432 and Redis
  16379 (both on every interface; put the host behind a firewall).
- **From the cloud's operator**: the three addresses the site connects
  to — the admin's enroll listener (`8446` in the compose cloud), its
  link listener (`8447`) and the SBC's SIP-over-TLS listener (`5079`,
  which the cloud tells the site about at enrollment, `[link] sip_host`)
  — and the CA that signs the cloud's certificates, as a PEM file. In
  the compose cloud that is `ca.crt` from its `fcp-certs` volume:

  ```bash
  docker run --rm -v docker-compose_fcp-certs:/certs:ro alpine:3.20 cat /certs/ca.crt
  ```

  A cloud under a public CA needs no file for the link (the system's
  roots are trusted beside `cloud_ca`) but the call manager's
  `[sip] tls_ca_path` is still read, so give it the public root.
- **From the tenant's administrator**: an enrollment token. On the
  cloud's admin UI, *Sites* (under Advanced) → *New Site* shows the token
  once; the API is `POST /v1/sites` in the tenant, which returns
  `enrollment_token` and `enrollment_token_expires_at`. The token is
  spent by the first enrollment and refused after it expires; a new
  one comes from the site's row (*New enrollment token*,
  `POST /v1/sites/:id/enrollment-token`).

## 2. Bringing it up

```bash
cd deploy/docker-compose/site
cp /path/to/the/clouds/ca.crt cloud-ca.crt
$EDITOR .env
docker compose config --quiet            # the file resolves with this .env
docker compose up -d --build             # or: FCP_IMAGE_TAG=nightly, pull, up -d --no-build
```

`.env` holds everything that is yours to set; the configurations under
`configs/` need no edit:

| Key | What |
|---|---|
| `HOST_IP` | This host's LAN address: what phones dial, what goes in SDP. |
| `FCP_LINK_CLOUD_URL`, `FCP_LINK_LINK_URL` | The cloud admin's enroll and link listeners, `https://` and the port. |
| `FCP_CLOUD_ADMIN_URL` | The cloud's admin, named when the site's admin refuses a write. |
| `FCP_LINK_ENROLLMENT_TOKEN` | The token, until the link has enrolled; blank it after. |
| `POSTGRES_PASSWORD` | The site's database. |
| `FCP_ADMIN_AUTH_TOKEN` | The site admin's operator token. Phones fetch from the site's provisioning URL (`docs/PROVISIONING.md`). |
| `FCP_CALL_MANAGER_API_TOKEN` | The call manager's operator routes. |
| `FCP_IMAGE_TAG` | `local` builds the images here; `nightly` or a commit sha pulls the cloud's. |

What happens, in order. `certs` makes the site's own CA, the call
manager's certificate and `master.key` into the `fcp-certs` volume,
once. Postgres and Redis come up. The admin opens the database and
migrates it (an empty site database gets the whole schema). The call
manager starts on the host's network, with no users yet. The link reads
its state volume, finds no certificate, and enrolls:

```
INFO fcp_link::link: Enrolling
INFO fcp_link::link: Enrolled; the certificate and secret are in the state directory
INFO fcp_link::sync: Taking the tenant's snapshot from the cloud
INFO fcp_link::sync: Snapshot applied
```

From the snapshot on, the site's database is a copy of the tenant's
slice of the cloud — the tenant, its users with the credentials phones
are challenged with, devices, extensions, numbers, routing rules,
trunks, queues, flows, prompts, templates, meetings, locations,
emergency settings and rate plans — and every change made in the cloud
follows within a second or two. Among the trunks is the site's **link
trunk**, made by the cloud when the site enrolled: the call manager
registers it to the cloud's SBC over TLS within `[trunk_routing]
refresh_secs` (5), and from then the cloud sends the site's calls down
that connection and the site sends calls it cannot complete itself up
it. On every later start the link says `Enrolled already; the state
directory has the certificate`, takes no token, and follows the feed
from its kept cursor.

To watch it, from this directory:

```bash
docker compose ps                                   # every service healthy
docker compose logs -f fcp-link
docker compose exec fcp-link curl -s http://127.0.0.1:8096/status | jq .
```

The link's `/status` says whether it is enrolled, when the last
heartbeat succeeded, the sync's cursor and what it last applied, the
shipper's cursors, the events forwarded and the requests answered. The
same is on the cloud's Sites page for the tenant's administrator: the
link up or down and since when, the site's nodes with their releases,
the sync cursor and how far behind, the registrations, the link trunk.

After the first enrollment, blank `FCP_LINK_ENROLLMENT_TOKEN` in `.env`
(the link ignores it once enrolled; the file need not carry a spent
secret).

## 3. Phones

Phones register at `HOST_IP:5090` over UDP or TCP, at `5091` over TLS
and at `5092` over WSS, in the realm `fcp.local` — the tenant's SIP
domain; the users and their passwords are the ones made in the cloud.
For TLS the phone trusts the site's own CA:

```bash
docker run --rm -v site_fcp-certs:/certs:ro alpine:3.20 cat /certs/ca.crt
```

Provisioning is served by the site's admin on `18080`, under the site's
own provisioning URL (`/provision/<key>/`, made on the site's Devices
page), from the templates the cloud synced (`docs/PROVISIONING.md`).

Calls between the site's phones stay on the site. A call to a number the
tenant does not own, or to a user who is registered in the cloud and
not here, goes up the link trunk to the cloud's SBC, which routes it as
the tenant's own call; a call from outside for a number the tenant's
administrator assigned to this site (*Answered at site* on the number,
`site_id`) comes down the same connection. A trunk of the site's own —
a local carrier, made in the cloud with this site as *Runs at* — is
tried before the link trunk, so the site keeps calling out when the
cloud is unreachable.

## 4. The site's admin

`http://HOST_IP:18080` is the same admin UI and API as the cloud's,
with the site's operator token, over the site's copy of the
configuration. It is for looking and for the site's own operations:
status, the cluster page (the site's own nodes), the call manager's
routes, active calls, registrations, exports. A write to anything the
cloud owns is refused:

```
403 users are managed by the cloud at https://cloud.example.com:8444; make this change there and the link brings it here
```

`[api] managed_by` in `configs/fcp-admin.toml` (from
`FCP_CLOUD_ADMIN_URL`) is what makes it say so.

## 5. When the cloud cannot be reached

The site goes on with what it has (`docs/HYBRID_CONTROL_PLANE.md` §3.8):
phones keep registering and calling one another, a local trunk keeps
carrying calls out, voicemail and the site's flows keep running. What
cannot happen is what only the cloud does: calls from outside through
the cloud's numbers, calls to users registered in the cloud, and
configuration changes (the site's copy is what it was when the link went
down). The records the site writes — call detail records, recording and
voicemail rows, events — wait in the site's database and Redis; the
link's `/status` shows the shipper's cursors standing still and
`last_heartbeat_error` saying why. The cloud marks the site's link down
within its `heartbeat_stale_secs` and its SBC drops the site's binding
as the connection closes.

When the cloud is back the link's next heartbeat succeeds, the sync
follows the feed from its cursor (a user made in the cloud meanwhile
registers here), the shipper sends what the site wrote, and the call
manager registers the link trunk again on the trunk registration's own
retry (up to 35 s after a refusal).

## 6. Keeping it

Everything the site holds is in seven volumes of the project `site`
(`docker volume ls --filter label=com.docker.compose.project=site`):

| Volume | Holds | Re-derivable? |
|---|---|---|
| `postgres-data` | The site's database: the configuration copy, its users' sealed credentials, and the records not yet shipped. | The configuration, from a fresh snapshot; the records, no. |
| `fcp-certs` | The site's CA and certificate, and `master.key`, which every secret in the database is sealed under. | No. Without the key the database's secrets cannot be opened and no service starts. |
| `call-manager-data` | Recordings, voicemail, prompts, the CDR spool. | No. |
| `admin-data` | Exports. | Mostly. |
| `link-data` | The link's key, certificate and the site's secret from enrollment; the sync and ship cursors. | With a new enrollment token from the cloud. |
| `redis-data` | Registrations, the event stream. | Yes. |
| `site-firmware` | The firmware images the tenant's targets name, fetched from the cloud by the link (`docs/PROVISIONING.md` §11). | Yes: the link fetches them again. |

The cloud stack's scripts keep a site the way they keep the cloud
(`docs/BACKUP_AND_RESTORE.md`), pointed at this project:

```bash
COMPOSE_DIR=$PWD ../scripts/backup.sh /backups/site-2026-09-26
```

takes the dump through the `postgres` container, a tar of each volume
above that exists (the cloud's volumes that a site has not are named in
the manifest as absent), the manifest and checksums; and

```bash
docker compose down -v
COMPOSE_DIR=$PWD ../scripts/restore.sh /backups/site-2026-09-26
```

brings it back: the checksums, the key against the manifest's key id,
the volumes, the database, `up`. The link comes back enrolled, with its
cursors, and catches up on the feed. A restore without `link-data` —
a site rebuilt from the database alone — needs a new enrollment token in
`.env`: the link enrolls afresh, and the cloud replaces the site's
certificate, secret and link trunk password.

## 7. Upgrading

The site runs the cloud's images. The cloud's Sites page shows the
release every node of the site runs and the link's own; keep a site on
the cloud's release, or the one before it. To upgrade:

```bash
COMPOSE_DIR=$PWD ../scripts/backup.sh /backups/site-before-upgrade
docker compose pull            # FCP_IMAGE_TAG=nightly or a sha; or git pull and up -d --build
docker compose up -d --no-build
docker compose ps
```

The first service to open the database migrates it; an older image
against the migrated schema refuses to start and names the migration
(`docs/BACKUP_AND_RESTORE.md` §5), so the way back is the backup. The
link's protocol is versioned (`protocol` on its status); a cloud that
no longer speaks the site's version refuses its heartbeat, and the log
line says so.

## 8. Taking it down

The tenant's administrator disables the site in the cloud (*Disable*,
`POST /v1/sites/:id/disable`): its certificate and secret are refused
from then on, the link's heartbeats fail, and the site runs on alone
until stopped. Deleting the site (`DELETE /v1/sites/:id`) removes its
row and its link trunk in the cloud. At the site, `docker compose
down -v` removes everything, the key included; take a backup first if
the records or recordings that never shipped are wanted.

## 9. When it does not work

- **`Enrollment failed; trying again in 30 s`** in the link's log. The
  line carries the cause: `401` is a token spent, expired or mistyped
  (a new one from the cloud's Sites page, into `.env`, `docker compose
  up -d fcp-link`); a TLS error is `cloud-ca.crt` not the CA that signs
  the cloud's listeners, or `FCP_LINK_CLOUD_URL` naming a host the
  cloud's certificate does not (the compose cloud's certificate names
  its `HOST_IP`); a connection refused is the port or a firewall.
- **Enrolled, but `Heartbeat failed`.** `FCP_LINK_LINK_URL` is the
  mutual-TLS listener (`8447`), not the enroll listener; or the site
  was disabled in the cloud.
- **`Sync failed; trying again`** with `sync_error` on the cloud's site
  record. The line names the object; the link retries from its cursor.
- **Phones do not register.** The user must exist in the cloud and have
  reached the site: the cloud's site record shows `sync_behind`; the
  site's admin lists the tenant's users. The realm is `fcp.local`;
  `HOST_IP` must be the address the phone reaches.
- **Calls to the cloud fail, or nothing comes down.** The link trunk
  must be registered: the call manager's `/status` (`18084`, its token)
  lists the trunk and its registration; the cloud SBC's
  `GET /admin/sites` lists the sites bound to it. The call manager
  reaches the SBC on the port `[link] sip_host` named in the cloud
  (`5079`) over TLS, verified against `cloud-ca.crt`.
- **A service exits at once.** `docker compose logs <service>`: the
  binary prints the rule it broke; `docker compose run --rm fcp-link
  --check-config --config /app/fcp-link.toml` reproduces it with this
  `.env`, secrets masked.
- **`postgres` is unhealthy.** Its volume holds a database from a
  different `POSTGRES_PASSWORD`; `down -v` and start again — with the
  backup first, as above.

## 10. What the nightly proves

The Images workflow brings a site up beside its cloud stack on one
runner, from the same images (`.github/workflows/images.yml`, the
`stack` job): it makes a site for the tenant `live` through the cloud's
admin API, writes the token and the runner's addresses into the site's
`.env` and the cloud's CA into `cloud-ca.crt`, starts the project, and
`crates/fcp-link/tests/compose_site.rs` checks that the cloud's site
record says enrolled with the link up and the site's call manager on it
with a link trunk, that a user made in the cloud registers at the site's
call manager, that the site is bound at the cloud's SBC, that the site's
admin refuses to make a user and names the cloud, and that the site is
no further behind than nothing; then `backup.sh` takes the site's
backup with its `link-data`. The site's three configurations are
checked by `--check-config` with the site's `.env` in the binaries'
smoke tests on every CI run.
