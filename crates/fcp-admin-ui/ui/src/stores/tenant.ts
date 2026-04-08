import { create } from 'zustand'

interface TenantState {
  /** Currently selected tenant ID for multi-tenant admin context */
  selectedTenantId: string | null
  /** Set the active tenant context */
  setSelectedTenantId: (id: string | null) => void
}

export const useTenantStore = create<TenantState>()((set) => ({
  selectedTenantId: null,
  setSelectedTenantId: (id) => set({ selectedTenantId: id }),
}))
