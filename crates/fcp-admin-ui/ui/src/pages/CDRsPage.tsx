import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { ArrowUpDown, Download, Search } from 'lucide-react'
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
import { useNavigate } from 'react-router-dom'
import { cdrs } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { CdrResponse, CdrQueryParams } from '@/types'

const dispositionBadge = (d: string) => {
  switch (d) {
    case 'answered': return 'success' as const
    case 'no_answer': return 'warning' as const
    case 'busy': return 'secondary' as const
    case 'failed': case 'canceled': return 'destructive' as const
    default: return 'outline' as const
  }
}

export function CDRsPage() {
  const navigate = useNavigate()
  const [filters, setFilters] = useState<CdrQueryParams>({ limit: 100, offset: 0 })
  const [selectedCdr, setSelectedCdr] = useState<CdrResponse | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['cdrs', filters],
    queryFn: () => cdrs.list(filters),
  })

  const cdrList = data?.cdrs ?? []
  const total = data?.total ?? 0

  const exportCsv = () => {
    if (cdrList.length === 0) return
    const headers = [
      'call_id', 'from_uri', 'to_uri', 'start_time', 'disposition',
      'hangup_cause', 'call_duration_ms', 'codec_a_leg', 'codec_b_leg',
    ]
    const rows = cdrList.map((c) => [
      c.call_id, c.from_uri, c.to_uri,
      new Date(c.start_time * 1000).toISOString(),
      c.disposition, c.hangup_cause,
      c.call_duration_ms ?? '', c.codec_a_leg ?? '', c.codec_b_leg ?? '',
    ])
    const csv = [headers.join(','), ...rows.map((r) => r.map((v) => `"${v}"`).join(','))].join('\n')
    const blob = new Blob([csv], { type: 'text/csv' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `cdrs-${new Date().toISOString().slice(0, 10)}.csv`
    a.click()
    URL.revokeObjectURL(url)
  }

  const columns: ColumnDef<CdrResponse>[] = [
    {
      accessorKey: 'call_id',
      header: 'Call ID',
      cell: ({ row }) => (
        <button
          className="text-primary hover:underline font-mono text-xs"
          onClick={() => setSelectedCdr(row.original)}
        >
          {row.original.call_id.slice(0, 12)}...
        </button>
      ),
    },
    {
      accessorKey: 'from_uri',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          From <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.from_uri}</span>
          {row.original.from_display_name && (
            <p className="text-xs text-muted-foreground">{row.original.from_display_name}</p>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'to_uri',
      header: 'To',
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.to_uri}</span>
          {row.original.to_display_name && (
            <p className="text-xs text-muted-foreground">{row.original.to_display_name}</p>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'start_time',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Time <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => new Date(row.original.start_time * 1000).toLocaleString(),
    },
    {
      accessorKey: 'disposition',
      header: 'Disposition',
      cell: ({ row }) => (
        <Badge variant={dispositionBadge(row.original.disposition)}>
          {row.original.disposition}
        </Badge>
      ),
    },
    {
      accessorKey: 'call_duration_ms',
      header: 'Duration',
      cell: ({ row }) => {
        const ms = row.original.call_duration_ms
        return ms != null ? formatDuration(Math.round(ms / 1000)) : '-'
      },
    },
    {
      accessorKey: 'hangup_cause',
      header: 'Hangup Cause',
      cell: ({ row }) => (
        <span className="text-xs font-mono">{row.original.hangup_cause}</span>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Call Detail Records</h1>
          <p className="text-muted-foreground">
            Search and analyze call history
            {total > 0 && <span className="ml-1">({total} total)</span>}
          </p>
        </div>
        <Button variant="outline" onClick={exportCsv} disabled={cdrList.length === 0}>
          <Download className="mr-2 h-4 w-4" />
          Export CSV
        </Button>
      </div>

      {/* Filters */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
        <div className="space-y-1">
          <Label className="text-xs">From URI</Label>
          <Input
            placeholder="sip:user@..."
            value={filters.from_uri ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, from_uri: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">To URI</Label>
          <Input
            placeholder="sip:user@..."
            value={filters.to_uri ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, to_uri: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Disposition</Label>
          <Select
            value={filters.disposition ?? '__all__'}
            onValueChange={(v) => setFilters((f) => ({ ...f, disposition: v === '__all__' ? undefined : v, offset: 0 }))}
          >
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All</SelectItem>
              <SelectItem value="answered">Answered</SelectItem>
              <SelectItem value="no_answer">No Answer</SelectItem>
              <SelectItem value="busy">Busy</SelectItem>
              <SelectItem value="failed">Failed</SelectItem>
              <SelectItem value="canceled">Canceled</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Date From</Label>
          <Input
            type="date"
            value={filters.start_time_from ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, start_time_from: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Date To</Label>
          <Input
            type="date"
            value={filters.start_time_to ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, start_time_to: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Call ID</Label>
          <Input
            placeholder="Search by call ID..."
            value={filters.call_id ?? ''}
            onChange={(e) => setFilters((f) => ({ ...f, call_id: e.target.value || undefined, offset: 0 }))}
          />
        </div>
        <div className="flex items-end col-span-2 md:col-span-2 gap-2">
          <Button
            className="flex-1"
            onClick={() => setFilters({ limit: 100, offset: 0 })}
            variant="outline"
          >
            Clear Filters
          </Button>
          <Button className="flex-1" onClick={() => setFilters((f) => ({ ...f }))}>
            <Search className="mr-2 h-4 w-4" />
            Search
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <>
          <DataTable columns={columns} data={cdrList} />

          {/* Server-side pagination */}
          {total > (filters.limit ?? 100) && (
            <div className="flex items-center justify-center gap-4">
              <Button
                variant="outline"
                size="sm"
                disabled={(filters.offset ?? 0) === 0}
                onClick={() => setFilters((f) => ({
                  ...f,
                  offset: Math.max(0, (f.offset ?? 0) - (f.limit ?? 100)),
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
                  ...f,
                  offset: (f.offset ?? 0) + (f.limit ?? 100),
                }))}
              >
                Next
              </Button>
            </div>
          )}
        </>
      )}

      {/* CDR Detail Dialog */}
      <Dialog open={!!selectedCdr} onOpenChange={() => setSelectedCdr(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>CDR Detail</DialogTitle>
            <DialogDescription>Complete call detail record.</DialogDescription>
          </DialogHeader>
          {selectedCdr && (
            <div className="grid grid-cols-2 gap-4 text-sm max-h-[60vh] overflow-y-auto">
              <Field label="Call ID" value={selectedCdr.call_id} mono />
              <Field label="SIP Call-ID" value={selectedCdr.sip_call_id} mono />
              <Field label="Tenant ID" value={selectedCdr.tenant_id} mono />
              <Field label="From" value={selectedCdr.from_uri} mono />
              <Field label="From Name" value={selectedCdr.from_display_name} />
              <Field label="To" value={selectedCdr.to_uri} mono />
              <Field label="To Name" value={selectedCdr.to_display_name} />
              <Field label="Start Time" value={new Date(selectedCdr.start_time * 1000).toLocaleString()} />
              <Field label="Answer Time" value={selectedCdr.answer_time ? new Date(selectedCdr.answer_time * 1000).toLocaleString() : null} />
              <Field label="End Time" value={selectedCdr.end_time ? new Date(selectedCdr.end_time * 1000).toLocaleString() : null} />
              <Field label="Setup Duration" value={selectedCdr.setup_duration_ms != null ? `${selectedCdr.setup_duration_ms}ms` : null} />
              <Field label="Call Duration" value={selectedCdr.call_duration_ms != null ? formatDuration(Math.round(selectedCdr.call_duration_ms / 1000)) : null} />
              <Field label="Total Duration" value={selectedCdr.total_duration_ms != null ? formatDuration(Math.round(selectedCdr.total_duration_ms / 1000)) : null} />
              <Field label="Disposition" value={selectedCdr.disposition} badge={dispositionBadge(selectedCdr.disposition)} />
              <Field label="Hangup Cause" value={selectedCdr.hangup_cause} />
              <Field label="Hangup Party" value={selectedCdr.hangup_party} />
              <Field label="SIP Status" value={selectedCdr.sip_status_code != null ? `${selectedCdr.sip_status_code} ${selectedCdr.sip_reason ?? ''}` : null} />
              <Field label="Media Anchored" value={selectedCdr.media_anchored ? 'Yes' : 'No'} />
              <Field label="Codec A-Leg" value={selectedCdr.codec_a_leg} />
              <Field label="Codec B-Leg" value={selectedCdr.codec_b_leg} />
              <Field label="Recording" value={selectedCdr.recording_url} />
              <Field label="Cost" value={selectedCdr.cost != null ? `${selectedCdr.cost} ${selectedCdr.currency ?? ''}` : null} />
              <div className="col-span-2 pt-2 border-t">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => navigate(`/admin/calls/${selectedCdr.call_id}/sip-trace`)}
                >
                  View SIP Trace
                </Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}

function Field({ label, value, mono, badge }: {
  label: string
  value: string | null | undefined
  mono?: boolean
  badge?: 'success' | 'warning' | 'secondary' | 'destructive' | 'outline' | 'default'
}) {
  return (
    <div>
      <p className="text-muted-foreground text-xs">{label}</p>
      {badge && value ? (
        <Badge variant={badge}>{value}</Badge>
      ) : (
        <p className={mono ? 'font-mono text-xs break-all' : ''}>{value ?? '-'}</p>
      )}
    </div>
  )
}
