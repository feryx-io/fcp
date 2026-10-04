# SCIM provisioning

A tenant's identity provider (Entra ID, Okta, or another SCIM 2.0
client) can make, change and remove the tenant's users itself, through
`/scim/v2` (RFC 7643 and RFC 7644). This is phase 6 of
`docs/IDENTITY_AND_DEVICES.md`; 6a is users, and 6b adds groups.

## 1. Connecting a provisioner

1. A tenant admin makes an API token holding **`scim_provision`**
   (`POST /api/v1/api-tokens` with `"permissions": ["scim_provision"]`, or
   the API Tokens page). Only a tenant admin holds the permission, so only
   one can hand it out.
2. The provisioner is given:
   - the base URL: `https://<admin host>/scim/v2`;
   - the token, as a bearer token.
3. Optionally, the tenant says its users come from SCIM: `sign_in.scim`
   on `PUT /api/v1/tenants/{id}`, or *Users come from SCIM* on the Tenants
   page (§4).

The token's tenant is the tenant provisioned. SCIM is the cloud's: a
site refuses it, and its users arrive through its link.

## 2. What is supported

| Endpoint | Methods |
|---|---|
| `/scim/v2/Users` | `GET` (filter, paging), `POST` |
| `/scim/v2/Users/{id}` | `GET`, `PUT`, `PATCH`, `DELETE` |
| `/scim/v2/ServiceProviderConfig`, `/ResourceTypes`, `/Schemas` | `GET` |

- **Filters:** one `eq` comparison on `userName`, `externalId`, `id`,
  or an email (`emails`, `emails.value`, `emails[value eq "…"]`). Anything
  else answers `400` with `scimType: invalidFilter`.
- **Paging:** `startIndex` (from 1) and `count` (at most 1000).
- **PATCH:** `add`, `replace` and `remove`, in any case, with or without
  a path; paths to an attribute (`active`), a sub-attribute
  (`name.familyName`), an extension attribute
  (`urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department`)
  or a filtered value (`emails[type eq "work"].value`). Booleans may be
  `"True"`/`"False"`, as Entra ID sends them.
- **ETags:** every user carries `meta.version` and an `ETag` header; a
  `PUT`, `PATCH` or `DELETE` with an `If-Match` that names another version
  answers `412`.
- **Errors** are SCIM errors (`urn:ietf:params:scim:api:messages:2.0:Error`)
  with `status` and, where it applies, `scimType` (`uniqueness`,
  `invalidValue`, `invalidFilter`, `invalidSyntax`, `invalidPath`).
- Not supported: bulk operations, sorting, `attributes` /
  `excludedAttributes` (every attribute is returned), changing a password.

## 3. How a SCIM user maps to an FCP user

| SCIM | FCP |
|---|---|
| `id` | the user's id (`user-…`) |
| `userName` | username; a change renames the user (`docs/USERS.md` §6) |
| `externalId` | kept as the provisioner's id, unique in the tenant |
| `active` | `false` suspends; `true` resumes (`docs/USERS.md` §2) |
| `displayName`, else `name.formatted`, else `name.givenName` + `name.familyName` | display name |
| `emails` (primary, else `work`, else the first) | email |
| `phoneNumbers` `work` / `mobile` | phone number / mobile number |
| `title` | job title |
| enterprise `department` | department |
| `urn:ietf:params:scim:schemas:extension:fcp:2.0:User` `extension`, `role`, `locationId` | extension, role (not `super_admin`), location |
| FCP extension `sipUsername` (read only) | the name the user's phones register as |
| `password` | ignored |

- **A new user** is made without a web password: they sign in through the
  tenant's identity provider. Their SIP password is generated. Their role
  is `end_user` unless the FCP extension says otherwise. The tenant's
  `max_users` holds.
- **A replace** (`PUT`) sets every core attribute it carries and clears
  those it leaves out. The enterprise and FCP extensions are changed only
  when the request carries them, so a provisioner that knows nothing of
  FCP's extension does not wipe a user's extension or role.
- **A patch** is applied to the user as SCIM sees them, and the result
  taken as a replace. FCP keeps one display name; a patch to one part of
  the name splits the display name at its first space for the other part.
- **A delete** deprovisions: the user is suspended at once and purged
  after the tenant's grace period, unless restored (`docs/USERS.md` §4).
  To SCIM they are gone (`404`), and their username stays taken until the
  purge.
- `user.created`, `user.updated`, `user.disabled`/`user.enabled`,
  `user.renamed` and `user.deprovisioned` are published as for the admin
  API; every change is in the audit log under the token.

## 4. A tenant whose users come from SCIM

With `sign_in.scim` set:

- single sign-on makes nobody just in time, whatever the provider's
  setting;
- it signs in only users SCIM made (`scim_managed` on the user); anyone
  else is refused as `not_provisioned`.

Password sign-in follows the tenant's password policy as before, so an
administrator made by hand still gets in (break-glass).
