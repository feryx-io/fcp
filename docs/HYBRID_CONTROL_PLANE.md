# Hybrid: the control plane and the site

Designed 2026-09-25, after the feature review that followed backup,
restore and upgrade, as the project to follow the nightly stack
(`docs/NIGHTLY_STACK.md`). Agreed 2026-09-25: the nine questions in §6,
each settled on its recommendation.

The business runs FCP three ways. **Cloud**: every binary in the
operator's cloud, tenants reached over the internet — what the compose
stack and the multi-region guide already describe. **Premises**: every
binary on the customer's own network, which `docs/SINGLE-NODE-SETUP.md`
and the compose stack already cover. **Hybrid**: a customer's phones,
calls and trunks served by binaries on the customer's network, while the
customer's numbers, carriers, administration, billing, compliance and
analytics live in the cloud — and the customer's site keeps calling when
the link to the cloud is down. Hybrid is the one the code cannot do, and
this is its design.

## 1. What the code has, and what it lacks

### 1.1 Every node opens the stores directly

A node is a binary with a Postgres URL and a Redis URL. It opens
thirty-seven store traits over the one database (users, devices,
extensions, line bindings, numbers, routing rules, trunks, queues, flows,
prompts, recordings, voicemail, CDRs, billing, compliance, the key ring,
…) and one Redis for everything else: cluster membership, the conference
registry and placement, parking, cluster-wide call and trunk leases, the
event bus, the CDR stream, presence, and registrations when the Redis
backend is on. A call manager has one Redis and the prefix is fixed
(`docs/MULTI_REGION_DEPLOYMENT.md` §1). Configuration is "database
primary, TOML seed": every section is versioned in the database with an
audit, and the TOML holds only bootstrap (`fcp_core::config_store`).

So a customer site running the binaries against the cloud's stores would
put the cloud's Postgres and Redis on the customer's network, pay a WAN
round trip on the hot path (registrations are primary-always: two reads
per call measured, `docs/MULTI_REGION_FOLLOWUPS.md`), and lose every call
the moment the link dropped, since to a node a store it cannot reach is
`store_unreachable`, not survivability. Isolation is by `tenant_id`
columns in one schema; a site that could reach the database could reach
every tenant. None of that is fixable by configuration. Hybrid needs the
site to have stores of its own and a way to keep them right.

### 1.2 What crosses the link already

Three of the four flows a site needs have a shape in the code:

- **Events** have an HTTP ingest (`POST /ingest` on the events server,
  with a token), and a bridge that forwards a region's stream to its
  peers with loop-safe envelopes (`docs/MULTI_REGION.md`).
- **CDRs** are spooled on the node, shipped to a stream, stored by a
  processor that writes `ON CONFLICT (cdr_id, start_time) DO NOTHING`,
  carry a `schema_version` the processor refuses when newer than its
  own, and are archived per tenant per day (`docs/CDR_PIPELINE.md`).
- **Health** travels in heartbeats that carry a node's version, region,
  capacity and its view of the cluster; nodes call each other with a
  shared cluster secret (`fcp_cluster::NodeClient`); the admin's cluster
  page shows every node and says when releases differ.

The fourth, **configuration down**, has no shape at all: nothing tells a
second database what changed in the first. What there is: every
configuration section is versioned, the deletion log is written by
triggers so that no code path can skip it (`docs/COMPLIANCE_FOLLOWUPS.md`),
and a `CallRecording` row already names the node that holds its file.

### 1.3 What the code calls an edge

`fcp_sip::EdgeConfig` and `[e911] trusted_edges` name an SBC or WebRTC
gateway that vouches for a caller to a call manager behind it. That is
a hop inside one deployment. This design calls the customer's premises
deployment a **site**, the word the E911 model already uses for a place
with an address, and leaves "edge" alone.

### 1.4 What is sound

The binaries already run, all fourteen of them, against a Postgres and a
Redis of their own: the compose stack is exactly the shape of a site, and
the nightly builds, publishes and drives it. Backup, restore and upgrade
are procedures with tests. The SBC takes calls from a source it knows as
a tenant's trunk and registers trunks over SRV; tenant trunks route a
tenant's calls out over the tenant's own carriers (`docs/TENANT_TRUNKS.md`).
Conference trunks between nodes are mutual TLS. E911 places a device by
the site it is at. The pieces of a hybrid are here; what is missing is
the link between two of everything.

## 2. The model

**The cloud is the system of record for configuration.** Tenants, users
and their credentials, devices, extensions, line bindings, numbers,
routing rules, queues, flows, prompts, templates, meetings, E911 sites
and locations, rate plans: made and changed through the cloud's API and
admin UI, nowhere else. The cloud runs the SBC toward carriers, billing,
the compliance stores, the CDR warehouse and archive, the events server
that fires webhooks, and, for tenants without a site, everything.

**A site is a tenant's own stack plus a link.** A compose stack (or a
cluster) of the existing binaries on the customer's network, with its
own Postgres, Redis and master key — every binary, so that what the site
does it does alone: registrations, calls between its phones, calls over
its own local trunks, voicemail, queues, IVR, conferencing, presence,
recording. Plus one new binary, **`fcp-link`**, which is enrolled to the
cloud and does four things: keeps the site's database a copy of the
tenant's slice of the cloud's configuration; ships the site's records up
(CDRs, events, the rows for recordings and voicemail); carries the
site's health to the cloud; and executes what the cloud asks of the site
(drain, a compliance deletion list, a diagnostic).

**Calls that need the cloud go over a trunk.** At enrollment the cloud
provisions the site a registering SIP trunk to the cloud SBC (TLS, SRTP,
credentials minted then), and the cloud SBC learns the site as that
tenant's trunk. The tenant's numbers are routed to the site's trunk;
outbound calls the site cannot carry on a local trunk leave through it;
emergency calls use a local emergency-capable trunk when the site has one
and the cloud's otherwise, placed by the site's E911 location as today.

**Direction of truth is fixed.** Configuration flows down and is
read-only at the site: the site's admin API refuses a write to a synced
object and says where to make it. Runtime records flow up. Nothing is
merged.

**The link down is a mode, not an outage.** With the cloud unreachable
the site runs on its last configuration; CDRs and events spool as they
do today; recordings stay where they are; phones register and call. On
reconnect the link catches up from its cursor and the spools drain. What
is lost meanwhile is what needs the cloud: carriers through the cloud
SBC (unless a local trunk carries them), administration, and the
cloud's view of the site.

## 3. The link

### 3.1 Enrollment

*As built in phase 1:* the enroll listener (`[link] enroll_bind`, TLS
with the admin's own certificate) takes the token and a certificate
signing request and answers with the certificate, the link CA and a
secret; the link listener (`[link] bind`) requires a client certificate
from the link CA at the TLS layer, and every heartbeat also carries the
secret as a bearer token, which names the site — so revoking a site is
refusing its secret, with no CRL to keep. `fcp-link` keeps the key, the
certificate, the CA and `site.json` in `[link] state_dir`.

A tenant administrator makes a site in the cloud (`POST /v1/sites`) and
receives a one-time enrollment token. `fcp-link` at the site is started
with the cloud's address and that token; it connects over TLS, presents
the token, and receives a client certificate from the cloud's link CA
with the tenant and site ids in its subject, and the credentials of its
SIP trunk. Every later connection is mutual TLS; the token is spent. The
link renews its certificate before it expires, and the cloud revokes a
site by refusing its certificate. The site opens no inbound port: every
connection is site to cloud.

### 3.2 Configuration down: the journal, the snapshot, the feed

Every table a site needs gets a **journal**: a per-tenant, monotonically
numbered log of (table, primary key, operation, version), written by
triggers on insert, update and delete — as the deletion log is, so that
no code path, migration or hand-run SQL can change a synced row without
the site hearing of it. The link's first pull is a **snapshot**: every
synced row of the tenant, with the journal's current sequence as its
cursor. After that the link follows the **feed** — `GET
/link/v1/changes?since=<cursor>` held open, or a WebSocket — and applies
each change to the site's database through the same storage crate the
cloud uses, in journal order, idempotently.

Secrets cross the link once, over mutual TLS, in the clear: a site must
challenge its phones with the cloud unreachable, so it holds their
digest credentials, sealed under its own master key on arrival. The
cloud's master key never leaves the cloud, and the site's never reaches
it.

What syncs: the tenant; users, credentials and roles; devices; extensions
and line bindings; numbers; routing rules and LCR routes; the tenant's
trunks (local ones run at the site, the link trunk is the site's own);
queues and agents; flows and prompts; configuration templates; meetings;
E911 sites, locations, networks and contacts; rate plans (the site rates
nothing, but shows costs). What does not: other tenants; the cloud's
own trunks; CDRs, billing ledgers, audit logs, compliance requests, the
key ring.

### 3.3 Records up

- **CDRs.** The site's own processor stores its records locally, as
  today; the link ships every stored record to the cloud (`POST
  /link/v1/cdrs`, in batches, from a cursor on the site's `cdrs`
  table), where the cloud's processor rates and stores it under the
  tenant, idempotently by `cdr_id`. The cloud bills; the site's copy is
  its own reporting and its survivability.
- **Events.** The site's events server forwards its stream to the cloud
  over the link as one more bridge peer, so webhooks fire in the cloud
  once, with the site named in the envelope.
- **Recordings and voicemail.** The files stay at the site. Their rows
  are shipped like CDRs, naming the site and the path; the cloud's
  download and playback routes fetch through the link on demand. A
  tenant policy may later copy files to the cloud's archive.
- **Registrations and presence.** A summary in the link's heartbeat
  (who is registered, at how many contacts, since when), so the cloud's
  pages show the site's phones; not the rows.

### 3.4 Health

The link heartbeats to the cloud with the site's cluster view, every
node's version, the sync cursor and its lag, the spool depths. The
cloud's cluster page gains a Sites section; `system.site_link_changed`
says when a site's link comes and goes; `fcp_site_link_up` and
`fcp_site_sync_lag_seconds` are the alerts.

### 3.5 Versions

The link protocol is versioned, and the cloud serves the current and the
previous version; a site may lag one release and is told, on the cluster
page and in `system.version_skew`, when it does. Synced rows carry the
schema they were written under, as CDRs do, and the migration discipline
(`docs/BACKUP_AND_RESTORE.md` §5) already forbids the contraction that
would break an older site.

### 3.6 Calls across

*As built in phase 4.* A site reaches the cloud over one trunk, made at
enrollment, and nothing reaches the site but down the connection the
site opened.

**The link trunk.** When the admin names `[link] sip_host` — the cloud
SBC's SIP-over-TLS address, `host:port`, as a site dials it — every site
enrolled there is given a tenant trunk: `site_id` the site's, `link` set
to `cloud`, hosts the SIP address, TLS, credentials whose username is
the site's id and whose password is minted at enrollment (and again at
every re-enrollment, since the link that held the old one is gone) in
`[link] sip_realm`, which must equal the SBC's `external_domain`;
registering, with a two-minute expiry so the connection and any NAT
binding on the way stay alive between calls; bidirectional; priority
100, after the site's own carriers; emergency-capable; never probed.
`Site.trunk_id` names it, `/v1/sites` shows it, disabling a site disables
it and deleting a site deletes it. With no `sip_host`, sites sync
configuration and carry no calls across, as before.

**Where a trunk runs.** `Trunk.site_id` says which site a trunk runs at:
a carrier of the site's own, made through `/v1/trunks` with `site_id`,
or its link. The cloud's routing nodes route out on no trunk that has
one, and a link peer is never a route: it is reached by its
registration. The link applies only its own site's trunks — the cloud's
carriers never reach the site, which phase 3 had let them do, so a site
was registering the cloud's carrier accounts — and turns the link
trunk's end from `cloud` into `site` as it lands, which is how every
node at the site knows which site it is (`TrunkSet::site`).

**The cloud SBC.** With storage it loads, of the tenants' trunks, the
link peers alone (`TrunkSet::link_peers_only`) and still routes by the
platform's trunks (`docs/TENANT_TRUNKS.md` §13.8). Its registrar now
authenticates: a site's REGISTER is challenged against the link accounts
(`fcp_sip::link_peers::SiteAccounts`), and a phone's would be refused,
which none sends there. A registration over TLS leaves an RFC 5626
flow, and the SBC keeps, per link peer, where the site registered from
and that flow (`fcp_sip::link_peers::LinkPeers`, `/admin/sites` on the
SBC): a binding lasts as long as the registration says and no longer
than its connection. A site's own calls are known by where they come
from — exactly the connection they share with the REGISTER, nothing
looser, since two peers may share an address — and are the tenant's
own, before any carrier's address is looked at.

**A number at a site.** `PhoneNumber.site_id` (`site_id` on
`/v1/phone-numbers`) says which site answers a number. A call for it,
from a carrier or from another site of the tenant's, goes down that
site's flow with the number as the target's user and the site's
registration as the contact; a site not bound answers 480. At the site
the call arrives on the link trunk, whose hosts are the SBC, and is the
tenant's: the number's line binding rings the site's phones as ever.

**A site's call to the outside.** At the SBC it is the site's tenant's
call (`Call::link_site`), never refused as a carrier's call sent back
out, and goes to the platform's trunk — the cloud's call manager — with
`X-FCP-Caller-Tenant` and `X-FCP-Caller-Site`, which the call manager
believes from its `[e911] trusted_edges`. A site's caller has no user
of the cloud's to check against — the site authenticated the phone — so
the routing engine's user check is skipped for it, and the call is
routed, rated and recorded as the tenant's, out on the tenant's
carriers or the platform's, as any of the tenant's calls.

**Media behind NAT.** A site's SDP names an address the cloud cannot
reach. Every leg to or from a site is latched rather than pinned
(`MediaManager::latch_leg`): the relay learns the leg's RTP address from
its first packet, from the site's signalling address and nowhere else,
and sends there. The SBC's own SDP carries `[core.nat] external_ip`.

**The site's call manager.** `[sip] tls_ca_path` trusts a CA beside the
system's roots — the cloud's, when its SBC's certificate is not from a
public CA — and the TLS client is built whether or not the node listens
on TLS. The site's end of a link is registered by call managers alone
(`TrunkRegistrationContext::registers_link`); the site's IVR, voicemail
and conference servers route out on it and leave the binding to the
node the cloud's calls must arrive at.

**Proved** by `fcp-link/tests/calls.rs`: a cloud (admin, SBC with TLS
from the test's CA, call manager, a carrier on the loopback) and a site
(link, call manager, Alice's phone). Enrollment made the trunk; the SBC
lists the site bound down a flow; the carrier's call to the site's
number rings Alice with audio both ways and the carrier's hangup reaches
her; Alice's call to the outside is asked of the carrier as hers, with
audio both ways, and the cloud's call manager recorded it as the
tenant's out on the tenant's carrier; the site's call manager killed,
the site is unbound within seconds and the carrier's next call answers
480.

**Two defects found on the way.** A TLS trunk's INVITE went out over
UDP: the route's URI was kept as a bare contact, whose parameters are
contact parameters, so `;transport=tls` was stripped; every route is
now kept in angle brackets. And the media relay told the two legs of a
call apart by IP alone, so a leg waiting to learn its peer from a named
address lost its packets to the other leg whenever that leg's peer
shared the address — two peers behind one NAT, every node on a
loopback; forge-media v2026.09.26 classifies a packet by its exact
address first, then lets a named-address latch claim the first packet
from its address.

**Left for later.** A site binds at one SBC: a carrier's call landing
on another SBC of the cloud's finds no binding (an SBC-to-SBC hop, or
the binding published to the cluster). A site's calls are recognised
by the exact connection its REGISTER came on; a call on a fresh
connection, before the next refresh re-binds, is not the site's. Media
over the link is RTP, signalling TLS: SRTP on the link trunk waits on
inbound SDES at the SBC. An emergency call from a site leaves on the
link trunk with ELIN delivery when the site has no emergency-capable
trunk of its own, and the cloud places it as a site's call with no
location of its own: the site's PIDF-LO is not yet carried through, and
the cloud does not yet place the call by the site's E911 location. The
cloud's BYE to a site goes down the flow, as its INVITE does; that no
other in-dialog request dials the site's private contact is to be
proved with the partition harness in phase 6.

### 3.7 Records up, as built

*Phase 5.* Four flows, each site to cloud, over the link's own
connection; nothing in the cloud connects to the site.

**A shipping order.** Migration 081 gives `cdrs`, `call_recordings`,
`conference_recordings`, `voicemail_mailboxes` and `voicemail_messages`
a `ship_seq`, from one sequence, taken as a row is written and again
whenever it changes. The link keeps a cursor per table beside its
certificate (`ship.json`) and reads "everything after my cursor" in
order (`cdrs_after`, `call_recordings_after`, `recordings_after`,
`mailboxes_after`, `messages_after`), in batches of `[site] ship_batch`,
every `[site] ship_interval_secs` when nothing waited. Rows from before
the migration have no sequence and are not shipped. Deletions come from
the deletion log, which triggers already write for every recording and
voicemail row, after a cursor of its own.

**Call records.** `POST /link/v1/cdrs` takes a batch; the cloud sets
aside whatever the site's copy of the plans said — the site rates with
the synced plans and charges nobody — marks each record with the site
(`CallDetailRecord.site_id`, a column on `cdrs`) and hands it to the
cloud's own processor: rated with the cloud's plans, charged to the
tenant, stored idempotently by `cdr_id`, announced as `cdr.written`
naming the site. The site's copy stays its own reporting.

**Recordings and voicemail.** `POST /link/v1/records` takes the rows —
`ShippedRecords`: puts by table, and deletions by class — each checked
to be the tenant's and written naming the site (`site_id` on the three
tables; a mailbox rides up with its messages, since a message names its
mailbox). A deletion the site logged is mirrored in the cloud under the
cause `site`. The files stay at the site.

**Files through the link.** A site opens no port, so what the cloud
wants of a site's file is written down (`site_requests`, migration 081:
`FetchFile` or `DeleteFile`, with the class and the path), collected by
the link on a long poll (`GET /link/v1/requests`, up to 30 s) and
answered on the link's own connection (`POST /link/v1/requests/:id`:
the outcome in `x-fcp-outcome`, the bytes as the body). The link touches
only what lies under `[site] file_roots`. In the cloud,
`fcp_admin::site_files::SiteRequests` does the asking: a recording
download (`GET /v1/recordings/:id/download`) or a voicemail playback
(`GET /v1/me/voicemail/messages/:id/audio`) of a row that names a site
waits up to a minute for the bytes; a subject export fetches a site's
files the same way, and names the site in `missing_files` when it
cannot; a deletion — the API's, a subject erasure's, or the reaper's for
a row retention removed (`pending_file_deletions.site_id`, copied by the
deletion triggers) — is queued for the site and counted as
`files_asked_of_sites`.

**Events.** The link reads the site's own event stream as one more
consumer group (`link`), keeps the tenant's events, leaves `cdr.written`
behind (the cloud announces its own once it has the record) and sends
them up (`POST /link/v1/events`) grouped by the node that published
them. The cloud's admin posts them to its events server's ingest under
that node with `x-origin-site`; the envelope carries `origin_site`, and
the events are the cloud's own — not relayed — so the tenant's webhooks
fire in the cloud, once. The site keeps no webhook endpoints (they are
not synced), so nothing fires twice.

**Registrations.** The heartbeat carries `registrations`: every address
of record registered at the site (from the site's Redis), at how many
contacts, since when, capped at a thousand, with the totals; `/v1/sites`
shows it.

**Proved** by `fcp-link/tests/records.rs`: a record, a call recording, a
conference recording, a mailbox and a message written at the site land
in the cloud naming the site, the record with its site rating set aside
and `cdr.written` announced naming the site; the cloud's download of the
conference recording is the site's file, through the link; deleted in
the cloud, the file goes at the site; deleted at the site, the message
goes in the cloud; an event published at the site reaches the cloud's
events server named as the site's under the node that published it;
and the heartbeat says who is registered.

**Left for later.** Retention and subject requests reach a site's rows
and files through what is above; a retention policy of the site's own
still runs there on the site's copies (a message removed at the site by
its retention is mirrored as a deletion). A site's mailbox made at the
site and one made in the cloud for the same user are two mailboxes.
A file sent up is held in memory on both ends (up to 512 MB); a policy
copying a site's files to the cloud's archive is not written.

### 3.8 Survivability, as built

*Phase 6.* The floor cuts the link — two TCP proxies the test drops and
restores (`fcp_testkit::cut::CutLink`), one in front of the admin's link
listener, one in front of the SBC's TLS listener, as a network between
a site and its cloud would fail — and watches both sides.

**Cut.** The cloud says the site's link is down within
`[link] heartbeat_stale_secs` and the SBC drops the site's binding as its
connection closes. The site goes on with what it has: phones register
and call one another with audio, a call to the outside leaves on the
site's own carrier (a trunk with the site's `site_id`, priority ahead of
the link's), the records and events it writes wait — the shipper's
cursors do not move, the event consumer's entries stay unacknowledged,
the link's status says why — and the cloud's requests wait in
`site_requests`.

**Return.** The cloud says the site is back on the next heartbeat; the
sync follows the feed from its cursor, so what the cloud changed during
the cut (a user made) registers at the site; the shipper sends what the
site wrote during the cut, records and events alike, and the cloud
stores and announces them naming the site; the site's link registers
again at the next tick and the cloud's calls reach the site as before.

**Mid-call, down the flow.** A call that came up a site's link is
answered, re-invited and hung up down the connection the site opened:
the caller's leg keeps the flow its registration left
(`Call::a_leg.flow_token`), a BYE toward it goes via that flow (it went
to the contact before), and a re-INVITE toward either leg that has a
flow goes via it (`send_reinvite_via_flow`, which the UAC had and the
handler never used). Proved on the loopback by the SBC's own log.

**Proved** by `fcp-link/tests/survivability.rs`: a carrier's call
rings the site, is held and hung up down the flow; the link is cut and
the cloud says so; Alice re-registers, calls Bob with audio, and calls
the outside on the site's carrier; Carol is made in the cloud; the link
returns and the cloud says so; Carol registers at the site; the site's
two records and its call events reach the cloud naming the site; the
site binds again and the carrier's call rings Alice.

**Left for later.** A site's link trunk re-registers on the trunk
registration's own retry (up to 35 s after a refusal) rather than the
moment the link returns. A call in progress across the link when it is
cut is not proved either way.

### 3.9 The site as a product, as built

*Phase 7.* What a customer's operator gets, and what proves it.

**The site stack.** `deploy/docker-compose/site` is a compose project
of its own (`name: site`): the site's Postgres and Redis, the cloud
stack's `make-certs.sh` making the site's CA, the call manager's
certificate and `master.key` into `fcp-certs`, the admin with
`[api] managed_by` from `FCP_CLOUD_ADMIN_URL`, the call manager on the
host's network on shifted ports (SIP 5090–5092, HTTP 18084, RTP
12000–12999) with `[sip] tls_ca_path` on the cloud's CA, `[events]
use_redis` and no CDR stream sink (records stay in the site's database
for the link), and `fcp-link` in its own image (`Dockerfile.link`, the
fifteenth in `images.yml`) with `state_dir` on the `link-data` volume.
The cloud's CA is bind-mounted from `site/cloud-ca.crt`; the cloud's
three addresses and the token are `FCP_LINK_*` and
`FCP_CLOUD_ADMIN_URL` in the site's `.env`. `backup.sh`, `restore.sh`
and `wait-healthy.sh` take `COMPOSE_DIR`, and `link-data` is in the
backup's volume list, so a site is kept as the cloud is.

**The documents.** `docs/SITE_SETUP.md` for the site's operator — what
to get from the cloud, the bring-up and the log lines of an
enrollment, phones, the site's admin and its refusal, life without the
cloud, the volumes and what is re-derivable, backup and restore,
upgrade, decommissioning, faults; the runbook's §9 for the cloud's
operator; the compose README's site section.

**The UI.** A Sites page under Advanced: the tenant's sites with
status, link, sync, registrations, nodes and link trunk; a site made
with its token shown once (as API tokens are); a new token; disable,
enable, delete; a detail dialog with the nodes and who is registered
at the site. Numbers get *Answered at site* and trunks *Runs at*
(`site_id`), shown only to a tenant that has sites. A platform admin
on every tenant sees all sites and makes none.

**Proved** in the nightly's `stack` job: the live tenant gets a site
through the admin API, the site stack comes up from the same images
beside the cloud stack, and `fcp-link/tests/compose_site.rs` (gated on
`FCP_LIVE_SITE`, the site's id) checks the cloud's site record —
enrolled, link up, the site's call manager on it with its release and
a link trunk — a user made in the cloud registering at the site, the
site bound at the cloud's SBC, the site's admin refusing a user with
the cloud's address, and `sync_behind` at zero; then `backup.sh`
takes the site with its `link-data`. On every CI run the binaries'
smoke tests `--check-config` the site's three configurations with the
site's `.env`, and `fcp-link/tests/smoke.rs` starts the link with
nowhere to enroll to.

**Left for later.** A call through the live site in the nightly (the
floor's `calls.rs` proves it on the loopback). A site's own
`fcp-events-server`, voicemail and conference servers are not in the
site compose project; the site runs what the call manager carries. The
link's protocol is versioned but no second version exists to refuse.

## 4. Tenancy and security at the site

A site belongs to one tenant and holds that tenant's slice only. The
site's database is the customer's: the cloud never connects to it, and
the site's master key, certificates and backups are the customer's to
keep (`docs/BACKUP_AND_RESTORE.md` applies per site). The site's
operator token is for the site's own routes — status, drain, logs — and
for reading; a write to a synced object is refused with the cloud's
address. What a site can never do: mint or route numbers, change rate
plans, see another tenant, reach the cloud's stores. What the cloud can
do to a site: enroll, disable, drain, push configuration, ask for its
health and, later, a diagnostic bundle.

## 5. Phases

One PR each, in order; each proved by a test in the two-stack floor
harness (`docs/TEST_FLOOR.md`, the two-node cluster harness with a
"cloud" stack and a "site" stack) and, at the end, by the nightly.

| Phase | What |
|---|---|
| 1 | ✅ v0.25.268 — **Sites and enrollment.** The `sites` table and `/v1/sites`; enrollment tokens; the link CA and certificate issue; `fcp-link` enrolls, holds its certificate, heartbeats; the cloud's cluster page shows sites. Test: a link enrolls once, a spent token is refused, a disabled site's certificate is refused. |
| 2 | ✅ v0.25.269 — **The journal, the snapshot and the feed.** Triggers on every synced table writing the per-tenant journal; `GET /link/v1/snapshot` and `/link/v1/changes`; a test that every put and delete of every synced store journals, and that a snapshot plus the feed reproduce a tenant's slice exactly. As built: `sync_journal` (migration 079) written by one trigger function on nineteen tables, the tenant's own row under its own id, a rate's change journaled as its plan's; `fcp_core::sync::SyncStore`; the snapshot lists every synced store of the tenant and the feed holds up to thirty seconds, fetching each changed object as it is now (a missing one becomes a delete), pages of up to five thousand with `more`. Tests: `fcp-storage-postgres/tests/sync_journal.rs`, `fcp-link/tests/sync.rs`. |
| 3 | ✅ v0.25.270 — **Configuration at the site.** `fcp-link` applies the snapshot and follows the feed into the site's Postgres, sealing secrets under the site's key; the site's admin API refuses writes to synced objects; lag on the heartbeat. Floor: a user made in the cloud registers at the site within seconds; a password changed in the cloud is the one the site challenges with. As built: `[site] database_url` opens the site's Postgres (its schema migrated as every binary does) and the link applies through the same stores the cloud writes with — `put_*` per table, `FlowStore::put_flow` as an exact copy, a rate plan with its rates — keeping its cursor in `sync.json`; the heartbeat carries `sync` (cursor, applied at, the last error) and the cloud shows `sync_cursor`, `sync_applied_at`, `sync_error` and `sync_behind` on every site; the site's admin with `[api] managed_by` refuses a write to a synced resource with 403 naming the cloud (`fcp_api_core::managed`). Test: `fcp-link/tests/apply.rs`. |
| 4 | ✅ v0.25.271 — **Calls across.** The link trunk provisioned at enrollment; the cloud SBC knows the site as the tenant's registering trunk; numbers route to the site; inbound reaches the site's phones and outbound leaves through the cloud; emergency per the site's trunks. Floor, with the SBC and a carrier stub in the harness. As built (§3.6): `[link] sip_host` makes one link trunk per site (`site_id`, `link = cloud`, TLS, credentials minted at enrollment); `Trunk.site_id` says where a trunk runs and the link keeps only its site's, turning the link's end to `site`; the SBC loads link peers, authenticates their REGISTER, keeps each site's binding and RFC 5626 flow (`/admin/sites`); `PhoneNumber.site_id` sends a number's calls down its site's flow; a site's calls are the tenant's own, forwarded to the cloud's call manager as a trusted edge's with `X-FCP-Caller-Site`; legs to and from a site latch their media. Test: `fcp-link/tests/calls.rs`. |
| 5 | ✅ v0.25.272 — **Records up.** CDRs shipped and rated in the cloud, idempotently; events bridged; recording and voicemail rows shipped and files fetched through the link; registration summaries; retention and subject requests reaching the site's rows and files, the report naming the site. As built (§3.7): a `ship_seq` on five tables and a cursor per table in the link; `POST /link/v1/cdrs` rated afresh by the cloud's processor and announced naming the site; `POST /link/v1/records` for recording, mailbox and message rows and the site's logged deletions; `site_requests` collected on the link's long poll for fetches and deletions, answered on its own connection, behind downloads, playback, exports, erasures and the reaper; the site's events forwarded from its stream with `origin_site`; registrations on the heartbeat. Test: `fcp-link/tests/records.rs`. |
| 6 | ✅ v0.25.273 — **Survivability, proved.** The floor cuts the link (a proxy cut, as `fcp-cluster/tests/partition.rs` does): phones keep registering and calling, a local trunk keeps carrying, CDRs and events spool; reconnect catches up; the cloud showed the site unreachable meanwhile and shows it back. As built (§3.8): `fcp_testkit::cut::CutLink` in front of the link listener and the SBC; the site's own carrier by `site_id`; mid-call requests toward a site down its flow (`Call::a_leg.flow_token`, `send_reinvite_via_flow`). Test: `fcp-link/tests/survivability.rs`. |
| 7 | ✅ v0.25.274 — **The site as a product.** `docs/SITE_SETUP.md` for the customer's operator (enroll, run, upgrade, back up); the runbook's site section; the admin UI's Sites page; the nightly gains a site stack against its cloud stack. As built (§3.9): the `site` compose project with `Dockerfile.link`, the backup scripts taking `COMPOSE_DIR`, the Sites page and the `site_id` fields on numbers and trunks, `fcp-link/tests/compose_site.rs` in the nightly's `stack` job. **The project is complete.** |

## 6. Questions

Nine, settled by the user on 2026-09-25, each on its recommendation,
which is what the phases above assume.

**Q1. Does a site run every binary, or a subset?** — *Every binary.* One
image set, one procedure, and the site does alone everything it did
before it was a site; cloud features are additive, never substitutes
that vanish with the link.

**Q2. The link's transport.** — *HTTPS and WebSocket carrying JSON*, as
every other service boundary in FCP; gRPC was descoped on 2026-09-02.
Mutual TLS from the link CA, client certificate per site.

**Q3. Do phone credentials reach the site?** — *Yes.* A site that cannot
challenge its phones without the cloud is not survivable. They cross
once over mutual TLS and are sealed under the site's own key; the site's
database is the customer's own.

**Q4. The journal: triggers, or the storage layer?** — *Triggers*, as
the deletion log is: a change made by a migration, a repair by hand or a
code path nobody remembered still reaches the site. The storage layer
stays as it is.

**Q5. Who connects to whom?** — *The site to the cloud, always.* No
inbound port at the customer's premises, nothing for their firewall to
open, no address of theirs the cloud must know.

**Q6. Voicemail and conferencing: site or cloud?** — *Site.* They
survive the link; a tenant who wants them in the cloud instead is a
later option, not the default.

**Q7. Recordings: stay, or upload?** — *Stay at the site*, rows shipped,
files fetched through the link on demand. An upload policy into the
cloud's archive is a later phase; the compliance report already knows
how to name a store it does not hold.

**Q8. Is a site a region of the cloud's cluster?** — *No.* A site has
its own Redis and its own membership; the cloud sees it through the
link's heartbeat, not through cluster discovery. `docs/MULTI_REGION_DEPLOYMENT.md`
§1 is why: a call manager has one Redis.

**Q9. Entitlements and licensing?** — *Not here.* The cloud enforces a
tenant's site count at enrollment and nothing else; metering is the
CDRs the site ships.

## 7. Testing

The two-stack floor: the existing two-node cluster harness started as a
cloud (its own Postgres and Redis) and a site (its own), joined by
`fcp-link` and the link trunk, with the cut-able proxy from the
partition test between them. Phases 1 to 6 each add a scenario; phase 7
added a site stack to the nightly's compose run (§3.9). Unit tests cover the
journal triggers per table and the link's apply order and idempotence.

## 8. Not in scope

A site serving more than one tenant. Calls between two sites of one
tenant without the cloud in the path (they go through the cloud SBC, or
over a local trunk the customer configures). Premises-only deployments,
which change nothing. High availability of the cloud itself, which is
`docs/MULTI_REGION.md`. Self-service signup, entitlements and invoicing,
which are the CPaaS product's own project.
