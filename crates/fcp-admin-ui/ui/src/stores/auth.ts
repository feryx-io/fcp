import { create } from 'zustand'
import type { AuthUser } from '@/types'
import { apiClient } from '@/api/client'
import { auth } from '@/api/endpoints'

/**
 * Who is signed in (docs/IDENTITY_AND_DEVICES.md phase 1).
 *
 * The access token lives only in memory, and lasts minutes. The session's
 * refresh secret is an HttpOnly cookie script cannot read: a reload asks
 * `/auth/refresh` for a new access token, and so does a timer a minute
 * before the token runs out, and a request refused with 401.
 */
interface AuthState {
  token: string | null
  user: AuthUser | null
  isAuthenticated: boolean
  /** Asking the cookie whether a session is still there, at start. */
  restoring: boolean
  login: (token: string, user: AuthUser, expiresIn?: number | null) => void
  logout: () => void
  setUser: (user: AuthUser) => void
  /** A new access token from the cookie; false when the session has ended. */
  refresh: () => Promise<boolean>
  /** At start: signed in again if the cookie names a live session. */
  restore: () => Promise<void>
}

let timer: ReturnType<typeof setTimeout> | null = null
let inFlight: Promise<boolean> | null = null

function schedule(expiresIn: number | null | undefined) {
  if (timer) clearTimeout(timer)
  timer = null
  if (!expiresIn) return
  const lead = Math.max(expiresIn - 60, 10)
  timer = setTimeout(() => {
    void useAuthStore.getState().refresh()
  }, lead * 1000)
}

function signedOut(set: (s: Partial<AuthState>) => void) {
  if (timer) clearTimeout(timer)
  timer = null
  apiClient.setToken(null)
  set({ token: null, user: null, isAuthenticated: false })
}

export const useAuthStore = create<AuthState>()(
  (set, get) => ({
    token: null,
    user: null,
    isAuthenticated: false,
    restoring: true,
    login: (token: string, user: AuthUser, expiresIn?: number | null) => {
      apiClient.setToken(token)
      schedule(expiresIn)
      set({ token, user, isAuthenticated: true, restoring: false })
    },
    logout: () => {
      // The server ends the session and clears the cookie.
      auth.logout().catch(() => {})
      signedOut(set)
    },
    setUser: (user: AuthUser) => {
      set({ user })
    },
    refresh: () => {
      // One refresh at a time: a burst of 401s shares it.
      if (inFlight) return inFlight
      inFlight = auth
        .refresh()
        .then((response) => {
          // A refresh always answers a token; anything else is signed out.
          if (!response.token) throw new Error('no token')
          get().login(response.token, response.user, response.expires_in)
          return true
        })
        .catch(() => {
          signedOut(set)
          return false
        })
        .finally(() => {
          inFlight = null
        })
      return inFlight
    },
    restore: async () => {
      await get().refresh()
      set({ restoring: false })
    },
  }),
)

apiClient.setRefresher(() => useAuthStore.getState().refresh())

// Clear any legacy storage entries left from previous versions
try { sessionStorage.removeItem('fcp-auth-token') } catch {}
try { sessionStorage.removeItem('fcp-auth-user') } catch {}
try { localStorage.removeItem('fcp-auth') } catch {}
