import createClient, { type Client, type ClientOptions } from 'openapi-fetch'
import type { paths } from './schema.js'
import { FcpError } from './errors.js'

/** How to reach an FCP admin API. */
export interface FcpClientOptions {
  /** The admin's address, e.g. `https://admin.example.com` (no `/api/v1`). */
  baseUrl: string
  /**
   * `Authorization: Bearer …`: an `fcp_…` API token, the static token, or a
   * session JWT (docs/API_TOKENS.md).
   */
  token: string
  /**
   * The tenant to act in, for the static token or a platform admin's
   * session (`X-FCP-Tenant-Id`). Tokens scoped to a tenant ignore it.
   */
  tenantId?: string
  /** Times to retry a 429 or a 503, waiting as `Retry-After` says (default 3). */
  retries?: number
  /** The longest wait between retries, in seconds (default 60). */
  maxRetryWaitSecs?: number
  /** The fetch to use; the global one by default. */
  fetch?: typeof fetch
}

/** A typed client over every route in the contract (openapi-fetch). */
export type FcpClient = Client<paths>

/**
 * A client for the admin API. Every method takes the path as the contract
 * writes it and is typed by it:
 *
 * ```ts
 * const fcp = createFcpClient({ baseUrl, token })
 * const user = await ok(fcp.GET('/api/v1/users/{username}', { params: { path: { username: 'alice' } } }))
 * ```
 */
export function createFcpClient(options: FcpClientOptions): FcpClient {
  const headers: Record<string, string> = { Authorization: `Bearer ${options.token}` }
  if (options.tenantId) headers['X-FCP-Tenant-Id'] = options.tenantId
  const clientOptions: ClientOptions = {
    baseUrl: options.baseUrl.replace(/\/+$/, ''),
    headers,
    fetch: retrying(
      options.fetch ?? globalThis.fetch,
      options.retries ?? 3,
      options.maxRetryWaitSecs ?? 60,
    ),
  }
  return createClient<paths>(clientOptions)
}

/**
 * The value of a call, or an `FcpError` thrown with the API's error:
 *
 * ```ts
 * const users = await ok(fcp.GET('/api/v1/users'))
 * ```
 */
export async function ok<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  const { data, error, response } = await call
  if (!response.ok || error !== undefined) {
    throw FcpError.from(response, error)
  }
  return data as T
}

/** Seconds a response asks the caller to wait, capped. */
function waitSecs(response: Response, attempt: number, cap: number): number {
  const header = response.headers.get('retry-after')
  const asked = header === null ? NaN : Number(header)
  const secs = Number.isFinite(asked) && asked >= 0 ? asked : 2 ** attempt
  return Math.min(secs, cap)
}

/**
 * A fetch that retries a 429 (rate limited) or a 503 (a service the admin
 * needs is away) up to `retries` times, waiting as `Retry-After` says.
 * Every attempt sends a fresh copy of the request, body included.
 */
export function retrying(base: typeof fetch, retries: number, cap: number): typeof fetch {
  return async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init)
    for (let attempt = 0; ; attempt++) {
      const response = await base(request.clone())
      if ((response.status !== 429 && response.status !== 503) || attempt >= retries) {
        return response
      }
      const secs = waitSecs(response, attempt, cap)
      await response.body?.cancel()
      await new Promise((resolve) => setTimeout(resolve, secs * 1000))
    }
  }
}
