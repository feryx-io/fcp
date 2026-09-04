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

export interface E911Address {
  address1: string
  address2: string | null
  city: string
  state: string
  postal_code: string
  country: string
  validated: boolean
  validated_at: string | null
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
  no_answer_timeout_secs: number
  forward_on_busy: string | null
  forward_on_no_answer: string | null
  forward_unconditional: string | null
  outbound_caller_id: string | null
  outbound_cnam: string | null
  flow_id: string | null
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
  no_answer_timeout_secs?: number
  forward_on_busy?: string
  forward_on_no_answer?: string
  forward_unconditional?: string
  flow_id?: string
}

export interface UpdateLineBindingRequest {
  devices?: DeviceBinding[]
  ring_strategy?: string
  no_answer_timeout_secs?: number
  forward_on_busy?: string
  forward_on_no_answer?: string
  forward_unconditional?: string
  outbound_caller_id?: string
  outbound_cnam?: string
  flow_id?: string
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
  health_status: string // "healthy" | "degraded" | "down" | "disabled" | "unknown"
  consecutive_failures: number
  uptime_secs: number | null
  created_at: string
  updated_at: string
}

export interface CreateTrunkRequest {
  name: string
  enabled?: boolean
  direction?: string
  priority?: number
  weight?: number
  hosts?: string[]
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
export interface AuditEntry {
  id: string
  tenant_id: string
  actor_id: string
  actor_type: string
  action: string
  resource_type: string
  resource_id: string
  changes: Record<string, unknown> | null
  ip_address: string | null
  timestamp: string
}

export interface AuditQueryParams {
  actor_id?: string
  resource_type?: string
  action?: string
  start_time?: string
  end_time?: string
  limit?: number
  offset?: number
}

export interface ListAuditResponse {
  entries: AuditEntry[]
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

export interface MeForwarding {
  id: string
  phone_number_id: string
  phone_number: string | null
  forward_unconditional: string | null
  forward_on_busy: string | null
  forward_on_no_answer: string | null
  no_answer_timeout_secs: number
}

export interface UpdateForwardingRequest {
  forward_unconditional?: string
  forward_on_busy?: string
  forward_on_no_answer?: string
  no_answer_timeout_secs?: number
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
}
