import { create } from 'zustand'
import { persist } from 'zustand/middleware'
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
  persist(
    (set) => ({
      token: null,
      user: null,
      isAuthenticated: false,
      login: (token: string, user: AuthUser) => {
        apiClient.setToken(token)
        set({ token, user, isAuthenticated: true })
      },
      logout: () => {
        apiClient.setToken(null)
        set({ token: null, user: null, isAuthenticated: false })
      },
      setUser: (user: AuthUser) => set({ user }),
    }),
    {
      name: 'fcp-auth',
    },
  ),
)

// Restore token on page load from persisted state
const initialState = useAuthStore.getState()
if (initialState.token) {
  apiClient.setToken(initialState.token)
}
