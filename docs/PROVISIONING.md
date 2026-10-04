# Provisioning phones

How a tenant's phones fetch their configuration from FCP. This is phase 7
of `docs/IDENTITY_AND_DEVICES.md`; 7a is the URL and how it is guarded,
and 7b adds templates picked by model, resync, pending devices and the
end-to-end proof.

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
tenant's, and renders its configuration from its template (as before).

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
  key: nothing tells a stranger what exists.
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
`unauthorized`, `network_refused`, `not_served`). `GET
/api/v1/devices/{id}/fetches`, or *Provisioning fetches* on a device's
menu, shows them; a phone that cannot get its file can be told why.

## 5. The old route

`GET /api/v1/provisioning/{mac}` with the deployment's
`x-fcp-provisioning-token` (and the device's
`x-fcp-device-provisioning-token`) still works for this release, and is
removed after it.
