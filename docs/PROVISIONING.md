# Provisioning phones

How a tenant's phones fetch their configuration from FCP. This is phase 7
of `docs/IDENTITY_AND_DEVICES.md`: 7a is the URL and how it is guarded;
7b the templates, picked by model, pending phones and import; 7c resync,
the registration on the device and the end-to-end proof.

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
MAC) and shown once; `PATCH /devices/{id}` with `provisioning_token`
rotates it.

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
| `sip_server`, `sip_port`, `sip_transport` | Where and how the phone registers: `[api.provisioning]` (below). `sip_transport` is `udp`, `tcp` or `tls`. |
| `sip_uri`, `sip_username`, `sip_auth_username`, `sip_password`, `sip_auth_ha1_hash` | The first account: the device's own credentials, else its owner's. `sip_provisioning_password` is `sip_password`, for older templates. |
| `owner_username`, `owner_display_name`, `owner_email`, `owner_user_id` | The owner. |
| `effective_user_username`, `effective_user_display_name`, `effective_user_extension`, `effective_user_id`, `hoteling_guest` | Who the phone is for now: a hoteling guest, else the owner. |
| `shared_line_enabled` (1/0), `shared_line_yes_no`, `shared_line_number`, `shared_line_appearances` | The shared line the device is on (`docs/ENTERPRISE_CALLING.md` §3.9). |
| `accounts` | The phone's accounts in order, each with `index`, `label`, `display_name`, `user_name`, `auth_name`, `password`, `shared`, `appearances`: its own, then a shared line's. |
| `line_keys` | The device's keys, each with `index` (from 1), `kind` (`blf`, `speed_dial`, `park`), `value`, `label`. |
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
| `yealink-starter` | `T*` | accounts with transport and SRTP on TLS, the shared line, time zone and NTP, `static.auto_provision.server.url`, line keys after the accounts' |
| `polycom-starter` | `VVX*` | lines with transport, the shared line, SNTP, BLF as `attendant.resourceList` |
| `grandstream-starter` | `G*` | accounts 1 and 2, transport, NTP (line-key P-values differ by model and are left out) |
| `cisco-starter` | `SPA*` | lines with transport, the shared line, NTP and time zone, `Profile_Rule`, extended functions |
| `fanvil-starter` | any | lines and DSS keys (check them against the model's guide) |
| `snom-starter` | `D*` | identities with transport, the shared line, NTP, `setting_server`, function keys |

They are starters: copy one, then edit it for the deployment.

### `[api.provisioning]`

On the admin:

| Key | Default | Meaning |
|---|---|---|
| `sip_server` | the SIP realm | The address phones register to. The admin's realm is `fcp.local`, which no phone can reach: set this. |
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

## 8. The old route

`GET /api/v1/provisioning/{mac}` with the deployment's
`x-fcp-provisioning-token` (and the device's
`x-fcp-device-provisioning-token`) still works, rendered as in §5, and is
removed in 7c, the next release.
