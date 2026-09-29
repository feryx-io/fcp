/** The API's error body (docs/API_TOKENS.md §6). */
export interface ErrorBody {
  error: string
  code: string
  details?: string | null
}

/** A call the API refused: its status, stable code, detail and request id. */
export class FcpError extends Error {
  readonly status: number
  /** `not_found`, `forbidden`, `rate_limited`, … */
  readonly code: string
  readonly details?: string
  /** The `X-Request-Id` to quote when asking why. */
  readonly requestId?: string
  readonly body: unknown

  constructor(status: number, body: unknown, requestId?: string) {
    const e = (body ?? {}) as Partial<ErrorBody>
    super(`${status} ${e.code ?? 'error'}: ${e.details ?? e.error ?? 'request failed'}`)
    this.name = 'FcpError'
    this.status = status
    this.code = e.code ?? `http_${status}`
    this.details = e.details ?? undefined
    this.requestId = requestId
    this.body = body
  }

  static from(response: Response, body: unknown): FcpError {
    return new FcpError(response.status, body, response.headers.get('x-request-id') ?? undefined)
  }
}
