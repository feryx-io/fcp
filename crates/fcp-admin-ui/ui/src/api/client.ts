import type { ApiError, Page } from '@/types'

const API_BASE = '/api/v1'

export class ApiClient {
  private token: string | null = null
  private refresher: (() => Promise<boolean>) | null = null
  private tenant: string | null = null

  setToken(token: string | null) {
    this.token = token
  }

  /**
   * The tenant a platform administrator picked in the top bar, sent with
   * every request (`x-fcp-tenant-id`) so every page works in it
   * (docs/TENANT_SCOPE.md §3.1); `null` for "All tenants". The server
   * ignores it from anyone else.
   */
  setTenant(tenantId: string | null) {
    this.tenant = tenantId
  }

  /**
   * What renews the access token when a request is refused with 401: the
   * session's refresh cookie (docs/IDENTITY_AND_DEVICES.md phase 1). The
   * request is tried once more if it worked.
   */
  setRefresher(refresher: (() => Promise<boolean>) | null) {
    this.refresher = refresher
  }

  /** Get current token (for components that need it — prefer using ApiClient methods instead). */
  getToken(): string | null {
    return this.token
  }

  private authHeaders(): Record<string, string> {
    const headers: Record<string, string> = {}
    if (this.token) headers.Authorization = `Bearer ${this.token}`
    if (this.tenant) headers['x-fcp-tenant-id'] = this.tenant
    return headers
  }

  private async request<T>(
    path: string,
    options: RequestInit = {},
    retried = false,
  ): Promise<T> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...this.authHeaders(),
      ...(options.headers as Record<string, string> || {}),
    }

    const response = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
      // The refresh cookie travels to the sign-in routes, same origin only.
      credentials: 'same-origin',
    })

    if (
      response.status === 401 &&
      !retried &&
      this.token &&
      this.refresher &&
      !path.startsWith('/auth/') &&
      (await this.refresher())
    ) {
      return this.request<T>(path, options, true)
    }

    if (!response.ok) {
      let error: ApiError
      try {
        error = await response.json()
      } catch {
        error = { error: `HTTP ${response.status}`, details: response.statusText }
      }
      throw new ApiRequestError(response.status, error)
    }

    if (response.status === 204) {
      return undefined as T
    }

    return response.json()
  }

  get<T>(path: string, headers?: Record<string, string>): Promise<T> {
    return this.request<T>(path, { method: 'GET', headers })
  }

  /** One page of a list (docs/DEVELOPER_SURFACE.md §3, phase 2c). */
  getPage<T>(path: string, headers?: Record<string, string>): Promise<Page<T>> {
    return this.request<Page<T>>(path, { method: 'GET', headers })
  }

  /**
   * A whole list: every page, following `next_cursor`, a thousand at a
   * time. The admin UI shows tenant-sized lists whole; the logs that grow
   * without bound (CDRs, audit, recordings) page instead.
   */
  async getAll<T>(path: string, headers?: Record<string, string>): Promise<T[]> {
    const items: T[] = []
    const join = path.includes('?') ? '&' : '?'
    let url = `${path}${join}limit=1000`
    for (;;) {
      const page = await this.getPage<T>(url, headers)
      items.push(...page.data)
      if (!page.next_cursor) return items
      url = `${path}${join}limit=1000&cursor=${encodeURIComponent(page.next_cursor)}`
    }
  }

  post<T>(path: string, body?: unknown, headers?: Record<string, string>): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: body ? JSON.stringify(body) : undefined,
      headers,
    })
  }

  put<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'PUT',
      body: body ? JSON.stringify(body) : undefined,
    })
  }

  patch<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'PATCH',
      body: body ? JSON.stringify(body) : undefined,
    })
  }

  delete<T>(path: string, headers?: Record<string, string>): Promise<T> {
    return this.request<T>(path, { method: 'DELETE', headers })
  }

  /** Upload a FormData body (multipart). No Content-Type header — browser sets boundary. */
  async uploadForm<T>(path: string, formData: FormData, method: 'POST' | 'PUT' = 'POST'): Promise<T> {
    const response = await fetch(`${API_BASE}${path}`, {
      method,
      body: formData,
      headers: this.authHeaders(),
    })
    if (!response.ok) {
      let error: ApiError
      try {
        error = await response.json()
      } catch {
        error = { error: `HTTP ${response.status}`, details: response.statusText }
      }
      throw new ApiRequestError(response.status, error)
    }
    if (response.status === 204) return undefined as T
    return response.json()
  }

  /** Upload a file as the request body, as it is. */
  async uploadRaw<T>(path: string, file: Blob): Promise<T> {
    const response = await fetch(`${API_BASE}${path}`, {
      method: 'POST',
      body: file,
      headers: { ...this.authHeaders(), 'Content-Type': 'application/octet-stream' },
    })
    if (!response.ok) {
      let error: ApiError
      try {
        error = await response.json()
      } catch {
        error = { error: `HTTP ${response.status}`, details: response.statusText }
      }
      throw new ApiRequestError(response.status, error)
    }
    return response.json()
  }

  /** Fetch a binary resource and return an object URL. */
  async fetchBlobUrl(path: string): Promise<string> {
    const response = await fetch(`${API_BASE}${path}`, {
      headers: this.authHeaders(),
    })
    if (!response.ok) {
      // The API's error shape carries a message worth showing.
      const message = await response.json().then((b) => b?.message ?? b?.error, () => null)
      throw new Error(message ? `${message} (${response.status})` : `Failed to fetch: ${response.status}`)
    }
    const blob = await response.blob()
    return URL.createObjectURL(blob)
  }

  /** Send a DELETE with auth headers (raw fetch, no JSON parsing). */
  async deleteRaw(path: string): Promise<void> {
    const response = await fetch(`${API_BASE}${path}`, {
      method: 'DELETE',
      headers: this.authHeaders(),
    })
    if (!response.ok) {
      throw new Error(`Delete failed: ${response.status}`)
    }
  }
}

export class ApiRequestError extends Error {
  constructor(
    public status: number,
    public body: ApiError,
  ) {
    super(body.details || body.error)
    this.name = 'ApiRequestError'
  }
}

export const apiClient = new ApiClient()
