import type { ApiError } from '@/types'

const API_BASE = '/api/v1'

export class ApiClient {
  private token: string | null = null

  setToken(token: string | null) {
    this.token = token
  }

  private async request<T>(
    path: string,
    options: RequestInit = {},
  ): Promise<T> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...(options.headers as Record<string, string> || {}),
    }

    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`
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
