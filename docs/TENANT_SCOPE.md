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
- **Phase 2** (v0.25.496): system prompts and overrides.
  - **The model.** `Prompt.tenant_id` is `Option<TenantId>`: `None` is a
    system prompt (`is_system` kept equal to it by a check constraint).
    Migration 133 drops `NOT NULL`, moves the seeded `is_system` rows to the
    platform (the newest per name and language; any other stays its tenant's
    as an override) and adds a unique index on `(name, language)` among
    system prompts.
  - **Seeding.** The call manager (voicemail prompts) and the conference
    server seed `Prompt::system_tts` once, for every tenant, and fill in
    their audio under `<prompts_dir>/system/`. The conference server's mute,
    lock and ending sounds are loaded once at startup for every room, so they
    are the system prompts' and not overridable; every other conference
    prompt is resolved per room.
  - **Lookup by name** (`PromptStore::resolve_prompt`): the call's tenant's
    prompt in the language, else the system prompt in it, else the tenant's
    then the system's in any language, else the caller's TTS. Without a
    tenant, only system prompts. `PromptService` no longer holds a tenant:
    every call passes its own (the call's, else its flow's), for voicemail,
    hoteling, flows (Play, Gather and its retry prompts) and conference rooms.
  - **Lookup by id** (`fcp_core::prompt::effective_prompt_by_id`): the
    tenant's own prompt as it is, a system prompt through the tenant's
    override of it, another tenant's prompt not at all. Used by a flow's
    Play URL, hold music, the recording-consent announcement and live call
    control's `play`. Choosing a prompt (hold music, consent) accepts a
    system prompt.
  - **The API.**
    - `GET /prompts` lists the tenant's prompts and the system prompts
      (`scope=all|tenant|system`). A system prompt carries `overridden_by`
      (the tenant's copy), a tenant's prompt `overrides` (the system prompt
      it replaces). `tenant_id` is `null` on a system prompt.
    - Only a platform administrator changes a system prompt (PUT, audio,
      synthesize) or makes one (`"system": true`); anyone else gets 403 and
      is told to override it. A system prompt is never deleted.
    - `POST /prompts/{id}/override` makes the tenant's copy of a system
      prompt: its name, language, category, words, voice, and a copy of its
      audio, so nothing changes for callers until the copy does. A second
      override answers 409. Deleting the copy restores the system prompt.
    - Creating a prompt whose name and language the owner already has
      answers 409 (it used to upsert, leaving the new id pointing at nothing).
  - **Sites.** System prompts are not journaled (the journal skips rows
    without a tenant) and not in a site's snapshot: a site's own call manager
    and conference server seed them there. A tenant's overrides sync as its
    other prompts do.
  - **The Prompts page.** System prompts are listed with the tenant's,
    marked System (and Overridden once the tenant has a copy). A tenant
    administrator has Override on a system prompt and Restore system prompt
    on its copy; a platform administrator also edits system prompts and may
    make one.
  - **Tests:**
    - `system_prompts_are_every_tenants_and_overridden_per_tenant`
      (`fcp-api-core/tests/api_postgres.rs`): a platform administrator's
      system prompt with audio; a tenant administrator sees and plays it and
      is refused every change; the override copies the audio, a second answers
      409, an upload to it leaves the system prompt's audio alone; the lists
      name the relation; tenant B sees the system prompt unoverridden and not
      A's copy; by id and by name A hears its copy, B and a tenantless call
      the system prompt; deleting the copy restores it; the platform
      administrator's edit is seen by B.
    - `a_system_prompt_plays_as_the_tenants_override_once_it_has_one`
      (`fcp-call-manager/tests/calling.rs`): a system prompt chosen as the
      tenant's hold music plays to a held caller, then the tenant's override
      of it does, and not the system prompt.
    - The UI suite's `a system prompt is overridden and restored`.
- **Phase 3** (v0.25.497): the audit log by the tenant acted on.
  - **Filing** (`fcp_api_core::audit::acted_on`): what the response says
    (`AuditScope`: a new tenant's creation is the new tenant's; everything
    behind `require_platform_admin` — the cluster and services routers — is
    the platform's), else none for a platform route (`/system/…`,
    `/cluster/…`, firmware images; a tenant's firmware targets are its own),
    else the tenant a `/tenants/:id…` route names, else the tenant the
    request worked in: a platform administrator's picked tenant or their own,
    anyone else's own.
  - **`actor_tenant_id`** (migration 134) keeps the actor's own tenant,
    `null` for the static token.
  - **Reading.**
    - A tenant administrator sees all of their tenant's rows, a platform
      administrator's actions on it included. An actor from outside the
      tenant (the static token, or anyone of another tenant) is shown as
      `platform administrator`, `actor_kind` `platform`, without their tenant
      or address. A `tenant` filter from them is ignored.
    - A platform administrator reads every row as it is, with `tenant=all`,
      `tenant=platform` or a tenant's id; without it, the picked tenant, else
      all. The listing and the export answer the same, the export with
      `actor_tenant_id` as its last CSV column.
  - **Compliance.** A subject export (and the erasure built on it) takes a
    tenant's audit rows of its own people only (`actor_tenant_id` null or
    the tenant), not a platform administrator's actions on it. Retention
    runs per tenant, so platform rows are not deleted by any tenant's
    window.
  - **Rows already written** keep their tenant (the actor's) and have no
    actor tenant (`BACKUP_AND_RESTORE.md` §5.4.1).
  - **The Audit Log page.** A platform administrator sees a Tenant column
    (by name, `Platform` for none), "from <tenant>" under an actor acting
    outside their own tenant, and a Rows filter (as picked, every tenant and
    the platform, the platform only).
  - **Tests:**
    - `an_audit_row_belongs_to_the_tenant_acted_on`
      (`fcp-api-core/tests/api_postgres.rs`): the static token in tenant B,
      a super administrator of A working in B (B's row, A's actor tenant),
      `PUT /tenants/B` with A picked (B's), a tenant's creation (the new
      tenant's), a firmware upload with a tenant picked (the platform's);
      the platform administrator's `platform`, one-tenant, `all` and picked
      views, and a bad `tenant`; B's administrator seeing the platform's
      three actions on B as `platform administrator` whatever they ask for,
      and their own under their name. Against the previous code the super
      administrator's, the tenant route's and the firmware rows were filed
      elsewhere.
    - `audit::tests::a_row_is_filed_under_the_tenant_acted_on`, the rule
      itself.
    - The UI suite's `e2e/audit.spec.ts`: the platform administrator's
      Tenant column and "The platform only"; a tenant administrator's view
      without either, a static-token action shown as `platform
      administrator`.
- **Phase 4a** (v0.25.498): flows, traces, parking, tokens, numbers.
  - **Flows.** `FlowHandler::start_flow` refuses another tenant's flow
    (`flow_belongs_to`: a flow's tenant compared as an id; a flow without a
    tenant is anyone's). Call control's redirect already checked.
    `GET /flows/untenanted` (a platform administrator's, `system_config` in the
    route table) first gives each flow
    without a tenant the tenant of what names it — line bindings, mailboxes,
    its own traces — when they all name one (`FlowStore::adopt_untenanted_flows`,
    its versions with it), then lists the rest; `POST /flows/{id}/assign`
    gives one to a tenant (409 for a flow with one; filed under that tenant in
    the audit log). The Flows page shows a platform administrator those left,
    each with "Give to this tenant".
  - **Flow traces.** Listed by `tenant_id` in SQL with a total, so a tenant's
    page is full and counted; traces written without a tenant take their
    call's record's before a tenant's first page. Paging stays by offset
    (`o:<n>`), as before, not keyset.
  - **Parking.** `*70` and a transfer to the park code park in the lot of the
    parker's location (`parker_location`: the device the request came from by
    its credentials, MAC or the parker's only device, else the parker's
    extension, the parker named by number or SIP name), else the tenant's lot
    without a location, else its first. Location ids compare as ids (`loc-` or
    bare).
  - **Tokens.** A token without a maker is held to a tenant administrator's
    reach (`effective_permissions`), a legacy `admin` or `*` token included.
    The tokens page says "Legacy: make a new one".
  - **Numbers.** Binding a number makes an `unassigned` one `assigned`;
    removing its last binding makes an `assigned` one `unassigned`; other
    states are left alone. Migration 135 marks the numbers already bound.
  - **Queue names** were still checked across tenants then; per-tenant names
    are phase 4b.
  - **Tests:**
    - `flows_traces_tokens_and_numbers_keep_to_their_tenant`
      (`fcp-api-core/tests/api_postgres.rs`): three flows without a tenant —
      one run by A alone (adopted, its versions readable), one by A and B
      (left, then assigned to B, then refused a move), one by none (left, a
      tenant that does not exist refused); a tenant administrator refused
      both routes; A's administrator paging A's five of twelve traces two at
      a time, every page full and the total 5; a legacy `admin` token reading
      users but refused `GET /config` (it reached it before); a number
      `unassigned`, `assigned` once bound, `unassigned` once unbound. Against
      the previous code the token and the trace total fail.
    - `a_call_parks_in_the_lot_of_the_parkers_location`
      (`fcp-call-manager/tests/calling.rs`): two lots, the branch's and one
      without a location; Bob, whose extension is at the branch, parks in the
      branch's lot, Carol in the other.
- **Phase 4b** (v0.25.499): queues by tenant.
  - **The key.** The queue manager keys a queue by `QueueKey` (its tenant and
    its name) and an agent by `AgentKey` (its tenant and its id), in every map
    it keeps: queues, agents, counters, history, campaigns, wrap-up timers.
    `DistributionPlan.agent_id` is an `AgentKey`. An agent logs in only to
    its own tenant's queues; a call moves only to a queue of its own tenant.
    The `_for_tenant` lookups went; the key holds the tenant.
  - **Names per tenant.** The API refuses a second queue of one name or one
    extension in a tenant (409) and looks at no other tenant. The call
    manager loads every tenant's queues; a second `sales` used to stop it
    loading any queue from the database. Two queues at one extension in a
    tenant now never reach it.
  - **What looked across tenants and no longer does:**
    - the queue rang an agent, a supervisor, an overflow target, a callback
      and a campaign number back through the node at the node's own address,
      which made the leg the node's default tenant's. Each now names its
      tenant (`SipUserAgentClient::invite_for_tenant`, the header
      `X-FCP-Own-Leg-Tenant`, believed only beside the own-leg token and
      stripped at ingress otherwise);
    - the queue handler worked out a call's tenant again from its domain; it
      takes the call's (`QueueHandler::can_handle(request, tenant)`), so a
      registered caller who dials the node's address reaches its tenant's
      queue;
    - `*50` looked for the extension in every tenant's queues; `*51` logged
      out by extension alone (it now resolves the caller's tenant);
    - do not disturb and on a call were read in any tenant, for queues and
      hunt groups alike; they are read in the call's tenant;
    - stats snapshots, the SLA warning and their history were published as
      the node's default tenant's; each tenant's are its own;
    - agents were restored after a restart for the default tenant only;
    - a queue's position announcement fell back to some TOML queue's.
  - **The node's own `/queues` and `/agents` routes** act in its default
    tenant, as before; `/callbacks` lists every tenant's, each with its
    `tenant_id`, and `/queues/reload` names each new queue's tenant.
  - **Found on the way:** a caller hanging up while the queue rang an agent
    sent no CANCEL. `remove_session` aborted the ring task as it signalled
    it, so the phone rang on until the B2BUA's ring timeout, and an agent
    who answered in that time was on a call with nobody. The ring task now
    cancels, and hangs up a leg whose 200 crossed the CANCEL (`cancel_ring`,
    as the B2BUA does for a fork's losers), on a caller leaving and on every
    ring timeout.
  - **Tests:**
    - `two_tenants_queues_and_agents_of_the_same_name_are_their_own`
      (`fcp-queue`): two `sales` at 6000 and two agents 100; each tenant's
      call offered to its own agent, no move across, a logout leaving the
      other, a login to another tenant's queue refused.
    - `queue_configs_crud_and_runtime_absent` (`api_postgres.rs`): B's
      `billing` at 7100 beside A's, A's second `billing` and second 7100
      refused 409, a rename onto a taken name refused.
    - `two_tenants_queues_of_one_name_ring_their_own_agents`
      (`fcp-call-manager/tests/calls.rs`): two tenants' `sales` at 7000, each
      with its agent at 1001; each caller rings its own agent and not the
      other's. Against the previous code the reload refused both queues.
    - `a_caller_leaving_the_queue_stops_the_agents_ring` (`calls.rs`): the
      agent's phone is cancelled within 1.5 s of the caller's hangup; with
      the abort back it rang until the 3 s ring timeout.
    - `only_an_own_leg_names_its_tenant` (`fcp-sip` claims) and
      `only_do_not_disturb_and_a_call_make_a_member_unavailable` (now in
      the tenant asking).

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
