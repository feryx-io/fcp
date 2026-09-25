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
| 1 | **Sites and enrollment.** The `sites` table and `/v1/sites`; enrollment tokens; the link CA and certificate issue; `fcp-link` enrolls, holds its certificate, heartbeats; the cloud's cluster page shows sites. Test: a link enrolls once, a spent token is refused, a disabled site's certificate is refused. |
| 2 | **The journal, the snapshot and the feed.** Triggers on every synced table writing the per-tenant journal; `GET /link/v1/snapshot` and `/link/v1/changes`; a test that every put and delete of every synced store journals, and that a snapshot plus the feed reproduce a tenant's slice exactly. |
| 3 | **Configuration at the site.** `fcp-link` applies the snapshot and follows the feed into the site's Postgres, sealing secrets under the site's key; the site's admin API refuses writes to synced objects; lag on the heartbeat. Floor: a user made in the cloud registers at the site within seconds; a password changed in the cloud is the one the site challenges with. |
| 4 | **Calls across.** The link trunk provisioned at enrollment; the cloud SBC knows the site as the tenant's registering trunk; numbers route to the site; inbound reaches the site's phones and outbound leaves through the cloud; emergency per the site's trunks. Floor, with the SBC and a carrier stub in the harness. |
| 5 | **Records up.** CDRs shipped and rated in the cloud, idempotently; events bridged; recording and voicemail rows shipped and files fetched through the link; registration summaries; retention and subject requests reaching the site's rows and files, the report naming the site. |
| 6 | **Survivability, proved.** The floor cuts the link (a proxy cut, as `fcp-cluster/tests/partition.rs` does): phones keep registering and calling, a local trunk keeps carrying, CDRs and events spool; reconnect catches up; the cloud showed the site unreachable meanwhile and shows it back. |
| 7 | **The site as a product.** `docs/SITE_SETUP.md` for the customer's operator (enroll, run, upgrade, back up); the runbook's site section; the admin UI's Sites page; the nightly gains a site stack against its cloud stack. |

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
adds a site stack to the nightly's compose run. Unit tests cover the
journal triggers per table and the link's apply order and idempotence.

## 8. Not in scope

A site serving more than one tenant. Calls between two sites of one
tenant without the cloud in the path (they go through the cloud SBC, or
over a local trunk the customer configures). Premises-only deployments,
which change nothing. High availability of the cloud itself, which is
`docs/MULTI_REGION.md`. Self-service signup, entitlements and invoicing,
which are the CPaaS product's own project.
