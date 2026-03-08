import { create } from 'zustand'
import type { AuthUser } from '@/types'
import { apiClient } from '@/api/client'
import { auth } from '@/api/endpoints'

interface AuthState {
  token: string | null
  user: AuthUser | null
  isAuthenticated: boolean
  login: (token: string, user: AuthUser) => void
  logout: () => void
  setUser: (user: AuthUser) => void
}

export const useAuthStore = create<AuthState>()(
  (set) => ({
    token: null,
    user: null,
    isAuthenticated: false,
    login: (token: string, user: AuthUser) => {
      apiClient.setToken(token)
      // Token lives only in JS heap (Zustand store + apiClient).
      // No Web Storage persistence — eliminates XSS token exfiltration.
      // Trade-off: page refresh requires re-login.
      set({ token, user, isAuthenticated: true })
    },
    logout: () => {
      // Fire server-side logout (best-effort, don't block on failure)
      auth.logout().catch(() => {})
      apiClient.setToken(null)
      // Clear any legacy storage entries from previous versions
      try { sessionStorage.removeItem('fcp-auth-token') } catch {}
      try { sessionStorage.removeItem('fcp-auth-user') } catch {}
      try { localStorage.removeItem('fcp-auth') } catch {}
      set({ token: null, user: null, isAuthenticated: false })
    },
    setUser: (user: AuthUser) => {
      set({ user })
    },
  }),
)

// Clear any legacy storage entries left from previous versions
try { sessionStorage.removeItem('fcp-auth-token') } catch {}
try { sessionStorage.removeItem('fcp-auth-user') } catch {}
try { localStorage.removeItem('fcp-auth') } catch {}
