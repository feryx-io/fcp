import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { ArrowUpDown } from 'lucide-react'
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
import type { AuditEntry, AuditQueryParams } from '@/types'

const actionBadge = (action: string) => {
  if (action.startsWith('create')) return 'success' as const
  if (action.startsWith('update') || action.startsWith('modify')) return 'default' as const
  if (action.startsWith('delete') || action.startsWith('remove')) return 'destructive' as const
  return 'secondary' as const
}

export function AuditLogPage() {
  const [filters, setFilters] = useState<AuditQueryParams>({ limit: 100, offset: 0 })
  const [selectedEntry, setSelectedEntry] = useState<AuditEntry | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['audit-logs', filters],
    queryFn: () => auditLog.list(filters),
  })

  const entries = data?.entries ?? []
  const total = data?.total ?? 0

  const columns: ColumnDef<AuditEntry>[] = [
    {
      accessorKey: 'timestamp',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Time <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => new Date(row.original.timestamp).toLocaleString(),
    },
    {
      accessorKey: 'actor_id',
      header: 'Actor',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.actor_id}</span>
          <p className="text-xs text-muted-foreground">{row.original.actor_type}</p>
        </div>
      ),
    },
    {
      accessorKey: 'action',
      header: 'Action',
      cell: ({ row }) => (
        <Badge variant={actionBadge(row.original.action)}>
          {row.original.action}
        </Badge>
      ),
    },
    {
      accessorKey: 'resource_type',
      header: 'Resource',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.resource_type}</span>
          <p className="text-xs text-muted-foreground font-mono">{row.original.resource_id}</p>
        </div>
      ),
    },
    {
      accessorKey: 'ip_address',
      header: 'IP',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.ip_address ?? '-'}</span>
      ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setSelectedEntry(row.original)}
        >
          View
        </Button>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Audit Log</h1>
        <p className="text-muted-foreground">
          Track who did what and when
          {total > 0 && <span className="ml-1">({total} entries)</span>}
        </p>
      </div>

      {/* Filters */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
        <div className="space-y-1">
          <Label className="text-xs">Actor ID</Label>
          <Input
            placeholder="Username or ID"
            value={filters.actor_id ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, actor_id: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Resource Type</Label>
          <Select
            value={filters.resource_type ?? '__all__'}
            onValueChange={(v) => setFilters((f) => ({ ...f, resource_type: v === '__all__' ? undefined : v, offset: 0 }))}
          >
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All</SelectItem>
              <SelectItem value="user">User</SelectItem>
              <SelectItem value="tenant">Tenant</SelectItem>
              <SelectItem value="extension">Extension</SelectItem>
              <SelectItem value="device">Device</SelectItem>
              <SelectItem value="phone_number">Phone Number</SelectItem>
              <SelectItem value="flow">Flow</SelectItem>
              <SelectItem value="routing_rule">Routing Rule</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Date From</Label>
          <Input
            type="date"
            value={filters.start_time ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, start_time: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Date To</Label>
          <Input
            type="date"
            value={filters.end_time ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, end_time: e.target.value || undefined, offset: 0 }))}
          />
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={entries} />
      )}

      {/* Pagination */}
      {total > (filters.limit ?? 100) && (
        <div className="flex items-center justify-center gap-4">
          <Button
            variant="outline"
            size="sm"
            disabled={(filters.offset ?? 0) === 0}
            onClick={() => setFilters((f) => ({
              ...f, offset: Math.max(0, (f.offset ?? 0) - (f.limit ?? 100)),
            }))}
          >
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            {(filters.offset ?? 0) + 1}-{Math.min((filters.offset ?? 0) + (filters.limit ?? 100), total)} of {total}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={(filters.offset ?? 0) + (filters.limit ?? 100) >= total}
            onClick={() => setFilters((f) => ({
              ...f, offset: (f.offset ?? 0) + (f.limit ?? 100),
            }))}
          >
            Next
          </Button>
        </div>
      )}

      {/* Detail Dialog */}
      <Dialog open={!!selectedEntry} onOpenChange={() => setSelectedEntry(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Audit Entry Detail</DialogTitle>
            <DialogDescription>Full audit record including changes.</DialogDescription>
          </DialogHeader>
          {selectedEntry && (
            <div className="space-y-3 text-sm">
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <p className="text-xs text-muted-foreground">Timestamp</p>
                  <p>{new Date(selectedEntry.timestamp).toLocaleString()}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Actor</p>
                  <p>{selectedEntry.actor_id} ({selectedEntry.actor_type})</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Action</p>
                  <Badge variant={actionBadge(selectedEntry.action)}>{selectedEntry.action}</Badge>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Resource</p>
                  <p>{selectedEntry.resource_type}: {selectedEntry.resource_id}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">IP Address</p>
                  <p className="font-mono">{selectedEntry.ip_address ?? '-'}</p>
                </div>
              </div>
              {selectedEntry.changes && (
                <div>
                  <p className="text-xs text-muted-foreground mb-1">Changes</p>
                  <pre className="text-xs bg-muted rounded p-3 overflow-auto max-h-48 font-mono">
                    {JSON.stringify(selectedEntry.changes, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
