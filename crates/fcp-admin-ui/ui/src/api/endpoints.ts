import { apiClient } from './client'
import type {
  User, CreateUserRequest, UpdateUserRequest,
  Tenant, CreateTenantRequest,
  Extension, CreateExtensionRequest, UpdateExtensionRequest,
  Device, CreateDeviceRequest, UpdateDeviceRequest,
  Location, CreateLocationRequest,
  PhoneNumber, CreatePhoneNumberRequest,
  LineBinding, CreateLineBindingRequest, UpdateLineBindingRequest,
  DashboardSummary, SystemHealth,
  LoginRequest, LoginResponse, AuthUser, ChangeExpiredPasswordRequest,
  CallInfo, OriginateCallRequest,
  CdrResponse, ListCdrsResponse, CdrQueryParams,
  QueueResponse, QueueStatsResponse, AgentResponse, AgentLoginRequest,
  QueueConfigResponse, CreateQueueConfigRequest,
  MailboxResponse, VoicemailMessage, MailboxStats, CreateMailboxRequest, UpdateMailboxRequest,
  PresenceResponse, PresenceStats,
  Registration,
  SipTrunk, CreateTrunkRequest, TrunkStatsResponse,
  TrunkSummary, TrunkDetail, QualityMetrics,
  RoutingRule, CreateRoutingRuleRequest,
  FlowResponse, CreateFlowRequest, UpdateFlowRequest, ListFlowsResponse, ListFlowsQuery,
  SystemStatus,
  PortRequest, CreatePortRequest,
  AuditEntry, AuditQueryParams, DeletionEntry, DeletionQueryParams,
  EmergencyConfig, UpdateEmergencyConfigRequest, EmergencyReadiness,
  EmergencyNetwork, CreateEmergencyNetworkRequest, EmergencyCall,
  EmergencyNotification, AddressValidation, LocationValidationResponse,
  DeviceValidationResponse, NumberValidationResponse, E911AddressInput,
  ConferenceRecording, RecordingQueryParams, ListRecordingsResponse,
  ApiToken, CreateApiTokenRequest, CreateApiTokenResponse,
  ClusterNodesResponse, ClusterStatsResponse,
  MeProfile, UpdateProfileRequest, ChangePasswordRequest,
  MeMailbox, MeMailboxStats, MeForwarding, UpdateForwardingRequest,
  MeetingResponse, CreateMeetingRequest, UpdateMeetingRequest,
} from '@/types'

// Auth
export const auth = {
  login: (data: LoginRequest) => apiClient.post<LoginResponse>('/auth/login', data),
  logout: () => apiClient.post<void>('/auth/logout'),
  refresh: () => apiClient.post<LoginResponse>('/auth/refresh'),
  me: () => apiClient.get<AuthUser>('/auth/me'),
  changeExpiredPassword: (data: ChangeExpiredPasswordRequest) =>
    apiClient.post<LoginResponse>('/auth/change-expired-password', data),
}

// Dashboard
export const dashboard = {
  summary: () => apiClient.get<DashboardSummary>('/dashboard/summary'),
}

// System
export const system = {
  health: () => apiClient.get<SystemHealth>('/health'),
  status: () => apiClient.get<SystemStatus>('/status'),
}

// Users
export const users = {
  list: () => apiClient.get<User[]>('/users'),
  get: (username: string) => apiClient.get<User>(`/users/${encodeURIComponent(username)}`),
  create: (data: CreateUserRequest) => apiClient.post<User>('/users', data),
  update: (username: string, data: UpdateUserRequest) =>
    apiClient.put<User>(`/users/${encodeURIComponent(username)}`, data),
  delete: (username: string) => apiClient.delete<void>(`/users/${encodeURIComponent(username)}`),
  updatePassword: (username: string, password: string) =>
    apiClient.put<void>(`/users/${encodeURIComponent(username)}/password`, { password }),
  updateDigestPassword: (username: string, digest_password: string) =>
    apiClient.put<void>(`/users/${encodeURIComponent(username)}/digest-password`, { digest_password }),
  deleteDigestPassword: (username: string) =>
    apiClient.delete<void>(`/users/${encodeURIComponent(username)}/digest-password`),
  unlock: (username: string) =>
    apiClient.post<User>(`/users/${encodeURIComponent(username)}/unlock`),
}

// Tenants
export const tenants = {
  list: () => apiClient.get<Tenant[]>('/tenants'),
  get: (id: string) => apiClient.get<Tenant>(`/tenants/${encodeURIComponent(id)}`),
  create: (data: CreateTenantRequest) => apiClient.post<Tenant>('/tenants', data),
  update: (id: string, data: Partial<CreateTenantRequest>) =>
    apiClient.put<Tenant>(`/tenants/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/tenants/${encodeURIComponent(id)}`),
}

// Extensions
export const extensions = {
  list: () => apiClient.get<Extension[]>('/extensions'),
  get: (number: string) => apiClient.get<Extension>(`/extensions/${encodeURIComponent(number)}`),
  create: (data: CreateExtensionRequest) => apiClient.post<Extension>('/extensions', data),
  update: (number: string, data: UpdateExtensionRequest) =>
    apiClient.patch<Extension>(`/extensions/${encodeURIComponent(number)}`, data),
  delete: (number: string) => apiClient.delete<void>(`/extensions/${encodeURIComponent(number)}`),
}

// Devices
export const devices = {
  list: () => apiClient.get<Device[]>('/devices'),
  get: (id: string) => apiClient.get<Device>(`/devices/${encodeURIComponent(id)}`),
  create: (data: CreateDeviceRequest) => apiClient.post<Device>('/devices', data),
  /**
   * `e911_override: null` removes the device's own address, and
   * `e911_trust_location: null` returns it to the tenant's device types
   * (`docs/E911.md`); every other field is left alone when it is absent.
   */
  update: (id: string, data: UpdateDeviceRequest) =>
    apiClient.patch<Device>(`/devices/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/devices/${encodeURIComponent(id)}`),
}

// Locations
export const locations = {
  list: () => apiClient.get<Location[]>('/locations'),
  get: (id: string) => apiClient.get<Location>(`/locations/${encodeURIComponent(id)}`),
  create: (data: CreateLocationRequest) => apiClient.post<Location>('/locations', data),
  update: (id: string, data: Partial<CreateLocationRequest>) =>
    apiClient.patch<Location>(`/locations/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/locations/${encodeURIComponent(id)}`),
}

// Phone Numbers
export const phoneNumbers = {
  list: () => apiClient.get<PhoneNumber[]>('/phone-numbers'),
  get: (id: string) => apiClient.get<PhoneNumber>(`/phone-numbers/${encodeURIComponent(id)}`),
  create: (data: CreatePhoneNumberRequest) => apiClient.post<PhoneNumber>('/phone-numbers', data),
  update: (id: string, data: Partial<CreatePhoneNumberRequest>) =>
    apiClient.patch<PhoneNumber>(`/phone-numbers/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/phone-numbers/${encodeURIComponent(id)}`),
  activate: (id: string) => apiClient.post<void>(`/phone-numbers/${encodeURIComponent(id)}/activate`),
}

// Line Bindings
export const lineBindings = {
  list: () => apiClient.get<LineBinding[]>('/line-bindings'),
  get: (id: string) => apiClient.get<LineBinding>(`/line-bindings/${encodeURIComponent(id)}`),
  create: (data: CreateLineBindingRequest) => apiClient.post<LineBinding>('/line-bindings', data),
  update: (id: string, data: UpdateLineBindingRequest) =>
    apiClient.patch<LineBinding>(`/line-bindings/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/line-bindings/${encodeURIComponent(id)}`),
}

// Calls (Phase 2 - live operations)
export const calls = {
  list: () => apiClient.get<CallInfo[]>('/calls'),
  get: (callId: string) => apiClient.get<CallInfo>(`/calls/${encodeURIComponent(callId)}`),
  originate: (data: OriginateCallRequest) => apiClient.post<CallInfo>('/calls', data),
  hangup: (callId: string) => apiClient.delete<void>(`/calls/${encodeURIComponent(callId)}`),
  hold: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/hold`),
  resume: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/resume`),
  transfer: (callId: string, target: string) =>
    apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/transfer`, { target }),
  mute: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/mute`),
  unmute: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/unmute`),
}

// CDRs (Phase 2 - call detail records)
export const cdrs = {
  list: (params?: CdrQueryParams) => {
    const searchParams = new URLSearchParams()
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          searchParams.set(key, String(value))
        }
      })
    }
    const query = searchParams.toString()
    return apiClient.get<ListCdrsResponse>(`/cdrs${query ? `?${query}` : ''}`)
  },
  get: (callId: string) => apiClient.get<CdrResponse>(`/cdrs/${encodeURIComponent(callId)}`),
}

// SIP Trace (for ladder diagram debugging)
export interface SipTraceMessage {
  id: number
  timestamp: number // Unix timestamp in milliseconds
  direction: string
  method_or_status: string
  cseq: string
  from_uri: string
  to_uri: string
  source_addr: string
  dest_addr: string
  transport: string
  raw_message: string
  node_id: string | null
}

export interface SipTraceListResponse {
  call_id: string
  messages: SipTraceMessage[]
}

export const sipTrace = {
  getMessages: (callId: string) =>
    apiClient.get<SipTraceListResponse>(`/calls/${encodeURIComponent(callId)}/sip-messages`),
}

// Flow Execution Traces
export interface FlowTraceEntry {
  id: number
  call_id: string
  flow_id: string
  flow_name: string | null
  node_id: string | null
  event_type: string
  timestamp: number // Unix timestamp in milliseconds
  detail: string | null
  flow_version: number | null
  elapsed_ms: number | null
}

export interface ListFlowTracesResponse {
  traces: FlowTraceEntry[]
  total: number
  limit: number
  offset: number
}

export interface FlowTraceQueryParams {
  call_id?: string
  flow_id?: string
  event_type?: string
  time_from?: string
  time_to?: string
  limit?: number
  offset?: number
}

export const flowTraces = {
  list: (params?: FlowTraceQueryParams) => {
    const searchParams = new URLSearchParams()
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          searchParams.set(key, String(value))
        }
      })
    }
    const query = searchParams.toString()
    return apiClient.get<ListFlowTracesResponse>(`/flow-traces${query ? `?${query}` : ''}`)
  },
  getByCall: (callId: string) =>
    apiClient.get<FlowTraceEntry[]>(`/flow-traces/${encodeURIComponent(callId)}`),
}

// Queue Configuration (database-backed CRUD)
export const queueConfigs = {
  list: () => apiClient.get<QueueConfigResponse[]>('/queue-configs'),
  get: (id: string) => apiClient.get<QueueConfigResponse>(`/queue-configs/${encodeURIComponent(id)}`),
  create: (data: CreateQueueConfigRequest) => apiClient.post<QueueConfigResponse>('/queue-configs', data),
  update: (id: string, data: Partial<CreateQueueConfigRequest>) =>
    apiClient.patch<QueueConfigResponse>(`/queue-configs/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/queue-configs/${encodeURIComponent(id)}`),
}

// Queues (Phase 2 - ACD runtime status)
export const queues = {
  list: () => apiClient.get<QueueResponse[]>('/queues'),
  get: (name: string) => apiClient.get<QueueResponse>(`/queues/${encodeURIComponent(name)}`),
  stats: (name: string) => apiClient.get<QueueStatsResponse>(`/queues/${encodeURIComponent(name)}/stats`),
  agents: (name: string) => apiClient.get<AgentResponse[]>(`/queues/${encodeURIComponent(name)}/agents`),
  loginAgent: (name: string, data: AgentLoginRequest) =>
    apiClient.post<void>(`/queues/${encodeURIComponent(name)}/agents`, data),
  logoutAgent: (name: string, agentId: string) =>
    apiClient.delete<void>(`/queues/${encodeURIComponent(name)}/agents/${encodeURIComponent(agentId)}`),
  pauseAgent: (name: string, agentId: string) =>
    apiClient.post<void>(`/queues/${encodeURIComponent(name)}/agents/${encodeURIComponent(agentId)}/pause`),
  resumeAgent: (name: string, agentId: string) =>
    apiClient.post<void>(`/queues/${encodeURIComponent(name)}/agents/${encodeURIComponent(agentId)}/resume`),
}

// Voicemail (Phase 2)
export const voicemail = {
  listMailboxes: () => apiClient.get<MailboxResponse[]>('/voicemail/mailboxes'),
  getMailbox: (id: string) => apiClient.get<MailboxResponse>(`/voicemail/mailboxes/${encodeURIComponent(id)}`),
  getMailboxStats: (id: string) => apiClient.get<MailboxStats>(`/voicemail/mailboxes/${encodeURIComponent(id)}/stats`),
  listMessages: (mailboxId: string) =>
    apiClient.get<VoicemailMessage[]>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/messages`),
  getMessage: (messageId: string) =>
    apiClient.get<VoicemailMessage>(`/voicemail/messages/${encodeURIComponent(messageId)}`),
  deleteMessage: (messageId: string) =>
    apiClient.delete<void>(`/voicemail/messages/${encodeURIComponent(messageId)}`),
  markHeard: (messageId: string) =>
    apiClient.put<void>(`/voicemail/messages/${encodeURIComponent(messageId)}`, { status: 'heard' }),
  createMailbox: (data: CreateMailboxRequest) =>
    apiClient.post<MailboxResponse>('/voicemail/mailboxes', data),
  updateMailbox: (id: string, data: UpdateMailboxRequest) =>
    apiClient.put<MailboxResponse>(`/voicemail/mailboxes/${encodeURIComponent(id)}`, data),
  deleteMailbox: (id: string) =>
    apiClient.delete<void>(`/voicemail/mailboxes/${encodeURIComponent(id)}`),
  listGreetings: (mailboxId: string) =>
    apiClient.get<import('@/types').GreetingResponse[]>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings`),
  uploadGreeting: async (mailboxId: string, greetingType: string, audioFile: File, expiresInHours?: number) => {
    const formData = new FormData()
    formData.append('greeting_type', greetingType)
    formData.append('audio', audioFile)
    if (expiresInHours) formData.append('expires_in', String(expiresInHours))
    return apiClient.uploadForm<import('@/types').GreetingResponse>(
      `/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings`, formData,
    )
  },
  deleteGreeting: (mailboxId: string, greetingType: string) =>
    apiClient.delete<void>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings/${encodeURIComponent(greetingType)}`),
  setGreetingActive: (mailboxId: string, greetingType: string, active: boolean) =>
    apiClient.put<void>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings/${encodeURIComponent(greetingType)}/active`, { active }),
  downloadGreetingUrl: (mailboxId: string, greetingType: string) =>
    `/api/v1/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings/${encodeURIComponent(greetingType)}/audio`,
}

// Audio Prompts
export interface PromptResponse {
  id: string
  tenant_id: string
  name: string
  description: string | null
  category: string
  language: string
  source_type: string
  tts_text: string | null
  tts_voice: string | null
  audio_path: string | null
  audio_format: string | null
  duration_ms: number | null
  file_size: number | null
  is_system: boolean
  has_audio: boolean
  created_at: number
  updated_at: number
}

export interface CreatePromptRequest {
  name: string
  description?: string
  category?: string
  language?: string
  source_type?: string
  tts_text?: string
  tts_voice?: string
}

export interface UpdatePromptRequest {
  description?: string
  category?: string
  language?: string
  tts_text?: string
  tts_voice?: string
}

export const prompts = {
  list: (params?: { category?: string; language?: string }) => {
    const query = new URLSearchParams()
    if (params?.category) query.set('category', params.category)
    if (params?.language) query.set('language', params.language)
    const qs = query.toString()
    return apiClient.get<PromptResponse[]>(`/prompts${qs ? `?${qs}` : ''}`)
  },
  get: (id: string) => apiClient.get<PromptResponse>(`/prompts/${encodeURIComponent(id)}`),
  create: (data: CreatePromptRequest) => apiClient.post<PromptResponse>('/prompts', data),
  update: (id: string, data: UpdatePromptRequest) => apiClient.put<PromptResponse>(`/prompts/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/prompts/${encodeURIComponent(id)}`),
  uploadAudio: async (id: string, audioFile: File) => {
    const formData = new FormData()
    formData.append('audio', audioFile)
    return apiClient.uploadForm<PromptResponse>(`/prompts/${encodeURIComponent(id)}/audio`, formData)
  },
  audioUrl: (id: string) => `/api/v1/prompts/${encodeURIComponent(id)}/audio`,
  synthesize: (id: string) => apiClient.post<PromptResponse>(`/prompts/${encodeURIComponent(id)}/synthesize`, {}),
}

// Presence (Phase 2)
export const presence = {
  list: () => apiClient.get<PresenceResponse[]>('/presence'),
  get: (aor: string) => apiClient.get<PresenceResponse>(`/presence/${encodeURIComponent(aor)}`),
  stats: () => apiClient.get<PresenceStats>('/presence/stats'),
}

// Registrations
export const registrations = {
  list: () => apiClient.get<Registration[]>('/registrations'),
  get: (aor: string) => apiClient.get<Registration>(`/registrations/${encodeURIComponent(aor)}`),
  delete: (aor: string) => apiClient.delete<void>(`/registrations/${encodeURIComponent(aor)}`),
}

// --- Phase 3: Advanced Features ---

// SIP Trunks (database-backed CRUD)
export const trunks = {
  list: () => apiClient.get<SipTrunk[]>('/trunks'),
  get: (id: string) => apiClient.get<SipTrunk>(`/trunks/${encodeURIComponent(id)}`),
  create: (data: CreateTrunkRequest) => apiClient.post<SipTrunk>('/trunks', data),
  update: (id: string, data: Partial<CreateTrunkRequest>) =>
    apiClient.patch<SipTrunk>(`/trunks/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/trunks/${encodeURIComponent(id)}`),
  test: (id: string) => apiClient.post<{ success: boolean; message: string }>(`/trunks/${encodeURIComponent(id)}/test`),
  clone: (id: string) => apiClient.post<SipTrunk>(`/trunks/${encodeURIComponent(id)}/clone`),
  stats: (id: string) => apiClient.get<TrunkStatsResponse>(`/trunks/${encodeURIComponent(id)}/stats`),
}

// SIP Trunk live status (SBC admin API)
const SBC_BASE = '/admin'
export const trunkStatus = {
  list: () => apiClient.get<TrunkSummary[]>(`${SBC_BASE}/trunks`),
  get: (name: string) => apiClient.get<TrunkDetail>(`${SBC_BASE}/trunks/${encodeURIComponent(name)}`),
  enable: (name: string) => apiClient.post<void>(`${SBC_BASE}/trunks/${encodeURIComponent(name)}/enable`),
  disable: (name: string) => apiClient.post<void>(`${SBC_BASE}/trunks/${encodeURIComponent(name)}/disable`),
  register: (name: string) => apiClient.post<void>(`${SBC_BASE}/trunks/${encodeURIComponent(name)}/register`),
  quality: (name: string) => apiClient.get<QualityMetrics>(`${SBC_BASE}/quality/${encodeURIComponent(name)}`),
  qualityAll: () => apiClient.get<QualityMetrics[]>(`${SBC_BASE}/quality`),
}

// Routing / Policy Rules
export const routing = {
  list: () => apiClient.get<RoutingRule[]>('/routing/rules'),
  get: (id: string) => apiClient.get<RoutingRule>(`/routing/rules/${encodeURIComponent(id)}`),
  create: (data: CreateRoutingRuleRequest) => apiClient.post<RoutingRule>('/routing/rules', data),
  update: (id: string, data: Partial<CreateRoutingRuleRequest>) =>
    apiClient.patch<RoutingRule>(`/routing/rules/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/routing/rules/${encodeURIComponent(id)}`),
}

// IVR Flows
export const flows = {
  list: (params?: ListFlowsQuery) => {
    const searchParams = new URLSearchParams()
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          searchParams.set(key, String(value))
        }
      })
    }
    const query = searchParams.toString()
    return apiClient.get<ListFlowsResponse>(`/flows${query ? `?${query}` : ''}`)
  },
  get: (id: string) => apiClient.get<FlowResponse>(`/flows/${encodeURIComponent(id)}`),
  create: (data: CreateFlowRequest) => apiClient.post<FlowResponse>('/flows', data),
  update: (id: string, data: UpdateFlowRequest) =>
    apiClient.put<FlowResponse>(`/flows/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/flows/${encodeURIComponent(id)}`),
  versions: (id: string) => apiClient.get<FlowResponse[]>(`/flows/${encodeURIComponent(id)}/versions`),
}

// System / Cluster
export const cluster = {
  status: () => apiClient.get<SystemStatus>('/status'),
  health: () => apiClient.get<{ status: string; version: string }>('/health'),
  nodes: () => apiClient.get<ClusterNodesResponse>('/cluster/nodes'),
  stats: () => apiClient.get<ClusterStatsResponse>('/cluster/stats'),
}

// Number Porting
export const porting = {
  list: () => apiClient.get<PortRequest[]>('/porting/requests'),
  get: (id: string) => apiClient.get<PortRequest>(`/porting/requests/${encodeURIComponent(id)}`),
  create: (data: CreatePortRequest) => apiClient.post<PortRequest>('/porting/requests', data),
  update: (id: string, data: Partial<CreatePortRequest>) =>
    apiClient.put<PortRequest>(`/porting/requests/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/porting/requests/${encodeURIComponent(id)}`),
  submit: (id: string) => apiClient.post<void>(`/porting/requests/${encodeURIComponent(id)}/submit`),
  cancel: (id: string) => apiClient.post<void>(`/porting/requests/${encodeURIComponent(id)}/cancel`),
}

// Audit Log
export const recordings = {
  list: (params?: RecordingQueryParams) => {
    const searchParams = new URLSearchParams()
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          searchParams.set(key, String(value))
        }
      })
    }
    const query = searchParams.toString()
    return apiClient.get<ListRecordingsResponse>(`/recordings${query ? `?${query}` : ''}`)
  },
  get: (id: string) =>
    apiClient.get<ConferenceRecording>(`/recordings/${encodeURIComponent(id)}`),
  remove: (id: string) =>
    apiClient.delete<{ status: string; id: string; file_deleted: boolean }>(
      `/recordings/${encodeURIComponent(id)}`,
    ),
  /** The file itself, as an object URL for a player or a download link. */
  fetchMedia: (id: string): Promise<string> =>
    apiClient.fetchBlobUrl(`/recordings/${encodeURIComponent(id)}/download`),
}

function query(params?: object): string {
  const searchParams = new URLSearchParams()
  Object.entries(params ?? {}).forEach(([key, value]) => {
    if (value !== undefined && value !== null && value !== '') {
      searchParams.set(key, String(value))
    }
  })
  const text = searchParams.toString()
  return text ? `?${text}` : ''
}

export const auditLog = {
  list: (params?: AuditQueryParams) => apiClient.get<AuditEntry[]>(`/audit-log${query(params)}`),
  deletions: (params?: DeletionQueryParams) =>
    apiClient.get<DeletionEntry[]>(`/compliance/deletions${query(params)}`),
}

// Emergency calling (E911), docs/E911.md
export const emergency = {
  config: () => apiClient.get<EmergencyConfig>('/emergency/config'),
  updateConfig: (data: UpdateEmergencyConfigRequest) =>
    apiClient.put<EmergencyConfig>('/emergency/config', data),
  readiness: () => apiClient.get<EmergencyReadiness>('/emergency/readiness'),
  networks: () => apiClient.get<EmergencyNetwork[]>('/emergency/networks'),
  createNetwork: (data: CreateEmergencyNetworkRequest) =>
    apiClient.post<EmergencyNetwork>('/emergency/networks', data),
  updateNetwork: (id: string, data: Partial<CreateEmergencyNetworkRequest>) =>
    apiClient.patch<EmergencyNetwork>(`/emergency/networks/${encodeURIComponent(id)}`, data),
  deleteNetwork: (id: string) =>
    apiClient.delete<void>(`/emergency/networks/${encodeURIComponent(id)}`),
  calls: (limit = 100) => apiClient.get<EmergencyCall[]>(`/emergency/calls?limit=${limit}`),
  notifications: (callId: string) =>
    apiClient.get<EmergencyNotification[]>(
      `/emergency/calls/${encodeURIComponent(callId)}/notifications`,
    ),
  // Validation (docs/E911.md, "Address validation"): the configured
  // validator answers; a refusal is a 422 with the issues.
  validateAddress: (address: E911AddressInput) =>
    apiClient.post<AddressValidation>('/emergency/validate-address', { address }),
  validateLocation: (id: string) =>
    apiClient.post<LocationValidationResponse>(
      `/locations/${encodeURIComponent(id)}/e911/validate`,
    ),
  validateDevice: (id: string) =>
    apiClient.post<DeviceValidationResponse>(`/devices/${encodeURIComponent(id)}/e911/validate`),
  validateNumber: (id: string) =>
    apiClient.post<NumberValidationResponse>(
      `/phone-numbers/${encodeURIComponent(id)}/e911/validate`,
    ),
}

// API Tokens
export const apiTokens = {
  list: () => apiClient.get<ApiToken[]>('/api-tokens'),
  create: (data: CreateApiTokenRequest) => apiClient.post<CreateApiTokenResponse>('/api-tokens', data),
  delete: (id: string) => apiClient.delete<void>(`/api-tokens/${encodeURIComponent(id)}`),
}

// Configuration Management
export interface ConfigEntry {
  service: string
  section: string
  tenant_id: string
  value: Record<string, unknown>
  version: number
  updated_at: string
  updated_by: string | null
}

export interface ConfigAuditEntry {
  id: number
  service: string
  section: string
  tenant_id: string
  old_value: Record<string, unknown> | null
  new_value: Record<string, unknown>
  changed_by: string | null
  changed_at: string
  version: number
}

export interface UpdateConfigResponse {
  version: number
  service: string
  section: string
  tenant_id: string
}

export interface ValidateConfigResponse {
  valid: boolean
  errors: string[]
}

export const config = {
  list: (service?: string, tenant_id?: string) => {
    const params = new URLSearchParams()
    if (service) params.set('service', service)
    if (tenant_id) params.set('tenant_id', tenant_id)
    const q = params.toString()
    return apiClient.get<ConfigEntry[]>(`/config${q ? `?${q}` : ''}`)
  },
  listService: (service: string, tenant_id = '_system') =>
    apiClient.get<ConfigEntry[]>(`/config/${encodeURIComponent(service)}?tenant_id=${encodeURIComponent(tenant_id)}`),
  get: (service: string, section: string, tenant_id = '_system') =>
    apiClient.get<ConfigEntry>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}?tenant_id=${encodeURIComponent(tenant_id)}`),
  update: (service: string, section: string, value: Record<string, unknown>, tenant_id = '_system', updated_by?: string) =>
    apiClient.put<UpdateConfigResponse>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}`, {
      value,
      tenant_id,
      updated_by,
    }),
  delete: (service: string, section: string, tenant_id = '_system') =>
    apiClient.delete<{ deleted: boolean }>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}?tenant_id=${encodeURIComponent(tenant_id)}`),
  validate: (service: string, section: string, value: Record<string, unknown>, tenant_id = '_system') =>
    apiClient.post<ValidateConfigResponse>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}/validate`, {
      value,
      tenant_id,
    }),
  audit: (service?: string, section?: string, limit = 50) => {
    const params = new URLSearchParams()
    if (service) params.set('service', service)
    if (section) params.set('section', section)
    params.set('limit', String(limit))
    return apiClient.get<ConfigAuditEntry[]>(`/config/audit?${params.toString()}`)
  },
}

// Self-service (end-user portal)
export const me = {
  // Profile
  getProfile: () => apiClient.get<MeProfile>('/me/profile'),
  updateProfile: (data: UpdateProfileRequest) => apiClient.put<MeProfile>('/me/profile', data),
  changePassword: (data: ChangePasswordRequest) => apiClient.put<void>('/me/password', data),

  // Voicemail
  getMailbox: () => apiClient.get<MeMailbox>('/me/voicemail'),
  updateMailbox: (data: { pin?: string; email_notification?: boolean; email_address?: string; attach_audio?: boolean }) =>
    apiClient.put<MeMailbox>('/me/voicemail', data),
  getMailboxStats: () => apiClient.get<MeMailboxStats>('/me/voicemail/stats'),
  listMessages: () => apiClient.get<VoicemailMessage[]>('/me/voicemail/messages'),
  updateMessage: (id: string, data: { status?: string }) =>
    apiClient.put<VoicemailMessage>(`/me/voicemail/messages/${encodeURIComponent(id)}`, data),
  deleteMessage: (id: string) => apiClient.delete<void>(`/me/voicemail/messages/${encodeURIComponent(id)}`),
  fetchMessageAudio: (id: string): Promise<string> =>
    apiClient.fetchBlobUrl(`/me/voicemail/messages/${encodeURIComponent(id)}/audio`),

  // Greetings
  listGreetings: () =>
    apiClient.get<import('@/types').GreetingResponse[]>('/me/voicemail/greetings'),
  uploadGreeting: async (greetingType: string, audioFile: File, expiresInHours?: number) => {
    const formData = new FormData()
    formData.append('greeting_type', greetingType)
    formData.append('audio', audioFile)
    if (expiresInHours) formData.append('expires_in', String(expiresInHours))
    return apiClient.uploadForm<import('@/types').GreetingResponse>(
      '/me/voicemail/greetings', formData,
    )
  },
  synthesizeGreeting: (data: {
    greeting_type: string
    text: string
    language?: string
    voice?: string
    expires_in_hours?: number
  }) => apiClient.post<import('@/types').GreetingResponse>('/me/voicemail/greetings/synthesize', data),
  deleteGreeting: (greetingType: string) =>
    apiClient.delete<void>(`/me/voicemail/greetings/${encodeURIComponent(greetingType)}`),
  setGreetingActive: (greetingType: string, active: boolean) =>
    apiClient.put<void>(`/me/voicemail/greetings/${encodeURIComponent(greetingType)}/active`, { active }),
  fetchGreetingAudio: (greetingType: string): Promise<string> =>
    apiClient.fetchBlobUrl(`/me/voicemail/greetings/${encodeURIComponent(greetingType)}/audio`),

  // Avatar
  uploadAvatar: async (file: File) => {
    const formData = new FormData()
    formData.append('avatar', file)
    await apiClient.uploadForm<void>('/me/profile/avatar', formData)
  },
  fetchAvatar: (): Promise<string> =>
    apiClient.fetchBlobUrl('/me/profile/avatar'),
  deleteAvatar: (): Promise<void> =>
    apiClient.deleteRaw('/me/profile/avatar'),

  // Forwarding
  getForwarding: () => apiClient.get<MeForwarding[]>('/me/forwarding'),
  updateForwarding: (id: string, data: UpdateForwardingRequest) =>
    apiClient.patch<MeForwarding>(`/me/forwarding/${encodeURIComponent(id)}`, data),

  // Conferences
  listConferences: () => apiClient.get<MeetingResponse[]>('/me/conferences'),
  /** `POST /me/conferences/{id}/join-tokens` — a browser join link for a conference the user owns. */
  mintJoinToken: (id: string, body: MintJoinTokenRequest) =>
    apiClient.post<JoinTokenResponse>(`/me/conferences/${encodeURIComponent(id)}/join-tokens`, body),
  /** Dial out from a conference the user owns. */
  dialOutConference: (id: string, destination: string, options?: { greeting_tts?: string; greeting_prompt?: string }) =>
    apiClient.post<DialOutResponse>(`/me/conferences/${encodeURIComponent(id)}/dial-out`, { destination, ...options }),
  getConference: (id: string) => apiClient.get<MeetingResponse>(`/me/conferences/${encodeURIComponent(id)}`),
  updateConference: (id: string, data: UpdateMeetingRequest) =>
    apiClient.patch<MeetingResponse>(`/me/conferences/${encodeURIComponent(id)}`, data),
}

// Conference / Meetings
export const conferences = {
  listMeetings: () => apiClient.get<MeetingResponse[]>('/conferences/meetings'),
  getMeeting: (id: string) => apiClient.get<MeetingResponse>(`/conferences/meetings/${encodeURIComponent(id)}`),
  createMeeting: (data: CreateMeetingRequest) => apiClient.post<MeetingResponse>('/conferences/meetings', data),
  updateMeeting: (id: string, data: UpdateMeetingRequest) =>
    apiClient.patch<MeetingResponse>(`/conferences/meetings/${encodeURIComponent(id)}`, data),
  deleteMeeting: (id: string) => apiClient.delete(`/conferences/meetings/${encodeURIComponent(id)}`),
  enableMeeting: (id: string) => apiClient.post<MeetingResponse>(`/conferences/meetings/${encodeURIComponent(id)}/enable`),
  disableMeeting: (id: string) => apiClient.post<MeetingResponse>(`/conferences/meetings/${encodeURIComponent(id)}/disable`),
}

// Live conference controls, proxied by the admin API to the conference
// server (`/conferences/live/...`). The API resolves the room to a meeting in
// the caller's tenant; the conference server forwards to whichever node hosts
// the room.
export interface DialOutResponse {
  status: string
  participant_id: string
  destination: string
  room_number: string
}

/** A participant's video as the conference server reports it. `state` is
 *  `off` (no video negotiated), `on`, `lost`, `disabled` (a host turned it
 *  off) or `failed`. */
export interface LiveParticipantVideo {
  state: 'off' | 'on' | 'lost' | 'disabled' | 'failed' | string
  codec?: string
  resolution?: string
  fps?: number
  bitrate_kbps?: number
  flavor?: string
  /** The bitrate-ladder rung they are being sent, e.g. `640x360` (§7). */
  rung?: string
  rung_kbps?: number
  /** What their link says it can carry. */
  remb_kbps?: number
  /** The stream they are being sent directly, e.g. `alice` or `alice#content` (§5.6). */
  forwarding?: string
  /** What the channel shows: `composite`, `cameras` or `content`. */
  view?: string
  keyframes_sent?: number
  frames_decoded?: number
  frames_lost?: number
  nacks?: number
  plis?: number
  speaking?: boolean
  is_inactive?: boolean
  /** Whether this participant's screen is the one the room shows (§15.6). */
  presenting?: boolean
  /** The shared screen's section, when their offer carried one: the same
   *  shape as the camera's, plus `channel` — whether they receive the
   *  room's content on it. */
  content?: LiveParticipantVideo & { channel?: boolean }
}

export interface LiveVideoParticipant {
  participant_id: string
  /** Which node the participant is on, in a room spread over several. */
  node_id?: string
  display_name?: string
  caller_id?: string
  is_host?: boolean
  is_dial_out?: boolean
  is_speaking: boolean
  muted: boolean
  video: LiveParticipantVideo
}

export interface LiveVideoFlavor {
  /** Where the flavor is encoded: the room's device, or `host` when the device has no encoder for the codec. */
  device?: string
  codec: string
  profile: string
  resolution: string
  fps: number
  max_kbps: number
  target_kbps: number
  subscribers: number
  keyframes: number
  encode_errors: number
}

/** The share under way (§15.6). */
export interface LiveRoomContent {
  participant_id: string
  display_name: string
  remote: boolean
  held_for_secs: number
  codec: string
  resolution: string
  fps: number
  /** Whether a frame has been decoded: the composite is showing it. */
  live: boolean
}

export interface LiveRoomVideo {
  layout: string
  /** `presentation` or `content_only`: how the composite shows a share. */
  content_layout: string
  /** `null` while nobody is sharing a screen. */
  content: LiveRoomContent | null
  spotlight: string | null
  pinned: string | null
  active_speaker: string | null
  fps: number
  target_fps: number
  resolution: string
  /** The device the room runs on (`host`, or a GPU as `cuda:0`); each output, flavor and source says where it actually runs. */
  device?: string
  sources: number
  encoders: number
  ticks: number
  overruns: number
  active_speaker_via: string | null
  outputs: {
    /** `all`, `excluding` (with `participant`), or `local_only` (a trunk's). */
    scope: { kind: string; participant?: string }
    /** `composite`, `cameras` or `content`. */
    view?: string
    resolution: string
    /** Where it is composed. */
    device?: string
    flavors: LiveVideoFlavor[]
  }[]
}

/** One hosting node's view, in the merged room-wide status. */
export interface LiveVideoNode {
  node_id: string
  participant_count: number
  active_speaker: string | null
  active_speaker_via: string | null
  layout: string | null
}

/** `GET /conferences/live/{room}/video` */
export interface LiveRoomVideoStatus {
  room_number: string
  /** The conference node that answered. */
  node_id: string
  /** Whether that node hosts the room itself. */
  hosted_here?: boolean
  /** Across every hosting node. */
  participant_count: number
  participants: LiveVideoParticipant[]
  /** `null` until somebody in the room has video. */
  video: LiveRoomVideo | null
  /** The room's speaker, agreed across the nodes hosting it. */
  active_speaker?: string | null
  /** The room's layout, which is set room-wide. */
  layout?: string | null
  /** False only while the nodes are still converging on a speaker. */
  agreed?: boolean
  /** Every hosting node's own view. */
  nodes?: LiveVideoNode[]
  /** The inter-node trunks carrying the room. */
  trunks?: { peer: string; participant_id: string; video: boolean; video_codec: string | null }[]
}

/** A layout, pin or spotlight control's answer. */
export interface LiveLayoutResponse {
  status: string
  layout: string
  content_layout?: string
  pinned: string | null
  spotlight: string | null
  participant?: string
}

export const VIDEO_LAYOUT_LABELS: Record<string, string> = {
  grid: 'Grid',
  active_speaker: 'Active speaker',
  spotlight: 'Spotlight',
  pip: 'Picture in picture',
}

export const CONTENT_LAYOUT_LABELS: Record<string, string> = {
  presentation: 'Presentation — screen large, cameras in a strip',
  content_only: 'Screen only',
}

const liveRoom = (room: string) => `/conferences/live/${encodeURIComponent(room)}`

export const conferenceLive = {
  /** The room's video and every participant's, from the hosting node. */
  video: (room: string) => apiClient.get<LiveRoomVideoStatus>(`${liveRoom(room)}/video`),
  setLayout: (room: string, layout: string) =>
    apiClient.put<LiveLayoutResponse>(`${liveRoom(room)}/video/layout`, { layout }),
  pin: (room: string, participantId: string) =>
    apiClient.post<LiveLayoutResponse>(`${liveRoom(room)}/video/pin/${encodeURIComponent(participantId)}`),
  unpin: (room: string) => apiClient.delete<LiveLayoutResponse>(`${liveRoom(room)}/video/pin`),
  spotlight: (room: string, participantId: string) =>
    apiClient.post<LiveLayoutResponse>(`${liveRoom(room)}/video/spotlight/${encodeURIComponent(participantId)}`),
  unspotlight: (room: string) => apiClient.delete<LiveLayoutResponse>(`${liveRoom(room)}/video/spotlight`),
  /** Host control: hide (`false`) or show (`true`) one participant's video. */
  setParticipantVideo: (room: string, participantId: string, on: boolean) =>
    apiClient.post<{ status: string; participant: string }>(
      `${liveRoom(room)}/participants/${encodeURIComponent(participantId)}/video/${on ? 'on' : 'off'}`,
    ),
  /** Host control: end the screen share under way (§15.6). */
  stopContent: (room: string) =>
    apiClient.post<{ status: string; participant: string }>(`${liveRoom(room)}/video/content/stop`),
  /** How the composite shows a shared screen: `presentation` or `content_only`. */
  setContentLayout: (room: string, layout: string) =>
    apiClient.put<LiveLayoutResponse>(`${liveRoom(room)}/video/content-layout`, { layout }),
  /** Dial out to invite a participant into the conference. */
  dialOut: (room: string, destination: string, options?: { greeting_tts?: string; greeting_prompt?: string }) =>
    apiClient.post<DialOutResponse>(`${liveRoom(room)}/dial-out`, { destination, ...options }),
}

/** `POST /conferences/meetings/{id}/join-tokens` body. */
export interface MintJoinTokenRequest {
  display_name?: string
  role?: 'guest' | 'host'
  ttl_secs?: number
}

/** A minted browser join token and, when the gateway URL is configured, its link. */
export interface JoinTokenResponse {
  token: string
  room_number: string
  display_name: string
  role: 'guest' | 'host'
  expires_at: number
  join_url?: string
}

export const joinTokens = {
  /** `POST /conferences/meetings/{id}/join-tokens` — a browser join link for a meeting (admins). */
  mint: (id: string, body: MintJoinTokenRequest) =>
    apiClient.post<JoinTokenResponse>(`/conferences/meetings/${encodeURIComponent(id)}/join-tokens`, body),
}
