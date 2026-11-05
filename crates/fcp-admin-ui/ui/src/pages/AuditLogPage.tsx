import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { X } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { auditLog, config as configApi, tenants, type ConfigAuditEntry } from '@/api/endpoints'
import { useIsPlatformAdmin } from '@/components/TenantSelector'
import { useCursorPages } from '@/lib/useCursorPages'
import { SERVICES } from '@/lib/configServices'
import type {
  AuditEntry, AuditQueryParams, DeletionEntry, DeletionQueryParams,
} from '@/types'

const methodBadge = (method: string) => {
  if (method === 'POST') return 'success' as const
  if (method === 'DELETE') return 'destructive' as const
  if (method === 'GET') return 'secondary' as const
  return 'default' as const
}

const causeBadge = (cause: string) => {
  if (cause === 'manual') return 'default' as const
  if (cause === 'unattributed') return 'destructive' as const
  return 'secondary' as const
}

/** Unix seconds for the start of a `YYYY-MM-DD` date input, local time. */
const sinceOf = (date: string): number | undefined =>
  date ? Math.floor(new Date(`${date}T00:00:00`).getTime() / 1000) : undefined

/** Unix seconds for the end of a `YYYY-MM-DD` date input: the next day's start. */
const untilOf = (date: string): number | undefined => {
  const start = sinceOf(date)
  return start === undefined ? undefined : start + 24 * 60 * 60
}

const when = (at: string) => new Date(at).toLocaleString()

const PAGE_SIZES = [100, 500, 1000]

const CLASSES = [
  'cdr', 'call_recording', 'conference_recording', 'voicemail',
  'sip_trace', 'flow_trace', 'audit_event', 'orphan_file',
]

type View = 'requests' | 'deletions' | 'config'

type Selected =
  | { kind: 'request'; entry: AuditEntry }
  | { kind: 'deletion'; entry: DeletionEntry }
  | { kind: 'config'; entry: ConfigAuditEntry }

/** Download an export the filters select, as the file the API names. */
async function download(url: Promise<string>, name: string) {
  const href = await url
  const a = document.createElement('a')
  a.href = href
  a.download = name
  a.click()
  URL.revokeObjectURL(href)
}

/** Previous and next under a paged table, and where in the log it is. */
function Pager({ pages, shown, size, next }: {
  pages: ReturnType<typeof useCursorPages>
  shown: number
  size: number
  next: string | null | undefined
}) {
  if (pages.pageIndex === 0 && !next) return null
  const first = pages.pageIndex * size + 1
  return (
    <div className="flex items-center justify-center gap-4">
      <Button variant="outline" size="sm" disabled={pages.pageIndex === 0} onClick={pages.previous}>
        Previous
      </Button>
      <span className="text-sm text-muted-foreground">
        {shown ? `${first}-${first + shown - 1}` : 'None'}
      </span>
      <Button variant="outline" size="sm" disabled={!next} onClick={() => next && pages.next(next)}>
        Next
      </Button>
    </div>
  )
}

/** The filter one entry's link set, shown so it can be cleared. */
function LinkFilter({ label, onClear }: { label: string; onClear: () => void }) {
  return (
    <div className="flex items-center gap-2">
      <Badge variant="secondary" className="font-mono">{label}</Badge>
      <Button variant="ghost" size="sm" onClick={onClear} aria-label="Clear the request filter">
        <X className="h-4 w-4" />
      </Button>
    </div>
  )
}

export function AuditLogPage() {
  const [view, setView] = useState<View>('requests')
  const [requestFilters, setRequestFilters] = useState<AuditQueryParams>({ limit: 100 })
  const [deletionFilters, setDeletionFilters] = useState<DeletionQueryParams>({ limit: 100 })
  const [configFilters, setConfigFilters] = useState<{ service?: string; section?: string }>({})
  const [requestDate, setRequestDate] = useState('')
  const [requestUntil, setRequestUntil] = useState('')
  const [deletionDate, setDeletionDate] = useState('')
  const [deletionUntil, setDeletionUntil] = useState('')
  const [exportError, setExportError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Selected | null>(null)
  const isPlatformAdmin = useIsPlatformAdmin()
  const requestPages = useCursorPages()
  const deletionPages = useCursorPages()
  const configPages = useCursorPages()
  useEffect(() => requestPages.reset(), [requestFilters, requestPages.reset])
  useEffect(() => deletionPages.reset(), [deletionFilters, deletionPages.reset])
  useEffect(() => configPages.reset(), [configFilters, configPages.reset])

  // A row is the tenant acted on (docs/TENANT_SCOPE.md §3.3): a platform
  // administrator sees whose, by name.
  const tenantList = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })
  const tenantName = (id: string | null | undefined) =>
    id == null ? 'Platform' : (tenantList.data?.find((t) => t.id === id)?.name ?? id)

  const requests = useQuery({
    queryKey: ['audit-log', requestFilters, requestPages.cursor],
    queryFn: () => auditLog.list({ ...requestFilters, cursor: requestPages.cursor }),
    enabled: view === 'requests',
  })
  const deletions = useQuery({
    queryKey: ['deletion-log', deletionFilters, deletionPages.cursor],
    queryFn: () => auditLog.deletions({ ...deletionFilters, cursor: deletionPages.cursor }),
    enabled: view === 'deletions',
  })
  // The configuration store's history is the platform's (system
  // configuration), so only a platform administrator has the tab.
  const configChanges = useQuery({
    queryKey: ['config-audit-log', configFilters, configPages.cursor],
    queryFn: () => configApi.auditPage({ ...configFilters, limit: 100, cursor: configPages.cursor }),
    enabled: view === 'config' && isPlatformAdmin,
  })

  // An entry's link: the deletions one request made, or the request that
  // made one deletion.
  const showDeletionsOf = (apiRequestId: string) => {
    setSelected(null)
    setDeletionDate('')
    setDeletionUntil('')
    setDeletionFilters((f) => ({ limit: f.limit, api_request_id: apiRequestId }))
    setView('deletions')
  }
  const showRequest = (apiRequestId: string) => {
    setSelected(null)
    setRequestDate('')
    setRequestUntil('')
    setRequestFilters((f) => ({ limit: f.limit, tenant: f.tenant, api_request_id: apiRequestId }))
    setView('requests')
  }

  const requestColumns: ColumnDef<AuditEntry>[] = [
    {
      accessorKey: 'occurred_at',
      header: 'Time',
      cell: ({ row }) => when(row.original.occurred_at),
    },
    ...(isPlatformAdmin
      ? [{
          id: 'tenant',
          header: 'Tenant',
          cell: ({ row }) => (
            <span className={row.original.tenant_id == null ? 'text-sm text-muted-foreground' : 'text-sm'}>
              {tenantName(row.original.tenant_id)}
            </span>
          ),
        } satisfies ColumnDef<AuditEntry>]
      : []),
    {
      accessorKey: 'actor',
      header: 'Actor',
      cell: ({ row }) => {
        const e = row.original
        // A platform administrator acting on another tenant than their own.
        const from = isPlatformAdmin && e.actor_tenant_id && e.actor_tenant_id !== e.tenant_id
          ? ` · from ${tenantName(e.actor_tenant_id)}`
          : ''
        return (
          <div>
            <span className="text-sm">{e.actor}</span>
            <p className="text-xs text-muted-foreground">{e.actor_kind}{from}</p>
          </div>
        )
      },
    },
    {
      accessorKey: 'action',
      header: 'Action',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Badge variant={methodBadge(row.original.method)}>{row.original.method}</Badge>
          <span className="font-mono text-xs">{row.original.path}</span>
        </div>
      ),
    },
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.status >= 400 ? 'destructive' : 'secondary'}>
          {row.original.status}
        </Badge>
      ),
    },
    {
      accessorKey: 'source_ip',
      header: 'Source IP',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.source_ip ?? '-'}</span>
      ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected({ kind: 'request', entry: row.original })}>
          View
        </Button>
      ),
    },
  ]

  const deletionColumns: ColumnDef<DeletionEntry>[] = [
    {
      accessorKey: 'deleted_at',
      header: 'Time',
      cell: ({ row }) => when(row.original.deleted_at),
    },
    {
      accessorKey: 'class',
      header: 'What',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.class.replace(/_/g, ' ')}</span>
          <p className="text-xs text-muted-foreground font-mono">{row.original.item}</p>
        </div>
      ),
    },
    {
      accessorKey: 'cause',
      header: 'Why',
      cell: ({ row }) => (
        <Badge variant={causeBadge(row.original.cause)}>
          {row.original.cause.replace(/_/g, ' ')}
        </Badge>
      ),
    },
    {
      accessorKey: 'actor',
      header: 'By',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.actor}</span>
          {row.original.via && (
            <p className="text-xs text-muted-foreground">via {row.original.via}</p>
          )}
        </div>
      ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected({ kind: 'deletion', entry: row.original })}>
          View
        </Button>
      ),
    },
  ]

  const configColumns: ColumnDef<ConfigAuditEntry>[] = [
    {
      accessorKey: 'changed_at',
      header: 'Time',
      cell: ({ row }) => when(row.original.changed_at),
    },
    {
      id: 'section',
      header: 'Section',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{SERVICES.find((s) => s.value === row.original.service)?.label ?? row.original.service}</span>
          <p className="text-xs text-muted-foreground font-mono">{row.original.section}</p>
        </div>
      ),
    },
    {
      accessorKey: 'changed_by',
      header: 'By',
      cell: ({ row }) => <span className="text-sm">{row.original.changed_by ?? '-'}</span>,
    },
    {
      accessorKey: 'version',
      header: 'Version',
      cell: ({ row }) => (
        <Badge variant={row.original.old_value == null ? 'success' : 'secondary'}>
          {row.original.old_value == null ? `v${row.original.version} (new)` : `v${row.original.version}`}
        </Badge>
      ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected({ kind: 'config', entry: row.original })}>
          View
        </Button>
      ),
    },
  ]

  // The whole log the filters match, not the page shown.
  const exportRequests = async (format: 'csv' | 'jsonl') => {
    setExportError(null)
    try {
      const { limit: _limit, cursor: _cursor, ...filters } = requestFilters
      await download(auditLog.exportUrl(format, filters), `audit-log.${format}`)
    } catch (e) {
      setExportError(e instanceof Error ? e.message : String(e))
    }
  }
  const exportDeletions = async (format: 'csv' | 'jsonl') => {
    setExportError(null)
    try {
      const { limit: _limit, cursor: _cursor, ...filters } = deletionFilters
      await download(auditLog.deletionsExportUrl(format, filters), `deletion-log.${format}`)
    } catch (e) {
      setExportError(e instanceof Error ? e.message : String(e))
    }
  }

  const exportBar = (exportAs: (format: 'csv' | 'jsonl') => void) => (
    <div className="flex items-center gap-2">
      <span className="text-sm text-muted-foreground">Export everything these filters match:</span>
      <Button variant="outline" size="sm" onClick={() => exportAs('csv')}>CSV</Button>
      <Button variant="outline" size="sm" onClick={() => exportAs('jsonl')}>JSON Lines</Button>
      {exportError && <span className="text-sm text-destructive">{exportError}</span>}
    </div>
  )

  const sizeSelect = (value: number | undefined, set: (limit: number) => void) => (
    <div className="space-y-1">
      <Label className="text-xs">Per page</Label>
      <Select value={String(value ?? 100)} onValueChange={(v) => set(Number(v))}>
        <SelectTrigger aria-label="Per page"><SelectValue /></SelectTrigger>
        <SelectContent>
          {PAGE_SIZES.map((n) => (
            <SelectItem key={n} value={String(n)}>{n}</SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )

  const dateInput = (
    label: string,
    value: string,
    set: (date: string) => void,
  ) => (
    <div className="space-y-1">
      <Label className="text-xs">{label}</Label>
      <Input type="date" aria-label={label} value={value} onChange={(e) => set(e.target.value)} />
    </div>
  )

  const loading = <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>

  const tab = (name: View, label: string) => (
    <Button variant={view === name ? 'default' : 'outline'} size="sm" onClick={() => setView(name)}>
      {label}
    </Button>
  )

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Audit Log</h1>
        <p className="text-muted-foreground">
          Who changed or read what through the API, what was deleted, when, why and by whom
          {isPlatformAdmin ? ', and how the configuration changed' : ''}.
          Each request is the tenant's it acted on{isPlatformAdmin ? '; the platform\'s own have no tenant' : ', a platform administrator\'s included'}.
        </p>
      </div>

      <div className="flex gap-2">
        {tab('requests', 'Requests')}
        {tab('deletions', 'Deletions')}
        {isPlatformAdmin && tab('config', 'Config changes')}
      </div>

      {view === 'requests' && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-6 gap-4 p-4 rounded-lg border bg-card">
            {isPlatformAdmin && (
              <div className="space-y-1">
                <Label className="text-xs">Rows</Label>
                <Select
                  value={requestFilters.tenant ?? '__picked__'}
                  onValueChange={(v) => setRequestFilters((f) => ({ ...f, tenant: v === '__picked__' ? undefined : v }))}
                >
                  <SelectTrigger aria-label="Rows"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__picked__">As picked in the top bar</SelectItem>
                    <SelectItem value="all">Every tenant and the platform</SelectItem>
                    <SelectItem value="platform">The platform only</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            )}
            <div className="space-y-1">
              <Label className="text-xs">Actor</Label>
              <Input
                placeholder="Username or token id"
                value={requestFilters.actor ?? ''}
                onChange={(e) => setRequestFilters((f) => ({ ...f, actor: e.target.value || undefined }))}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">Action starts with</Label>
              <Input
                placeholder="DELETE /api/v1/users"
                value={requestFilters.action ?? ''}
                onChange={(e) => setRequestFilters((f) => ({ ...f, action: e.target.value || undefined }))}
              />
            </div>
            {dateInput('Since', requestDate, (date) => {
              setRequestDate(date)
              setRequestFilters((f) => ({ ...f, since: sinceOf(date) }))
            })}
            {dateInput('Until', requestUntil, (date) => {
              setRequestUntil(date)
              setRequestFilters((f) => ({ ...f, until: untilOf(date) }))
            })}
            {sizeSelect(requestFilters.limit, (limit) => setRequestFilters((f) => ({ ...f, limit })))}
          </div>
          {requestFilters.api_request_id && (
            <LinkFilter
              label={`Request ${requestFilters.api_request_id}`}
              onClear={() => setRequestFilters((f) => ({ ...f, api_request_id: undefined }))}
            />
          )}
          {exportBar(exportRequests)}
          {requests.isLoading ? loading : (
            <>
              <DataTable columns={requestColumns} data={requests.data?.data ?? []} />
              <Pager
                pages={requestPages}
                shown={requests.data?.data.length ?? 0}
                size={requestFilters.limit ?? 100}
                next={requests.data?.next_cursor}
              />
            </>
          )}
        </>
      )}

      {view === 'deletions' && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
            <div className="space-y-1">
              <Label className="text-xs">What</Label>
              <Select
                value={deletionFilters.class ?? '__all__'}
                onValueChange={(v) => setDeletionFilters((f) => ({ ...f, class: v === '__all__' ? undefined : v }))}
              >
                <SelectTrigger aria-label="What"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__all__">Everything</SelectItem>
                  {CLASSES.map((c) => (
                    <SelectItem key={c} value={c}>{c.replace(/_/g, ' ')}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {dateInput('Since', deletionDate, (date) => {
              setDeletionDate(date)
              setDeletionFilters((f) => ({ ...f, since: sinceOf(date) }))
            })}
            {dateInput('Until', deletionUntil, (date) => {
              setDeletionUntil(date)
              setDeletionFilters((f) => ({ ...f, until: untilOf(date) }))
            })}
            {sizeSelect(deletionFilters.limit, (limit) => setDeletionFilters((f) => ({ ...f, limit })))}
          </div>
          {deletionFilters.api_request_id && (
            <LinkFilter
              label={`Deleted by request ${deletionFilters.api_request_id}`}
              onClear={() => setDeletionFilters((f) => ({ ...f, api_request_id: undefined }))}
            />
          )}
          {exportBar(exportDeletions)}
          {deletions.isLoading ? loading : (
            <>
              <DataTable columns={deletionColumns} data={deletions.data?.data ?? []} />
              <Pager
                pages={deletionPages}
                shown={deletions.data?.data.length ?? 0}
                size={deletionFilters.limit ?? 100}
                next={deletions.data?.next_cursor}
              />
            </>
          )}
        </>
      )}

      {view === 'config' && isPlatformAdmin && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
            <div className="space-y-1">
              <Label className="text-xs">Service</Label>
              <Select
                value={configFilters.service ?? '__all__'}
                onValueChange={(v) => setConfigFilters((f) => ({ ...f, service: v === '__all__' ? undefined : v }))}
              >
                <SelectTrigger aria-label="Service"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__all__">Every service</SelectItem>
                  {SERVICES.map((s) => (
                    <SelectItem key={s.value} value={s.value}>{s.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label className="text-xs">Section</Label>
              <Input
                placeholder="auth"
                value={configFilters.section ?? ''}
                onChange={(e) => setConfigFilters((f) => ({ ...f, section: e.target.value || undefined }))}
              />
            </div>
          </div>
          <p className="text-sm text-muted-foreground">
            The newest 500 changes to the configuration store, with what each replaced.
          </p>
          {configChanges.isLoading ? loading : (
            <>
              <DataTable columns={configColumns} data={configChanges.data?.data ?? []} />
              <Pager
                pages={configPages}
                shown={configChanges.data?.data.length ?? 0}
                size={100}
                next={configChanges.data?.next_cursor}
              />
            </>
          )}
        </>
      )}

      <Dialog open={!!selected} onOpenChange={() => setSelected(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>
              {selected?.kind === 'request' ? 'Request' : selected?.kind === 'deletion' ? 'Deletion' : 'Configuration change'}
            </DialogTitle>
            <DialogDescription>The full entry.</DialogDescription>
          </DialogHeader>
          {selected && (
            <pre className="text-xs bg-muted rounded p-3 overflow-auto max-h-96 font-mono">
              {JSON.stringify(selected.entry, null, 2)}
            </pre>
          )}
          {selected?.kind === 'request' && selected.entry.api_request_id && selected.entry.method !== 'GET' && (
            <Button variant="outline" size="sm" onClick={() => showDeletionsOf(selected.entry.api_request_id!)}>
              What this request deleted
            </Button>
          )}
          {selected?.kind === 'deletion' && selected.entry.api_request_id && (
            <Button variant="outline" size="sm" onClick={() => showRequest(selected.entry.api_request_id!)}>
              The request that deleted it
            </Button>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
