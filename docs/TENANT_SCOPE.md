# Every tenant its own system

D8 findings 48–61 (2026-10-31), raised on the admin UI:

- **48–52, 54–56.** Trunks, schedules, routing rules, IVR flows, webhooks, classes of
  service, least-cost routes and parking lots "are not scoped per tenant".
- **50.** Each location should be able to have its own routing rules.
- **53.** "Needs a full tenant scope review. Every tenant should act as their own system."
- **57.** API tokens should be scoped per tenant, and never carry platform
  administration such as managing tenants.
- **58.** What is the difference between sites and locations, and should they be
  tenant scoped?
- **59.** Audio prompts: system prompts shared by every tenant, which a tenant can
  override with its own.
- **60.** Audit log: a platform administrator sees every tenant's; a tenant
  administrator sees all of their tenant's, and only theirs.
- **61.** Flow traces: a tenant administrator sees only their tenant's.

Agreed 2026-10-31 on every recommendation in §6.

## 1. What exists

A review of every tenant-owned resource (the database, the API and the call path) found
that **tenancy is already in the model almost everywhere**. Each of these has a
`tenant_id NOT NULL` column, an API that lists only the caller's tenant and answers 404
for another tenant's id, and a call path that reads only the calling tenant's rows:

| Resource | Table | Call path |
|---|---|---|
| Trunks | `trunks` (032) | `TrunkSet::for_tenant`: the tenant's own, then the platform's (below) |
| Schedules | `schedules` (083) | `schedule_divert` takes only the call's tenant's |
| Routing rules | `routing_rules` (035) | `TenantRuleSet` keyed by tenant |
| Flows | `flows` (005; tenant nullable) | bindings are checked; `start_flow` is not (§1.3) |
| Classes of service | `classes_of_service` (084) | `class_for(tenant, user)` |
| Least-cost routes | `lcr_routes` (076) | `TrunkSet::lcr_for(tenant)` |
| Parking lots | `parking_lots` (128) | the call's tenant's lots |
| Webhooks | `webhook_endpoints` (038) | delivered only to the event's tenant |
| Sites | `sites` (078) | the tenant's own stack; the all-tenant list is platform-only |
| Locations | `locations` (022) | addresses for E911 and placement |
| API tokens | `api_tokens` (017) | never more than a tenant administrator holds |

Why it all **looks** shared:

### 1.1 The tenant picker does nothing for a person signed in

`TenantContext` (`fcp-api-core/src/auth.rs:528`) returns the identity's own tenant
first, and reads the picked tenant (`x-fcp-tenant-id`) only when the identity has none.
A signed-in session always has one, the user's home tenant (`auth.rs:325`), even for a
platform administrator. So a platform administrator who picks tenant B still lists,
creates and edits in their own tenant. Only the static token honours the header, and
only the static token is tested with it.

Handlers that offer platform administrators an all-tenant view (`cdrs.rs:265`,
`compliance.rs:559`, `flow_traces.rs:198`) read "a header was sent" as "one tenant", then
use the session's home tenant rather than the one sent.

### 1.2 Seven pages never send a tenant

Trunks, Routing, Least-Cost Routes, Flows, Schedules, Parking and Classes of service
call the API without the tenant header (`endpoints.ts`), and none uses the
`TenantSelector` helpers: no tenant column, no tenant on create. The Trunks page is
half-aware: its Site list follows the picker while the trunk is made in the home
tenant, which the API then refuses as another tenant's site.

With every test object in one tenant, as on the D8 stack, every page shows the same
rows whatever is picked.

### 1.3 Real gaps

- **Prompts (59).**
  - System prompts are seeded into the default tenant only: the call manager's
    voicemail prompts and the conference server's prompts.
  - Prompts by name are looked up in that tenant only (`fcp-core/src/prompt.rs:241`),
    so other tenants never get them (speech synthesis instead) and cannot override
    one.
  - A flow's Play by prompt id (`resolve_by_id`, `prompt.rs:302`) does not check the
    tenant: a flow can play another tenant's prompt.
- **Audit (60).**
  - Reads are scoped: a tenant administrator sees their tenant's rows, and a platform
    administrator without a tenant sees all.
  - But a row is filed under the **actor's** tenant (`audit.rs:157`). A platform
    administrator creating, suspending or deleting tenant X is logged in the platform
    administrator's own tenant (or the default tenant, for the static token). That
    tenant's administrators can read it, and X's never see it.
- **Flow traces (61).** Scoped, but the list queries without a tenant and filters each
  row afterwards (`flow_traces.rs:196`). A tenant administrator gets short or empty
  pages and no total, and the trace's own `tenant_id` column is unused.
- **Flows (51).** `FlowHandler::start_flow` loads a flow by id with no tenant check
  (`fcp-sip/src/flow_handler.rs:508`). Today only the checked bindings guard it.
- **Parking (56).** A lot's location is stored but never used: `parking_lot_for` passes
  no location (`fcp-sip/src/handler.rs:14169`), so `*70` takes the tenant's first lot,
  against `docs/PARKING.md`.
- **Routing (50).** Rules are the tenant's, but:
  - the node's routing rules file (`[routing] rules_file`) applies to every tenant before
    them;
  - no rule can be limited to a location.
- **Trunks (48).** A tenant's trunks are its own. The trunks in each node's TOML
  (`[[trunks]]`) are a shared fallback for every tenant whose `platform_trunk_fallback`
  is on (the default). The admin UI shows neither the fallback nor the switch beside
  the trunks.
- **Tokens (57).**
  - A token never holds more than a tenant administrator: no tenant creation or
    deletion, no system configuration, never a platform administrator. Its maker's
    permissions bound it at every use.
  - The gap is legacy tokens with no recorded maker that carry `admin` or `*`. They
    pass every permission check within their tenant, system configuration included.
- **Smaller.**
  - Queue names are checked for uniqueness across every tenant (`queue_config.rs:32`),
    so a tenant can probe another's queue names.
  - A number with a line binding still says `unassigned` (seen on D8: Alice's
    +1 425 585 2628, finding 46).

### 1.4 Sites and locations (58)

- **A location** is an address: an office or a floor, with its E911 details (civic
  address, ELIN, validation). Devices, extensions, numbers and parking lots name one.
  The Rust type is `SiteLocation`, a name that adds to the confusion.
- **A site** is a customer's own FCP stack on its premises, enrolled to the cloud
  through `fcp-link` (`docs/HYBRID_CONTROL_PLANE.md`).
- Both belong to a tenant. A tenant administrator manages their own sites; the
  all-tenant site list is the platform administrator's.
- The Parking page labels its location field "Site", which is wrong.

## 2. The model

**A platform administrator works in one tenant at a time, the one picked, or in "All
tenants" where a page has an all-tenant view.** A tenant administrator works only in
their own. Every tenant-owned page says which tenant it shows.

**What is the platform's, not a tenant's**, and stays so: tenants themselves, node
configuration (the platform's trunks and LCR, the node routing rules file, `[pbx]`
defaults), firmware images, the vendor template library, the cluster and services,
and system prompts. A tenant sees the platform's only where it affects them, read-only:
the system prompts it can override, and whether its calls may leave on platform
trunks.

## 3. The changes

### 3.1 The picked tenant, honoured (48–56)

- `TenantContext`: for a **platform administrator** (a super_admin session, the static
  token), a valid `x-fcp-tenant-id` decides the tenant. Without it, their home tenant,
  as now.
  - Anyone else's header is ignored, as now: a tenant administrator cannot leave their
    tenant.
  - An unknown tenant answers 404.
- All-tenant views (CDRs, the audit log, flow traces, sites) treat "no header from a
  platform administrator" as all tenants, and a header as that tenant, the one sent.
- Every tenant-owned page follows the picker: the seven pages in §1.2 send the header,
  create in the picked tenant, and show a **tenant badge** ("Tenant: Acme") in the page
  header.
  - With "All tenants" picked, a page without an all-tenant view asks for a tenant
    rather than showing the home tenant's.
- **Trunks page:** shows whether the tenant's calls may fall back to the platform's
  trunks (the tenant's `platform_trunk_fallback`), with the switch for a platform
  administrator.
- **Parking page:** its "Site" field becomes "Location".
- **Tests:**
  - The route matrix gains a third probe. A platform administrator's **session**
    working in tenant B through the header sees B's objects and creates in B. Without
    the header, it sees its own tenant's (or all).
  - A tenant administrator's header naming B changes nothing.

### 3.2 System prompts and overrides (59)

- **System prompts** are the platform's: `prompts.tenant_id` becomes nullable, and a
  row without a tenant is a system prompt (migration: the default tenant's `is_system`
  rows become system rows).
  - The call manager and the conference server seed them once, for everyone.
  - A platform administrator manages them (with "All tenants" picked, or under a
    "System prompts" tab).
- **An override** is a tenant's prompt of the same name and language.
  - Lookup at playback: the tenant's override, then the system prompt, then speech
    synthesis.
  - Prompts are resolved with the call's tenant everywhere: voicemail, flows, IVR,
    conference, hold music.
  - Removing an override returns to the system prompt.
- `resolve_by_id` takes the call's tenant and plays only that tenant's prompt or a
  system prompt.
- **Prompts page:** a tenant administrator sees the system prompts read-only, each with
  **Override** (upload or record their own, same name and language) or, once
  overridden, **Restore**.

### 3.3 The audit log by the tenant acted on (60)

- A row's tenant is the tenant **acted on**:
  - the request's tenant (`TenantContext`);
  - for `/tenants/:id`, that tenant;
  - for platform-level actions (nodes, firmware, system settings), no tenant: a
    platform row.
- A new `actor_tenant_id` keeps where the actor came from.
- **A tenant administrator** sees every row of their tenant, including a platform
  administrator's actions on it (shown as "platform administrator").
- **A platform administrator** sees everything, with a tenant column and filter:
  - All tenants;
  - one tenant;
  - platform rows only.
- Rows already written stay as they are. The upgrade notes say platform actions
  before this release were filed under the actor's tenant.

### 3.4 Flows and flow traces (51, 61)

- `start_flow` takes the call's tenant and refuses another tenant's flow. Built-in
  flows remain everyone's.
- New flows always carry their tenant. A flow without one (from before tenancy) is
  adopted by the tenant of the bindings that name it, or else listed for platform
  administrators to assign.
- Flow traces are listed by `tenant_id` in SQL, with keyset paging and a total. The
  adoption sweep (`adopt_untenanted_traces`) also runs before a tenant's first page.

### 3.5 Parking by location (56)

- `*70` and a transfer to the park code park in the lot of the **parker's location**:
  - the device's location, else its owner's extension's;
  - else the tenant's lot with no location;
  - else its first lot.
- This is `docs/PARKING.md`'s rule, now carried out.

### 3.6 Routing rules per location (50)

- A routing rule may name a **location** (`routing_rules.location_id`, nullable).
- A call's location is the caller's device's location, else their extension's, else
  their line's number's.
- Evaluation order:
  1. the node's rules file;
  2. the call's location's rules;
  3. the tenant's rules without a location.
- A location's rule wins over a tenant-wide one at the same priority.
- The Routing page shows a rule's location and filters by it. The route preview takes
  a location.

### 3.7 Tokens and the rest (57)

- **Legacy tokens** (no maker, `admin` or `*`) are held to a tenant administrator's
  permissions at use, as made tokens are. A tenant administrator never holds system
  configuration, so neither do these tokens.
  - The token list marks them "legacy: make a new one". The upgrade notes say so.
- Queue names are unique per tenant.
- A line binding makes its number `assigned`. Removing the last binding makes it
  `unassigned` again.

## 4. Phases

1. **The picked tenant** (§3.1).
   - `TenantContext` for platform administrators, the all-tenant handlers, and the
     seven pages with the tenant badge.
   - The trunk fallback shown, Parking's "Location", and the route matrix's third probe.
   - This alone answers how 48, 49, 52, 54 and 55 look.
2. **Prompts** (§3.2).
   - The migration, seeding once, lookup by the call's tenant with override then system
     then speech, and `resolve_by_id` held to the tenant.
   - The Prompts page's system prompts, Override and Restore.
3. **The audit log** (§3.3).
   - Rows by the tenant acted on, `actor_tenant_id`, platform rows, the platform
     administrator's tenant column and filter.
4. **Flows, traces, parking, tokens** (§3.4, §3.5, §3.7).
5. **Routing rules per location** (§3.6).

## 4a. Done

- **Phase 1** (v0.25.495): the picked tenant.
  - `TenantContext` lets a platform administrator's header decide the tenant
    (a super_admin session or the static token; an unknown tenant answers 404).
    Anyone else's header is ignored, as before.
  - The all-tenant handlers (CDRs, call quality, SIP and flow traces,
    registrations, the audit log) need no change: a header now means the tenant
    sent, not the session's own.
  - The admin UI's API client sends the picked tenant with every request
    (`ApiClient.setTenant`, set by the tenant store and cleared at sign-out). The
    picker resets the query cache when it changes.
  - `TenantScope` frames Trunks, Routing, Least-cost routes, Flows and the flow
    builder, Schedules, Parking and Classes of service. It shows "Tenant: X", and
    with "All tenants" picked asks for a tenant.
  - Other tenant-owned pages (webhooks, tokens, prompts, …) follow the picker
    through the header without the frame.
  - The Trunks page shows whether the tenant's calls may fall back to the
    platform's trunks, with the switch for a platform administrator. The API still
    lets a tenant administrator change it (`platform_trunk_fallback` on
    `PUT /tenants/{id}`), as before.
  - Parking's "Site" field is "Location".
  - **Tests:**
    - The route matrix's third probe,
      `a_platform_administrator_works_in_the_tenant_picked`: on every read naming
      tenant B's objects, a platform administrator's session with B's header
      answers as the static token does, and shows the same B objects.
      - It also checks that a schedule made with the header is B's, and that an
        unknown tenant answers 404.
      - Against the previous code, 99 of 145 reads differed.
    - The UI suite's trunk test picks the tenant first, then sees the badge and
      the fallback card.

## 5. Answers

- **46:** Alice's caller ID is her line binding's: she is bound to +1 425 585 2628, with
  that number as the binding's outbound caller ID. It was made through the API on
  2026-10-18 during D8's setup. A call leaving on a trunk takes the caller's line's
  outbound caller ID, else the line's number (`caller_presentation`,
  `fcp-sip/src/handler.rs`).
- **47:** fixed in v0.25.493: the flow canvas follows the app's theme.
- **58:** §1.4.

## 6. Questions (agreed 2026-10-31 on their recommendations)

1. **A platform administrator works in one picked tenant**, with "All tenants" only where
   a page has an all-tenant view. Recommended: yes; it is how the picker already
   reads, and it keeps a create from landing in the wrong tenant.
2. **System prompts are tenantless rows**, overridden by a tenant's prompt of the same
   name and language. Recommended: yes; it needs no copy per tenant and an update to a
   system prompt reaches everyone who has not overridden it.
3. **An audit row belongs to the tenant acted on**, with the actor's tenant kept.
   Recommended: yes; a tenant sees what was done to it, by whom.
4. **Legacy `admin` tokens held to a tenant administrator's permissions**, not revoked.
   Recommended: yes; nothing in use stops working, and nothing can exceed a tenant
   administrator.
5. **Per-location routing rules evaluated before the tenant's**, a location's caller
   being its device's, extension's or number's. Recommended: yes.
6. **The platform's trunks stay node configuration**, with each tenant's fallback switch
   shown on its Trunks page. Recommended: yes; moving them into the database is its own
   project.
