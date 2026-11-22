import type { components } from '@/api/schema'
import { apiClient, ApiRequestError } from './client'
import type {
  ParkingLot, ParkingLotRequest, ParkedCall,
  Brand, BrandAsset, BrandRequest,
  DiscoverResponse,
  IdentityProvider,
  IdentityProviderRequest,
  ProviderCheck,
  RecoveryCodes,
  TotpEnrollment,
  TotpStatus,
  User,
  Group,
  GroupRequest,
  GroupMember,
  ProvisioningKey,
  DeviceFetch,
  CaptureResponse,
  CaptureAllowResponse,
  CaptureAllowRequest,
  PendingDevice,
  ClaimDeviceRequest,
  RenderedConfig,
  RenderDeviceRequest,
  ConfigTemplate,
  CreateConfigTemplateRequest,
  UpdateConfigTemplateRequest,
  LibraryTemplate,
  PreviewTemplateResponse,
  RedirectionAccount,
  PutRedirectionAccountRequest,
  DeviceRedirection,
  FirmwareImage,
  FirmwareTarget,
  FirmwareTargetRequest,
  PurgeResponse,
  CreateUserRequest,
  UpdateUserRequest,
  Tenant,
  CreateTenantRequest,
  Extension,
  CreateExtensionRequest,
  UpdateExtensionRequest,
  Device,
  CreateDeviceRequest,
  HotelingSession,
  UpdateDeviceRequest,
  Location,
  CreateLocationRequest,
  PhoneNumber,
  CreatePhoneNumberRequest,
  LineBinding,
  CreateLineBindingRequest,
  UpdateLineBindingRequest,
  DashboardSummary,
  SystemHealth,
  LoginRequest,
  LoginResponse,
  AuthUser,
  ChangeExpiredPasswordRequest,
  CallInfo,
  CdrResponse,
  CallQualityResponse,
  ListCdrsResponse,
  CdrQueryParams,
  QueueResponse,
  QueueStatsResponse,
  AgentResponse,
  AgentLoginRequest,
  QueueConfigResponse,
  CreateQueueConfigRequest,
  MailboxResponse,
  VoicemailMessage,
  MailboxStats,
  CreateMailboxRequest,
  UpdateMailboxRequest,
  PresenceResponse,
  PresenceStats,
  Registration,
  SipTrunk,
  CreateTrunkRequest,
  TrunkStatsResponse,
  TrunkSummary,
  TrunkDetail,
  QualityMetrics,
  RoutingRule,
  CreateRoutingRuleRequest,
  RoutePreview,
  LcrRoute,
  CreateLcrRouteRequest,
  FlowResponse,
  CreateFlowRequest,
  UpdateFlowRequest,
  ListFlowsQuery,
  SystemStatus,
  AuditEntry,
  AuditQueryParams,
  DeletionEntry,
  DeletionQueryParams,
  EmergencyConfig,
  UpdateEmergencyConfigRequest,
  EmergencyReadiness,
  EmergencyNetwork,
  CreateEmergencyNetworkRequest,
  EmergencyCall,
  EmergencyNotification,
  AddressValidation,
  LocationValidationResponse,
  DeviceValidationResponse,
  NumberValidationResponse,
  E911AddressInput,
  ConferenceRecording,
  RecordingQueryParams,
  ListRecordingsResponse,
  ApiToken,
  Page,
  CreateApiTokenRequest,
  CreateApiTokenResponse,
  CreatePersonalTokenRequest,
  GrantablePermission,
  RotateApiTokenRequest,
  ClusterNodesResponse,
  ClusterStatsResponse,
  MeProfile,
  UpdateProfileRequest,
  ChangePasswordRequest,
  MeMailbox,
  MeMailboxStats,
  CallingSettings,
  MyExecutive,
  UpdateCallingRequest,
  Schedule,
  ScheduleRequest,
  ClassOfService,
  ClassOfServiceRequest,
  ClassifyResponse,
  MeetingResponse,
  CreateMeetingRequest,
  UpdateMeetingRequest,
  ClusterSitesResponse,
  SiteInfo,
  SiteWithToken,
  Webhook,
  WebhookWithSecret,
  CreateWebhookRequest,
  UpdateWebhookRequest,
  WebhookDelivery,
  WebhookDeliveryOutcome,
  WebhookEventType,
  CompliancePolicy,
  UpdateCompliancePolicy,
  RetentionRun,
  LegalHold,
  PlaceHoldRequest,
  SubjectRequest,
  FileSubjectRequest,
  UserDevice,
  PersonalContact,
  PersonalContactRequest,
  ContactImportResponse,
} from '@/types'

// Auth
export const auth = {
  /** Where a login signs in: the tenant its email domain is, and the ways in. */
  discover: (login: string) => apiClient.post<DiscoverResponse>('/auth/discover', { login }),
  login: (data: LoginRequest) => apiClient.post<LoginResponse>('/auth/login', data),
  /** The second step of a sign-in with a second factor (docs/IDENTITY_AND_DEVICES.md phase 2b). */
  loginTotp: (mfa_token: string, answer: { code?: string; recovery_code?: string }) =>
    apiClient.post<LoginResponse>('/auth/login/totp', { mfa_token, ...answer }),
  /** Setting up the second factor a tenant requires, mid sign-in. */
  enrollTotp: (mfa_token: string) => apiClient.post<TotpEnrollment>('/auth/totp/enroll', { mfa_token }),
  confirmTotp: (mfa_token: string, code: string) =>
    apiClient.post<LoginResponse>('/auth/totp/confirm', { mfa_token, code }),
  /** Where the browser goes to sign in through a provider (docs/IDENTITY_AND_DEVICES.md phase 3). */
  ssoStartUrl: (providerId: string) => `/api/v1/auth/sso/${encodeURIComponent(providerId)}/start`,
  /** A session for the one-time code the provider's sign-in landed with. */
  ssoExchange: (code: string) => apiClient.post<LoginResponse>('/auth/sso/exchange', { code }),
  /** Grant an app's sign-in (docs/SOFTPHONE.md §2.2): where to send the browser back. */
  appApprove: (request: string) =>
    apiClient.post<{ redirect_url: string; client_id: string }>('/auth/app/approve', { request }),
  /** Ends the session; a provider that signs out there too names where the browser goes next (docs/SSO.md §5). */
  logout: () => apiClient.post<{ logout_url?: string } | undefined>('/auth/logout'),
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
  list: () => apiClient.getAll<User>('/users'),
  get: (username: string) => apiClient.get<User>(`/users/${encodeURIComponent(username)}`),
  byId: (id: string) => apiClient.get<User>(`/users/by-id/${encodeURIComponent(id)}`),
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
  restore: (username: string) =>
    apiClient.post<User>(`/users/${encodeURIComponent(username)}/restore`),
  purge: (username: string) =>
    apiClient.post<PurgeResponse>(`/users/${encodeURIComponent(username)}/purge`),
  getCalling: (username: string) =>
    apiClient.get<CallingSettings>(`/users/${encodeURIComponent(username)}/calling`),
  updateCalling: (username: string, data: UpdateCallingRequest) =>
    apiClient.put<CallingSettings>(`/users/${encodeURIComponent(username)}/calling`, data),
  /** What others see of a user and the status they chose (docs/PRESENCE.md); read only. */
  getPresence: (username: string) =>
    apiClient.get<components['schemas']['UserPresenceResponse']>(
      `/users/${encodeURIComponent(username)}/presence`,
    ),
}

// Tenants
/** The default tenant: a single-tenant deployment's, listed before it has a row. */
export const DEFAULT_TENANT_ID = 'tenant-00000000-0000-0000-0000-000000000000'

export const tenants = {
  list: async () => {
    const all = await apiClient.getAll<Tenant>('/tenants')
    if (all.some((t) => t.id === DEFAULT_TENANT_ID)) return all
    try {
      return [await apiClient.get<Tenant>(`/tenants/${DEFAULT_TENANT_ID}`), ...all]
    } catch {
      return all
    }
  },
  get: (id: string) => apiClient.get<Tenant>(`/tenants/${encodeURIComponent(id)}`),
  create: (data: CreateTenantRequest) => apiClient.post<Tenant>('/tenants', data),
  update: (id: string, data: Partial<CreateTenantRequest>) =>
    apiClient.put<Tenant>(`/tenants/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/tenants/${encodeURIComponent(id)}`),
}

// Extensions
export const extensions = {
  list: () => apiClient.getAll<Extension>('/extensions'),
  get: (number: string) => apiClient.get<Extension>(`/extensions/${encodeURIComponent(number)}`),
  create: (data: CreateExtensionRequest) => apiClient.post<Extension>('/extensions', data),
  update: (number: string, data: UpdateExtensionRequest) =>
    apiClient.patch<Extension>(`/extensions/${encodeURIComponent(number)}`, data),
  delete: (number: string) => apiClient.delete<void>(`/extensions/${encodeURIComponent(number)}`),
}

// Devices
export const devices = {
  list: () => apiClient.getAll<Device>('/devices'),
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
  fetches: (id: string) =>
    apiClient.getAll<DeviceFetch>(`/devices/${encodeURIComponent(id)}/fetches`),
  /** The configuration the phone would get now, or from the template given. */
  render: (id: string, data: RenderDeviceRequest = {}) =>
    apiClient.post<RenderedConfig>(`/devices/${encodeURIComponent(id)}/render`, data),
  /** Tell the phone to fetch its configuration again (§8). */
  resync: (id: string) =>
    apiClient.post<{ device_id: string; sent: number }>(`/devices/${encodeURIComponent(id)}/resync`),
  // A test push to the device's apps (docs/PUSH.md §2.3).
  pushTest: (id: string) =>
    apiClient.post<{
      device_id: string
      results: { provider: string; token: string; outcome: string; reason?: string | null }[]
    }>(`/devices/${encodeURIComponent(id)}/push-test`),
}

// Firmware (docs/PROVISIONING.md §11)
export const firmware = {
  images: () => apiClient.getAll<FirmwareImage>('/firmware'),
  /** The platform administrator's. */
  upload: (data: { vendor: string; version: string; models: string; notes: string; file: File }) => {
    const form = new FormData()
    form.append('vendor', data.vendor)
    form.append('version', data.version)
    form.append('models', data.models)
    if (data.notes) form.append('notes', data.notes)
    form.append('file', data.file, data.file.name)
    return apiClient.uploadForm<FirmwareImage>('/firmware', form)
  },
  deleteImage: (id: string) => apiClient.delete<void>(`/firmware/${encodeURIComponent(id)}`),
  targets: () => apiClient.getAll<FirmwareTarget>('/firmware/targets'),
  createTarget: (data: FirmwareTargetRequest) => apiClient.post<FirmwareTarget>('/firmware/targets', data),
  updateTarget: (id: string, data: FirmwareTargetRequest) =>
    apiClient.put<FirmwareTarget>(`/firmware/targets/${encodeURIComponent(id)}`, data),
  deleteTarget: (id: string) => apiClient.delete<void>(`/firmware/targets/${encodeURIComponent(id)}`),
}

// Provisioning templates (docs/PROVISIONING.md §5)
export const configTemplates = {
  list: () => apiClient.getAll<ConfigTemplate>('/config-templates'),
  create: (data: CreateConfigTemplateRequest) =>
    apiClient.post<ConfigTemplate>('/config-templates', data),
  update: (id: string, data: UpdateConfigTemplateRequest) =>
    apiClient.patch<ConfigTemplate>(`/config-templates/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/config-templates/${encodeURIComponent(id)}`),
  library: () => apiClient.getAll<LibraryTemplate>('/config-templates/library'),
  fromLibrary: (vendor: string) =>
    apiClient.post<ConfigTemplate>(`/config-templates/library/${encodeURIComponent(vendor)}`),
  /** Tell every phone given this template to fetch again (§8). */
  resync: (id: string) =>
    apiClient.post<{ devices: number; sent: number }>(`/config-templates/${encodeURIComponent(id)}/resync`),
  /** A body rendered against sample values. */
  preview: (template_body: string, vendor?: string) =>
    apiClient.post<PreviewTemplateResponse>('/config-templates/preview', { template_body, vendor }),
}

export const provisioning = {
  /** The tenant's URL, or `null` before one is made. */
  getKey: async (): Promise<ProvisioningKey | null> => {
    try {
      return await apiClient.get<ProvisioningKey>('/provisioning/key')
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 404) return null
      throw e
    }
  },
  rotateKey: () => apiClient.post<ProvisioningKey>('/provisioning/key'),
  /** Phones that fetched with a MAC no device has (§6). */
  pending: () => apiClient.getAll<PendingDevice>('/provisioning/pending'),
  claim: (mac: string, data: ClaimDeviceRequest) =>
    apiClient.post<Device>(`/provisioning/pending/${encodeURIComponent(mac)}/claim`, data),
  dismiss: (mac: string) =>
    apiClient.delete<void>(`/provisioning/pending/${encodeURIComponent(mac)}`),
  /** Zero-touch: the vendors' redirection services (§10). */
  redirection: () => apiClient.getAll<RedirectionAccount>('/provisioning/redirection'),
  putRedirection: (service: string, data: PutRedirectionAccountRequest) =>
    apiClient.put<RedirectionAccount>(`/provisioning/redirection/${encodeURIComponent(service)}`, data),
  deleteRedirection: (service: string) =>
    apiClient.delete<void>(`/provisioning/redirection/${encodeURIComponent(service)}`),
  checkRedirection: (service: string) =>
    apiClient.post<{ ok: boolean; detail?: string | null }>(`/provisioning/redirection/${encodeURIComponent(service)}/check`),
  syncRedirection: (service: string) =>
    apiClient.post<{ devices: number; registered: number; failed: number }>(
      `/provisioning/redirection/${encodeURIComponent(service)}/sync`,
    ),
  registerDevice: (id: string) =>
    apiClient.post<DeviceRedirection>(`/devices/${encodeURIComponent(id)}/redirection`),
}

// Hoteling sessions (docs/ENTERPRISE_CALLING.md §3.8)
export const hoteling = {
  /** Active sessions; with `all`, the last ones whatever their state */
  list: (all = false) => apiClient.getAll<HotelingSession>(`/hoteling/sessions${all ? '?all=true' : ''}`),
  end: (id: string) => apiClient.delete<void>(`/hoteling/sessions/${encodeURIComponent(id)}`),
}

// Locations
export const locations = {
  list: () => apiClient.getAll<Location>('/locations'),
  get: (id: string) => apiClient.get<Location>(`/locations/${encodeURIComponent(id)}`),
  create: (data: CreateLocationRequest) => apiClient.post<Location>('/locations', data),
  update: (id: string, data: Partial<CreateLocationRequest>) =>
    apiClient.patch<Location>(`/locations/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/locations/${encodeURIComponent(id)}`),
}

// Phone Numbers
export const phoneNumbers = {
  list: () => apiClient.getAll<PhoneNumber>('/phone-numbers'),
  get: (id: string) => apiClient.get<PhoneNumber>(`/phone-numbers/${encodeURIComponent(id)}`),
  create: (data: CreatePhoneNumberRequest) => apiClient.post<PhoneNumber>('/phone-numbers', data),
  update: (id: string, data: Partial<CreatePhoneNumberRequest>) =>
    apiClient.patch<PhoneNumber>(`/phone-numbers/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/phone-numbers/${encodeURIComponent(id)}`),
  activate: (id: string) => apiClient.post<void>(`/phone-numbers/${encodeURIComponent(id)}/activate`),
}

// Line Bindings
export const lineBindings = {
  list: () => apiClient.getAll<LineBinding>('/line-bindings'),
  get: (id: string) => apiClient.get<LineBinding>(`/line-bindings/${encodeURIComponent(id)}`),
  create: (data: CreateLineBindingRequest) => apiClient.post<LineBinding>('/line-bindings', data),
  update: (id: string, data: UpdateLineBindingRequest) =>
    apiClient.patch<LineBinding>(`/line-bindings/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/line-bindings/${encodeURIComponent(id)}`),
}

// Calls (Phase 2 - live operations)
export const calls = {
  list: () => apiClient.getAll<CallInfo>('/calls'),
  get: (callId: string) => apiClient.get<CallInfo>(`/calls/${encodeURIComponent(callId)}`),
  /** Live call control, put to the node holding the call (docs/DEVELOPER_SURFACE.md phase 8). */
  hangup: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/hangup`),
  hold: (callId: string, party: 'caller' | 'callee' = 'caller') =>
    apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/hold`, { party }),
  resume: (callId: string) => apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/resume`),
  transfer: (callId: string, target: string, party: 'caller' | 'callee' = 'caller') =>
    apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/transfer`, { target, party }),
  /** Put a supervisor on the call: they are dialed and join when they answer (§3.11). */
  supervise: (
    callId: string,
    mode: 'monitor' | 'whisper' | 'barge',
    supervisor: string,
    heardBy?: 'a' | 'b',
  ) =>
    apiClient.post<void>(`/calls/${encodeURIComponent(callId)}/${mode}`, {
      supervisor,
      heard_by: heardBy,
    }),
  leaveSupervisor: (callId: string) =>
    apiClient.delete<void>(`/calls/${encodeURIComponent(callId)}/supervisor`),
}

// CDRs (Phase 2 - call detail records)
// Apps' captures and who may take them (docs/CALL_DIAGNOSTICS.md §3)
export const captures = {
  list: (params: { device_id?: string; username?: string } = {}) => {
    const q = new URLSearchParams()
    if (params.device_id) q.set('device_id', params.device_id)
    if (params.username) q.set('username', params.username)
    const qs = q.toString()
    return apiClient.getAll<CaptureResponse>(`/captures${qs ? `?${qs}` : ''}`)
  },
  allows: () => apiClient.getAll<CaptureAllowResponse>('/captures/allows'),
  allow: (data: CaptureAllowRequest) => apiClient.post<CaptureAllowResponse>('/captures/allows', data),
  endAllow: (id: string) => apiClient.delete<void>(`/captures/allows/${encodeURIComponent(id)}`),
  delete: (id: string) => apiClient.delete<void>(`/captures/${encodeURIComponent(id)}`),
  /** Save the capture's file; the download is in the audit log. */
  download: async (capture: CaptureResponse) => {
    const url = await apiClient.fetchBlobUrl(`/captures/${encodeURIComponent(capture.id)}/file`)
    const a = document.createElement('a')
    a.href = url
    a.download = `${capture.id}.pcapng`
    a.click()
    URL.revokeObjectURL(url)
  },
}

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
  /** Every phone's report of the call's quality. */
  quality: (callId: string) =>
    apiClient.get<Page<CallQualityResponse>>(`/cdrs/${encodeURIComponent(callId)}/quality`),
}

// SIP Trace (for ladder diagram debugging)
export type SipTraceMessage = components['schemas']['SipTraceResponse']

export type SipTraceListResponse = components['schemas']['SipTraceListResponse']

export const sipTrace = {
  getMessages: (callId: string) =>
    apiClient.get<SipTraceListResponse>(`/calls/${encodeURIComponent(callId)}/sip-messages`),
}

// Flow Execution Traces
export type FlowTraceEntry = components['schemas']['FlowTraceResponse']

export type ListFlowTracesResponse = Page<FlowTraceEntry>

export interface FlowTraceQueryParams {
  call_id?: string
  flow_id?: string
  event_type?: string
  time_from?: string
  time_to?: string
  limit?: number
  /** The `next_cursor` of the page before. */
  cursor?: string
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
    apiClient.getAll<FlowTraceEntry>(`/flow-traces/${encodeURIComponent(callId)}`),
}

// Queue Configuration (database-backed CRUD)
export const queueConfigs = {
  list: () => apiClient.getAll<QueueConfigResponse>('/queue-configs'),
  get: (id: string) => apiClient.get<QueueConfigResponse>(`/queue-configs/${encodeURIComponent(id)}`),
  create: (data: CreateQueueConfigRequest) => apiClient.post<QueueConfigResponse>('/queue-configs', data),
  update: (id: string, data: Partial<CreateQueueConfigRequest>) =>
    apiClient.patch<QueueConfigResponse>(`/queue-configs/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/queue-configs/${encodeURIComponent(id)}`),
}

// Queues (Phase 2 - ACD runtime status)
export const queues = {
  list: () => apiClient.getAll<QueueResponse>('/queues'),
  get: (name: string) => apiClient.get<QueueResponse>(`/queues/${encodeURIComponent(name)}`),
  stats: (name: string) => apiClient.get<QueueStatsResponse>(`/queues/${encodeURIComponent(name)}/stats`),
  agents: (name: string) => apiClient.getAll<AgentResponse>(`/queues/${encodeURIComponent(name)}/agents`),
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
  listMailboxes: () => apiClient.getAll<MailboxResponse>('/voicemail/mailboxes'),
  getMailbox: (id: string) => apiClient.get<MailboxResponse>(`/voicemail/mailboxes/${encodeURIComponent(id)}`),
  getMailboxStats: (id: string) => apiClient.get<MailboxStats>(`/voicemail/mailboxes/${encodeURIComponent(id)}/stats`),
  listMessages: (mailboxId: string) =>
    apiClient.getAll<VoicemailMessage>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/messages`),
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
    apiClient.getAll<import('@/types').GreetingResponse>(`/voicemail/mailboxes/${encodeURIComponent(mailboxId)}/greetings`),
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
export type PromptResponse = components['schemas']['PromptResponse']

export type CreatePromptRequest = components['schemas']['CreatePromptRequest']

export type UpdatePromptRequest = components['schemas']['UpdatePromptRequest']

export const prompts = {
  // `tenantId` names another tenant than the session's, for a platform
  // admin editing it (the tenant header).
  list: (params?: { category?: string; language?: string; tenantId?: string }) => {
    const query = new URLSearchParams()
    if (params?.category) query.set('category', params.category)
    if (params?.language) query.set('language', params.language)
    const qs = query.toString()
    return apiClient.getAll<PromptResponse>(
      `/prompts${qs ? `?${qs}` : ''}`,
      params?.tenantId ? { 'x-fcp-tenant-id': params.tenantId } : undefined,
    )
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
  // The tenant's own copy of a system prompt (docs/TENANT_SCOPE.md §3.2);
  // deleting the copy restores the system prompt.
  override: (id: string) => apiClient.post<PromptResponse>(`/prompts/${encodeURIComponent(id)}/override`, {}),
}

// Presence (Phase 2)
export const presence = {
  list: () => apiClient.getAll<PresenceResponse>('/presence'),
  get: (aor: string) => apiClient.get<PresenceResponse>(`/presence/${encodeURIComponent(aor)}`),
  stats: () => apiClient.get<PresenceStats>('/presence/stats'),
}

// Registrations
export const registrations = {
  list: () => apiClient.getAll<Registration>('/registrations'),
  get: (aor: string) => apiClient.get<Registration>(`/registrations/${encodeURIComponent(aor)}`),
  delete: (aor: string) => apiClient.delete<void>(`/registrations/${encodeURIComponent(aor)}`),
}

// --- Phase 3: Advanced Features ---

// SIP Trunks (database-backed CRUD)
export const trunks = {
  list: () => apiClient.getAll<SipTrunk>('/trunks'),
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
  /** What a live call manager would do with this tenant's call to `to`. */
  preview: (to: string, from?: string, locationId?: string) => {
    const params = new URLSearchParams({ to })
    if (from) params.set('from', from)
    if (locationId) params.set('location_id', locationId)
    return apiClient.get<RoutePreview>(`/routing/preview?${params.toString()}`)
  },
  list: () => apiClient.getAll<RoutingRule>('/routing/rules'),
  get: (id: string) => apiClient.get<RoutingRule>(`/routing/rules/${encodeURIComponent(id)}`),
  create: (data: CreateRoutingRuleRequest) => apiClient.post<RoutingRule>('/routing/rules', data),
  update: (id: string, data: Partial<CreateRoutingRuleRequest>) =>
    apiClient.patch<RoutingRule>(`/routing/rules/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/routing/rules/${encodeURIComponent(id)}`),
}

// Least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c)
export const lcrRoutes = {
  list: () => apiClient.getAll<LcrRoute>('/lcr-routes'),
  create: (data: CreateLcrRouteRequest) => apiClient.post<LcrRoute>('/lcr-routes', data),
  update: (id: string, data: Partial<CreateLcrRouteRequest>) =>
    apiClient.patch<LcrRoute>(`/lcr-routes/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/lcr-routes/${encodeURIComponent(id)}`),
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
    return apiClient.getAll<FlowResponse>(`/flows${query ? `?${query}` : ''}`)
  },
  get: (id: string) => apiClient.get<FlowResponse>(`/flows/${encodeURIComponent(id)}`),
  create: (data: CreateFlowRequest) => apiClient.post<FlowResponse>('/flows', data),
  update: (id: string, data: UpdateFlowRequest) =>
    apiClient.put<FlowResponse>(`/flows/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/flows/${encodeURIComponent(id)}`),
  versions: (id: string) => apiClient.getAll<FlowResponse>(`/flows/${encodeURIComponent(id)}/versions`),
  // Flows from before tenancy that nothing tied to one tenant, for a
  // platform administrator to assign (docs/TENANT_SCOPE.md §3.4).
  untenanted: () => apiClient.getAll<FlowResponse>('/flows/untenanted'),
  assign: (id: string, tenantId: string) =>
    apiClient.post<FlowResponse>(`/flows/${encodeURIComponent(id)}/assign`, { tenant_id: tenantId }),
}

// Sites: a tenant's own stack on its network, enrolled to this cloud
// (docs/HYBRID_CONTROL_PLANE.md). A platform admin acting in a pinned
// tenant names it in the tenant header; a tenant admin's own tenant is
// implied.
const tenantHeader = (tenantId?: string): Record<string, string> | undefined =>
  tenantId ? { 'x-fcp-tenant-id': tenantId } : undefined
export const schedules = {
  list: () => apiClient.getAll<Schedule>('/schedules'),
  create: (data: ScheduleRequest) => apiClient.post<Schedule>('/schedules', data),
  update: (id: string, data: ScheduleRequest) =>
    apiClient.put<Schedule>(`/schedules/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/schedules/${encodeURIComponent(id)}`),
}

export const parking = {
  lots: () => apiClient.getAll<ParkingLot>('/parking/lots'),
  createLot: (data: ParkingLotRequest) => apiClient.post<ParkingLot>('/parking/lots', data),
  updateLot: (id: string, data: ParkingLotRequest) =>
    apiClient.put<ParkingLot>(`/parking/lots/${encodeURIComponent(id)}`, data),
  deleteLot: (id: string) => apiClient.delete<void>(`/parking/lots/${encodeURIComponent(id)}`),
  /** The calls parked now, on every call manager. */
  calls: () => apiClient.getAll<ParkedCall>('/parking/calls'),
  /** Ring `target` (an extension or a user) with the call in the slot. */
  retrieve: (slot: string, target: string) =>
    apiClient.post<unknown>(`/parking/calls/${encodeURIComponent(slot)}/retrieve`, { target }),
  hangUp: (slot: string) => apiClient.delete<unknown>(`/parking/calls/${encodeURIComponent(slot)}`),
}

export const groups = {
  list: () => apiClient.getAll<Group>('/groups'),
  create: (data: GroupRequest) => apiClient.post<Group>('/groups', data),
  update: (id: string, data: GroupRequest) =>
    apiClient.put<Group>(`/groups/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/groups/${encodeURIComponent(id)}`),
  members: (id: string) =>
    apiClient.getAll<GroupMember>(`/groups/${encodeURIComponent(id)}/members`),
  addMember: (id: string, username: string) =>
    apiClient.post<void>(`/groups/${encodeURIComponent(id)}/members`, { username }),
  removeMember: (id: string, username: string) =>
    apiClient.delete<void>(
      `/groups/${encodeURIComponent(id)}/members/${encodeURIComponent(username)}`,
    ),
}

export const classesOfService = {
  list: () => apiClient.getAll<ClassOfService>('/classes-of-service'),
  create: (data: ClassOfServiceRequest) => apiClient.post<ClassOfService>('/classes-of-service', data),
  update: (id: string, data: ClassOfServiceRequest) =>
    apiClient.put<ClassOfService>(`/classes-of-service/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/classes-of-service/${encodeURIComponent(id)}`),
  classify: (dialed: string) =>
    apiClient.get<ClassifyResponse>(`/classes-of-service/classify?dialed=${encodeURIComponent(dialed)}`),
}

export const sites = {
  list: (tenantId?: string) => apiClient.getAll<SiteInfo>('/sites', tenantHeader(tenantId)),
  get: (id: string, tenantId?: string) =>
    apiClient.get<SiteInfo>(`/sites/${encodeURIComponent(id)}`, tenantHeader(tenantId)),
  create: (data: { name: string }, tenantId?: string) =>
    apiClient.post<SiteWithToken>('/sites', data, tenantHeader(tenantId)),
  newToken: (id: string, tenantId?: string) =>
    apiClient.post<SiteWithToken>(`/sites/${encodeURIComponent(id)}/enrollment-token`, undefined, tenantHeader(tenantId)),
  disable: (id: string, tenantId?: string) =>
    apiClient.post<SiteInfo>(`/sites/${encodeURIComponent(id)}/disable`, undefined, tenantHeader(tenantId)),
  enable: (id: string, tenantId?: string) =>
    apiClient.post<SiteInfo>(`/sites/${encodeURIComponent(id)}/enable`, undefined, tenantHeader(tenantId)),
  delete: (id: string, tenantId?: string) =>
    apiClient.delete<void>(`/sites/${encodeURIComponent(id)}`, tenantHeader(tenantId)),
}

// System / Cluster
export const cluster = {
  status: () => apiClient.get<SystemStatus>('/status'),
  health: () => apiClient.get<{ status: string; version: string }>('/health'),
  nodes: () => apiClient.get<ClusterNodesResponse>('/cluster/nodes'),
  stats: () => apiClient.get<ClusterStatsResponse>('/cluster/stats'),
  sites: () => apiClient.get<ClusterSitesResponse>('/cluster/sites'),
}

// Every running service, the platform's and its sites' (docs/SERVICES.md):
// platform administrators only.
export type ServiceView = components['schemas']['ServiceView']
export type ServicesSummary = components['schemas']['ServicesSummary']

const siteQuery = (siteId?: string | null) =>
  siteId ? `?site_id=${encodeURIComponent(siteId)}` : ''

export const services = {
  list: () => apiClient.getAll<ServiceView>('/system/services'),
  summary: () => apiClient.get<ServicesSummary>('/system/services/summary'),
  get: (id: string, siteId?: string | null) =>
    apiClient.get<ServiceView>(`/system/services/${encodeURIComponent(id)}${siteQuery(siteId)}`),
  forget: (id: string, siteId?: string | null) =>
    apiClient.delete<void>(`/system/services/${encodeURIComponent(id)}${siteQuery(siteId)}`),
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
  /** One page, newest first; `next_cursor` for the page after it. */
  list: (params?: AuditQueryParams) =>
    apiClient.getPage<AuditEntry>(`/audit-log${query(params)}`),
  /** Every entry the filters match, as a CSV or JSON Lines file: an object URL. */
  exportUrl: (format: 'csv' | 'jsonl', params?: Omit<AuditQueryParams, 'limit' | 'cursor'>) =>
    apiClient.fetchBlobUrl(`/audit-log/export${query({ ...params, format })}`),
  deletions: (params?: DeletionQueryParams) =>
    apiClient.getPage<DeletionEntry>(`/compliance/deletions${query(params)}`),
  deletionsExportUrl: (
    format: 'csv' | 'jsonl',
    params?: Omit<DeletionQueryParams, 'limit' | 'cursor'>,
  ) => apiClient.fetchBlobUrl(`/compliance/deletions/export${query({ ...params, format })}`),
}

// Emergency calling (E911), docs/E911.md
export const compliance = {
  policy: () => apiClient.get<CompliancePolicy>('/compliance/policy'),
  updatePolicy: (data: UpdateCompliancePolicy) =>
    apiClient.put<CompliancePolicy>('/compliance/policy', data),
  runRetention: () => apiClient.post<RetentionRun>('/compliance/retention/run', {}),
  retentionRuns: () =>
    apiClient.getPage<RetentionRun>('/compliance/retention/runs?limit=50').then((p) => p.data),
  holds: (all: boolean) => apiClient.getAll<LegalHold>(`/compliance/holds${all ? '?all=true' : ''}`),
  hold: (id: string) => apiClient.get<LegalHold>(`/compliance/holds/${encodeURIComponent(id)}`),
  placeHold: (data: PlaceHoldRequest) => apiClient.post<LegalHold>('/compliance/holds', data),
  releaseHold: (id: string, reason: string) =>
    apiClient.post<LegalHold>(`/compliance/holds/${encodeURIComponent(id)}/release`, { reason }),
  subjectRequests: () => apiClient.getAll<SubjectRequest>('/compliance/subject-requests'),
  subjectRequest: (id: string) =>
    apiClient.get<SubjectRequest>(`/compliance/subject-requests/${encodeURIComponent(id)}`),
  fileSubjectRequest: (data: FileSubjectRequest) =>
    apiClient.post<SubjectRequest>('/compliance/subject-requests', data),
  confirmErasure: (id: string) =>
    apiClient.post<SubjectRequest>(`/compliance/subject-requests/${encodeURIComponent(id)}/confirm`, {}),
  /** The export's bundle (a gzipped tar), as an object URL; each download is audited. */
  bundleUrl: (id: string) =>
    apiClient.fetchBlobUrl(`/compliance/subject-requests/${encodeURIComponent(id)}/bundle`),
}

export const emergency = {
  config: () => apiClient.get<EmergencyConfig>('/emergency/config'),
  updateConfig: (data: UpdateEmergencyConfigRequest) =>
    apiClient.put<EmergencyConfig>('/emergency/config', data),
  readiness: () => apiClient.get<EmergencyReadiness>('/emergency/readiness'),
  networks: () => apiClient.getAll<EmergencyNetwork>('/emergency/networks'),
  createNetwork: (data: CreateEmergencyNetworkRequest) =>
    apiClient.post<EmergencyNetwork>('/emergency/networks', data),
  updateNetwork: (id: string, data: Partial<CreateEmergencyNetworkRequest>) =>
    apiClient.patch<EmergencyNetwork>(`/emergency/networks/${encodeURIComponent(id)}`, data),
  deleteNetwork: (id: string) =>
    apiClient.delete<void>(`/emergency/networks/${encodeURIComponent(id)}`),
  calls: (limit = 100) =>
    apiClient.getPage<EmergencyCall>(`/emergency/calls?limit=${limit}`).then((p) => p.data),
  notifications: (callId: string) =>
    apiClient.getAll<EmergencyNotification>(
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
  list: () => apiClient.getAll<ApiToken>('/api-tokens'),
  permissions: () => apiClient.getAll<GrantablePermission>('/api-tokens/permissions'),
  create: (data: CreateApiTokenRequest) => apiClient.post<CreateApiTokenResponse>('/api-tokens', data),
  rotate: (id: string, data: RotateApiTokenRequest = {}) =>
    apiClient.post<CreateApiTokenResponse>(`/api-tokens/${encodeURIComponent(id)}/rotate`, data),
  /** Revokes: the token is kept, marked, and refused. */
  delete: (id: string) => apiClient.delete<void>(`/api-tokens/${encodeURIComponent(id)}`),
}

/** The tenant's brand for its softphones (docs/BRANDING.md). */
export const branding = {
  /** `null` when the tenant has none. */
  get: async (): Promise<Brand | null> => {
    try {
      return await apiClient.get<Brand>('/branding')
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 404) return null
      throw e
    }
  },
  put: (data: BrandRequest) => apiClient.put<Brand>('/branding', data),
  remove: () => apiClient.delete<void>('/branding'),
  assets: () => apiClient.getAll<BrandAsset>('/branding/assets'),
  upload: (file: File) =>
    apiClient.uploadRaw<BrandAsset>(`/branding/assets?name=${encodeURIComponent(file.name)}`, file),
  deleteAsset: (sha256: string) => apiClient.delete<void>(`/branding/assets/${encodeURIComponent(sha256)}`),
  /** The softphone's deployment profile (`profile.json`), with `query`'s choices. */
  deploymentProfile: (query: string) =>
    apiClient.get<Record<string, unknown>>(`/branding/deployment-profile?format=json${query}`),
  /** The profile as a .zip, or its settings as a Windows policy .reg: an object URL. */
  deploymentFile: (format: 'zip' | 'reg', query: string) =>
    apiClient.fetchBlobUrl(`/branding/deployment-profile?format=${format}${query}`),
}

export const identityProviders = {
  list: () => apiClient.getAll<IdentityProvider>('/identity-providers'),
  create: (data: IdentityProviderRequest) => apiClient.post<IdentityProvider>('/identity-providers', data),
  update: (id: string, data: IdentityProviderRequest) =>
    apiClient.put<IdentityProvider>(`/identity-providers/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/identity-providers/${encodeURIComponent(id)}`),
  check: (id: string) => apiClient.post<ProviderCheck>(`/identity-providers/${encodeURIComponent(id)}/check`, {}),
  /** SAML: a new key of FCP's own; the provider must import FCP's metadata again. */
  rotateSpKey: (id: string) => apiClient.post<IdentityProvider>(`/identity-providers/${encodeURIComponent(id)}/sp-key`, {}),
  /** SAML: fetch the provider's metadata again from its URL. */
  refreshMetadata: (id: string) =>
    apiClient.post<IdentityProvider>(`/identity-providers/${encodeURIComponent(id)}/refresh-metadata`, {}),
}

export const webhooks = {
  list: () => apiClient.getAll<Webhook>('/webhooks'),
  eventTypes: () => apiClient.getAll<WebhookEventType>('/webhooks/event-types'),
  create: (data: CreateWebhookRequest) => apiClient.post<WebhookWithSecret>('/webhooks', data),
  update: (id: string, data: UpdateWebhookRequest) =>
    apiClient.patch<Webhook>(`/webhooks/${encodeURIComponent(id)}`, data),
  delete: (id: string) => apiClient.delete<void>(`/webhooks/${encodeURIComponent(id)}`),
  /** A new secret; the old one also signs for `overlap_seconds` (default a day). */
  rotate: (id: string, overlap_seconds?: number) =>
    apiClient.post<WebhookWithSecret>(`/webhooks/${encodeURIComponent(id)}/rotate-secret`, { overlap_seconds }),
  test: (id: string) =>
    apiClient.post<WebhookDeliveryOutcome>(`/webhooks/${encodeURIComponent(id)}/test`, {}),
  deliveries: (id: string) =>
    apiClient.getPage<WebhookDelivery>(`/webhooks/${encodeURIComponent(id)}/deliveries?limit=50`),
  replay: (id: string, deliveryId: string) =>
    apiClient.post<WebhookDeliveryOutcome>(
      `/webhooks/${encodeURIComponent(id)}/deliveries/${encodeURIComponent(deliveryId)}/replay`,
    ),
}

// Configuration Management
export type ConfigEntry = components['schemas']['ConfigEntryResponse']

export type ConfigAuditEntry = components['schemas']['AuditEntryResponse']

export type UpdateConfigResponse = components['schemas']['UpdateConfigResponse']

export type ValidateConfigResponse = components['schemas']['ValidateConfigResponse']

export const config = {
  list: (service?: string, tenant_id?: string) => {
    const params = new URLSearchParams()
    if (service) params.set('service', service)
    if (tenant_id) params.set('tenant_id', tenant_id)
    const q = params.toString()
    return apiClient.getAll<ConfigEntry>(`/config${q ? `?${q}` : ''}`)
  },
  listService: (service: string, tenant_id = '_system') =>
    apiClient.getAll<ConfigEntry>(`/config/${encodeURIComponent(service)}?tenant_id=${encodeURIComponent(tenant_id)}`),
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
    return apiClient
      .getPage<ConfigAuditEntry>(`/config/audit?${params.toString()}`)
      .then((p) => p.data)
  },
  /** One page of the configuration's changes: every service's without `service`. */
  auditPage: (params: { service?: string; section?: string; limit?: number; cursor?: string }) =>
    apiClient.getPage<ConfigAuditEntry>(`/config/audit${query(params)}`),
}

// Self-service (end-user portal)
export const me = {
  // Contacts: the user's own, shared with every app they sign in to
  // (docs/RECORD_AND_CONTACTS.md §3)
  listContacts: () => apiClient.getAll<PersonalContact>('/me/contacts'),
  createContact: (data: PersonalContactRequest) =>
    apiClient.post<PersonalContact>('/me/contacts', data),
  updateContact: (id: string, data: PersonalContactRequest) =>
    apiClient.put<PersonalContact>(`/me/contacts/${encodeURIComponent(id)}`, data),
  deleteContact: (id: string) => apiClient.delete<void>(`/me/contacts/${encodeURIComponent(id)}`),
  /** The contacts as a vCard or CSV file: an object URL. */
  exportContacts: (format: 'vcard' | 'csv') =>
    apiClient.fetchBlobUrl(`/me/contacts/export?format=${format}`),
  /** Contacts from a vCard or CSV file, kept on the account. */
  importContacts: (file: Blob, format: 'vcard' | 'csv') =>
    apiClient.uploadRaw<ContactImportResponse>(`/me/contacts/import?format=${format}`, file),
  // Devices: phones and app installs (docs/SOFTPHONE.md §2.1)
  listDevices: () => apiClient.getAll<UserDevice>('/me/devices'),
  removeDevice: (id: string) => apiClient.delete<void>(`/me/devices/${encodeURIComponent(id)}`),
  // Profile
  listApiTokens: () => apiClient.getAll<ApiToken>('/me/api-tokens'),
  createApiToken: (data: CreatePersonalTokenRequest) =>
    apiClient.post<CreateApiTokenResponse>('/me/api-tokens', data),
  rotateApiToken: (id: string, data: RotateApiTokenRequest = {}) =>
    apiClient.post<CreateApiTokenResponse>(`/me/api-tokens/${encodeURIComponent(id)}/rotate`, data),
  revokeApiToken: (id: string) => apiClient.delete<void>(`/me/api-tokens/${encodeURIComponent(id)}`),
  getProfile: () => apiClient.get<MeProfile>('/me/profile'),
  updateProfile: (data: UpdateProfileRequest) => apiClient.put<MeProfile>('/me/profile', data),
  changePassword: (data: ChangePasswordRequest) => apiClient.put<void>('/me/password', data),
  // A second factor
  totpStatus: () => apiClient.get<TotpStatus>('/me/totp'),
  enrollTotp: () => apiClient.post<TotpEnrollment>('/me/totp/enroll'),
  confirmTotp: (code: string) => apiClient.post<RecoveryCodes>('/me/totp/confirm', { code }),
  disableTotp: (code: string) => apiClient.post<void>('/me/totp/disable', { code }),
  renewRecoveryCodes: (code: string) => apiClient.post<RecoveryCodes>('/me/totp/recovery-codes', { code }),

  // Voicemail
  /** `null` when the user has no mailbox. */
  getMailbox: async (): Promise<MeMailbox | null> => {
    try {
      return await apiClient.get<MeMailbox>('/me/voicemail')
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 404) return null
      throw e
    }
  },
  updateMailbox: (data: import('@/types').UpdateMeVoicemailRequest) =>
    apiClient.put<MeMailbox>('/me/voicemail', data),
  getMailboxStats: () => apiClient.get<MeMailboxStats>('/me/voicemail/stats'),
  listMessages: () => apiClient.getAll<VoicemailMessage>('/me/voicemail/messages'),
  updateMessage: (id: string, data: { status?: string }) =>
    apiClient.put<VoicemailMessage>(`/me/voicemail/messages/${encodeURIComponent(id)}`, data),
  deleteMessage: (id: string) => apiClient.delete<void>(`/me/voicemail/messages/${encodeURIComponent(id)}`),
  fetchMessageAudio: (id: string): Promise<string> =>
    apiClient.fetchBlobUrl(`/me/voicemail/messages/${encodeURIComponent(id)}/audio`),

  // Greetings
  listGreetings: () =>
    apiClient.getAll<import('@/types').GreetingResponse>('/me/voicemail/greetings'),
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

  // Calling settings (docs/ENTERPRISE_CALLING.md §3.1)
  getCalling: () => apiClient.get<CallingSettings>('/me/calling'),
  updateCalling: (data: UpdateCallingRequest) => apiClient.put<CallingSettings>('/me/calling', data),
  /** The executives the user assists, and whether they take their calls (§3.10). */
  getExecutives: () => apiClient.getAll<MyExecutive>('/me/executives'),
  setExecutive: (userId: string, optedIn: boolean) =>
    apiClient.put<void>(`/me/executives/${encodeURIComponent(userId)}`, { opted_in: optedIn }),

  // Conferences
  listConferences: () => apiClient.getAll<MeetingResponse>('/me/conferences'),
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
  listMeetings: () => apiClient.getAll<MeetingResponse>('/conferences/meetings'),
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
export type MintJoinTokenRequest = components['schemas']['MintJoinTokenRequest']

/** A minted browser join token and, when the gateway URL is configured, its link. */
export type JoinTokenResponse = components['schemas']['JoinTokenResponse']

export const joinTokens = {
  /** `POST /conferences/meetings/{id}/join-tokens` — a browser join link for a meeting (admins). */
  mint: (id: string, body: MintJoinTokenRequest) =>
    apiClient.post<JoinTokenResponse>(`/conferences/meetings/${encodeURIComponent(id)}/join-tokens`, body),
}

// A webinar's life, registrations and attendance (`docs/WEBINARS.md` §2.6).
export type RegistrantResponse = components['schemas']['RegistrantResponse']
export type RegisteredResponse = components['schemas']['RegisteredResponse']
export type RegisterRequest = components['schemas']['RegisterRequest']
export type SittingResponse = components['schemas']['SittingResponse']
export type AttendanceResponse = components['schemas']['AttendanceResponse']
export type StreamStateResponse = components['schemas']['StreamStateResponse']
export type StreamLinkResponse = components['schemas']['StreamLinkResponse']
export type RtmpDestinationResponse = components['schemas']['RtmpDestinationResponse']
export type RtmpDestinationRequest = components['schemas']['RtmpDestinationRequest']

/** A webinar's push over RTMP as its node tells it (W7). */
export interface RtmpPush {
  state: 'connecting' | 'live' | 'retrying' | 'stopped'
  since: string
  error?: string
  bytes_sent: number
  connects: number
  dropped_frames: number
  ingest: string
}

/** The room's sitting as its node tells it. */
export interface LiveSitting {
  id: string
  kind: string
  phase: 'practice' | 'live' | 'ended'
  started_at: string
  live_at: string | null
  lobby: number
  /** The push over RTMP, while there is one. */
  rtmp?: RtmpPush
}

/** One attendee as the node lists them. */
export interface AudienceMember {
  participant_id: string
  display_name: string | null
  caller_id: string | null
  registrant: string | null
  joined_at: string
  hand: string | null
  lobby: boolean
  on_stage: boolean
}

export interface AudiencePage {
  data: AudienceMember[]
  next_cursor: string | null
  total: number
}

const meetingPath = (id: string) => `/conferences/meetings/${encodeURIComponent(id)}`

export const webinars = {
  /** The room's sitting now; 404 while nobody is in it. */
  sitting: (room: string) => apiClient.get<LiveSitting>(`${liveRoom(room)}/webinar`),
  goLive: (room: string) =>
    apiClient.post<{ changed: boolean; sitting: LiveSitting | null }>(`${liveRoom(room)}/webinar/live`),
  end: (room: string) => apiClient.post<{ released: number }>(`${liveRoom(room)}/webinar/end`),
  audience: (room: string, cursor?: string | null, limit = 50) =>
    apiClient.get<AudiencePage>(
      `${liveRoom(room)}/audience?limit=${limit}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`,
    ),
  registrants: (id: string) => apiClient.getAll<RegistrantResponse>(`${meetingPath(id)}/registrants`),
  register: (id: string, body: RegisterRequest) =>
    apiClient.post<RegisteredResponse>(`${meetingPath(id)}/registrants`, body),
  revoke: (id: string, rid: string) =>
    apiClient.delete<RegistrantResponse>(`${meetingPath(id)}/registrants/${encodeURIComponent(rid)}`),
  token: (id: string, rid: string) =>
    apiClient.post<JoinTokenResponse>(`${meetingPath(id)}/registrants/${encodeURIComponent(rid)}/token`, {}),
  /** The webinar's stream out (§2.8): off, waiting, live or ended. */
  stream: (id: string) => apiClient.get<StreamStateResponse>(`${meetingPath(id)}/stream`),
  /** Where the webinar is pushed over RTMP; the key is never returned. */
  rtmp: (id: string) => apiClient.get<RtmpDestinationResponse>(`${meetingPath(id)}/stream/rtmp`),
  setRtmp: (id: string, body: RtmpDestinationRequest) =>
    apiClient.put<RtmpDestinationResponse>(`${meetingPath(id)}/stream/rtmp`, body),
  deleteRtmp: (id: string) => apiClient.delete<void>(`${meetingPath(id)}/stream/rtmp`),
  /** A registrant's own link and watch page (W8), for `ttlSecs` (a week
   * unless set); their watching is counted under it. */
  registrantWatchLink: (id: string, rid: string, ttlSecs?: number) =>
    apiClient.post<StreamLinkResponse>(
      `${meetingPath(id)}/registrants/${encodeURIComponent(rid)}/stream-link`,
      { ttl_secs: ttlSecs },
    ),
  /** A link to watch the stream, for `ttlSecs` (a day unless set). */
  streamLink: (id: string, ttlSecs?: number) =>
    apiClient.post<StreamLinkResponse>(`${meetingPath(id)}/stream/links`, { ttl_secs: ttlSecs }),
  sittings: (id: string) => apiClient.getAll<SittingResponse>(`${meetingPath(id)}/sittings`),
  attendance: (id: string, sitting?: string) =>
    apiClient.get<AttendanceResponse>(
      `${meetingPath(id)}/attendance${sitting ? `?sitting=${encodeURIComponent(sitting)}` : ''}`,
    ),
}
