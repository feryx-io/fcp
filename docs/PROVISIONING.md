# Provisioning phones

How a tenant's phones fetch their configuration from FCP, how FCP keeps
them in step, and how a phone out of its box finds FCP. This is phases 7
and 8 of `docs/IDENTITY_AND_DEVICES.md`: 7a the URL and how it is guarded;
7b the templates, picked by model, pending phones and import; 7c the
registration on the device, resync, and the old route removed; 8a
zero-touch through Yealink RPS and Poly ZTP, and DHCP; 9a firmware.

## 1. The tenant's provisioning URL

A phone can be told a URL and little else: it cannot send a bearer
token. So each tenant has a **provisioning key**, a secret in the path:

```
https://<admin host>/provision/<key>/
```

- A tenant administrator makes it, sees it again and replaces it at
  `/api/v1/provisioning/key` (`GET` to read, `POST` to make a new one;
  `device_update`), or on the Devices page. A new key stops the old URL
  working at once.
- The key is kept sealed at rest and found by its hash.
- The URL is given to phones the usual ways: DHCP option 66, the phone's
  web page, or a vendor's redirection service (phase 8).
- **HTTPS only** (`[api] provisioning_require_https`, the default): a
  request that did not arrive over HTTPS (`X-Forwarded-Proto` from the
  proxy) is answered 404.
- A site's phones fetch from the site's own admin, which makes its own
  key.

## 2. File names

Under the URL, each vendor's phones ask for their file by their own
name for it. FCP reads the MAC from the name, finds the device among the
tenant's, and renders its configuration from its template (§5).

| Vendor | Names | Served as |
|---|---|---|
| Yealink, Fanvil | `<mac>.cfg` | `text/plain` |
| Polycom (Poly) | `<mac>.cfg`, then `<mac>-fcp.cfg` | a master file naming the second, then the configuration as `application/xml` |
| Grandstream | `cfg<mac>.xml`, `cfg<mac>` | `application/xml` / `text/plain` |
| Cisco | `SEP<MAC>.cnf.xml`, `spa<mac>.xml` | `application/xml` |
| Snom | `snom<model>-<MAC>.htm`, `<mac>.xml` | `text/html` / `application/xml` |
| Any | `<mac>` | `text/plain` |

- The MAC may be in either case.
- A Polycom is known by its User-Agent: its `<mac>.cfg` is a master file
  (`CONFIG_FILES="<mac>-fcp.cfg"`).
- Files every phone of a vendor asks for and FCP has nothing in
  (`y0000000000xx.cfg`, `000000000000.cfg`) are answered 404, which the
  phones take in their stride.
- Anything that names no device, a MAC the tenant has no device for, a
  disabled device or one without a template, answers 404, like the wrong
  key: nothing tells a stranger what exists. A MAC no device has is kept
  as a pending phone (§6).
- Responses are `no-store`.

## 3. Guarding it further

A tenant's `provisioning` settings (`PUT /api/v1/tenants/{id}`, or the
Tenants page):

| Key | Default | Meaning |
|---|---|---|
| `require_device_credentials` | `false` | Each phone must also send HTTP Basic credentials whose password is its device's provisioning token (any username). Without them the answer is `401` with `WWW-Authenticate: Basic realm="FCP provisioning"`, which phones answer with the credentials they were given. |
| `networks` | none | CIDRs (or addresses) phones may fetch from; empty is anywhere. The address is the proxy's `X-Forwarded-For`. |

A phone that sends Basic credentials has them checked even when the
tenant does not require them.

The device's provisioning token is made with the device (when it has a
MAC) and shown once: 32 characters, the most a vendor's redirection
service hands a phone (§10). `PATCH /devices/{id}` with
`provisioning_token` replaces it.

## 4. Every fetch is recorded

Each fetch is kept on the device it named, the newest 500 per tenant:
the file, when, the address, the User-Agent and the outcome (`served`,
`unauthorized`, `network_refused`, `not_served`: disabled, no template,
or the template failed, which is also logged with the error). `GET
/api/v1/devices/{id}/fetches`, or *Provisioning fetches* on a device's
menu, shows them; a phone that cannot get its file can be told why.

## 5. Templates

A device's configuration is its template rendered. Templates are a
tenant's own (`/api/v1/config-templates`, the **Phone templates** page),
started from a vendor's starter or written from scratch.

### Which template a phone gets

The one its device names (`config_template_id`). A device that names none
gets one picked by its **vendor** and **model**:

1. one of its vendor's templates whose `model_pattern` matches its model,
   the pattern that fixes the most characters first (`T54W` before `T5*`
   before `T*`);
2. else one of its vendor's with no pattern;
3. else a `generic` template with no pattern;
4. else none, and the phone is answered 404 (`not_served`).

A tie goes to the name that sorts first. A pattern is a glob: `*` is any
run of characters, `?` any one; case and the separators ` `, `-`, `_` do
not count, so `VVX_450`, `VVX 450` and `vvx450` are one model.

The device's `vendor` (`yealink`, `polycom`, `grandstream`, `cisco`,
`fanvil`, `snom`) and `model` are what an administrator set. Where they
are not set, FCP reads them from the request: the file name (only a Cisco
asks for `SEP<MAC>.cnf.xml`, only a Grandstream for `cfg<mac>`), the
User-Agent (`Yealink SIP-T54W 96.86.0.70 …`), and, for the vendor, the
MAC's first three bytes. A device made with a MAC whose prefix is a
vendor's gets that vendor.

### The language

Templates are Jinja ([minijinja](https://docs.rs/minijinja)): `{{ name }}`,
`{% if %}`, `{% for %}`, filters (`|upper`, `|default`, `|length`). An
older `${name}` placeholder still renders, as `{{ name }}`.

- A value that is not there renders as nothing.
- For an XML vendor (Polycom, Grandstream, Cisco, Snom) every value is
  escaped for XML as it is written (`&` as `&amp;`); `|safe` writes one
  as it is.
- `trim_blocks` and `lstrip_blocks` are on, so a `{% … %}` line leaves no
  blank line behind, and a trailing newline is kept.
- A template that does not parse is refused when it is saved, naming the
  line. One that fails when it renders (an unknown filter, a loop that
  runs past its budget) is answered `not_served` and logged.

### Variables

Every one is always there, empty when it does not apply.

| Variable | What |
|---|---|
| `mac_address`, `device_name`, `vendor`, `model`, `firmware_version` | The device (the request's vendor, model and firmware where the device has none). |
| `sip_domain` | The domain the phone registers in: the tenant's first SIP domain (`sip_domains`), else `sip_server`. It is how the registrar knows the phone's tenant, so a phone must register in it. |
| `sip_server`, `sip_port`, `sip_transport` | Where the phone sends, and how: `[api.provisioning]` (below). The starters make it the outbound proxy when it is not `sip_domain`. `sip_transport` is `udp`, `tcp` or `tls`. |
| `sip_uri`, `sip_username`, `sip_auth_username`, `sip_password`, `sip_auth_ha1_hash` | The first account: the device's own credentials, else its owner's. `sip_provisioning_password` is `sip_password`, for older templates. |
| `owner_username`, `owner_display_name`, `owner_email`, `owner_user_id` | The owner. |
| `effective_user_username`, `effective_user_display_name`, `effective_user_extension`, `effective_user_id`, `hoteling_guest` | Who the phone is for now: a hoteling guest, else the owner. |
| `shared_line_enabled` (1/0), `shared_line_yes_no`, `shared_line_number`, `shared_line_appearances` | The shared line the device is on (`docs/ENTERPRISE_CALLING.md` §3.9). |
| `accounts` | The phone's accounts in order, each with `index`, `label`, `display_name`, `user_name`, `auth_name`, `password`, `shared`, `appearances`: its own, then a shared line's. |
| `line_keys` | The device's keys, each with `index` (from 1), `kind` (`blf`, `speed_dial`, `park`), `value`, `label`. |
| `firmware_url`, `firmware_directory`, `firmware_target_version` | The firmware image the phone is pointed at (§11): its URL under the tenant's, the URL of its directory (for phones that ask for their own file name there), and its version. Empty without a target. |
| `timezone`, `utc_offset`, `utc_offset_hhmm`, `utc_offset_minutes` | The tenant's `calling.timezone` (UTC when unset) and its offset now: `-5`, `+5:30`; `-05:00`; `-300`. |
| `ntp_server` | `[api.provisioning] ntp_server`, `pool.ntp.org` by default. |
| `provisioning_url` | The tenant's URL (`https://…/provision/<key>/`), so a template can tell the phone to keep fetching from it. |

`POST /api/v1/config-templates/preview` renders a body against sample
values and returns them with it; `POST /api/v1/devices/{id}/render`
renders what a device's phone would get now (or a template or body given
in place of its own). The preview of a device holds its credentials, so it
needs `device_update`.

### Line keys

A device's `line_keys` (`PATCH /devices/{id}`, or the device's edit
dialog) are its programmed keys, in order:

```json
"line_keys": [
  {"kind": "blf", "value": "202", "label": "Bob"},
  {"kind": "speed_dial", "value": "+15551230000", "label": "Lobby"},
  {"kind": "park", "value": "701"}
]
```

At most 100; a value of 1 to 128 characters, a label of up to 64.

### Starters

`GET /api/v1/config-templates/library` lists them and
`POST /api/v1/config-templates/library/{vendor}` copies one into the
tenant's templates:

| Starter | Pattern | Writes |
|---|---|---|
| `yealink-starter` | `T*` | accounts in the tenant's domain through the outbound proxy, transport and SRTP on TLS, the shared line, time zone and NTP, `static.auto_provision.server.url`, line keys after the accounts' |
| `polycom-starter` | `VVX*` | lines as `user@domain` with transport, the shared line, SNTP, BLF as `attendant.resourceList` |
| `grandstream-starter` | `G*` | accounts 1 and 2 with the domain and outbound proxy, transport, NTP (line-key P-values differ by model and are left out) |
| `cisco-starter` | `SPA*` | lines with the domain and outbound proxy, transport, the shared line, NTP and time zone, `Profile_Rule`, extended functions |
| `fanvil-starter` | any | lines with the domain and DSS keys (check them against the model's guide) |
| `snom-starter` | `D*` | identities in the domain through the outbound proxy, the shared line, NTP, `setting_server`, function keys |

They are starters: copy one, then edit it for the deployment.

### `[api.provisioning]`

On the admin:

| Key | Default | Meaning |
|---|---|---|
| `sip_server` | the SIP realm | The address phones send to. The realm (`[sip] realm`, `docs/SINGLE-NODE-SETUP.md`) is seldom an address phones reach: set this. |
| `sip_port` | 5061 for TLS, else 5060 | |
| `sip_transport` | `udp` | `udp`, `tcp` or `tls`. |
| `ntp_server` | `pool.ntp.org` | |
| `public_url` | `[api.sso] public_url`, then the request's host | Where phones reach the admin, for `provisioning_url`. |

## 6. Pending phones

A phone out of its box, given the tenant's URL (by DHCP, its web page or
a redirection service), asks for its file before anyone has made a device
for it. Its fetch is answered 404 like any unknown MAC, and the phone is
kept as **pending**: its MAC, when it was first and last seen, its
address and User-Agent, the vendor, model and firmware read from them, and
how many times it has asked. A phone fetching from outside the tenant's
`networks` is not kept, and a MAC another tenant's device has is not
either. The newest 1000 per tenant are kept.

- `GET /api/v1/provisioning/pending` (`device_read`), or *Phones waiting
  to be claimed* on the Devices page, lists them, the most recently seen
  first.
- `POST /api/v1/provisioning/pending/{mac}/claim` (`device_create`) makes
  one a device in one step: its vendor, model and firmware as it gave
  them, a name (unset: vendor, model and MAC), an owner (`owner_user_id`
  or `owner_username`), a template (unset: picked by vendor and model),
  a location. The answer is the device with its provisioning token, shown
  once. On its next fetch the phone gets its configuration.
- `DELETE /api/v1/provisioning/pending/{mac}` (`device_delete`) forgets
  one; it comes back if it asks again.
- A device made or changed to have a pending phone's MAC takes it off the
  list.

A tenant that requires device credentials (§3) answers a claimed phone
401 until it is given its token, which a redirection service carries
(phase 8).

## 7. Importing phones

`POST /api/v1/devices/import` (`device_create`) makes up to 1000 devices
by MAC:

```json
{"devices": [
  {"mac_address": "80:5E:C0:D1:E2:F3", "owner_username": "dave", "model": "T54W"},
  {"mac_address": "0004f2aaaa02", "name": "Lab", "vendor": "polycom", "config_template_id": "…"}
]}
```

Each row has `mac_address` and optionally `name`, `model`, `vendor`,
`device_type` (`desk_phone` unless given), `owner_user_id` or
`owner_username`, `config_template_id` and `location_id`. Every row is
checked before any device is made: a MAC that is malformed, in the file
twice or already a device's, an owner, template or location the tenant
does not have. One fault and nothing is made; the answer is a 400 naming
each faulty row. Otherwise every device is made, each with a provisioning
token shown once in the answer (`created`), and each MAC leaves the
pending list.

The Devices page's *Bulk Import* reads a CSV in the browser and makes the
devices one at a time, updating those that exist when asked; the import
route is for scripts that want all or none.

## 8. Keeping phones in step

### The registration on the device

When a phone registers, the call manager (or registrar) that takes the
REGISTER writes it on its device: `registration_state`, `last_registration`,
`last_ip_address` (the contact's host and port) and `user_agent`, and from
the User-Agent the `firmware_version` it runs, and the `vendor` and `model`
where the device has none. A de-REGISTER writes `unregistered`.

The device is the one whose own credentials the phone used; else the one
whose MAC its REGISTER's User-Agent carries; else, among the devices of
the user it registered as, the one whose last registration or last served
provisioning fetch came from the REGISTER's source address (a desk phone
registering with its owner's credentials: most phones' REGISTER names no
MAC, while their fetch does, and the fetch log keeps its address); else
the one whose SIP URI is the name it registered as; else that user's only
device. A phone matching none is written on no device. Behind one NAT,
two phones of the same user share an address and neither is matched by
it.

An unchanged registration is written again at most every ten minutes. The
device views check what is written against the live registrations: a
device written as registered that has no live registration carrying its MAC
or from its last address is shown `expired` (a phone that went away without
unregistering).

### Resync

- `POST /api/v1/devices/{id}/resync` (`device_update`, or *Tell it to
  fetch again* on a device's menu) tells the phone to fetch its
  configuration again. `sent` is how many of its registered contacts were
  told: 0 when it is not registered.
- `POST /api/v1/config-templates/{id}/resync` (`device_update`, or the
  button on a template) does it for every enabled device the template is
  rendered for, named or picked: `devices` and `sent`.

The admin has no SIP of its own: it asks a call manager (`[call_manager]
url`, else each healthy one in the cluster in turn) at the node's
`POST /devices/{id}/resync`, which sends each contact an unsolicited
`NOTIFY` with the vendor's event:

| Vendor | `Event` |
|---|---|
| Yealink, Polycom, Snom | `check-sync;reboot=false` |
| Cisco | `resync` |
| Grandstream, Fanvil, unknown | `check-sync` |

The contacts are those registered under the device's SIP URI or own
credentials; else, among its owner's, the one whose User-Agent carries its
MAC or registered from its last address (or the owner's only one, when the
device is the owner's only phone). A phone must accept the NOTIFY (most
take it only from their registrar's address). A phone over TCP or TLS is
reached through whichever call manager answers, which opens its own
connection where it holds none.

## 9. The old route

`GET /api/v1/provisioning/{mac}`, with the deployment's
`x-fcp-provisioning-token` and the device's
`x-fcp-device-provisioning-token`, is removed (v0.25.323): phones fetch
from the tenant's URL. `[api] provisioning_auth_token` is read by nothing;
it is still accepted, with a warning at startup, and a later release
refuses it. The `--provisioning-auth-token` flag and
`FCP_ADMIN_PROVISIONING_AUTH_TOKEN` are gone.

## 10. Zero-touch

A phone out of its box knows nobody's URL. It learns it one of three ways:
from its vendor's redirection service, from DHCP, or from whoever types it
into its web page. FCP does the first itself and documents the second.

### Redirection services

A phone asks its vendor's service, by its MAC, where its provisioning
server is. With a tenant's account with the service, FCP registers each of
the tenant's phones of that vendor:

| Service | FCP registers | Per-device credentials |
|---|---|---|
| Yealink RPS (`yealink_rps`) | the MAC, with the tenant's URL as its own server (`uniqueServerUrl`) and the device's name as the remark | yes: the device's provisioning token as the password, the MAC as the username |
| Poly ZTP (`poly_ztp`) | the MAC, associated with a profile under the tenant's Reference ID | no |

Grandstream GDMS and Fanvil FDPS are not connected: neither publishes the
API a connector needs (GDMS's guide stops short of redirecting a device,
and FDPS has none). Point their consoles at the tenant's URL by hand, or
use DHCP (below).

**When.** A device with a MAC whose vendor (`vendor`, else the MAC's
prefix) has an account that is on is registered when it is made, claimed
or imported, and on any change after that it has not been. A new MAC
takes the old one off and puts the new one on; a device disabled or
deleted is taken off. It all happens after the request answers, so a
vendor's outage does not hold up an administrator; the outcome is on the
device:

- `GET /api/v1/devices/{id}/redirection` (`device_read`): the service, the
  MAC registered, `registered` or `failed` with the service's reason,
  whether the phone was given credentials.
- `POST /api/v1/devices/{id}/redirection` (`device_update`, or *Register
  with its vendor* on a device's menu): register it now, or again.

**Credentials.** Where the service carries them (RPS), the phone is given
its provisioning token: the one made with the device, or, when the tenant
requires device credentials (§3) and FCP no longer holds it, a new one,
which replaces the old. RPS keeps at most 32 characters, which is why
tokens are 32. With ZTP the phone has no credentials of its own: a tenant
that requires them cannot use ZTP's phones without giving each its token
by hand.

### The accounts

On the Devices page (*Zero-touch*), or:

| Route | Permission | |
|---|---|---|
| `GET /api/v1/provisioning/redirection` | `device_read` | each service and the tenant's account with it (never the secret) |
| `PUT /api/v1/provisioning/redirection/{service}` | `device_update` | `key_id`, `secret` (required to make it, kept when left out), `enabled`; for Poly `account_id` and `profile` |
| `DELETE /api/v1/provisioning/redirection/{service}` | `device_update` | forget it (phones stay registered at the vendor) |
| `POST /api/v1/provisioning/redirection/{service}/check` | `device_update` | whether the credentials work: `ok` and the service's reason |
| `POST /api/v1/provisioning/redirection/{service}/sync` | `device_update` | register every phone of the vendor not yet registered: an account made after its phones |

The secret is sealed at rest, like every other secret (`docs/ENCRYPTION.md`).

**Yealink RPS.** In the RPS management platform, *System Management › API
Service › Acquire* gives an AccessKey ID (`key_id`) and Secret (`secret`).
FCP signs each request as the RPS JSON API guide describes (HMAC-SHA256
over the method, `Content-MD5`, `X-Ca-Key`, `X-Ca-Nonce`,
`X-Ca-Timestamp`, the path and the query). Phones need not be on an RPS
server: each gets the tenant's URL as its own.

**Poly ZTP.** Poly gives the API account (`key_id`, `secret`) on request
(ztpinfo@polycom.com). The ZTP API cannot make profiles: make one in the
ZTP console whose provisioning server is the tenant's URL, and a Reference
ID (`account_id`), and give FCP both. Each phone is associated with that
profile under that Reference ID; one already associated elsewhere in the
account is moved.

**Where FCP reaches them** is the operator's, not a tenant's
(`[api.redirection]` on the admin): `yealink_rps_url` (default
`https://api-dm.yealink.com:8443`) and `poly_ztp_url` (default
`https://ztpconsole.polycom.com/inboundservlet/GenericServlet`). The URL a
service is told is `[api.provisioning] public_url` (else `[api.sso]
public_url`, else the address the administrator's request came to) with
the tenant's key: set `public_url` so it is the address phones reach.

Both connectors are tested against stand-ins written from the vendors'
guides (`fcp_testkit::redirection`); they have not been run against the
real services, which need vendor accounts. Whoever has one should check
them by hand (the *Check* button is the first step).

### DHCP

Where a network's DHCP server can be set, it can hand phones the URL:

| Option | Read by |
|---|---|
| 66 (TFTP server name), a full `https://…/provision/<key>/` URL | Yealink, Polycom, Grandstream, Cisco, Fanvil, Snom |
| 160 (provisioning server), the same URL | Polycom (before 66), Yealink |
| 43 (vendor-specific), sub-option as each vendor's guide says | Cisco and others; not set up by FCP |

A URL in DHCP is seen by everything on the network, and the key is in it:
use it on networks the tenant trusts, and list them in its `networks`
(§3).

## 11. Firmware

The platform administrator uploads vendors' images; every tenant sees
them and chooses its own targets (Q10). One copy serves everyone.

### Images

- `POST /api/v1/firmware` (multipart: `vendor`, `version`, `models`,
  `notes`, `file`), the platform administrator's only (`system_config`
  and a platform session or the static token). `version` is what the
  phones report in their User-Agent (`96.86.0.80`); `models` are patterns,
  comma-separated, empty for any of the vendor's; the file keeps the name
  the vendor ships it under, which is the name it is served by. Its size
  and SHA-256 are kept.
- `GET /api/v1/firmware` (`device_read`) lists them.
- `DELETE /api/v1/firmware/{id}` (platform administrator) removes one no
  tenant targets (409 while one does), with its file.
- They are kept in `[api.firmware] directory` on the admin
  (`/app/data/firmware` in the compose stack, in `admin-data`), or in an
  S3-compatible bucket (below), at most `max_image_mb` (1024) each.
  Neither set, nothing can be uploaded. A proxy in front must let the
  upload through (the compose stack's allows 1.1 GB).

### Images in a bucket

With several admins, or to keep large files off the admin's disk, the
images go in a bucket of any store that speaks S3 with SigV4 (AWS, MinIO,
Garage, Ceph, R2):

```toml
[api.firmware]
max_image_mb = 1024
# Where an upload is staged before it goes up; the system's temporary
# directory when unset.
directory = "/var/lib/fcp/firmware-staging"

[api.firmware.s3]
bucket = "fcp-firmware"
region = "eu-west-1"
# endpoint = "https://s3.example.net"   # unset: AWS's regional endpoint
# path_style = true                      # most self-hosted stores
# access_key_id / secret_access_key / session_token: unset, AWS_* from
# the environment
prefix = "firmware/"                     # keys are <prefix><image>/<file>
```

An upload arrives on local disk, goes up with its SHA-256 signed (the
store refuses a body that is not the one sent) and is confirmed by size
and hash before the image is listed. A phone's fetch is streamed from the
bucket through the admin, so the phone needs no access to it; a removal
deletes the object. The credentials are masked wherever the configuration
is printed. The S3 client is the CDR archive's (`docs/CDR_PIPELINE.md`
§13).

### Targets

A target says: these phones of the tenant run this image.

| Route | Permission |
|---|---|
| `GET /api/v1/firmware/targets` | `device_read`: each target with the phones it is for and how many run its version (`current`), another (`behind`) or have not said (`unknown`) |
| `POST /api/v1/firmware/targets` | `device_update`: `vendor`, `model_pattern` (empty for every model), `image_id` (the vendor's), `stage`, `pilot_devices` |
| `PUT /api/v1/firmware/targets/{id}` | `device_update`: another image, the pilot set, `stage: all` to roll out |
| `DELETE /api/v1/firmware/targets/{id}` | `device_update`: phones keep what they run; their files stop naming an image |

A phone's target is its vendor's whose pattern names its model most
closely, one with no pattern last. While a target's `stage` is `pilot`
(the default), only its `pilot_devices` are pointed at the image; the
rest of its phones are pointed at none until it is `all`.

### What the phone is told

A phone with a target gets `firmware_url` in its template (§5); the
starters write it as each vendor reads it:

| Starter | |
|---|---|
| Yealink | `static.firmware.url` |
| Polycom | `upgrade.custom.server.url`, and the master file's `APP_FILE_PATH` |
| Grandstream | `P192` (the image's directory, which the phone asks for its own file name in) with `P212 = 2` (HTTPS) |
| Cisco | `<Upgrade_Rule>` |
| Fanvil, Snom | not written: add it per the model's guide |

The image is served at `/provision/<key>/firmware/<image>/<file>`, under
the same guards as the phones' files (HTTPS, the tenant's networks), to
the tenant whose key it is while one of its targets names the image.
Each fetch is in the fetch log as `firmware/<file>`.

### Who is behind

What each phone runs comes from its User-Agent, written on its device
when it registers (§8). The device views show `firmware_target_version`
and `firmware_status` (`current`, `behind`, `unknown`); the Devices page
shows them, and the Firmware page each target's counts. When a phone
registers running another version than its target installs, the node
that took the REGISTER says `device.firmware_behind` (the tenant, the
device, what it runs, what its target installs), once each time what it
runs changes; a tenant's webhooks can take it.

### At a site

A site's phones fetch firmware from the site's admin, as they fetch
everything else (Q11):

- The tenant's targets reach the site with the rest of its configuration,
  each carrying its image's record. A site's admin refuses firmware
  writes: targets and images are the cloud's.
- The site's link fetches each targeted image over the link
  (`GET /link/v1/firmware/<image>`, under the site's mutual TLS, only an
  image its tenant targets) into `[site] firmware_dir`, checks it against
  the image's SHA-256 and moves it into place; it looks again whenever a
  target arrives and every five minutes, so a cut link catches up when it
  comes back.
- The site's admin serves them from that directory, its
  `[api.firmware] directory`, under the site's own provisioning URL.
- A target gone at the cloud takes the image's record and file off the
  site once no other target names it.

In the site compose project the directory is the `site-firmware` volume,
written by the link and read by the admin.
