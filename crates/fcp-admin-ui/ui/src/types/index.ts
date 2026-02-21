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
  language: string
  timezone: string
  account_status: AccountStatus
  last_login: string | null
}

export type UserRole = 'super_admin' | 'telecom_admin' | 'tenant_admin' | 'operator' | 'user' | 'api_service'

export type AccountStatus = 'active' | 'suspended' | 'pending' | 'disabled'

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
  enabled?: boolean
  account_status?: string
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
}

export interface Extension {
  id: string
  number: string
  tenant_id: string
  display_name: string | null
  user_id: string | null
  type: string
  enabled: boolean
  created_at: string
}

export interface CreateExtensionRequest {
  number: string
  tenant_id?: string
  display_name?: string
  user_id?: string
  type?: string
}

export interface Device {
  id: string
  mac_address: string
  tenant_id: string
  vendor: string
  model: string
  name: string | null
  firmware_version: string | null
  provisioning_template: string | null
  status: string
  last_seen: string | null
  created_at: string
}

export interface CreateDeviceRequest {
  mac_address: string
  tenant_id?: string
  vendor: string
  model: string
  name?: string
  firmware_version?: string
  provisioning_template?: string
}

export interface PhoneNumber {
  id: string
  number: string
  tenant_id: string | null
  type: string
  status: string
  display_name: string | null
  assigned_to: string | null
  e911_address: E911Address | null
  created_at: string
}

export interface E911Address {
  street: string
  city: string
  state: string
  zip: string
  country: string
}

export interface CreatePhoneNumberRequest {
  number: string
  tenant_id?: string
  type?: string
  display_name?: string
}

export interface LineBinding {
  id: string
  tenant_id: string
  extension_id: string
  device_id: string | null
  phone_number_id: string | null
  line_index: number
  label: string | null
  created_at: string
}

export interface CreateLineBindingRequest {
  tenant_id?: string
  extension_id: string
  device_id?: string
  phone_number_id?: string
  line_index?: number
  label?: string
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
  contacts: RegistrationContact[]
}

export interface RegistrationContact {
  uri: string
  expires: number
  q: number
  user_agent: string | null
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
  uptime_secs: number
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

// Presence (matches presence.rs)
export interface PresenceResponse {
  aor: string
  state: string
  note: string | null
  watcher_count: number
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
