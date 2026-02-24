import { create } from 'zustand'
import type { ConfigEntry, ConfigAuditEntry } from '@/api/endpoints'

interface ConfigState {
  /** Currently selected service tab */
  selectedService: string
  /** Cached config entries for the selected service */
  entries: ConfigEntry[]
  /** Audit log entries */
  auditLog: ConfigAuditEntry[]
  /** Section currently being edited (null = list view) */
  editingSection: string | null
  /** Dirty editor value (JSON) */
  editorValue: string
  /** Whether the editor has unsaved changes */
  isDirty: boolean

  setSelectedService: (service: string) => void
  setEntries: (entries: ConfigEntry[]) => void
  setAuditLog: (entries: ConfigAuditEntry[]) => void
  startEditing: (section: string, value: Record<string, unknown>) => void
  setEditorValue: (value: string) => void
  stopEditing: () => void
}

export const useConfigStore = create<ConfigState>()((set) => ({
  selectedService: 'call-manager',
  entries: [],
  auditLog: [],
  editingSection: null,
  editorValue: '',
  isDirty: false,

  setSelectedService: (service) => set({ selectedService: service, editingSection: null, isDirty: false }),
  setEntries: (entries) => set({ entries }),
  setAuditLog: (entries) => set({ auditLog: entries }),
  startEditing: (section, value) =>
    set({ editingSection: section, editorValue: JSON.stringify(value, null, 2), isDirty: false }),
  setEditorValue: (value) => set({ editorValue: value, isDirty: true }),
  stopEditing: () => set({ editingSection: null, editorValue: '', isDirty: false }),
}))
