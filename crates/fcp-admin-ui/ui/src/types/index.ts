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
