import type { components } from '@/api/schema'

/** The API's schemas, generated from docs/openapi.json (`npm run gen:api`). */
type S = components['schemas']

// TypeScript mirrors of FCP Rust types

export type User = S['UserResponse']

/** What purging a deleted user removed (`docs/USERS.md` §4). */
export type PurgeResponse = S['PurgeResponse']

export type UserRole =
  | 'super_admin'
  | 'tenant_admin'
  | 'telecom_admin'
  | 'user_admin'
  | 'department_admin'
  | 'read_only'
  | 'end_user'

export type AccountStatus = 'active' | 'suspended' | 'locked' | 'pending' | 'disabled'

export type CreateUserRequest = S['CreateUserRequest']

export type UpdateUserRequest = S['UpdateUserRequest']

export type Tenant = S['TenantResponse']

export type DestinationClass =
  | 'internal'
  | 'emergency'
  | 'toll_free'
  | 'national'
  | 'international'
  | 'premium'
  | 'unknown'

export type DestinationOverride = S['DestinationOverride']

export type TenantCalling = S['TenantCalling']

export type ClassOfService = S['ClassOfServiceResponse']

export type ClassOfServiceRequest = S['ClassOfServiceRequest']

export type ClassifyResponse = S['ClassifyResponse']

export type TenantStatus = 'active' | 'suspended' | 'trial' | 'disabled'

export type CreateTenantRequest = S['CreateTenantRequest']

export type ExtensionTarget = S['ExtensionTargetResponse']

export type Extension = S['ExtensionResponse']

export type CreateExtensionRequest = S['CreateExtensionRequest']

export type UpdateExtensionRequest = S['UpdateExtensionRequest']

export type Device = S['DeviceResponse']

export type CreateDeviceRequest = S['CreateDeviceRequest']

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
export type HotelingSession = S['SessionResponse']

/** An address on the way in: the lines and, optionally, civic elements. */
export interface E911AddressInput extends CivicElements {
  address1: string
  address2?: string | null
  city: string
  state: string
  postal_code: string
  country: string
}

export type PhoneNumber = S['PhoneNumberResponse']

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
export type CivicElements = S['CivicElements']

export type CreatePhoneNumberRequest = S['CreatePhoneNumberRequest']

export type DeviceBinding = S['DeviceBindingResponse']

export type LineBinding = S['LineBindingResponse']

export type CreateLineBindingRequest = S['CreateLineBindingRequest']

export type UpdateLineBindingRequest = S['UpdateLineBindingRequest']

export type CallDetailRecord = S['CdrResponse']

export type ActiveCall = S['CallInfo']

export type Registration = S['RegistrationResponse']
export type SpeechSettings = S['SpeechSettingsResponse']
export type SpeechSettingsRequest = S['SpeechSettingsRequest']
export type SpeechProvider = S['SpeechProvider']
export type SpeechVoice = S['SpeechVoice']
export type SpeechAccount = S['SpeechAccountView']
export type SpeechAccountRequest = S['SpeechAccountRequest']
export type SpeechAccountKind = S['SpeechAccountKind']
export type PlatformSpeech = S['PlatformSpeech']
export type SpeechUsagePeriod = S['SpeechUsagePeriod']
export type SpeechCap = S['SpeechCapView']
export type PlatformSpeechCap = S['PlatformSpeechCap']
export type CapUnit = S['CapUnit']
export type LogLine = S['LogLine']
export type LogLevel = S['LogLevel']
export type PersonPresence = S['PersonPresenceResponse']

export type DashboardSummary = S['DashboardSummary']

export interface SystemHealth {
  status: string
  version: string
}

export type AuthUser = S['AuthUserResponse']

export type LoginRequest = S['LoginRequest']

export type LoginResponse = S['LoginResponse']
export type DiscoverResponse = S['DiscoverResponse']
export type TotpEnrollment = S['TotpEnrollment']
export type Brand = S['BrandResponse']
export type BrandRequest = S['BrandRequest']
export type BrandAsset = S['BrandAssetResponse']
export type BrandColors = S['BrandColors']
export type IdentityProvider = S['IdentityProviderResponse']
export type IdentityProviderRequest = S['IdentityProviderRequest']
export type ProviderSettings = S['ProviderSettings']
export type ProviderCheck = S['ProviderCheck']
export type UserIdentity = S['UserIdentityResponse']
export type TotpStatus = S['TotpStatus']
export type RecoveryCodes = S['RecoveryCodes']

export type ChangeExpiredPasswordRequest = S['ChangeExpiredPasswordRequest']

export interface ApiError {
  error: string
  details?: string
}

// --- Phase 2: Operations & Live Features ---

// Calls (matches media.rs CallInfo)
export type CallInfo = S['CallInfo']

// CDRs (matches cdrs.rs CdrResponse)
export type CdrResponse = S['CdrResponse']

/** One phone's report of a call's quality (docs/CALL_DIAGNOSTICS.md §2). */
export type CallQualityResponse = S['CallQualityResponse']

/** A page of CDRs: the list envelope; the total may be an estimate. */
export type ListCdrsResponse = Page<CdrResponse> & { total: number; total_exact: boolean }

export interface CdrQueryParams {
  tenant_id?: string
  call_id?: string
  from_uri?: string
  to_uri?: string
  disposition?: string
  start_time_from?: string
  start_time_to?: string
  limit?: number
  /** The `next_cursor` of the page before. */
  cursor?: string
}

// Queues (matches queues.rs)
export type QueueResponse = S['QueueResponse']

export type QueueStatsResponse = S['QueueStatsResponse']

export type AgentResponse = S['AgentResponse']

export type AgentLoginRequest = S['AgentLoginRequest']

// Queue Configuration (database-backed CRUD)
export type QueueConfigResponse = S['QueueConfigResponse']

export type CreateQueueConfigRequest = S['CreateQueueConfigRequest']

// Voicemail (matches voicemail.rs)
export type MailboxResponse = S['MailboxResponse']

export type VoicemailMessage = S['MessageResponse']

export type MailboxStats = S['MailboxStatsResponse']

export type GreetingResponse = S['GreetingResponse']

export type CreateMailboxRequest = S['CreateMailboxRequest']

export type UpdateMailboxRequest = S['UpdateMailboxRequest']

// Presence (matches presence.rs)
export type PresenceResponse = S['PresenceResponse']

export type PresenceStats = S['PresenceStatsResponse']

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
  send_e164?: boolean
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
export type TrunkNodeView = S['TrunkNodeView']

/** A tenant's least-cost route (`docs/TENANT_TRUNKS.md` §12, phase 6c). */
export type LcrRoute = S['LcrRouteResponse']

export type CreateLcrRouteRequest = S['CreateLcrRouteRequest']

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

export type CreateTrunkRequest = S['CreateTrunkRequest']

export type TrunkStatsResponse = S['TrunkStats']

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
export type RoutingRule = S['RoutingRuleResponse']

export type CreateRoutingRuleRequest = S['CreateRoutingRuleRequest']

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

export type TimeWindow = S['TimeWindow']

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
export type FlowResponse = S['FlowResponse']

export interface FlowNode {
  type: string
  label?: string
  [key: string]: unknown
}

export type CreateFlowRequest = S['CreateFlowRequest']

export type UpdateFlowRequest = S['UpdateFlowRequest']

export type ListFlowsResponse = Page<FlowResponse>

export interface ListFlowsQuery {
  tenant_id?: string
  enabled?: boolean
  name?: string
  limit?: number
  /** The `next_cursor` of the page before. */
  cursor?: string
}

// Cluster (matches cluster health endpoints)
export type ClusterNode = S['ClusterNodeInfo']

export type SystemStatus = S['StatusResponse']

// Cluster node monitoring (matches fcp-cluster reader types)
export type ClusterNodeInfo = S['ClusterNodeInfo']

/** Whether a node's view of the cluster can be trusted. Said, never acted on. */
export type ClusterView = S['ClusterView']

/** A node's admission limits and what it is using of them. */
export type NodeCapacity = S['NodeCapacity']

export type VideoDeviceBudget = S['VideoDeviceBudget']

/** A tenant's site on its own network, as the cloud keeps it (docs/HYBRID_CONTROL_PLANE.md). */
export type SiteInfo = S['SiteResponse']

export type SiteRegistrations = S['SiteRegistrationsView']

/** A site with the one-time enrollment token the cloud shows once. */
export interface SiteWithToken extends SiteInfo {
  enrollment_token: string
  enrollment_token_expires_at: string
}

export type ClusterSitesResponse = S['ClusterSitesResponse']

export type ClusterNodesResponse = S['ClusterNodesResponse']

export type ClusterStatsResponse = S['ClusterStatsResponse']

// Audit Log
/** One audited admin request (`GET /audit-log`). */
export type AuditEntry = S['AuditEventResponse']

export interface AuditQueryParams {
  actor?: string
  /** Route-template prefix, e.g. `DELETE /api/v1/users`. */
  action?: string
  /** Unix seconds. */
  since?: number
  /** Unix seconds: only events before it. */
  until?: number
  /** One request: what a deletion's `api_request_id` names. */
  api_request_id?: string
  /** A platform administrator's rows: `all`, `platform` or a tenant id;
   *  unset follows the tenant picked in the top bar. */
  tenant?: string
  limit?: number
  cursor?: string
}

/** One entry of the deletion log (`GET /compliance/deletions`). */
export type DeletionEntry = S['DeletionResponse']

export interface DeletionQueryParams {
  class?: string
  since?: number
  /** Unix seconds: only deletions before it. */
  until?: number
  /** What one API request deleted: its audit row's `api_request_id`. */
  api_request_id?: string
  limit?: number
  cursor?: string
}

// Compliance (docs/COMPLIANCE.md, docs/COMPLIANCE_FOLLOWUPS.md)
export type CompliancePolicy = S['PolicyResponse']
export type UpdateCompliancePolicy = S['UpdatePolicyRequest']
export type RetentionRun = S['RetentionRunResponse']
export type LegalHold = S['HoldResponse']
export type PlaceHoldRequest = S['PlaceHoldRequest']
export type SubjectIdentifier = S['SubjectIdentifier']
export type DataClass = S['DataClass']
export type SubjectRequest = S['SubjectRequestResponse']
export type FileSubjectRequest = S['FileSubjectRequest']

// Conference Recordings
export type ConferenceRecording = S['RecordingResponse']

export interface RecordingQueryParams {
  room_number?: string
  meeting_id?: string
  /** Unix seconds. */
  since?: number
  until?: number
  running?: boolean
  limit?: number
  /** The `next_cursor` of the page before. */
  cursor?: string
}

export type ListRecordingsResponse = Page<ConferenceRecording>

/** One page of a list (docs/DEVELOPER_SURFACE.md §3, phase 2c). */
export interface Page<T> {
  data: T[]
  /** Pass as `cursor` for the next page; null on the last. */
  next_cursor: string | null
  /** The whole list's size, where the server knows it. */
  total?: number
}

// API Tokens (docs/DEVELOPER_SURFACE.md §3, phase 1)
export type ApiToken = S['ApiTokenResponse']
/** A user's own contact (docs/RECORD_AND_CONTACTS.md §3). */
export type PersonalContact = S['PersonalContactResponse']
export type PersonalContactRequest = S['PersonalContactRequest']
export type ContactImportResponse = S['ContactImportResponse']
export type ContactNumber = S['ContactNumber']

/** One of a user's devices, with an app install's session (docs/SOFTPHONE.md §2.1). */
export type UserDevice = S['UserDeviceResponse']

export type CreateApiTokenRequest = S['CreateApiTokenRequest']

export type CreatePersonalTokenRequest = S['CreatePersonalTokenRequest']

export type RotateApiTokenRequest = S['RotateApiTokenRequest']

export type CreateApiTokenResponse = S['CreateApiTokenResponse']

export type GrantablePermission = S['GrantablePermission']

// Webhooks (docs/DEVELOPER_SURFACE.md §3, phase 5)
export type Webhook = S['WebhookResponse']

export type WebhookWithSecret = S['WebhookWithSecret']

export type CreateWebhookRequest = S['CreateWebhookRequest']

export type UpdateWebhookRequest = S['UpdateWebhookRequest']

export type WebhookDelivery = S['WebhookDeliveryResponse']

export type WebhookDeliveryOutcome = S['WebhookDeliveryOutcome']

export type WebhookEventType = S['WebhookEventType']

// --- Self-service (end-user portal) types ---

export type MeProfile = S['ProfileResponse']

export type UpdateProfileRequest = S['UpdateProfileRequest']

export type ChangePasswordRequest = S['ChangePasswordRequest']

export type MeMailbox = S['me.MailboxResponse']
export type UpdateMeVoicemailRequest = S['UpdateVoicemailRequest']

export type MeMailboxStats = S['me.MailboxStatsResponse']

/** Find-me / follow-me (docs/ENTERPRISE_CALLING.md §3.6): targets are `devices`, `ext:…`, `number:+…`, `user:…` or a SIP URI. */
export type FindMeStep = S['FindMeStep']

export type FindMe = S['FindMe']

/** A user's calling settings (docs/ENTERPRISE_CALLING.md §3.1); targets as `voicemail`, `number:+1…`, `ext:2001`, `user:<id>` or a SIP URI. */
export type CallingSettings = S['CallingResponse']

export type ExecutiveFilter = 'all' | 'external' | 'none'
export type ExecutiveAlert = 'assistants' | 'assistants_and_executive'

export type Executive = S['ExecutiveView']

/** An executive the signed-in user assists (§3.10). */
export type MyExecutive = S['MyExecutive']

/** A forward given as an empty string is cleared; one left out is kept. */
export type UpdateCallingRequest = S['UpdateCallingRequest']

// Locations
export type Location = S['LocationResponse']

export type CreateLocationRequest = S['CreateLocationRequest']

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
export type MeetingResponse = S['MeetingResponse']

export type CreateMeetingRequest = S['CreateMeetingRequest']

export type UpdateMeetingRequest = S['UpdateMeetingRequest']

// ── Emergency calling (E911), docs/E911.md ───────────────────────────────

/** How an on-site notification reaches someone. */
export type ContactKind = 'email' | 'sms' | 'call' | 'webhook'

export type NotifyContact = S['ContactResponse']

export type EmergencyConfig = S['EmergencyConfigResponse']

export type UpdateEmergencyConfigRequest = S['UpdateEmergencyConfigRequest']

export type DispatchableLocation = S['emergency.LocationResponse']

export type UserReadiness = S['UserReadiness']

export type ContactReadiness = S['ContactReadiness']

export type EmergencyReadiness = S['ReadinessResponse']

export type EmergencyNetwork = S['NetworkResponse']

export type CreateEmergencyNetworkRequest = S['CreateNetworkRequest']

export type EmergencyCall = S['EmergencyCallResponse']

export type EmergencyNotification = S['NotificationResponse']

export type AddressValidation = S['ValidationResponse']

export type LocationValidationResponse = S['LocationValidationResponse']

export type DeviceValidationResponse = S['DeviceValidationResponse']

export type NumberValidationResponse = S['NumberValidationResponse']

// Schedules (docs/ENTERPRISE_CALLING.md §3.5)
export type Weekday = 'Mon' | 'Tue' | 'Wed' | 'Thu' | 'Fri' | 'Sat' | 'Sun'

export type WeeklyWindow = S['WeeklyWindow']

export type Holiday = S['Holiday']

export type Schedule = S['ScheduleResponse']

/** A tenant's parking lot and the calls waiting in its slots (`docs/PARKING.md`). */
export type ParkingLot = S['ParkingLotResponse']
export type ParkingLotRequest = S['ParkingLotRequest']
export type ParkedCall = S['ParkedCallResponse']

/** A tenant's group (`docs/USERS.md` §5). */
export type Group = S['GroupResponse']

/** The tenant's provisioning URL (`docs/PROVISIONING.md`). */
export type ProvisioningKey = S['ProvisioningKeyResponse']
export type DeviceFetch = S['DeviceFetchResponse']

/** A capture an app uploaded, and an administrator's allow to take them
 * (docs/CALL_DIAGNOSTICS.md §3). */
export type CaptureResponse = S['CaptureResponse']
export type CaptureAllowResponse = S['CaptureAllowResponse']
export type CaptureAllowRequest = S['CaptureAllowRequest']
export type TenantDiagnostics = S['TenantDiagnostics']
/** A phone waiting to be claimed (`docs/PROVISIONING.md` §6). */
export type PendingDevice = S['PendingDeviceResponse']
export type ClaimDeviceRequest = S['ClaimDeviceRequest']
export type LineKey = S['LineKey']
/** A line on a device (`docs/DEVICE_LINES.md`). */
export type DeviceLine = S['DeviceLine']
export type RenderedConfig = S['RenderedConfigResponse']
export type RenderDeviceRequest = S['RenderDeviceRequest']
export type ConfigTemplate = S['ConfigTemplateResponse']
export type CreateConfigTemplateRequest = S['CreateConfigTemplateRequest']
export type UpdateConfigTemplateRequest = S['UpdateConfigTemplateRequest']
export type LibraryTemplate = S['LibraryTemplateResponse']
export type PreviewTemplateResponse = S['PreviewTemplateResponse']
/** Zero-touch: a tenant's account with a vendor's redirection service (§10). */
export type RedirectionAccount = S['RedirectionAccountResponse']
export type PutRedirectionAccountRequest = S['PutRedirectionAccountRequest']
export type DeviceRedirection = S['DeviceRedirectionResponse']
/** Managed firmware (`docs/PROVISIONING.md` §11). */
export type FirmwareImage = S['FirmwareImageResponse']
export type FirmwareTarget = S['FirmwareTargetResponse']
export type FirmwareTargetRequest = S['FirmwareTargetRequest']
export type GroupRequest = S['GroupRequest']
export type GroupMember = S['GroupMemberResponse']

export type ScheduleRequest = S['ScheduleRequest']
