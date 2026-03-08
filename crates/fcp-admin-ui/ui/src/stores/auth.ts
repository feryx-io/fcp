import { create } from 'zustand'
import type { AuthUser } from '@/types'
import { apiClient } from '@/api/client'

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
      // Persist to sessionStorage for same-tab reload survival only.
      // sessionStorage is not shared across tabs (limits blast radius)
      // and is cleared when the tab/browser closes.
      try { sessionStorage.setItem('fcp-auth-token', token) } catch {}
      try { sessionStorage.setItem('fcp-auth-user', JSON.stringify(user)) } catch {}
      set({ token, user, isAuthenticated: true })
    },
    logout: () => {
      apiClient.setToken(null)
      try { sessionStorage.removeItem('fcp-auth-token') } catch {}
      try { sessionStorage.removeItem('fcp-auth-user') } catch {}
      // Also clear legacy localStorage entry if present
      try { localStorage.removeItem('fcp-auth') } catch {}
      set({ token: null, user: null, isAuthenticated: false })
    },
    setUser: (user: AuthUser) => {
      try { sessionStorage.setItem('fcp-auth-user', JSON.stringify(user)) } catch {}
      set({ user })
    },
  }),
)

// Restore token on page load from sessionStorage (survives F5 refresh)
try {
  const savedToken = sessionStorage.getItem('fcp-auth-token')
  const savedUser = sessionStorage.getItem('fcp-auth-user')
  if (savedToken) {
    const user = savedUser ? JSON.parse(savedUser) as AuthUser : null
    apiClient.setToken(savedToken)
    useAuthStore.setState({ token: savedToken, user, isAuthenticated: true })
  }
} catch {}

// Clear any legacy localStorage token left from before this migration
try { localStorage.removeItem('fcp-auth') } catch {}
