import { apiClient } from './client'
import type {
  User, CreateUserRequest, UpdateUserRequest,
  Tenant, CreateTenantRequest,
  Extension, CreateExtensionRequest, UpdateExtensionRequest,
  Device, CreateDeviceRequest,
  Location, CreateLocationRequest,
  PhoneNumber, CreatePhoneNumberRequest,
  LineBinding, CreateLineBindingRequest, UpdateLineBindingRequest,
  DashboardSummary, SystemHealth,
  LoginRequest, LoginResponse, AuthUser, ChangeExpiredPasswordRequest,
  CallInfo, OriginateCallRequest,
  CdrResponse, ListCdrsResponse, CdrQueryParams,
  QueueResponse, QueueStatsResponse, AgentResponse, AgentLoginRequest,
  MailboxResponse, VoicemailMessage, MailboxStats, CreateMailboxRequest, UpdateMailboxRequest,
  PresenceResponse, PresenceStats,
  Registration,
  SipTrunk, CreateTrunkRequest,
  TrunkSummary, TrunkDetail, QualityMetrics,
  PolicyRule, CreatePolicyRuleRequest,
  FlowResponse, CreateFlowRequest, UpdateFlowRequest, ListFlowsResponse, ListFlowsQuery,
  SystemStatus,
  PortRequest, CreatePortRequest,
  AuditEntry, AuditQueryParams, ListAuditResponse,
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
  update: (id: string, data: Partial<CreateDeviceRequest>) =>
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

// Queues (Phase 2 - ACD queues)
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
  getMessage: (mailboxId: string, messageId: string) =>
    apiClient.get<VoicemailMessage>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/messages/${encodeURIComponent(messageId)}`),
  deleteMessage: (mailboxId: string, messageId: string) =>
    apiClient.delete<void>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/messages/${encodeURIComponent(messageId)}`),
  markHeard: (mailboxId: string, messageId: string) =>
    apiClient.post<void>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/messages/${encodeURIComponent(messageId)}/heard`),
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
  list: () => apiClient.get<PolicyRule[]>('/routing/rules'),
  get: (id: string) => apiClient.get<PolicyRule>(`/routing/rules/${encodeURIComponent(id)}`),
  create: (data: CreatePolicyRuleRequest) => apiClient.post<PolicyRule>('/routing/rules', data),
  update: (id: string, data: Partial<CreatePolicyRuleRequest>) =>
    apiClient.put<PolicyRule>(`/routing/rules/${encodeURIComponent(id)}`, data),
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
export const auditLog = {
  list: (params?: AuditQueryParams) => {
    const searchParams = new URLSearchParams()
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          searchParams.set(key, String(value))
        }
      })
    }
    const query = searchParams.toString()
    return apiClient.get<ListAuditResponse>(`/audit-logs${query ? `?${query}` : ''}`)
  },
  get: (id: string) => apiClient.get<AuditEntry>(`/audit-logs/${encodeURIComponent(id)}`),
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

// Conference server live API (direct connection for real-time operations)
// The conference server HTTP API runs on a separate port from the admin API.
// Auto-detect: same hostname, port 8090.
const CONF_API_BASE = `${window.location.protocol}//${window.location.hostname}:8090`

export interface DialOutResponse {
  status: string
  participant_id: string
  destination: string
  room_number: string
}

export interface LiveParticipant {
  participant_id: string
  state: string
  gain: number
  is_host: boolean
  is_dial_out?: boolean
  display_name?: string
  caller_id?: string
  codec?: string
  duration_secs?: number
}

export interface LiveConference {
  room_number: string
  participant_count: number
  is_locked: boolean
  is_recording: boolean
  participants: LiveParticipant[]
}

export const conferenceLive = {
  /** List active conferences with participants */
  list: async (): Promise<LiveConference[]> => {
    const res = await fetch(`${CONF_API_BASE}/conferences`)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    return res.json()
  },
  /** Get a single active conference */
  get: async (roomNumber: string): Promise<LiveConference> => {
    const res = await fetch(`${CONF_API_BASE}/conferences/${encodeURIComponent(roomNumber)}/stats`)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    return res.json()
  },
  /** Dial out to invite a participant into the conference */
  dialOut: async (roomNumber: string, destination: string, options?: { greeting_tts?: string; greeting_prompt?: string }): Promise<DialOutResponse> => {
    const res = await fetch(`${CONF_API_BASE}/conferences/${encodeURIComponent(roomNumber)}/dial-out`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ destination, ...options }),
    })
    if (!res.ok) {
      const err = await res.json().catch(() => ({ error: `HTTP ${res.status}` }))
      throw new Error(err.error || `HTTP ${res.status}`)
    }
    return res.json()
  },
  /** Kick a participant */
  kick: async (roomNumber: string, participantId: string): Promise<void> => {
    const res = await fetch(`${CONF_API_BASE}/conferences/${encodeURIComponent(roomNumber)}/kick/${encodeURIComponent(participantId)}`, { method: 'POST' })
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
  },
  /** Mute a participant */
  mute: async (roomNumber: string, participantId: string): Promise<void> => {
    const res = await fetch(`${CONF_API_BASE}/conferences/${encodeURIComponent(roomNumber)}/mute/${encodeURIComponent(participantId)}`, { method: 'POST' })
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
  },
  /** Unmute a participant */
  unmute: async (roomNumber: string, participantId: string): Promise<void> => {
    const res = await fetch(`${CONF_API_BASE}/conferences/${encodeURIComponent(roomNumber)}/unmute/${encodeURIComponent(participantId)}`, { method: 'POST' })
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
  },
}
