import type { ApiError } from '@/types'

const API_BASE = '/api/v1'

export class ApiClient {
  private token: string | null = null

  setToken(token: string | null) {
    this.token = token
  }

  /** Get current token (for components that need it — prefer using ApiClient methods instead). */
  getToken(): string | null {
    return this.token
  }

  private authHeaders(): Record<string, string> {
    if (this.token) {
      return { Authorization: `Bearer ${this.token}` }
    }
    return {}
  }

  private async request<T>(
    path: string,
    options: RequestInit = {},
  ): Promise<T> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...this.authHeaders(),
      ...(options.headers as Record<string, string> || {}),
    }

    const response = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
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

    if (response.status === 204) {
      return undefined as T
    }

    return response.json()
  }

  get<T>(path: string): Promise<T> {
    return this.request<T>(path, { method: 'GET' })
  }

  post<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: body ? JSON.stringify(body) : undefined,
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

  delete<T>(path: string): Promise<T> {
    return this.request<T>(path, { method: 'DELETE' })
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

  /** Fetch a binary resource and return an object URL. */
  async fetchBlobUrl(path: string): Promise<string> {
    const response = await fetch(`${API_BASE}${path}`, {
      headers: this.authHeaders(),
    })
    if (!response.ok) {
      throw new Error(`Failed to fetch: ${response.status}`)
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
