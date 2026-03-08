// Ephemeral in-memory store for expired-password flow.
// Never persisted to storage — lives only in JS heap for the current page lifecycle.

interface ExpiredPasswordState {
  username: string
  currentPassword: string
}

let pending: ExpiredPasswordState | null = null

export function setExpiredPasswordState(state: ExpiredPasswordState) {
  pending = state
}

/** Consume the state (returns it once, then clears). */
export function consumeExpiredPasswordState(): ExpiredPasswordState | null {
  const s = pending
  pending = null
  return s
}
