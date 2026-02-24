import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { config as configApi } from '@/api/endpoints'
import type { ConfigEntry, ConfigAuditEntry } from '@/api/endpoints'
import { useConfigStore } from '@/stores/config'
import { useAuthStore } from '@/stores/auth'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { Save, RotateCcw, X, History, AlertCircle, CheckCircle2, Trash2, RefreshCw } from 'lucide-react'

const SERVICES = [
  { value: 'call-manager', label: 'Call Manager' },
  { value: 'admin', label: 'Admin' },
  { value: 'events-server', label: 'Events Server' },
  { value: 'ivr', label: 'IVR' },
  { value: 'voicemail', label: 'Voicemail' },
  { value: 'conference', label: 'Conference' },
  { value: 'presence', label: 'Presence' },
  { value: 'media', label: 'Media Server' },
  { value: 'sbc', label: 'SBC' },
]

export function SystemSettingsPage() {
  const queryClient = useQueryClient()
  const { user } = useAuthStore()
  const {
    selectedService, setSelectedService,
    editingSection, startEditing, stopEditing,
    editorValue, setEditorValue, isDirty,
  } = useConfigStore()

  const [auditOpen, setAuditOpen] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saveSuccess, setSaveSuccess] = useState(false)

  // Fetch config entries for selected service
  const { data: entries = [], isLoading } = useQuery({
    queryKey: ['config', selectedService],
    queryFn: () => configApi.listService(selectedService),
  })

  // Fetch audit log
  const { data: auditEntries = [] } = useQuery({
    queryKey: ['config-audit', selectedService],
    queryFn: () => configApi.audit(selectedService, undefined, 50),
    enabled: auditOpen,
  })

  // Save mutation
  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!editingSection) return
      const parsed = JSON.parse(editorValue)
      // Validate first
      const validation = await configApi.validate(selectedService, editingSection, parsed)
      if (!validation.valid) {
        throw new Error(validation.errors.join('; '))
      }
      return configApi.update(
        selectedService,
        editingSection,
        parsed,
        '_system',
        user?.username ? `admin:${user.username}` : 'admin',
      )
    },
    onSuccess: () => {
      setSaveError(null)
      setSaveSuccess(true)
      setTimeout(() => setSaveSuccess(false), 3000)
      queryClient.invalidateQueries({ queryKey: ['config', selectedService] })
      stopEditing()
    },
    onError: (err: Error) => {
      setSaveError(err.message)
    },
  })

  // Delete mutation
  const deleteMutation = useMutation({
    mutationFn: (section: string) => configApi.delete(selectedService, section),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['config', selectedService] })
      stopEditing()
    },
  })

  const handleSave = () => {
    setSaveError(null)
    try {
      JSON.parse(editorValue)
    } catch {
      setSaveError('Invalid JSON')
      return
    }
    saveMutation.mutate()
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">System Settings</h1>
          <p className="text-muted-foreground">
            Database-backed configuration for all FCP services
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setAuditOpen(true)}
          >
            <History className="mr-2 h-4 w-4" />
            Audit Log
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => queryClient.invalidateQueries({ queryKey: ['config', selectedService] })}
          >
            <RefreshCw className="mr-2 h-4 w-4" />
            Refresh
          </Button>
        </div>
      </div>

      {/* Service selector */}
      <div className="flex items-center gap-4">
        <label className="text-sm font-medium">Service</label>
        <Select value={selectedService} onValueChange={setSelectedService}>
          <SelectTrigger className="w-[220px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SERVICES.map((s) => (
              <SelectItem key={s.value} value={s.value}>
                {s.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {entries.length > 0 && (
          <Badge variant="secondary">{entries.length} sections</Badge>
        )}
      </div>

      {/* Success banner */}
      {saveSuccess && (
        <div className="flex items-center gap-2 rounded-md border border-green-200 bg-green-50 p-3 text-sm text-green-800 dark:border-green-800 dark:bg-green-950 dark:text-green-200">
          <CheckCircle2 className="h-4 w-4" />
          Configuration saved successfully
        </div>
      )}

      {/* Section list / Editor */}
      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      ) : editingSection ? (
        <SectionEditor
          service={selectedService}
          section={editingSection}
          value={editorValue}
          onChange={setEditorValue}
          onSave={handleSave}
          onCancel={stopEditing}
          onDelete={() => deleteMutation.mutate(editingSection)}
          isDirty={isDirty}
          isSaving={saveMutation.isPending}
          error={saveError}
        />
      ) : entries.length === 0 ? (
        <Card>
          <CardContent className="flex h-32 items-center justify-center text-muted-foreground">
            No configuration sections found for {selectedService}.
            Start the service to seed default configuration.
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {entries.map((entry) => (
            <SectionCard
              key={entry.section}
              entry={entry}
              onEdit={() => startEditing(entry.section, entry.value)}
            />
          ))}
        </div>
      )}

      {/* Audit Log Dialog */}
      <Dialog open={auditOpen} onOpenChange={setAuditOpen}>
        <DialogContent className="max-w-3xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Configuration Audit Log</DialogTitle>
            <DialogDescription>
              Recent changes to {selectedService} configuration
            </DialogDescription>
          </DialogHeader>
          <AuditLogView entries={auditEntries} />
        </DialogContent>
      </Dialog>
    </div>
  )
}

// --- Sub-components ---

function SectionCard({ entry, onEdit }: { entry: ConfigEntry; onEdit: () => void }) {
  const fieldCount = entry.value ? Object.keys(entry.value).length : 0
  const updatedAgo = formatTimeAgo(entry.updated_at)

  return (
    <Card className="cursor-pointer transition-colors hover:bg-accent/50" onClick={onEdit}>
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between">
          <CardTitle className="text-base font-semibold">{entry.section}</CardTitle>
          <Badge variant="outline" className="text-xs">v{entry.version}</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-1 text-sm text-muted-foreground">
        <div>{fieldCount} fields</div>
        <div>Updated {updatedAgo}{entry.updated_by ? ` by ${entry.updated_by}` : ''}</div>
      </CardContent>
    </Card>
  )
}

function SectionEditor({
  service, section, value, onChange, onSave, onCancel, onDelete,
  isDirty, isSaving, error,
}: {
  service: string
  section: string
  value: string
  onChange: (v: string) => void
  onSave: () => void
  onCancel: () => void
  onDelete: () => void
  isDirty: boolean
  isSaving: boolean
  error: string | null
}) {
  const [confirmDelete, setConfirmDelete] = useState(false)

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="text-lg">
              {service} / {section}
            </CardTitle>
            <p className="text-sm text-muted-foreground">
              Edit the JSON configuration below
            </p>
          </div>
          <div className="flex items-center gap-2">
            {isDirty && (
              <Badge variant="secondary">Unsaved changes</Badge>
            )}
            <Button variant="ghost" size="icon" onClick={onCancel}>
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {error && (
          <div className="flex items-center gap-2 rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-800 dark:border-red-800 dark:bg-red-950 dark:text-red-200">
            <AlertCircle className="h-4 w-4 flex-shrink-0" />
            {error}
          </div>
        )}

        <textarea
          className="w-full min-h-[400px] rounded-md border bg-muted/50 p-4 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
        />

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Button variant="destructive" size="sm" onClick={() => setConfirmDelete(true)}>
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </Button>
            {confirmDelete && (
              <span className="text-sm text-destructive">
                Are you sure?{' '}
                <button className="underline" onClick={onDelete}>Yes, delete</button>
                {' / '}
                <button className="underline" onClick={() => setConfirmDelete(false)}>Cancel</button>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={onCancel} disabled={isSaving}>
              <RotateCcw className="mr-2 h-4 w-4" />
              Cancel
            </Button>
            <Button onClick={onSave} disabled={!isDirty || isSaving}>
              <Save className="mr-2 h-4 w-4" />
              {isSaving ? 'Saving...' : 'Save'}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function AuditLogView({ entries }: { entries: ConfigAuditEntry[] }) {
  if (entries.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center text-muted-foreground">
        No audit entries found
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {entries.map((entry) => (
        <div key={entry.id} className="rounded-md border p-3 text-sm">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Badge variant="outline">{entry.section}</Badge>
              <span className="text-muted-foreground">v{entry.version}</span>
            </div>
            <span className="text-xs text-muted-foreground">
              {formatTimeAgo(entry.changed_at)}
              {entry.changed_by ? ` by ${entry.changed_by}` : ''}
            </span>
          </div>
          {entry.old_value && (
            <details className="mt-2">
              <summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground">
                Show diff
              </summary>
              <div className="mt-1 grid grid-cols-2 gap-2">
                <div>
                  <div className="text-xs font-medium text-red-600 dark:text-red-400">Previous</div>
                  <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 text-xs">
                    {JSON.stringify(entry.old_value, null, 2)}
                  </pre>
                </div>
                <div>
                  <div className="text-xs font-medium text-green-600 dark:text-green-400">New</div>
                  <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 text-xs">
                    {JSON.stringify(entry.new_value, null, 2)}
                  </pre>
                </div>
              </div>
            </details>
          )}
        </div>
      ))}
    </div>
  )
}

// --- Helpers ---

function formatTimeAgo(isoDate: string): string {
  const now = Date.now()
  const then = new Date(isoDate).getTime()
  const diffMs = now - then
  const diffSec = Math.floor(diffMs / 1000)

  if (diffSec < 60) return 'just now'
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`
  return `${Math.floor(diffSec / 86400)}d ago`
}
