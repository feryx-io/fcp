

  Here is my thought: Separate PhoneNumber entity (E.164 like
  +15183217034) from User and Device; map numbers to users/devices via assignments.
  Keeps inventory/routing clean and handles shared lines.
  Treat numbers as a type of LineAppearance on devices: a number can appear on 0..N
  devices (simulring, hunt policy), and a device can host multiple numbers.
  Users have identities and permissions; numbers are routable endpoints. Avoid
  conflating “user” with “number” so you can handle IVRs, hunt groups, parking or
  temporary numbers.
  Suggested data model

  User: auth identity, profile, voicemail, policies.
  Device: physical or soft endpoint, capabilities, registration state.
  PhoneNumber: inventory record with state (unassigned/reserved/active), country,
  locality, carrier, E911 address, CNAM, tags.
  LineBinding: links PhoneNumber → User (owner) and 1..N Devices (appearances) with ring
  strategy (all, order, primary/secondary), caller ID settings.
  Optional: Extension entity for short dialing; translate extension ↔ phone number when
  routing.
  Routing flow

  Inbound PSTN: normalize to E.164 → lookup PhoneNumber → get bindings → ring devices
  per policy; if no registration, fail to voicemail/forward.
  Internal call by username/extension: resolve to user, then either pick primary number
  or do direct device ringing based on presence/policy.
  Outbound: apply user/device outbound policy, choose caller ID (user primary number),
  run dial plan transforms, send to trunk.
  CUCM-like separation vs “numbers as users”

  CUCM-style separation is preferred: numbers are independent resources that can be
  assigned, parked, or moved without recreating users/devices.
  Avoid “number as user”; it makes shared lines, services, IVRs, and renumbering
  painful.
  Feature considerations

  Multi-device: allow multiple appearances with priority and timeout.
  Services: allow numbers bound to IVRs/queues/hunts without users.
  Policies: per-number call block/allow, CNAM, emergency address, recording, forwarding.
  Lifecycle: reserved → active → quarantine; audit who assigned/when.
  Porting: track LOA/CSR, FOC date, activation window.
  Dial plan: normalize inputs (+E.164 internally), handle local 10D/7D, extensions.
  E911: enforce address and callback number before activation.
  Voicemail: tie mailbox to user; number forwards on no-answer/busy.

● Your architectural thinking is excellent and aligns with carrier-grade telephony systems.
  Let me validate and expand on your proposal:

  ✅ Your Core Design is Correct

  Separation of Concerns is the right approach:
  - User = Identity/authentication/permissions (Alice, Bob)
  - PhoneNumber = Routable E.164 endpoint (+15183217034)
  - Device = Physical/soft endpoint (Zoiper on 192.168.1.80)
  - LineBinding = The glue that connects them

  This mirrors CUCM, BroadWorks, and modern CPaaS platforms (Twilio, Vonage).

  Refined Data Model

  // fcp-core/src/phone_number.rs
  pub struct PhoneNumber {
      pub id: PhoneNumberId,           // Internal UUID
      pub e164: String,                 // +15183217034
      pub country_code: String,         // "1" (US/Canada)
      pub npa: String,                  // "518" (area code)
      pub nxx: String,                  // "321"
      pub line: String,                 // "7034"
      pub state: NumberState,           // Unassigned/Reserved/Active/Quarantine/Ported
      pub tenant_id: TenantId,

      // Regulatory/Compliance
      pub e911_address: Option<E911Address>,
      pub e911_callback: Option<String>,  // Must be validated before activation
      pub cnam: Option<String>,           // Caller ID name (15 chars max)
      pub spam_score: Option<f32>,        // STIR/SHAKEN attestation level

      // Carrier metadata
      pub carrier: Option<String>,        // Provider who owns the number
      pub lrn: Option<String>,            // Location Routing Number (LNP)
      pub ocn: Option<String>,            // Operating Company Number
      pub rate_center: Option<String>,
      pub lata: Option<String>,

      // Lifecycle
      pub acquired_at: SystemTime,
      pub activated_at: Option<SystemTime>,
      pub ported_at: Option<SystemTime>,
      pub loa_date: Option<SystemTime>,   // Letter of Authorization
      pub foc_date: Option<SystemTime>,   // Firm Order Commitment

      // Features
      pub recording_enabled: bool,
      pub block_anonymous: bool,
      pub allow_list: Vec<String>,        // Whitelist
      pub block_list: Vec<String>,        // Blacklist
      pub tags: Vec<String>,              // Billing codes, departments, etc.
  }

  pub enum NumberState {
      Unassigned,     // In inventory, not assigned
      Reserved,       // Held for future use (port-in pending, etc.)
      Active,         // In use, routing calls
      Quarantine,     // Recently released (90-day cool-off for spam reputation)
      Ported,         // Ported out to another carrier
  }

  // Line binding - connects number to users/devices
  pub struct LineBinding {
      pub id: LineBindingId,
      pub phone_number_id: PhoneNumberId,
      pub owner_user_id: Option<UserId>,  // Who "owns" this number (for billing, voicemail)
      pub binding_type: BindingType,
      pub devices: Vec<DeviceBinding>,
      pub ring_strategy: RingStrategy,
      pub tenant_id: TenantId,

      // Call handling
      pub no_answer_timeout: Duration,     // Ring duration before voicemail
      pub forward_on_busy: Option<String>,
      pub forward_on_no_answer: Option<String>,
      pub forward_unconditional: Option<String>,

      // Caller ID
      pub outbound_caller_id: Option<String>,  // Override for this line
      pub outbound_cnam: Option<String>,

      pub created_at: SystemTime,
  }

  pub enum BindingType {
      User,           // Normal user line
      IVR,            // Auto-attendant
      Queue,          // Call center queue
      HuntGroup,      // Ring multiple users in order/round-robin
      ConferenceBridge,
      Voicemail,      // Direct voicemail access
      Parking,        // Call parking slot
      Paging,         // Overhead paging
      Fax,            // Fax-to-email
      Emergency,      // E911 callback
  }

  pub struct DeviceBinding {
      pub device_id: DeviceId,
      pub priority: u8,           // 1 = primary, 2+ = secondary
      pub line_appearance: u8,    // Which button/line on the phone (1-6 typically)
      pub shared_line: bool,      // Shared call appearance (BLF, pickup)
      pub enabled: bool,
  }

  pub enum RingStrategy {
      Simultaneous,               // Ring all devices at once
      Sequential,                 // Ring in priority order with timeout
      RoundRobin,                 // Distribute calls evenly (hunt group)
      LongestIdle,                // Ring device that's been idle longest
      Primary,                    // Only ring primary device
  }

  // Extensions - short dialing
  pub struct Extension {
      pub id: ExtensionId,
      pub number: String,         // "1234" or "5000"
      pub target_type: ExtensionTarget,
      pub tenant_id: TenantId,
  }

  pub enum ExtensionTarget {
      User(UserId),               // Extension dials a user's primary line
      PhoneNumber(PhoneNumberId), // Extension maps to specific DID
      Device(DeviceId),           // Direct device dial
      Service(ServiceId),         // IVR, queue, etc.
  }

  // Device - enhanced from current model
  pub struct Device {
      pub id: DeviceId,
      pub name: String,
      pub device_type: DeviceType,
      pub mac_address: Option<String>,
      pub model: Option<String>,      // "Cisco 8861", "Zoiper 5"
      pub firmware_version: Option<String>,

      // SIP registration
      pub sip_uri: Option<String>,    // sip:alice@192.168.1.80
      pub registration_state: RegistrationState,
      pub last_registration: Option<SystemTime>,

      // Capabilities
      pub max_lines: u8,              // How many line appearances
      pub supports_video: bool,
      pub supports_blf: bool,         // Busy Lamp Field
      pub codec_support: Vec<AudioCodec>,

      // Location
      pub location: Option<String>,   // "Building A, Floor 2, Room 205"
      pub e911_override: Option<E911Address>,  // Per-device E911 if mobile

      pub tenant_id: TenantId,
      pub owner_user_id: Option<UserId>,
  }

  pub enum RegistrationState {
      Unregistered,
      Registered,
      Failed,
      Expired,
  }

  Routing Flow Examples

  Inbound PSTN Call to +15183217034

  1. INVITE from trunk: INVITE sip:+15183217034@fcp-sbc.example.com
  2. Normalize to E.164: "+15183217034"
  3. Query PhoneNumber table: WHERE e164 = '+15183217034' AND state = Active
  4. Get LineBinding for that PhoneNumber
  5. Resolve DeviceBindings based on ring_strategy:
     - Simultaneous: Fork INVITE to all devices
     - Sequential: Try device 1 (30s timeout), then device 2, etc.
  6. If no answer after timeout: Forward to user's voicemail
  7. CDR records: +1caller → +15183217034 → alice@device1

  Internal Extension Dial: 1234

  1. INVITE: sip:1234@192.168.1.225
  2. Query Extension table: WHERE number = '1234'
  3. Resolve target:
     - If User: Get user's primary PhoneNumber
     - If PhoneNumber: Direct lookup
     - If Device: Direct device call
  4. Apply ring strategy and fork

  Outbound Call from Alice

  1. INVITE from Alice's device: sip:+15185551212@...
  2. Authenticate/authorize Alice (existing User lookup)
  3. Get Alice's primary PhoneNumber (for caller ID)
  4. Apply dial plan:
     - Normalize destination to E.164
     - Check user's outbound permissions
     - Select trunk based on LCR (Least Cost Routing)
  5. Set caller ID: From: <sip:+15183217034@...> (Alice's number)
  6. Route to trunk

  Database Schema Considerations

  -- Numbers inventory
  CREATE TABLE phone_numbers (
      id UUID PRIMARY KEY,
      e164 VARCHAR(15) UNIQUE NOT NULL,
      country_code VARCHAR(3),
      npa VARCHAR(3),
      nxx VARCHAR(3),
      line VARCHAR(4),
      state VARCHAR(20) NOT NULL,
      tenant_id UUID NOT NULL,

      -- Regulatory
      e911_address_id UUID REFERENCES e911_addresses(id),
      e911_callback VARCHAR(15),
      cnam VARCHAR(15),

      -- Carrier metadata
      carrier VARCHAR(100),
      lrn VARCHAR(15),
      rate_center VARCHAR(100),

      -- Lifecycle
      acquired_at TIMESTAMP NOT NULL,
      activated_at TIMESTAMP,

      -- Features
      recording_enabled BOOLEAN DEFAULT FALSE,
      block_anonymous BOOLEAN DEFAULT FALSE,

      CONSTRAINT e911_required_when_active CHECK (
          state != 'Active' OR e911_address_id IS NOT NULL
      )
  );

  CREATE INDEX idx_phone_numbers_e164 ON phone_numbers(e164);
  CREATE INDEX idx_phone_numbers_tenant ON phone_numbers(tenant_id);
  CREATE INDEX idx_phone_numbers_state ON phone_numbers(state);

  -- Line assignments
  CREATE TABLE line_bindings (
      id UUID PRIMARY KEY,
      phone_number_id UUID NOT NULL REFERENCES phone_numbers(id) ON DELETE CASCADE,
      owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
      binding_type VARCHAR(50) NOT NULL,
      ring_strategy VARCHAR(50) NOT NULL,
      tenant_id UUID NOT NULL,

      no_answer_timeout_secs INTEGER DEFAULT 30,
      forward_on_busy VARCHAR(50),
      forward_on_no_answer VARCHAR(50),

      created_at TIMESTAMP NOT NULL DEFAULT NOW()
  );

  CREATE INDEX idx_line_bindings_number ON line_bindings(phone_number_id);
  CREATE INDEX idx_line_bindings_user ON line_bindings(owner_user_id);

  -- Device assignments (many-to-many: one number can appear on multiple devices)
  CREATE TABLE device_bindings (
      id UUID PRIMARY KEY,
      line_binding_id UUID NOT NULL REFERENCES line_bindings(id) ON DELETE CASCADE,
      device_id UUID NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
      priority INTEGER NOT NULL DEFAULT 1,
      line_appearance INTEGER NOT NULL DEFAULT 1,
      shared_line BOOLEAN DEFAULT FALSE,
      enabled BOOLEAN DEFAULT TRUE,

      UNIQUE(line_binding_id, device_id)
  );

  -- Extensions for short dialing
  CREATE TABLE extensions (
      id UUID PRIMARY KEY,
      number VARCHAR(10) NOT NULL,
      target_type VARCHAR(50) NOT NULL,
      target_id UUID NOT NULL,
      tenant_id UUID NOT NULL,

      UNIQUE(tenant_id, number)
  );

  API Endpoints Design

  # Phone Number Management
  POST   /api/v1/phone-numbers           # Add to inventory
  GET    /api/v1/phone-numbers           # List inventory
  GET    /api/v1/phone-numbers/:id
  PATCH  /api/v1/phone-numbers/:id       # Update CNAM, E911, state
  DELETE /api/v1/phone-numbers/:id       # Remove from inventory

  # Line Bindings (assignments)
  POST   /api/v1/line-bindings           # Assign number to user/devices
  GET    /api/v1/line-bindings
  GET    /api/v1/line-bindings/:id
  PATCH  /api/v1/line-bindings/:id       # Update ring strategy, forwarding
  DELETE /api/v1/line-bindings/:id       # Unassign number

  # User convenience endpoints
  GET    /api/v1/users/:id/phone-numbers      # List user's numbers
  POST   /api/v1/users/:id/phone-numbers      # Assign number to user
  DELETE /api/v1/users/:id/phone-numbers/:num # Unassign

  # Device convenience endpoints
  GET    /api/v1/devices/:id/lines            # List device's line appearances
  POST   /api/v1/devices/:id/lines            # Add line to device

  # Extensions
  POST   /api/v1/extensions
  GET    /api/v1/extensions
  DELETE /api/v1/extensions/:number

  # Number porting
  POST   /api/v1/phone-numbers/:id/port-in    # Initiate port-in
  POST   /api/v1/phone-numbers/:id/port-out   # Initiate port-out
  GET    /api/v1/phone-numbers/:id/port-status

  Edge Cases to Handle

  1. Shared Line Appearance: Multiple devices show same line, can pick up each other's calls
    - Requires BLF (Busy Lamp Field) notifications
    - Dialog sharing via SUBSCRIBE/NOTIFY
  2. Number Portability: User ports their number from/to FCP
    - Reserve number in "Reserved" state during port window
    - Update LRN after successful port
    - 90-day quarantine after port-out
  3. Emergency Services: E911 address must be validated
    - Block activation if E911 not configured
    - Override E911 per-device for mobile users
    - Send correct callback number to PSAP
  4. Multi-tenant Isolation: Numbers are tenant-scoped
    - Tenant A's +15183217034 != Tenant B's +15183217034
    - Prevent cross-tenant number stealing
  5. Voicemail Integration: Number forwards to user's mailbox
    - LineBinding.owner_user_id determines mailbox owner
    - Forward on no-answer/busy goes to User.voicemail_box
  6. Service Numbers: IVRs, queues don't have "users"
    - LineBinding.binding_type = IVR
    - LineBinding.owner_user_id = NULL
    - Route to IVR service instead of devices
  7. Caller ID Spoofing Prevention: Validate outbound caller ID
    - User can only send calls from their assigned numbers
    - Check LineBinding.owner_user_id matches authenticated user

  Integration with Existing FCP

  Your current User entity in fcp-core already has tenant_id, so you'd add:

  // fcp-core/src/user.rs (additions)
  pub struct User {
      // ... existing fields ...

      /// Primary phone number for outbound caller ID
      pub primary_phone_number_id: Option<PhoneNumberId>,

      /// Voicemail box settings
      pub voicemail_box_id: Option<VoicemailBoxId>,
      pub voicemail_pin: Option<String>,
  }

  Next Steps (When Ready to Implement)

  Phase 1: Data Model (Week 1-2)
  - Add PhoneNumber, LineBinding, Extension entities to fcp-core
  - Update User and Device entities
  - Write migrations
  - Add storage trait methods

  Phase 2: API Layer (Week 3-4)
  - REST endpoints for CRUD operations
  - Validation logic (E911, E.164 format)
  - TUI updates to manage numbers/bindings

  Phase 3: Routing Engine (Week 5-8)
  - Inbound call routing (E.164 → LineBinding → Devices)
  - Extension resolution
  - Ring strategies (simultaneous, sequential)
  - Call forking logic

  Phase 4: Advanced Features (Week 9-12)
  - Shared line appearance (BLF)
  - Call forwarding
  - Voicemail integration
  - Number porting workflow

  Your design is solid. This separation will scale to thousands of users and enable all the
  features you mentioned (shared lines, hunt groups, IVRs, etc.) without architectural pain
  later.

✻ Crunched for 1m 30s
