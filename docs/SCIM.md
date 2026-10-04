# SCIM provisioning

A tenant's identity provider (Entra ID, Okta, or another SCIM 2.0
client) can make, change and remove the tenant's users itself, through
`/scim/v2` (RFC 7643 and RFC 7644). This is phase 6 of
`docs/IDENTITY_AND_DEVICES.md`.

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
   page (§5).

The token's tenant is the tenant provisioned. SCIM is the cloud's: a
site refuses it, and its users arrive through its link.

## 2. What is supported

| Endpoint | Methods |
|---|---|
| `/scim/v2/Users` | `GET` (filter, paging), `POST` |
| `/scim/v2/Users/{id}` | `GET`, `PUT`, `PATCH`, `DELETE` |
| `/scim/v2/Groups` | `GET` (filter, paging, `excludedAttributes=members`), `POST` |
| `/scim/v2/Groups/{id}` | `GET`, `PUT`, `PATCH`, `DELETE` |
| `/scim/v2/ServiceProviderConfig`, `/ResourceTypes`, `/Schemas` | `GET` |

- **Filters:** one `eq` comparison. Users: `userName`, `externalId`, `id`,
  or an email (`emails`, `emails.value`, `emails[value eq "…"]`). Groups:
  `displayName`, `externalId` or `id`. Anything else answers `400` with
  `scimType: invalidFilter`.
- **Paging:** `startIndex` (from 1) and `count` (at most 1000).
- **PATCH:** `add`, `replace` and `remove`, in any case, with or without
  a path; paths to an attribute (`active`), a sub-attribute
  (`name.familyName`), an extension attribute
  (`urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department`)
  or a filtered value (`emails[type eq "work"].value`,
  `members[value eq "…"]`). Booleans may be `"True"`/`"False"`, as Entra ID
  sends them. A `remove` of `members` may carry the members to remove as
  its value, as Entra ID sends it.
- **ETags:** every user and group carries `meta.version` and an `ETag` header; a
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

## 4. How a SCIM group maps to an FCP group

| SCIM | FCP (`docs/USERS.md` §5) |
|---|---|
| `id` | the group's id (`grp-…`) |
| `displayName` | name, unique in the tenant (case aside) |
| `externalId` | kept as the provisioner's id, unique in the tenant |
| `members[].value` | members, by user id; each must be a user of the tenant |
| `urn:ietf:params:scim:schemas:extension:fcp:2.0:Group` `role` | the role its members hold (not `super_admin` or `department_admin`) |

- A group SCIM makes is an ordinary group: the admin UI shows it, and a
  queue may take its agents from it. Its members hold its role at once.
- A replace sets the members to exactly those listed; a patch adds and
  removes them. Each change is published as `user.updated` naming
  `groups`.
- A delete removes the group; its members stay, holding their own roles.
- A group that follows an identity provider's group (`idp_group`) is
  filled at sign-in as well; with SCIM, leave `idp_group` unset and let the
  provisioner fill it.

## 5. A tenant whose users come from SCIM

With `sign_in.scim` set:

- single sign-on makes nobody just in time, whatever the provider's
  setting;
- it signs in only users SCIM made (`scim_managed` on the user); anyone
  else is refused as `not_provisioned`.

Password sign-in follows the tenant's password policy as before, so an
administrator made by hand still gets in (break-glass).

## 6. Setting up a provisioner

These follow each console as documented by its vendor; they were written
from that documentation, not checked against a live tenant.

### Entra ID (Microsoft)

1. *Enterprise applications → New application → Create your own
   application → Integrate any other application you don't find in the
   gallery (Non-gallery)*. Single sign-on can be set up on the same
   application (`docs/SSO.md` §7).
2. *Provisioning → Get started*; *Provisioning Mode: Automatic*.
3. *Admin Credentials*:
   - *Tenant URL*: `https://<admin host>/scim/v2`;
   - *Secret Token*: the `scim_provision` token.
   *Test Connection* reads `/scim/v2/Users` with a filter; it should
   succeed.
4. *Mappings*:
   - **Users:** keep `userPrincipalName → userName`, `mail →
     emails[type eq "work"].value`, `displayName`, `givenName`, `surname`,
     `Switch([IsSoftDeleted]…) → active`, `jobTitle → title`, `department
     → urn:…:enterprise:2.0:User:department`, `objectId → externalId`.
     `userPrincipalName` must be a valid FCP username (letters, digits and
     `@ . - _`).
   - To set FCP's own attributes, add attributes to the target schema
     under *Show advanced options → Edit attribute list for customappsso*:
     `urn:ietf:params:scim:schemas:extension:fcp:2.0:User:extension`
     (and `:role`, `:locationId`), and map them.
   - **Groups:** keep `displayName`, `objectId → externalId` and
     `members`.
5. *Users and groups*: assign who is provisioned. *Provisioning Status:
   On*; Entra ID runs a cycle every 40 minutes.
6. Optionally set *Users come from SCIM* on the tenant (§5).

### Okta

1. *Applications → Browse App Catalog → SCIM 2.0 Test App (OAuth Bearer
   Token)*, or your own app integration with SCIM enabled. Single sign-on
   can be on the same application.
2. *Provisioning → Integration*:
   - *SCIM 2.0 Base Url*: `https://<admin host>/scim/v2`;
   - *OAuth Bearer Token*: the `scim_provision` token.
   *Test API Credentials* should succeed.
3. *Provisioning → To App*: enable *Create Users*, *Update User
   Attributes* and *Deactivate Users*. Okta deactivates with
   `active: false` (FCP suspends) and deletes nobody; remove the user in
   FCP when they leave (`docs/USERS.md` §4), or unassign and delete in
   Okta's own flow.
4. *Profile Editor* for the app: `userName` is the user's login by
   default; to set FCP's own attributes add custom attributes with the
   external namespace `urn:ietf:params:scim:schemas:extension:fcp:2.0:User`
   and names `extension`, `role`, `locationId`.
5. *Push Groups*: push the groups whose members FCP should know; Okta
   adds and removes members with `PATCH`.
6. Optionally set *Users come from SCIM* on the tenant (§5).
