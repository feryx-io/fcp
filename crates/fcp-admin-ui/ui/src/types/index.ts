// TypeScript mirrors of FCP Rust types

export interface User {
  id: string
  username: string
  display_name: string | null
  tenant_id: string
  enabled: boolean
  role: UserRole
  email: string | null
  phone_number: string | null
  extension: string | null
  department: string | null
  job_title: string | null
  location: string | null
  location_id: string | null
  language: string
  timezone: string
  account_status: AccountStatus
  last_login: string | null
  failed_login_attempts: number
  account_locked_until: string | null
}

export type UserRole =
  | 'super_admin'
  | 'tenant_admin'
  | 'telecom_admin'
  | 'user_admin'
  | 'department_admin'
  | 'read_only'
  | 'end_user'

export type AccountStatus = 'active' | 'suspended' | 'locked' | 'pending' | 'disabled'

export interface CreateUserRequest {
  username: string
  password: string
  display_name?: string
  tenant_id?: string
  role?: string
  email?: string
  phone_number?: string
  extension?: string
  department?: string
  job_title?: string
  location?: string
  location_id?: string
  pin?: string
}

export interface UpdateUserRequest {
  display_name?: string
  role?: string
  email?: string
  phone_number?: string
  extension?: string
  department?: string
  job_title?: string
  location?: string
  location_id?: string
  enabled?: boolean
  account_status?: string
  tenant_id?: string
}

export interface Tenant {
  id: string
  name: string
  description: string | null
  status: TenantStatus
  contact_email: string | null
  contact_phone: string | null
  max_users: number | null
  max_phone_numbers: number | null
  max_concurrent_calls: number | null
  billing_id: string | null
  created_at: string
  updated_at: string
  /** SIP domains the tenant claims (e.g. ["acme.fcp.net", "sip.acme.com"]). */
  sip_domains: string[]
  /**
   * Whether the platform's own trunks may carry this tenant's calls when
   * its own cannot (docs/TENANT_TRUNKS.md §5).
   */
  platform_trunk_fallback: boolean
  /** Country, timezone and number-range overrides (docs/ENTERPRISE_CALLING.md §3.7). */
  calling: TenantCalling
}

export type DestinationClass =
  | 'internal'
  | 'emergency'
  | 'toll_free'
  | 'national'
  | 'international'
  | 'premium'
  | 'unknown'

export interface DestinationOverride {
  /** E.164 with a leading `+`, or the digits as dialled */
  prefix: string
  class: DestinationClass
}

export interface TenantCalling {
  /** ISO 3166-1 alpha-2: US, CA, GB, DE, FR, AU, NZ */
  country?: string | null
  /** IANA; the default for new schedules */
  timezone?: string | null
  destination_overrides?: DestinationOverride[]
  /** How long a hoteling login lasts; unset is 12 */
  hoteling_session_hours?: number | null
}

export interface ClassOfService {
  id: string
  tenant_id: string
  name: string
  allowed: DestinationClass[]
  /** null: no limit */
  max_calls: number | null
  caller_id: 'line' | 'withheld'
  is_default: boolean
  updated_at: number
}

export interface ClassOfServiceRequest {
  name: string
  allowed: DestinationClass[]
  max_calls: number | null
  caller_id: 'line' | 'withheld'
  is_default: boolean
}

export interface ClassifyResponse {
  dialed: string
  class: DestinationClass
  e164: string | null
}

export type TenantStatus = 'active' | 'suspended' | 'trial' | 'disabled'

export interface CreateTenantRequest {
  name: string
  description?: string
  contact_email?: string
  contact_phone?: string
  max_users?: number
  max_phone_numbers?: number
  max_concurrent_calls?: number
  billing_id?: string
  /** SIP domains to claim for this tenant. Empty array = none claimed. */
  sip_domains?: string[]
  /** Leave unset to allow the platform's trunks to carry this tenant's calls. */
  platform_trunk_fallback?: boolean
  /** Replaces the tenant's calling defaults whole. */
  calling?: TenantCalling
}

export interface ExtensionTarget {
  type: 'user' | 'phone_number' | 'device' | 'service'
  id: string
}

export interface Extension {
  id: string
  number: string
  tenant_id: string
  target: ExtensionTarget
  description: string | null
  location_id: string | null
  enabled: boolean
  created_at: string
  updated_at: string
}

export interface CreateExtensionRequest {
  number: string
  target: ExtensionTarget
  tenant_id?: string
  description?: string
  location_id?: string
  enabled?: boolean
}

export interface UpdateExtensionRequest {
  target?: ExtensionTarget
  description?: string
  location_id?: string
  enabled?: boolean
  tenant_id?: string
}

export interface Device {
  id: string
  name: string
  device_type: string
  mac_address: string | null
  model: string | null
  firmware_version: string | null
  tenant_id: string
  owner_user_id: string | null
  /** The username of the device's own credentials (docs/ENTERPRISE_CALLING.md §3.8) */
  sip_username: string | null
  has_sip_credentials: boolean
  /** A shared phone users may log in on */
  hoteling_host: boolean
  sip_uri: string | null
  registration_state: string
  last_registration: string | null
  last_ip_address: string | null
  user_agent: string | null
  max_lines: number
  supports_video: boolean
  supports_blf: boolean
  codec_support: string[]
  location: string | null
  location_id: string | null
  provisioning_url: string | null
  config_template_id: string | null
  has_provisioning_token: boolean
  /// Returned once on create when a token is auto-generated; never populated on list/get.
  provisioning_token?: string
  /** An address for this phone alone, when it sits somewhere its site is not. */
  e911_override: E911Address | null
  /** Whether a location this device sends with a call is believed (`docs/E911.md`). */
  e911_trust_location: boolean | null
  enabled: boolean
  created_at: string
  updated_at: string
}

export interface CreateDeviceRequest {
  name: string
  device_type: string
  tenant_id?: string
  owner_user_id?: string
  mac_address?: string
  model?: string
  sip_uri?: string
  max_lines?: number
  supports_video?: boolean
  supports_blf?: boolean
  location?: string
  location_id?: string
  /// Explicit provisioning token. Empty string on PATCH clears the token;
  /// omitting the field on create auto-generates one when mac_address is set.
  provisioning_token?: string
  /// An address for this phone alone; it arrives unvalidated whatever it says.
  e911_override?: E911AddressInput
  e911_trust_location?: boolean
  /** Credentials of the device's own: both, or neither */
  sip_username?: string
  sip_password?: string
  hoteling_host?: boolean
}

/**
 * A change to a device. Absent fields are left alone; `e911_override`,
 * `e911_trust_location`, `owner_user_id` and `sip_username` set to `null`
 * are cleared. A new `sip_username` needs `sip_password` too.
 */
export interface UpdateDeviceRequest
  extends Partial<Omit<CreateDeviceRequest, 'e911_override' | 'e911_trust_location' | 'owner_user_id' | 'sip_username'>> {
  enabled?: boolean
  firmware_version?: string
  e911_override?: E911AddressInput | null
  e911_trust_location?: boolean | null
  owner_user_id?: string | null
  sip_username?: string | null
}

/** A hoteling guest on a host phone (docs/ENTERPRISE_CALLING.md §3.8) */
export interface HotelingSession {
  id: string
  device_id: string
  device_name: string | null
  user_id: string
  username: string | null
  started_at: string
  expires_at: string
  ended_at: string | null
  /** logout, admin, expired or replaced */
  ended_by: string | null
  active: boolean
}

/** An address on the way in: the lines and, optionally, civic elements. */
export interface E911AddressInput extends CivicElements {
  address1: string
  address2?: string | null
  city: string
  state: string
  postal_code: string
  country: string
}

export interface PhoneNumber {
  id: string
  e164: string
  country_code: string
  npa: string
  nxx: string
  line: string
  state: string
  tenant_id: string
  e911_address: E911Address | null
  e911_callback: string | null
  cnam: string | null
  spam_score: number | null
  location_id: string | null
  /** The site whose phones answer this number (docs/HYBRID_CONTROL_PLANE.md §3.6). */
  site_id?: string | null
  carrier: string | null
  lrn: string | null
  rate_center: string | null
  acquired_at: string
  activated_at: string | null
  recording_enabled: boolean
  block_anonymous: boolean
  tags: string[]
  created_at: string
  updated_at: string
}

export interface E911Address extends CivicElements {
  address1: string
  address2: string | null
  city: string
  state: string
  postal_code: string
  country: string
  /** Read-only: set by a validation route, cleared when the address is edited. */
  validated: boolean
  validated_at: string | null
  validated_by?: string | null
  provider_reference?: string | null
}

/**
 * RFC 5139 civic elements, flattened into the address they belong to
 * (`docs/E911.md`): what an emergency call's PIDF-LO carries.
 */
export interface CivicElements {
  house_number?: string | null
  house_number_suffix?: string | null
  street_predirection?: string | null
  street_name?: string | null
  street_suffix?: string | null
  street_postdirection?: string | null
  additional?: string | null
  floor?: string | null
  unit?: string | null
  room?: string | null
  building?: string | null
  name?: string | null
}

export interface CreatePhoneNumberRequest {
  e164: string
  tenant_id?: string
  e911_address?: E911Address
  e911_callback?: string
  cnam?: string
  carrier?: string
  tags?: string[]
  location_id?: string
  /** The site whose phones answer this number; empty to answer it here. */
  site_id?: string
}

export interface DeviceBinding {
  device_id: string
  priority: number
  line_appearance: number
  shared_line: boolean
  enabled: boolean
}

export interface LineBinding {
  id: string
  phone_number_id: string
  owner_user_id: string | null
  binding_type: string
  devices: DeviceBinding[]
  ring_strategy: string
  tenant_id: string
  outbound_caller_id: string | null
  outbound_cnam: string | null
  flow_id: string | null
  schedule_id: string | null
  closed_target: string | null
  holiday_target: string | null
  /** A shared line: its devices share its appearances (BroadSoft SCA). */
  shared: boolean
  max_appearances: number
  /** A phone on the shared line may barge into an active appearance (§3.11). */
  barge_in: boolean
  created_at: string
  updated_at: string
}

export interface CreateLineBindingRequest {
  phone_number_id: string
  owner_user_id?: string
  binding_type: string
  tenant_id?: string
  devices?: DeviceBinding[]
  ring_strategy?: string
  flow_id?: string
}

export interface UpdateLineBindingRequest {
  devices?: DeviceBinding[]
  ring_strategy?: string
  outbound_caller_id?: string
  outbound_cnam?: string
  flow_id?: string
  schedule_id?: string
  closed_target?: string
  holiday_target?: string
  shared?: boolean
  max_appearances?: number
  barge_in?: boolean
  tenant_id?: string
}

export interface CallDetailRecord {
  call_id: string
  tenant_id: string
  from_uri: string
  to_uri: string
  start_time: string
  end_time: string
  duration_secs: number
  disposition: string
  hangup_cause: string
  hangup_party: string
  media_codec: string | null
}

export interface ActiveCall {
  call_id: string
  from: string
  to: string
  state: string
  start_time: string
  duration_secs: number
  codec: string | null
}

export interface Registration {
  aor: string
  contact: string
  expires_at: { secs_since_epoch: number; nanos_since_epoch: number }
  call_id: string
  cseq: number
  user_agent: string | null
  q: number | null
  registered_at: { secs_since_epoch: number; nanos_since_epoch: number }
  expires_in_seconds: number
}

export interface DashboardSummary {
  active_calls: number
  registered_devices: number
  total_users: number
  total_extensions: number
  total_phone_numbers: number
  calls_today: number
  avg_call_duration_secs: number
}

export interface SystemHealth {
  status: string
  version: string
}

export interface AuthUser {
  username: string
  role: UserRole
  tenant_id: string
  display_name: string | null
}

export interface LoginRequest {
  username: string
  password: string
}

export interface LoginResponse {
  token: string
  user: AuthUser
  password_expired?: boolean
}

export interface ChangeExpiredPasswordRequest {
  username: string
  current_password: string
  new_password: string
}

export interface ApiError {
  error: string
  details?: string
}

// --- Phase 2: Operations & Live Features ---

// Calls (matches media.rs CallInfo)
export interface CallInfo {
  id: string
  state: string
  from_uri: string
  to_uri: string
  from_display_name: string | null
  to_display_name: string | null
  created_at: string
  established_at: string | null
  ended_at: string | null
  duration_seconds: number | null
  media_anchored: boolean
  failure_reason: string | null
}

export interface OriginateCallRequest {
  from: string
  to: string
  from_display_name?: string
  tenant_id?: string
}

// CDRs (matches cdrs.rs CdrResponse)
export interface CdrResponse {
  call_id: string
  tenant_id: string
  sip_call_id: string
  from_uri: string
  from_display_name: string | null
  to_uri: string
  to_display_name: string | null
  start_time: number
  answer_time: number | null
  end_time: number | null
  setup_duration_ms: number | null
  call_duration_ms: number | null
  total_duration_ms: number | null
  disposition: string
  hangup_cause: string
  hangup_party: string | null
  sip_status_code: number | null
  sip_reason: string | null
  media_anchored: boolean
  codec_a_leg: string | null
  codec_b_leg: string | null
  recording_url: string | null
  cost: number | null
  currency: string | null
}

export interface ListCdrsResponse {
  cdrs: CdrResponse[]
  total: number
  limit: number
  offset: number
}

export interface CdrQueryParams {
  tenant_id?: string
  call_id?: string
  from_uri?: string
  to_uri?: string
  disposition?: string
  start_time_from?: string
  start_time_to?: string
  limit?: number
  offset?: number
}

// Queues (matches queues.rs)
export interface QueueResponse {
  name: string
  display_name: string
  extension: string
  strategy: string
  max_wait_time_secs: number
  max_queue_depth: number
  overflow_destination: string | null
  agents_configured: number
  calls_waiting: number
}

export interface QueueStatsResponse {
  queue_name: string
  calls_waiting: number
  agents_available: number
  agents_on_call: number
  agents_total: number
  avg_wait_time_secs: number
  longest_wait_secs: number
  calls_handled_today: number
  calls_abandoned_today: number
  service_level_pct: number
}

export interface AgentResponse {
  id: string
  extension: string
  state: string
  queues: string[]
  calls_handled: number
  priority: number
}

export interface AgentLoginRequest {
  agent_id: string
  extension: string
  skills?: string[]
}

// Queue Configuration (database-backed CRUD)
export interface QueueConfigResponse {
  id: string
  tenant_id: string
  name: string
  extension: string
  strategy: string
  max_wait_time_secs: number
  max_queue_depth: number
  overflow_destination: string | null
  moh_class: string | null
  announce_position: boolean
  announce_interval_secs: number
  retry_on_no_answer: boolean
  agents: string[]
  priority: number
  required_skills: string[]
  record_calls: boolean
  callback_enabled: boolean
  timezone: string
  enabled: boolean
  created_at: string
  updated_at: string
  schedule_id: string | null
  closed_target: string | null
}

export interface CreateQueueConfigRequest {
  name: string
  extension: string
  strategy?: string
  max_wait_time_secs?: number
  max_queue_depth?: number
  overflow_destination?: string
  moh_class?: string
  announce_position?: boolean
  announce_interval_secs?: number
  retry_on_no_answer?: boolean
  agents?: string[]
  priority?: number
  required_skills?: string[]
  record_calls?: boolean
  callback_enabled?: boolean
  timezone?: string
  enabled?: boolean
  schedule_id?: string
  closed_target?: string
}

// Voicemail (matches voicemail.rs)
export interface MailboxResponse {
  id: string
  tenant_id: string
  user_id: string
  line_binding_id: string
  extension: string
  enabled: boolean
  max_message_duration: number
  max_messages: number
  email_notification: boolean
  email_address: string | null
  attach_audio: boolean
  include_transcription: boolean
  transcription_enabled: boolean
  retention_days: number | null
  created_at: number
  updated_at: number
}

export interface VoicemailMessage {
  id: string
  mailbox_id: string
  caller: string
  caller_name: string | null
  status: string
  priority: string
  duration: number
  recording_path: string
  transcription: string | null
  created_at: number
  accessed_at: number | null
}

export interface MailboxStats {
  mailbox_id: string
  total_messages: number
  new_messages: number
  heard_messages: number
  saved_messages: number
  storage_used: number
  avg_duration: number
}

export interface GreetingResponse {
  greeting_type: string // "unavailable" | "busy" | "name" | "temporary"
  audio_path: string
  is_active: boolean
  created_at: number
  expires_at: number | null
}

export interface CreateMailboxRequest {
  tenant_id: string
  user_id: string
  line_binding_id: string
  pin: string
  enabled?: boolean
  max_message_duration?: number
  max_messages?: number
}

export interface UpdateMailboxRequest {
  pin?: string
  enabled?: boolean
  max_message_duration?: number
  max_messages?: number
  email_notification?: boolean
  email_address?: string
  attach_audio?: boolean
  include_transcription?: boolean
  transcription_enabled?: boolean
  retention_days?: number | null
}

// Presence (matches presence.rs)
export interface PresenceResponse {
  aor: string
  state: string
  note: string | null
  watcher_count: number
  extension?: string
  display_name?: string
}

export interface PresenceStats {
  total_presentities: number
  total_watchers: number
}

// Platform Events (matches fcp-events types.rs)
export interface PlatformEvent {
  event_type: string
  [key: string]: unknown
}

export interface CallEvent extends PlatformEvent {
  call_id: string
  timestamp: number
}

export interface QueueEvent extends PlatformEvent {
  queue_name?: string
  call_id?: string
  agent_id?: string
  timestamp: number
}

export interface ToastMessage {
  id: string
  title: string
  description: string
  variant: 'default' | 'success' | 'warning' | 'destructive'
  timestamp: number
}

// --- Phase 3: Advanced Features ---

// SIP Trunks (database-backed CRUD)
export interface SipTrunk {
  id: string
  tenant_id: string
  name: string
  enabled: boolean
  direction: string // "upstream" | "downstream" | "bidirectional"
  priority: number
  weight: number
  hosts: string[]
  /** Where the carrier's calls come from: addresses and CIDR ranges. Empty means the hosts. */
  inbound_sources: string[]
  /** The site this trunk runs at, in a hybrid deployment (docs/HYBRID_CONTROL_PLANE.md §3.6). */
  site_id?: string
  /** Which end of a site's link this trunk is, when it is one; made at enrollment. */
  link?: 'cloud' | 'site'
  transport_protocol: string // "udp" | "tcp" | "tls"
  transport_port: number
  verify_tls: boolean
  has_credentials: boolean
  auth_username: string | null
  auth_realm: string | null
  register: boolean
  register_expiry: number
  max_concurrent_calls: number
  max_failures: number
  recovery_interval_secs: number
  health_check_enabled: boolean
  health_check_interval_secs: number
  codec_prefs: string[]
  caller_id_override: string | null
  from_domain: string | null
  strip_digits: number
  prepend_digits: string | null
  cps_limit: number
  description: string | null
  srtp_mode: string
  dtmf_mode: string
  session_timer_secs: number
  t38_enabled: boolean
  outbound_proxy: string | null
  trunk_group: string | null
  header_rules: string | null
  /**
   * Whether this trunk may carry emergency calls: the tenant's own
   * emergency-capable trunks carry its 911 calls ahead of the platform's
   * (`docs/TENANT_TRUNKS.md` §5).
   */
  emergency_capable: boolean
  health_status: string // "healthy" | "degraded" | "down" | "disabled" | "unknown"
  consecutive_failures: number
  uptime_secs: number | null
  /** What each node that routes by this trunk sees of it; absent without a cluster. */
  nodes?: TrunkNodeView[]
  /** Calls on this trunk across every node that reported. */
  calls_in_flight?: number
  /** Whether any node would route a call to it now. */
  routable_somewhere?: boolean
  created_at: string
  updated_at: string
}

/** One node's view of a trunk (`docs/TENANT_TRUNKS.md` §9). */
export interface TrunkNodeView {
  node_id?: string
  trunk_id: string
  name: string
  tenant_id?: string
  enabled: boolean
  /** "healthy" | "degraded" | "down" | "unknown": this node's own probes. */
  health: string
  consecutive_failures: number
  /** This node's calls on it: capacity is per node. */
  calls_in_flight: number
  routable: boolean
  /** For a registering trunk: "registered" | "pending" | "failed" | "unregistered". */
  registration?: string
  account?: string
}

/** A tenant's least-cost route (`docs/TENANT_TRUNKS.md` §12, phase 6c). */
export interface LcrRoute {
  id: string
  tenant_id: string
  /** Digits, optionally with a leading "+"; the longest match wins. */
  prefix: string
  trunk_id: string
  /** Orders trunks only: what the carrier charges, not what the tenant is charged. */
  cost_per_minute: number
  priority: number
  enabled: boolean
  description: string | null
  created_at: number
  updated_at: number
}

export interface CreateLcrRouteRequest {
  prefix: string
  trunk_id: string
  cost_per_minute: number
  priority?: number
  enabled?: boolean
  description?: string
}

/** One trunk a route preview would offer the call, in order. */
export interface RoutePreviewTrunk {
  trunk_id: string
  name: string
  owner: 'tenant' | 'platform'
  priority: number
  weight: number
  trunk_group: string | null
  health: string
  price?: { prefix: string; cost_per_minute: number; source: 'tenant' | 'node' }
}

export interface RoutePreviewDecision {
  /** "allow" | "deny" | "error" | "no_match" | "not_applied" | "route" */
  decision: string
  rule?: string
  reason?: string
  target?: string
}

/** What a live call manager would do with a call (`GET /v1/routing/preview`). */
export interface RoutePreview {
  node_id: string
  tenant_id: string
  destination: string
  emergency?: string
  policy: RoutePreviewDecision
  rule: RoutePreviewDecision
  outcome: 'routes' | 'refused' | 'unavailable' | 'no_route'
  status?: number
  routes: RoutePreviewTrunk[]
  not_offered: RoutePreviewTrunk[]
}

export interface CreateTrunkRequest {
  name: string
  enabled?: boolean
  direction?: string
  priority?: number
  weight?: number
  hosts?: string[]
  inbound_sources?: string[]
  /** The site this trunk runs at; empty to make it the cloud's own. */
  site_id?: string
  transport_protocol?: string
  transport_port?: number
  verify_tls?: boolean
  auth_username?: string
  auth_password?: string
  auth_realm?: string
  register?: boolean
  register_expiry?: number
  max_concurrent_calls?: number
  max_failures?: number
  recovery_interval_secs?: number
  health_check_enabled?: boolean
  health_check_interval_secs?: number
  codec_prefs?: string[]
  caller_id_override?: string
  from_domain?: string
  strip_digits?: number
  prepend_digits?: string
  cps_limit?: number
  description?: string
  srtp_mode?: string
  dtmf_mode?: string
  session_timer_secs?: number
  t38_enabled?: boolean
  outbound_proxy?: string
  trunk_group?: string
  header_rules?: string
  emergency_capable?: boolean
}

export interface TrunkStatsResponse {
  total_calls_24h: number
  answered_calls_24h: number
  asr: number
  acd_secs: number
  pdd_ms: number
  hourly_volume: number[]
}

// Legacy SBC trunk types (for live status monitoring)
export interface TrunkSummary {
  name: string
  enabled: boolean
  admin_disabled: boolean
  health_status: string // "healthy" | "degraded" | "down" | "unknown"
  direction: string // "upstream" | "downstream" | "bidirectional"
  concurrent_calls: number
  max_concurrent_calls: number
  priority: number
  weight: number
  registration_state: string | null
}

export interface TrunkDetail extends TrunkSummary {
  hosts: string[]
  transport_protocol: string
  transport_port: number | null
  consecutive_failures: number
  has_credentials: boolean
  register: boolean
  registration_error: string | null
}

// Routing Rules (database-backed, matches fcp-api-core routing.rs)
export interface RoutingRule {
  id: string
  tenant_id: string
  name: string
  description: string | null
  priority: number
  enabled: boolean
  destination_pattern: string | null
  source_pattern: string | null
  source_ip_ranges: string[]
  time_window_days: string[]
  time_window_start: string | null
  time_window_end: string | null
  time_window_timezone: string
  action: string // "allow" | "deny" | "route_to_trunk"
  target_trunk_id: string | null
  target_trunk_group: string | null
  reason: string | null
  created_at: string
  updated_at: string
}

export interface CreateRoutingRuleRequest {
  name: string
  description?: string
  priority?: number
  enabled?: boolean
  destination_pattern?: string
  source_pattern?: string
  source_ip_ranges?: string[]
  time_window_days?: string[]
  time_window_start?: string
  time_window_end?: string
  time_window_timezone?: string
  action?: string
  target_trunk_id?: string
  target_trunk_group?: string
  reason?: string
}

// Legacy Routing / Policy Rules (matches fcp-routing policy.rs)
export interface PolicyRule {
  id: string
  name: string
  tenant_id: string
  priority: number
  enabled: boolean
  applies_to: AppliesTo
  destination_match: DestinationMatch | null
  time_windows: TimeWindow[]
  source_ip_ranges: string[]
  action: 'allow' | 'deny'
  reason: string | null
}

export interface AppliesTo {
  all_users: boolean
  users: string[] | null
  roles: string[] | null
  groups: string[] | null
}

export interface DestinationMatch {
  pattern: string
  tags: string[] | null
}

export interface TimeWindow {
  days: string[]
  start_time: string
  end_time: string
  timezone: string
}

export interface CreatePolicyRuleRequest {
  name: string
  priority?: number
  enabled?: boolean
  applies_to?: Partial<AppliesTo>
  destination_match?: DestinationMatch
  time_windows?: TimeWindow[]
  source_ip_ranges?: string[]
  action: 'allow' | 'deny'
  reason?: string
}

// IVR Flows (matches fcp-api-core flows.rs)
export interface FlowResponse {
  id: string
  name: string
  description: string | null
  version: number
  initial_node: string
  nodes: Record<string, FlowNode>
  variables: Record<string, unknown>
  tenant_id: string | null
  enabled: boolean
  webhook_url: string | null
  created_at: string | null
  updated_at: string | null
}

export interface FlowNode {
  type: string
  label?: string
  [key: string]: unknown
}

export interface CreateFlowRequest {
  id: string
  name: string
  description?: string
  initial_node: string
  nodes: Record<string, FlowNode>
  variables?: Record<string, unknown>
  tenant_id?: string
  enabled?: boolean
  webhook_url?: string
}

export interface UpdateFlowRequest {
  name?: string
  description?: string
  initial_node?: string
  nodes?: Record<string, FlowNode>
  variables?: Record<string, unknown>
  enabled?: boolean
  webhook_url?: string
}

export interface ListFlowsResponse {
  flows: FlowResponse[]
  total: number
  limit: number
  offset: number
}

export interface ListFlowsQuery {
  tenant_id?: string
  enabled?: boolean
  name?: string
  limit?: number
  offset?: number
}

// Cluster (matches cluster health endpoints)
export interface ClusterNode {
  node_id: string
  address: string
  status: string // "healthy" | "degraded" | "down"
  uptime_seconds: number
  active_calls: number
  last_heartbeat: string
  version: string
}

export interface SystemStatus {
  status: string
  version: string
  uptime_seconds: number
  users_count: number
  active_registrations_count: number
}

// Cluster node monitoring (matches fcp-cluster reader types)
export interface ClusterNodeInfo {
  node_id: string
  node_type: string
  http_address: string
  sip_address: string
  capabilities: string[]
  active_calls: number
  started_at: number
  version: string
  status: 'healthy' | 'degraded' | 'offline'
  uptime_seconds: number
  draining: boolean
  region?: string
  /** Video capacity per device, advertised by conference nodes (work units). */
  video_budget?: VideoDeviceBudget[]
  /** What the node will carry and what it is carrying, when it keeps limits. */
  capacity?: NodeCapacity
  /** Calls it will still take; absent when it advertises no limit. */
  calls_left?: number
  /**
   * The node's own view of the cluster, when it does not look whole
   * (docs/MULTI_REGION_FOLLOWUPS.md §2.3). Absent while whole.
   */
  view?: ClusterView
}

/** Whether a node's view of the cluster can be trusted. Said, never acted on. */
export interface ClusterView {
  state: 'whole' | 'store_unreachable' | 'remote_regions_lost'
  /** When this state began (Unix seconds). */
  since?: number
  /** The regions whose every node went at once. */
  lost_regions?: string[]
  /** Seconds since the node's discovery last worked. */
  view_age_secs?: number
  /** Whether the node still reaches its database, when it can tell. */
  database_reachable?: boolean
  summary: string
}

/** A node's admission limits and what it is using of them. */
export interface NodeCapacity {
  max_concurrent_calls: number
  max_cps: number
  max_concurrent_registrations: number
  registrations: number
  cps: number
}

export interface VideoDeviceBudget {
  device: string
  /** SDP names of the codecs the device encodes. */
  codecs: string[]
  /** On a device with engines, the tightest engine's. */
  left: number
  total: number
  /** A GPU's engines, each a budget of its own; absent on the host. */
  engines?: {
    encode: { left: number; total: number }
    decode: { left: number; total: number }
    compose: { left: number; total: number }
  }
}

/** A tenant's site on its own network, as the cloud keeps it (docs/HYBRID_CONTROL_PLANE.md). */
export interface SiteInfo {
  id: string
  tenant_id: string
  name: string
  status: 'pending' | 'enrolled' | 'disabled'
  created_at: number
  updated_at: number
  enrolled_at?: number
  certificate_expires_at?: number
  last_seen_at?: number
  link_up: boolean
  link_version?: string
  /** Release → node ids, from the site's last heartbeat. */
  versions: Record<string, string[]>
  nodes: { node_id: string; node_type: string; version: string; healthy: boolean }[]
  /** The journal sequence the site has applied through; absent from a link without a site database. */
  sync_cursor?: number
  sync_applied_at?: number
  sync_error?: string
  /** Journal entries the site has not applied yet. */
  sync_behind?: number
  /** The site's link trunk, made at enrollment when the cloud names a SIP address for sites. */
  trunk_id?: string
  /** Who is registered at the site, from its last heartbeat (§3.3). */
  registrations?: SiteRegistrations
}

export interface SiteRegistrations {
  users: { aor: string; contacts: number; since?: number }[]
  total_users: number
  total_contacts: number
}

/** A site with the one-time enrollment token the cloud shows once. */
export interface SiteWithToken extends SiteInfo {
  enrollment_token: string
  enrollment_token_expires_at: number
}

export interface ClusterSitesResponse {
  available: boolean
  message?: string
  sites: SiteInfo[]
}

export interface ClusterNodesResponse {
  available: boolean
  message?: string
  nodes: ClusterNodeInfo[]
}

export interface ClusterStatsResponse {
  available: boolean
  message?: string
  stats?: {
    total_nodes: number
    healthy_nodes: number
    degraded_nodes: number
    offline_nodes: number
    total_active_calls: number
    nodes_by_type: Record<string, number>
    nodes_by_region: Record<string, number>
    nodes_without_region: number
    /** Nodes whose own view of the cluster does not look whole. */
    nodes_with_partial_view?: number
    /** Release → the node ids on it (docs/BACKUP_AND_RESTORE.md §5). */
    versions?: Record<string, string[]>
    /** More than one release is running: an upgrade under way, or one that stopped half way. */
    version_skew?: boolean
  }
}

// Number Porting (matches fcp-core porting.rs)
export type PortStatus = 'draft' | 'submitted' | 'pending' | 'approved' | 'scheduled'
  | 'in_progress' | 'completed' | 'rejected' | 'cancelled' | 'failed'

export interface PortRequest {
  id: string
  tenant_id: string
  phone_numbers: string[]
  port_type: string
  status: PortStatus
  rejection_reason: string | null
  customer_name: string
  account_number: string
  pin: string | null
  service_address: PortAddress
  authorized_contact_name: string
  authorized_contact_phone: string
  authorized_contact_email: string
  losing_carrier: string
  winning_carrier: string
  requested_foc_date: string | null
  actual_foc_date: string | null
  submitted_at: string | null
  completed_at: string | null
  notes: string | null
  created_at: string
  updated_at: string
}

export interface PortAddress {
  street: string
  city: string
  state: string
  postal_code: string
  country: string
}

export interface CreatePortRequest {
  phone_numbers: string[]
  port_type?: string
  customer_name: string
  account_number: string
  pin?: string
  service_address: PortAddress
  authorized_contact_name: string
  authorized_contact_phone: string
  authorized_contact_email: string
  losing_carrier: string
  winning_carrier?: string
  requested_foc_date?: string
  notes?: string
}

// Audit Log
/** One audited admin request (`GET /audit-log`). */
export interface AuditEntry {
  id: number
  tenant_id: string | null
  /** A username, an API token's id, `static_token` or `anonymous`. */
  actor: string
  actor_kind: 'session' | 'api_token' | 'static_token' | 'anonymous' | string
  method: string
  path: string
  status: number
  /** Method and route template, e.g. `DELETE /api/v1/users/:username`. */
  action: string
  /** A deletion's route parameters, `{ params: { … } }`. */
  detail: Record<string, unknown> | null
  source_ip: string | null
  /** Unix seconds. */
  occurred_at: number
}

export interface AuditQueryParams {
  actor?: string
  /** Route-template prefix, e.g. `DELETE /api/v1/users`. */
  action?: string
  /** Unix seconds. */
  since?: number
  limit?: number
}

/** One entry of the deletion log (`GET /compliance/deletions`). */
export interface DeletionEntry {
  id: number
  tenant_id: string | null
  class: string
  /** The item's id, or a time range for a batch. */
  item: string
  detail: Record<string, unknown> | null
  cause: 'retention' | 'manual' | 'node_sweep' | 'upkeep' | 'cascade' | 'unattributed' | string
  actor: string
  /** Unix seconds. */
  deleted_at: number
}

export interface DeletionQueryParams {
  class?: string
  since?: number
  limit?: number
}

// Conference Recordings
export interface ConferenceRecording {
  id: string
  tenant_id: string
  meeting_id: string | null
  room_number: string
  node_id: string
  file_name: string
  /** `wav` (mixed audio), `webm` or `mp4` (composite video and audio). */
  format: string
  /** `auto`, `api` or `dtmf`. */
  trigger: string
  /** `not_required`, `announced` or `failed`. */
  consent: string
  /** Unix seconds. */
  started_at: number
  ended_at: number | null
  duration_secs: number | null
  size_bytes: number | null
  participants: number
  running: boolean
  download_url: string
}

export interface RecordingQueryParams {
  room_number?: string
  meeting_id?: string
  /** Unix seconds. */
  since?: number
  until?: number
  running?: boolean
  limit?: number
  offset?: number
}

export interface ListRecordingsResponse {
  recordings: ConferenceRecording[]
  total: number
  limit: number
  offset: number
}

// API Tokens
export interface ApiToken {
  id: string
  name: string
  prefix: string
  user_id: string
  tenant_id: string
  permissions: string[]
  expires_at: string | null
  created_at: string
  last_used_at: string | null
}

export interface CreateApiTokenRequest {
  name: string
  permissions?: string[]
  expires_at?: string
}

export interface CreateApiTokenResponse {
  token: ApiToken
  secret: string // Only returned once
}

// --- Self-service (end-user portal) types ---

export interface MeProfile {
  username: string
  display_name: string | null
  email: string | null
  phone_number: string | null
  extension: string | null
  department: string | null
  job_title: string | null
  location: string | null
  language: string
  timezone: string
  role: string
  has_profile_picture: boolean
}

export interface UpdateProfileRequest {
  display_name?: string
  email?: string
  phone_number?: string
  timezone?: string
  language?: string
}

export interface ChangePasswordRequest {
  current_password: string
  new_password: string
}

export interface MeMailbox {
  id: string
  extension: string
  enabled: boolean
  max_message_duration: number
  max_messages: number
  email_notification: boolean
  email_address: string | null
  attach_audio: boolean
  retention_days: number | null
}

export interface MeMailboxStats {
  mailbox_id: string
  total_messages: number
  new_messages: number
  heard_messages: number
  saved_messages: number
}

/** Find-me / follow-me (docs/ENTERPRISE_CALLING.md §3.6): targets are `devices`, `ext:…`, `number:+…`, `user:…` or a SIP URI. */
export interface FindMeStep {
  targets: string[]
  ring_secs: number
  confirm: boolean
}

export interface FindMe {
  enabled: boolean
  steps: FindMeStep[]
}

/** A user's calling settings (docs/ENTERPRISE_CALLING.md §3.1); targets as `voicemail`, `number:+1…`, `ext:2001`, `user:<id>` or a SIP URI. */
export interface CallingSettings {
  user_id: string
  username: string
  dnd: boolean
  call_waiting: boolean
  forward_all: string | null
  forward_busy: string | null
  forward_no_answer: string | null
  forward_unreachable: string | null
  after_hours_schedule_id: string | null
  after_hours_forward: string | null
  find_me: FindMe | null
  /** A prompt's id, `none` for silence, null for the tenant's hold music */
  hold_music: string | null
  /** null: the node's default ring time */
  no_answer_secs: number | null
  /** null: the tenant's default class (§3.7) */
  class_of_service_id: string | null
  /** The user as an executive, with assistants (§3.10) */
  executive: Executive | null
  updated_at: number
}

export type ExecutiveFilter = 'all' | 'external' | 'none'
export type ExecutiveAlert = 'assistants' | 'assistants_and_executive'

export interface Executive {
  enabled: boolean
  filter: ExecutiveFilter
  alert: ExecutiveAlert
  /** In order; `opted_in` is the assistant's own choice. */
  assistants: { user_id: string; opted_in: boolean }[]
}

/** An executive the signed-in user assists (§3.10). */
export interface MyExecutive {
  user_id: string
  username: string
  display_name: string | null
  opted_in: boolean
  enabled: boolean
  filter: ExecutiveFilter
}

/** A forward given as an empty string is cleared; one left out is kept. */
export interface UpdateCallingRequest {
  dnd?: boolean
  call_waiting?: boolean
  forward_all?: string
  forward_busy?: string
  forward_no_answer?: string
  forward_unreachable?: string
  /** A schedule's id, or '' to clear */
  after_hours_schedule_id?: string
  after_hours_forward?: string
  find_me?: FindMe
  /** A prompt's id, `none`, or '' for the tenant's hold music */
  hold_music?: string
  /** 0 clears it: the node's default */
  no_answer_secs?: number
  /** A class's id, or '' for the tenant's default. Admins only. */
  class_of_service_id?: string
  /** `assistants` (usernames or ids, in order) is an admin's to set. */
  executive?: {
    enabled?: boolean
    filter?: ExecutiveFilter
    alert?: ExecutiveAlert
    assistants?: string[]
  }
}

// Locations
export interface Location {
  id: string
  tenant_id: string
  name: string
  description: string | null
  address_line1: string | null
  address_line2: string | null
  city: string | null
  state: string | null
  postal_code: string | null
  country: string | null
  floor: string | null
  building: string | null
  latitude: number | null
  longitude: number | null
  phone_number: string | null
  timezone: string | null
  emergency_contact_name: string | null
  emergency_contact_phone: string | null
  /** The ELIN the carrier maps to this address; the caller ID on emergency calls from here. */
  e911_callback_number: string | null
  /** The street in RFC 5139 civic elements, once a validator has structured it. */
  civic?: CivicElements
  e911_validated: boolean
  e911_validated_at: string | null
  e911_validated_by: string | null
  e911_validation_reference: string | null
  created_at: string
  updated_at: string
}

export interface CreateLocationRequest {
  name: string
  tenant_id?: string
  description?: string
  address_line1?: string
  address_line2?: string
  city?: string
  state?: string
  postal_code?: string
  country?: string
  floor?: string
  building?: string
  latitude?: number
  longitude?: number
  phone_number?: string
  timezone?: string
  emergency_contact_name?: string
  emergency_contact_phone?: string
  e911_callback_number?: string
}

// SBC Quality Metrics
export interface QualityMetrics {
  trunk_name: string
  asr: number // Answer-Seizure Ratio
  acd: number // Average Call Duration
  pdd: number // Post-Dial Delay (ms)
  ner: number // Network Effectiveness Ratio
  total_calls: number
  failed_calls: number
}

// Conference / Meeting (matches conferences.rs)
export interface MeetingResponse {
  id: string
  tenant_id: string
  room_number: string
  name: string
  guest_pin: string | null
  host_pin: string | null
  require_guest_pin: boolean
  max_participants: number
  wait_for_moderator: boolean
  enabled: boolean
  auto_record: boolean
  default_locked: boolean
  permanent: boolean
  owner_id: string | null
  pin_prompt_sound?: string | null
  invalid_pin_sound?: string | null
  moh_sound?: string | null
  pin_prompt_tts?: string | null
  invalid_pin_tts?: string | null
  welcome_tts?: string | null
  play_entry_tone: boolean
  play_exit_tone: boolean
  join_sound?: string | null
  exit_sound?: string | null
  video_enabled: boolean
  video_layout: string
  video_max_participants: number
  video_max_resolution: string
  video_fps: number
  video_exclude_self: boolean
  /** `auto`, `on` or `off`: when a viewer is sent one sender's own stream. */
  video_passthrough: string
  /** Who may share a screen: `off`, `hosts` or `anyone`. */
  video_content_share: string
  /** How the composite shows a shared screen: `presentation` or `content_only`. */
  video_content_layout: string
  video_codecs: string[]
  created_at: string
  updated_at: string
}

export interface CreateMeetingRequest {
  room_number: string
  name: string
  guest_pin?: string | null
  host_pin?: string | null
  require_guest_pin?: boolean
  max_participants?: number
  wait_for_moderator?: boolean
  enabled?: boolean
  auto_record?: boolean
  default_locked?: boolean
  permanent?: boolean
  owner_id?: string | null
  pin_prompt_sound?: string | null
  invalid_pin_sound?: string | null
  moh_sound?: string | null
  pin_prompt_tts?: string | null
  invalid_pin_tts?: string | null
  welcome_tts?: string | null
  play_entry_tone?: boolean
  play_exit_tone?: boolean
  join_sound?: string | null
  exit_sound?: string | null
  video_enabled?: boolean
  video_layout?: string
  video_max_participants?: number
  video_max_resolution?: string
  video_fps?: number
  video_exclude_self?: boolean
  video_passthrough?: string
  video_content_share?: string
  video_content_layout?: string
  video_codecs?: string[]
}

export interface UpdateMeetingRequest {
  name?: string
  guest_pin?: string | null
  host_pin?: string | null
  require_guest_pin?: boolean
  max_participants?: number
  wait_for_moderator?: boolean
  enabled?: boolean
  auto_record?: boolean
  default_locked?: boolean
  permanent?: boolean
  owner_id?: string | null
  pin_prompt_sound?: string | null
  invalid_pin_sound?: string | null
  moh_sound?: string | null
  pin_prompt_tts?: string | null
  invalid_pin_tts?: string | null
  welcome_tts?: string | null
  play_entry_tone?: boolean
  play_exit_tone?: boolean
  join_sound?: string | null
  exit_sound?: string | null
  video_enabled?: boolean
  video_layout?: string
  video_max_participants?: number
  video_max_resolution?: string
  video_fps?: number
  video_exclude_self?: boolean
  video_passthrough?: string
  video_content_share?: string
  video_content_layout?: string
  video_codecs?: string[]
}

// ── Emergency calling (E911), docs/E911.md ───────────────────────────────

/** How an on-site notification reaches someone. */
export type ContactKind = 'email' | 'sms' | 'call' | 'webhook'

export interface NotifyContact {
  kind: ContactKind
  /** An address, an E.164 number, an extension or SIP URI, or a webhook endpoint id. */
  target: string
  name?: string | null
  /** What keeps this contact from being delivered to; empty when it can be. */
  issues?: string[]
}

export interface EmergencyConfig {
  tenant_id: string
  emergency_numbers: string[]
  test_numbers: string[]
  dial_prefixes: string[]
  contacts: NotifyContact[]
  /** The same contacts as the plain strings the event has always carried. */
  notify_contacts: string[]
  require_dispatchable_location: boolean
  trust_device_locations: string[]
  declared_location_ttl_secs: number
  /** False while the tenant has never saved a configuration (defaults shown). */
  configured: boolean
  updated_at: number
}

export interface UpdateEmergencyConfigRequest {
  emergency_numbers?: string[]
  test_numbers?: string[]
  dial_prefixes?: string[]
  contacts?: Array<{ kind: ContactKind; target: string; name?: string }>
  require_dispatchable_location?: boolean
  trust_device_locations?: string[]
  declared_location_ttl_secs?: number
}

export interface DispatchableLocation {
  address: E911Address
  callback_number: string | null
  /** `device_override`, `device_supplied`, `network`, `device_location`, `user_location`, `declared`, `phone_number`. */
  source: string
  location_id: string | null
  location_name: string | null
  summary: string
  client_address: string | null
  declared_age_secs: number | null
}

export interface UserReadiness {
  user_id: string
  username: string
  ready: boolean
  location: DispatchableLocation | null
  /** Whether every device of this user's moves, so where it was provisioned may not be where it is. */
  nomadic: boolean
  issues: string[]
}

export interface ContactReadiness {
  kind: ContactKind
  target: string
  name: string | null
  issues: string[]
  last_delivery_at: number | null
  last_test_at: number | null
}

export interface EmergencyReadiness {
  summary: {
    users: number
    ready: number
    missing_location: number
    missing_callback: number
    unvalidated_address: number
    unplaced_nomadic: number
  }
  notifications: {
    contacts: ContactReadiness[]
    deliverable: number
    issues: string[]
  }
  users: UserReadiness[]
}

export interface EmergencyNetwork {
  id: string
  tenant_id: string
  cidr: string
  location_id: string
  name: string | null
  priority: number
  /** `nomadic` (softphones, mobiles, browsers) or `all`. */
  applies_to: string
  created_at: number
  updated_at: number
}

export interface CreateEmergencyNetworkRequest {
  cidr: string
  location_id: string
  name?: string
  priority?: number
  applies_to?: string
}

export interface EmergencyCall {
  id: number
  tenant_id: string
  call_id: string
  sip_call_id: string | null
  caller_uri: string
  caller_username: string | null
  dialed_number: string
  /** `live` or `test`. */
  kind: string
  callback_number: string | null
  location: DispatchableLocation | null
  trunk_name: string | null
  outcome: string
  placed_at: number
  notified: boolean
  sip_status: number | null
  answered_at: number | null
  ended_at: number | null
  duration_secs: number | null
  /** How the location travelled: `elin`, `pidf_lo`, `elin+pidf_lo`. */
  delivery: string | null
  location_structured: boolean | null
  geolocation_error: string | null
}

export interface EmergencyNotification {
  id: number
  call_id: string
  call_record_id: number | null
  kind: ContactKind
  target: string
  name: string | null
  attempt: number
  /** `delivered`, `retrying` or `failed`. */
  status: string
  error: string | null
  delivered_at: number | null
  next_retry_at: number | null
  test: boolean
  created_at: number
}

export interface AddressValidation {
  valid: boolean
  normalized: E911Address
  issues: string[]
  /** What the validator could not do on an otherwise good address. */
  notes: string[]
  validator: string
  provider_reference: string | null
}

export interface LocationValidationResponse {
  validation: AddressValidation
  location_id: string
  name: string
  validated: boolean
  validated_at: number | null
}

export interface DeviceValidationResponse {
  validation: AddressValidation
  device_id: string
  name: string
  validated: boolean
  validated_at: number | null
}

export interface NumberValidationResponse {
  validation: AddressValidation
  phone_number_id: string
  e164: string
  validated: boolean
  validated_at: number | null
}

// Schedules (docs/ENTERPRISE_CALLING.md §3.5)
export type Weekday = 'Mon' | 'Tue' | 'Wed' | 'Thu' | 'Fri' | 'Sat' | 'Sun'

export interface WeeklyWindow {
  days: Weekday[]
  /** HH:MM; an end equal to the start is the whole day, before it runs past midnight */
  start: string
  end: string
}

export interface Holiday {
  name: string
  from: string
  to?: string | null
}

export interface Schedule {
  id: string
  tenant_id: string
  name: string
  timezone: string
  weekly: WeeklyWindow[]
  holidays: Holiday[]
  state: 'open' | 'closed' | 'holiday'
  updated_at: number
}

export interface ScheduleRequest {
  name: string
  timezone: string
  weekly: WeeklyWindow[]
  holidays: Holiday[]
}
