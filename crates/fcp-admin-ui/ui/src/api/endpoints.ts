import { apiClient } from './client'
import type {
  User, CreateUserRequest, UpdateUserRequest,
  Tenant, CreateTenantRequest,
  Extension, CreateExtensionRequest,
  Device, CreateDeviceRequest,
  PhoneNumber, CreatePhoneNumberRequest,
  LineBinding, CreateLineBindingRequest,
  DashboardSummary, SystemHealth,
  LoginRequest, LoginResponse, AuthUser,
  CallInfo, OriginateCallRequest,
  CdrResponse, ListCdrsResponse, CdrQueryParams,
  QueueResponse, QueueStatsResponse, AgentResponse, AgentLoginRequest,
  MailboxResponse, VoicemailMessage, MailboxStats,
  PresenceResponse, PresenceStats,
  Registration,
  TrunkSummary, TrunkDetail, QualityMetrics,
  PolicyRule, CreatePolicyRuleRequest,
  FlowResponse, CreateFlowRequest, UpdateFlowRequest, ListFlowsResponse, ListFlowsQuery,
  SystemStatus,
  PortRequest, CreatePortRequest,
  AuditEntry, AuditQueryParams, ListAuditResponse,
  ApiToken, CreateApiTokenRequest, CreateApiTokenResponse,
  ClusterNodesResponse, ClusterStatsResponse,
} from '@/types'

// Auth
export const auth = {
  login: (data: LoginRequest) => apiClient.post<LoginResponse>('/auth/login', data),
  logout: () => apiClient.post<void>('/auth/logout'),
  refresh: () => apiClient.post<LoginResponse>('/auth/refresh'),
  me: () => apiClient.get<AuthUser>('/auth/me'),
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
}

// Tenants
export const tenants = {
  list: () => apiClient.get<Tenant[]>('/tenants'),
  get: (id: string) => apiClient.get<Tenant>(`/tenants/${id}`),
  create: (data: CreateTenantRequest) => apiClient.post<Tenant>('/tenants', data),
  update: (id: string, data: Partial<CreateTenantRequest>) =>
    apiClient.put<Tenant>(`/tenants/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/tenants/${id}`),
}

// Extensions
export const extensions = {
  list: () => apiClient.get<Extension[]>('/extensions'),
  get: (number: string) => apiClient.get<Extension>(`/extensions/${encodeURIComponent(number)}`),
  create: (data: CreateExtensionRequest) => apiClient.post<Extension>('/extensions', data),
  update: (number: string, data: Partial<CreateExtensionRequest>) =>
    apiClient.patch<Extension>(`/extensions/${encodeURIComponent(number)}`, data),
  delete: (number: string) => apiClient.delete<void>(`/extensions/${encodeURIComponent(number)}`),
}

// Devices
export const devices = {
  list: () => apiClient.get<Device[]>('/devices'),
  get: (id: string) => apiClient.get<Device>(`/devices/${id}`),
  create: (data: CreateDeviceRequest) => apiClient.post<Device>('/devices', data),
  update: (id: string, data: Partial<CreateDeviceRequest>) =>
    apiClient.patch<Device>(`/devices/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/devices/${id}`),
}

// Phone Numbers
export const phoneNumbers = {
  list: () => apiClient.get<PhoneNumber[]>('/phone-numbers'),
  get: (id: string) => apiClient.get<PhoneNumber>(`/phone-numbers/${id}`),
  create: (data: CreatePhoneNumberRequest) => apiClient.post<PhoneNumber>('/phone-numbers', data),
  update: (id: string, data: Partial<CreatePhoneNumberRequest>) =>
    apiClient.patch<PhoneNumber>(`/phone-numbers/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/phone-numbers/${id}`),
  activate: (id: string) => apiClient.post<void>(`/phone-numbers/${id}/activate`),
}

// Line Bindings
export const lineBindings = {
  list: () => apiClient.get<LineBinding[]>('/line-bindings'),
  get: (id: string) => apiClient.get<LineBinding>(`/line-bindings/${id}`),
  create: (data: CreateLineBindingRequest) => apiClient.post<LineBinding>('/line-bindings', data),
  update: (id: string, data: Partial<CreateLineBindingRequest>) =>
    apiClient.patch<LineBinding>(`/line-bindings/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/line-bindings/${id}`),
}

// Calls (Phase 2 - live operations)
export const calls = {
  list: () => apiClient.get<CallInfo[]>('/calls'),
  get: (callId: string) => apiClient.get<CallInfo>(`/calls/${callId}`),
  originate: (data: OriginateCallRequest) => apiClient.post<CallInfo>('/calls', data),
  hangup: (callId: string) => apiClient.delete<void>(`/calls/${callId}`),
  hold: (callId: string) => apiClient.post<void>(`/calls/${callId}/hold`),
  resume: (callId: string) => apiClient.post<void>(`/calls/${callId}/resume`),
  transfer: (callId: string, target: string) =>
    apiClient.post<void>(`/calls/${callId}/transfer`, { target }),
  mute: (callId: string) => apiClient.post<void>(`/calls/${callId}/mute`),
  unmute: (callId: string) => apiClient.post<void>(`/calls/${callId}/unmute`),
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
  get: (callId: string) => apiClient.get<CdrResponse>(`/cdrs/${callId}`),
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
    apiClient.get<SipTraceListResponse>(`/calls/${callId}/sip-messages`),
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
    apiClient.delete<void>(`/queues/${encodeURIComponent(name)}/agents/${agentId}`),
  pauseAgent: (name: string, agentId: string) =>
    apiClient.post<void>(`/queues/${encodeURIComponent(name)}/agents/${agentId}/pause`),
  resumeAgent: (name: string, agentId: string) =>
    apiClient.post<void>(`/queues/${encodeURIComponent(name)}/agents/${agentId}/resume`),
}

// Voicemail (Phase 2)
export const voicemail = {
  listMailboxes: () => apiClient.get<MailboxResponse[]>('/voicemail/mailboxes'),
  getMailbox: (id: string) => apiClient.get<MailboxResponse>(`/voicemail/mailboxes/${id}`),
  getMailboxStats: (id: string) => apiClient.get<MailboxStats>(`/voicemail/mailboxes/${id}/stats`),
  listMessages: (mailboxId: string) =>
    apiClient.get<VoicemailMessage[]>(`/voicemail/mailboxes/${mailboxId}/messages`),
  getMessage: (mailboxId: string, messageId: string) =>
    apiClient.get<VoicemailMessage>(`/voicemail/mailboxes/${mailboxId}/messages/${messageId}`),
  deleteMessage: (mailboxId: string, messageId: string) =>
    apiClient.delete<void>(`/voicemail/mailboxes/${mailboxId}/messages/${messageId}`),
  markHeard: (mailboxId: string, messageId: string) =>
    apiClient.post<void>(`/voicemail/mailboxes/${mailboxId}/messages/${messageId}/heard`),
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

// SIP Trunks (SBC admin API - different base path)
const SBC_BASE = '/admin'
export const trunks = {
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
  get: (id: string) => apiClient.get<PolicyRule>(`/routing/rules/${id}`),
  create: (data: CreatePolicyRuleRequest) => apiClient.post<PolicyRule>('/routing/rules', data),
  update: (id: string, data: Partial<CreatePolicyRuleRequest>) =>
    apiClient.put<PolicyRule>(`/routing/rules/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/routing/rules/${id}`),
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
  get: (id: string) => apiClient.get<FlowResponse>(`/flows/${id}`),
  create: (data: CreateFlowRequest) => apiClient.post<FlowResponse>('/flows', data),
  update: (id: string, data: UpdateFlowRequest) =>
    apiClient.put<FlowResponse>(`/flows/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/flows/${id}`),
  versions: (id: string) => apiClient.get<FlowResponse[]>(`/flows/${id}/versions`),
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
  get: (id: string) => apiClient.get<PortRequest>(`/porting/requests/${id}`),
  create: (data: CreatePortRequest) => apiClient.post<PortRequest>('/porting/requests', data),
  update: (id: string, data: Partial<CreatePortRequest>) =>
    apiClient.put<PortRequest>(`/porting/requests/${id}`, data),
  delete: (id: string) => apiClient.delete<void>(`/porting/requests/${id}`),
  submit: (id: string) => apiClient.post<void>(`/porting/requests/${id}/submit`),
  cancel: (id: string) => apiClient.post<void>(`/porting/requests/${id}/cancel`),
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
  get: (id: string) => apiClient.get<AuditEntry>(`/audit-logs/${id}`),
}

// API Tokens
export const apiTokens = {
  list: () => apiClient.get<ApiToken[]>('/api-tokens'),
  create: (data: CreateApiTokenRequest) => apiClient.post<CreateApiTokenResponse>('/api-tokens', data),
  delete: (id: string) => apiClient.delete<void>(`/api-tokens/${id}`),
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
    apiClient.get<ConfigEntry[]>(`/config/${encodeURIComponent(service)}?tenant_id=${tenant_id}`),
  get: (service: string, section: string, tenant_id = '_system') =>
    apiClient.get<ConfigEntry>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}?tenant_id=${tenant_id}`),
  update: (service: string, section: string, value: Record<string, unknown>, tenant_id = '_system', updated_by?: string) =>
    apiClient.put<UpdateConfigResponse>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}`, {
      value,
      tenant_id,
      updated_by,
    }),
  delete: (service: string, section: string, tenant_id = '_system') =>
    apiClient.delete<{ deleted: boolean }>(`/config/${encodeURIComponent(service)}/${encodeURIComponent(section)}?tenant_id=${tenant_id}`),
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
