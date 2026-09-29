import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
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
import { auditLog } from '@/api/endpoints'
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

const when = (at: string) => new Date(at).toLocaleString()

const LIMITS = [100, 500, 1000]

const CLASSES = [
  'cdr', 'call_recording', 'conference_recording', 'voicemail',
  'sip_trace', 'flow_trace', 'audit_event', 'orphan_file',
]

export function AuditLogPage() {
  const [view, setView] = useState<'requests' | 'deletions'>('requests')
  const [requestFilters, setRequestFilters] = useState<AuditQueryParams>({ limit: 100 })
  const [deletionFilters, setDeletionFilters] = useState<DeletionQueryParams>({ limit: 100 })
  const [requestDate, setRequestDate] = useState('')
  const [deletionDate, setDeletionDate] = useState('')
  const [selected, setSelected] = useState<AuditEntry | DeletionEntry | null>(null)

  const requests = useQuery({
    queryKey: ['audit-log', requestFilters],
    queryFn: () => auditLog.list(requestFilters),
    enabled: view === 'requests',
  })
  const deletions = useQuery({
    queryKey: ['deletion-log', deletionFilters],
    queryFn: () => auditLog.deletions(deletionFilters),
    enabled: view === 'deletions',
  })

  const requestColumns: ColumnDef<AuditEntry>[] = [
    {
      accessorKey: 'occurred_at',
      header: 'Time',
      cell: ({ row }) => when(row.original.occurred_at),
    },
    {
      accessorKey: 'actor',
      header: 'Actor',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.actor}</span>
          <p className="text-xs text-muted-foreground">{row.original.actor_kind}</p>
        </div>
      ),
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
      header: 'IP',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.source_ip ?? '-'}</span>
      ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected(row.original)}>
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
      cell: ({ row }) => <span className="text-sm">{row.original.actor}</span>,
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected(row.original)}>
          View
        </Button>
      ),
    },
  ]

  const limitSelect = (value: number | undefined, set: (limit: number) => void) => (
    <div className="space-y-1">
      <Label className="text-xs">Show</Label>
      <Select value={String(value ?? 100)} onValueChange={(v) => set(Number(v))}>
        <SelectTrigger><SelectValue /></SelectTrigger>
        <SelectContent>
          {LIMITS.map((n) => (
            <SelectItem key={n} value={String(n)}>{n} newest</SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Audit Log</h1>
        <p className="text-muted-foreground">
          Who changed what through the API, and what was deleted, when, why and by whom
        </p>
      </div>

      <div className="flex gap-2">
        <Button
          variant={view === 'requests' ? 'default' : 'outline'}
          size="sm"
          onClick={() => setView('requests')}
        >
          Requests
        </Button>
        <Button
          variant={view === 'deletions' ? 'default' : 'outline'}
          size="sm"
          onClick={() => setView('deletions')}
        >
          Deletions
        </Button>
      </div>

      {view === 'requests' ? (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
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
            <div className="space-y-1">
              <Label className="text-xs">Since</Label>
              <Input
                type="date"
                value={requestDate}
                onChange={(e) => {
                  setRequestDate(e.target.value)
                  setRequestFilters((f) => ({ ...f, since: sinceOf(e.target.value) }))
                }}
              />
            </div>
            {limitSelect(requestFilters.limit, (limit) => setRequestFilters((f) => ({ ...f, limit })))}
          </div>
          {requests.isLoading ? (
            <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
          ) : (
            <DataTable columns={requestColumns} data={requests.data ?? []} />
          )}
        </>
      ) : (
        <>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-4 p-4 rounded-lg border bg-card">
            <div className="space-y-1">
              <Label className="text-xs">What</Label>
              <Select
                value={deletionFilters.class ?? '__all__'}
                onValueChange={(v) => setDeletionFilters((f) => ({ ...f, class: v === '__all__' ? undefined : v }))}
              >
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__all__">Everything</SelectItem>
                  {CLASSES.map((c) => (
                    <SelectItem key={c} value={c}>{c.replace(/_/g, ' ')}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label className="text-xs">Since</Label>
              <Input
                type="date"
                value={deletionDate}
                onChange={(e) => {
                  setDeletionDate(e.target.value)
                  setDeletionFilters((f) => ({ ...f, since: sinceOf(e.target.value) }))
                }}
              />
            </div>
            {limitSelect(deletionFilters.limit, (limit) => setDeletionFilters((f) => ({ ...f, limit })))}
          </div>
          {deletions.isLoading ? (
            <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
          ) : (
            <DataTable columns={deletionColumns} data={deletions.data ?? []} />
          )}
        </>
      )}

      <Dialog open={!!selected} onOpenChange={() => setSelected(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{selected && 'path' in selected ? 'Request' : 'Deletion'}</DialogTitle>
            <DialogDescription>The full entry.</DialogDescription>
          </DialogHeader>
          {selected && (
            <pre className="text-xs bg-muted rounded p-3 overflow-auto max-h-96 font-mono">
              {JSON.stringify(selected, null, 2)}
            </pre>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
