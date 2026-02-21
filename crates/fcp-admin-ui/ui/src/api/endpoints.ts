import { apiClient } from './client'
import type {
  User, CreateUserRequest, UpdateUserRequest,
  Tenant, CreateTenantRequest,
  Extension, CreateExtensionRequest,
  Device, CreateDeviceRequest,
  PhoneNumber, CreatePhoneNumberRequest,
  LineBinding, CreateLineBindingRequest,
  CallDetailRecord, ActiveCall,
  DashboardSummary, SystemHealth,
  LoginRequest, LoginResponse, AuthUser,
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

// Calls
export const calls = {
  list: () => apiClient.get<ActiveCall[]>('/calls'),
  get: (callId: string) => apiClient.get<ActiveCall>(`/calls/${callId}`),
  hangup: (callId: string) => apiClient.delete<void>(`/calls/${callId}`),
}

// CDRs
export const cdrs = {
  list: (params?: Record<string, string>) => {
    const query = params ? '?' + new URLSearchParams(params).toString() : ''
    return apiClient.get<CallDetailRecord[]>(`/cdrs${query}`)
  },
  get: (callId: string) => apiClient.get<CallDetailRecord>(`/cdrs/${callId}`),
}
