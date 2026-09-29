import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { flowTraces, type FlowTraceQueryParams, type FlowTraceEntry } from '@/api/endpoints'
import { DataTable } from '@/components/DataTable'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { useCursorPages } from '@/lib/useCursorPages'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { type ColumnDef } from '@tanstack/react-table'
import { RefreshCw, Download } from 'lucide-react'

const EVENT_TYPE_COLORS: Record<string, string> = {
  started: 'bg-blue-100 text-blue-800',
  node_executed: 'bg-green-100 text-green-800',
  waiting_for_input: 'bg-yellow-100 text-yellow-800',
  completed: 'bg-gray-100 text-gray-800',
  failed: 'bg-red-100 text-red-800',
}

const EVENT_TYPE_LABELS: Record<string, string> = {
  started: 'Started',
  node_executed: 'Node Executed',
  waiting_for_input: 'Waiting for Input',
  completed: 'Completed',
  failed: 'Failed',
}

function formatTimestamp(at: string): string {
  const ms = Date.parse(at)
  return new Date(ms).toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    fractionalSecondDigits: 3,
  } as Intl.DateTimeFormatOptions)
}

function escapeCSVField(value: string): string {
  const first = value.match(/[^\s\x00-\x1f]/)?.[0]
  if (first && '=+-@'.includes(first)) {
    value = "'" + value
  }
  if (value.includes(',') || value.includes('"') || value.includes('\n')) {
    return `"${value.replace(/"/g, '""')}"`
  }
  return value
}

const columns: ColumnDef<FlowTraceEntry>[] = [
  {
    accessorKey: 'timestamp',
    header: 'Time',
    cell: ({ row }) => (
      <span className="font-mono text-xs whitespace-nowrap">
        {formatTimestamp(row.original.timestamp)}
      </span>
    ),
  },
  {
    accessorKey: 'event_type',
    header: 'Event',
    cell: ({ row }) => {
      const et = row.original.event_type
      return (
        <Badge variant="outline" className={EVENT_TYPE_COLORS[et] || ''}>
          {EVENT_TYPE_LABELS[et] || et}
        </Badge>
      )
    },
  },
  {
    accessorKey: 'node_id',
    header: 'Node',
    cell: ({ row }) => (
      <span className="font-mono text-sm">{row.original.node_id || '-'}</span>
    ),
  },
  {
    accessorKey: 'detail',
    header: 'Detail',
    cell: ({ row }) => (
      <span className="text-sm text-muted-foreground max-w-[300px] truncate block" title={row.original.detail || undefined}>
        {row.original.detail || '-'}
      </span>
    ),
  },
  {
    accessorKey: 'call_id',
    header: 'Call ID',
    cell: ({ row }) => (
      <span className="font-mono text-xs">{row.original.call_id.replace('call-', '').substring(0, 8)}</span>
    ),
  },
  {
    accessorKey: 'flow_name',
    header: 'Flow',
    cell: ({ row }) => (
      <span className="text-sm">{row.original.flow_name || row.original.flow_id.replace('flow-', '').substring(0, 8)}</span>
    ),
  },
]

export function FlowTracesPage() {
  const [filters, setFilters] = useState<FlowTraceQueryParams>({ limit: 100 })
  const pages = useCursorPages()
  // A filter changed: back to the first page.
  useEffect(() => pages.reset(), [filters, pages.reset])

  const { data, refetch } = useQuery({
    queryKey: ['flow-traces', filters, pages.cursor],
    queryFn: () => flowTraces.list({ ...filters, cursor: pages.cursor }),
    refetchInterval: 10000,
  })

  const traces = data?.data ?? []
  const total = data?.total ?? 0

  const handleExportCSV = () => {
    const headers = ['Time', 'Event Type', 'Node', 'Detail', 'Call ID', 'Flow ID', 'Flow Name', 'Version']
    const rows = traces.map((t) => [
      new Date(t.timestamp).toISOString(),
      t.event_type,
      t.node_id || '',
      t.detail || '',
      t.call_id,
      t.flow_id,
      t.flow_name || '',
      String(t.flow_version ?? ''),
    ])

    const csv = [
      headers.join(','),
      ...rows.map((r) => r.map(escapeCSVField).join(',')),
    ].join('\n')

    const blob = new Blob([csv], { type: 'text/csv' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `flow-traces-${new Date().toISOString().slice(0, 10)}.csv`
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Flow Execution Traces</h1>
          <p className="text-sm text-muted-foreground">
            View every IVR flow node traversed per call ({total} traces)
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-1" />
            Refresh
          </Button>
          <Button variant="outline" size="sm" onClick={handleExportCSV} disabled={traces.length === 0}>
            <Download className="h-4 w-4 mr-1" />
            CSV
          </Button>
        </div>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap gap-4 items-end p-4 border rounded-lg bg-muted/50">
        <div className="space-y-1">
          <Label className="text-xs">Call ID</Label>
          <Input
            placeholder="call-..."
            className="w-48 h-8 text-sm"
            value={filters.call_id || ''}
            onChange={(e) =>
              setFilters((f) => ({ ...f, call_id: e.target.value || undefined }))
            }
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Flow ID</Label>
          <Input
            placeholder="flow-..."
            className="w-48 h-8 text-sm"
            value={filters.flow_id || ''}
            onChange={(e) =>
              setFilters((f) => ({ ...f, flow_id: e.target.value || undefined }))
            }
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">Event Type</Label>
          <Select
            value={filters.event_type || '__all'}
            onValueChange={(v) =>
              setFilters((f) => ({ ...f, event_type: v === '__all' ? undefined : v }))
            }
          >
            <SelectTrigger className="w-44 h-8 text-sm">
              <SelectValue placeholder="All" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all">All</SelectItem>
              <SelectItem value="started">Started</SelectItem>
              <SelectItem value="node_executed">Node Executed</SelectItem>
              <SelectItem value="waiting_for_input">Waiting for Input</SelectItem>
              <SelectItem value="completed">Completed</SelectItem>
              <SelectItem value="failed">Failed</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setFilters({ limit: 100 })}
        >
          Clear
        </Button>
      </div>

      {/* Data table */}
      <DataTable columns={columns} data={traces} />

      {/* Pagination */}
      {(pages.pageIndex > 0 || data?.next_cursor) && (
        <div className="flex items-center justify-center gap-4">
          <Button
            variant="outline"
            size="sm"
            disabled={pages.pageIndex === 0}
            onClick={pages.previous}
          >
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            {pages.pageIndex * (filters.limit ?? 100) + 1}-{pages.pageIndex * (filters.limit ?? 100) + traces.length}
            {total ? ` of ${total}` : ''}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={!data?.next_cursor}
            onClick={() => data?.next_cursor && pages.next(data.next_cursor)}
          >
            Next
          </Button>
        </div>
      )}
    </div>
  )
}
