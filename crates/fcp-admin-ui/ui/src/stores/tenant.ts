import { create } from 'zustand'
import { apiClient } from '@/api/client'

interface TenantState {
  /** The tenant a platform administrator works in; `null` for "All tenants". */
  selectedTenantId: string | null
  /** Pick a tenant: every request after this works in it. */
  setSelectedTenantId: (id: string | null) => void
}

export const useTenantStore = create<TenantState>()((set) => ({
  selectedTenantId: null,
  setSelectedTenantId: (id) => {
    apiClient.setTenant(id)
    set({ selectedTenantId: id })
  },
}))
