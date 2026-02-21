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
  status: () => apiClient.get<Record<string, unknown>>('/status'),
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
  list: () => apiClient.get<CallInfo[]>('/media/calls'),
  get: (callId: string) => apiClient.get<CallInfo>(`/media/calls/${callId}`),
  originate: (data: OriginateCallRequest) => apiClient.post<CallInfo>('/media/calls', data),
  hangup: (callId: string) => apiClient.delete<void>(`/media/calls/${callId}`),
  hold: (callId: string) => apiClient.post<void>(`/media/calls/${callId}/hold`),
  resume: (callId: string) => apiClient.post<void>(`/media/calls/${callId}/resume`),
  transfer: (callId: string, target: string) =>
    apiClient.post<void>(`/media/calls/${callId}/transfer`, { target }),
  mute: (callId: string) => apiClient.post<void>(`/media/calls/${callId}/mute`),
  unmute: (callId: string) => apiClient.post<void>(`/media/calls/${callId}/unmute`),
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
