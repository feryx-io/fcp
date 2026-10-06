# Branding

A tenant's brand is what its softphone takes on once one of its people signs
in: the app's name, its colors in a light and a dark theme, a logo, an icon,
ringtones, the tenant's support, privacy and terms links, and hints for the
dialer. One generic Anvil build becomes Acme's app on Acme's sign-in, with no
build of its own (`docs/SOFTPHONE.md` §2.5, phase F3).

The contract is Anvil's `docs/BRANDING.md` v1, with two changes agreed in the
softphone's design (Q5): the app always sends its bearer token (the `Basic`
mode is gone), and FCP hosts the assets, by hash.

## 1. Making the brand

A tenant administrator makes it, in the admin UI's **Branding** page or
through the API, under `branding_manage` (held by `super_admin` and
`tenant_admin`).

**Assets first.** Each file is uploaded on its own, as the request body:

```
POST /api/v1/branding/assets?name=logo.svg
Content-Type: application/octet-stream

<the file>
```

| Kind  | Types taken                 |
|-------|-----------------------------|
| image | PNG, JPEG, WebP, SVG        |
| audio | Ogg, MP3, WAV               |

What a file is comes from its bytes, not from what the upload says; anything
else is refused (400). At most 5 MB. The answer names the file by its SHA-256,
which is what the brand refers to it by; a PNG's width and height are read
from its header. The same file uploaded again is the same asset.

`GET /api/v1/branding/assets` lists them; `DELETE
/api/v1/branding/assets/{sha256}` removes one the brand does not name (one it
names answers 409).

**Then the brand:**

```
PUT /api/v1/branding
{
  "app_name": "Acme Voice",
  "colors": {"primary": "#1A73E8", "on_primary": "#FFFFFF"},
  "dark_colors": {"primary": "#8AB4F8"},
  "logo": "<sha256>",
  "logo_dark": null,
  "icon": "<sha256>",
  "ringtones": [{"name": "Acme", "default": true, "sha256": "<sha256>"}],
  "links": {"support": "https://acme.example/support"},
  "dial_plan": {"prefix_strip": ["+1"],
                "short_codes": {"1000": "sip:frontdesk@acme.example"}}
}
```

What is checked:

- `app_name`: 1 to 64 characters.
- Colors: `#RRGGBB` or `#RRGGBBAA`, in the slots `primary`, `accent`,
  `background`, `surface`, `on_primary`, `on_surface`, `error`. A slot left out
  is the app's own. `dark_colors` left out, the app derives its dark theme.
- Links: `https://` URLs.
- Assets: each one of the tenant's own uploads, of the kind its slot takes.
  The logos are any image. The icon is a PNG, JPEG or WebP image (not SVG: the
  contract gives it a size). Ringtones are audio: at most eight, at most one
  the default.
- The dial plan: at most 16 prefixes of digits, `+`, `*` and `#`. At most 200
  short codes, each a code of the same characters mapped to a `sip:`, `sips:`
  or `tel:` URI. These are hints for the app's dialer only; the call manager
  still routes every call.

`GET /api/v1/branding` reads it back with what its assets are, and
`brand_url`, where the apps fetch it. `DELETE /api/v1/branding` removes it,
and the apps show their own theme again. Each change publishes
`system.config_changed` with `entity_type` `branding`.

## 2. What the apps fetch

```
GET {public_url}/brand/v1?aor=sip%3Aalice%40acme.example
Authorization: Bearer <the app's access token>
If-None-Match: "<etag>"
```

Any of the tenant's tokens is accepted, an app's session above all
(`docs/APP_SIGN_IN.md`). The token names the tenant, so `aor` is accepted but
not needed. `locale` and `theme` are accepted and change nothing: v1 has no
translations, and both themes are always sent.

| Answer | When |
|--------|------|
| 200 with `ETag` | The tenant has a brand; the body below |
| 304 | `If-None-Match` names what it would send |
| 401 | No token, or one that no longer works |
| 404 | The tenant has no brand: the app shows its own theme |
| 503 | FCP knows no public URL to name the assets by |

The body is the contract's v1:

```json
{
  "schema_version": 1,
  "tenant_id": "tenant-…",
  "etag": "3f0c…",
  "app_name": "Acme Voice",
  "colors": {"primary": "#1A73E8", "on_primary": "#FFFFFF"},
  "dark_colors": {"primary": "#8AB4F8"},
  "logo": {"url": "https://pbx.acme.example/brand/v1/assets/<sha256>",
           "mime": "image/svg+xml", "sha256": "<sha256>"},
  "icon": {"url": "…", "mime": "image/png", "sha256": "…",
           "width": 512, "height": 512},
  "ringtones": [{"name": "Acme", "default": true,
                 "asset": {"url": "…", "mime": "audio/ogg", "sha256": "…"}}],
  "links": {"support": "https://acme.example/support"},
  "dial_plan": {"prefix_strip": ["+1"],
                "short_codes": {"1000": "sip:frontdesk@acme.example"}}
}
```

A field with nothing in it is left out (`logo_dark` above), as the contract
lets it be. The ETag is a hash of the body, so it changes when the brand does
and when an asset's URL does.

**Assets** are at `{public_url}/brand/v1/assets/{sha256}`, with no
credentials: the hash names the file, and a file never changes, so they are
served `Cache-Control: public, max-age=31536000, immutable`. They also carry
the `Content-Type` found at upload and `X-Content-Type-Options: nosniff`. An
SVG also carries a `Content-Security-Policy` that sandboxes it and loads
nothing, in case someone opens it directly. The app checks every file against
its hash (contract §6).

`{public_url}` is `[api.provisioning] public_url`, else `[api.sso]
public_url`, else the `Host` the request came to, as for the provisioning URL
(`docs/PROVISIONING.md`). The app finds it as `docs/SOFTPHONE.md` §2.6 says.

## 3. Where it is kept

Assets are small, so the brand and its assets are kept in Postgres
(`tenant_brands`, `brand_assets`, migration 109) rather than in a directory or
a bucket as firmware is. Every admin node sees them without shared storage,
and the backup (`docs/BACKUP_AND_RESTORE.md`) holds them with the rest of the
database. Both tables go with their tenant.

## 4. Sites

The brand is synced to the tenant's sites like the rest of its configuration
(`docs/HYBRID_CONTROL_PLANE.md`):

- The journal names `tenant_brands`, and the feed carries the brand with what
  its assets are.
- The link fetches the bytes it lacks from the cloud
  (`GET /link/v1/brand-assets/{sha256}`, under the link's mutual TLS and only
  for assets the tenant's brand names) and checks them against the hash.
- The site keeps exactly the assets the brand names, and the brand gone at the
  cloud goes from the site.
- The site's admin serves `/brand/v1` and the assets to the site's softphones
  from its own database, so branding works while the link is down.
- A site's admin refuses writes to `/api/v1/branding` (403); the brand is the
  cloud's to change.

Proved by `fcp-api-core/tests/api_postgres.rs`
(`a_tenant_brands_its_softphone`, `a_sites_admin_leaves_the_brand_to_the_cloud`)
and `fcp-link/tests/branding.rs`.
