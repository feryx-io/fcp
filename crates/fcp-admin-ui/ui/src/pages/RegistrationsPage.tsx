import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { ArrowUpDown, Trash2, RefreshCw } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { registrations } from '@/api/endpoints'
import type { Registration } from '@/types'
// NOTE: Registration responses currently do not carry tenant_id, so the
// client-side tenant filter is applied at the list endpoint level by the
// backend for tenant-scoped identities. Super-admins will see all tenants'
// registrations until the backend response is extended with tenant_id.

function formatExpiry(seconds: number): string {
  if (seconds <= 0) return 'Expired'
  if (seconds < 60) return `${seconds}s`
  const mins = Math.floor(seconds / 60)
  const secs = seconds % 60
  if (mins < 60) return `${mins}m ${secs}s`
  const hours = Math.floor(mins / 60)
  return `${hours}h ${mins % 60}m`
}

function formatTimestamp(ts: { secs_since_epoch: number }): string {
  return new Date(ts.secs_since_epoch * 1000).toLocaleString()
}

function extractAorUser(aor: string): string {
  // "sip:alice@domain" -> "alice"
  const match = aor.match(/^sip:([^@]+)@/)
  return match ? match[1] : aor
}

function extractContactAddr(contact: string): string {
  // "<sip:alice@192.168.1.80:62726;transport=tcp>" -> "192.168.1.80:62726 (tcp)"
  const match = contact.match(/<sip:[^@]+@([^;>]+)/)
  const transport = contact.match(/transport=(\w+)/)
  if (!match) return contact
  return transport ? `${match[1]} (${transport[1]})` : match[1]
}

export function RegistrationsPage() {
  const queryClient = useQueryClient()

  const { data: items = [], isLoading, dataUpdatedAt } = useQuery({
    queryKey: ['registrations'],
    queryFn: registrations.list,
    refetchInterval: 30_000,
  })

  const deleteMutation = useMutation({
    mutationFn: registrations.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['registrations'] }),
  })

  const columns: ColumnDef<Registration>[] = [
    {
      accessorKey: 'aor',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          User <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{extractAorUser(row.original.aor)}</div>
          <div className="text-xs text-muted-foreground">{row.original.aor}</div>
        </div>
      ),
    },
    {
      accessorKey: 'contact',
      header: 'Contact',
      cell: ({ row }) => (
        <div>
          <div className="font-mono text-sm">{extractContactAddr(row.original.contact)}</div>
        </div>
      ),
    },
    {
      accessorKey: 'user_agent',
      header: 'User Agent',
      cell: ({ row }) => (
        <span className="text-sm">{row.original.user_agent || '-'}</span>
      ),
    },
    {
      accessorKey: 'expires_in_seconds',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Expires In <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => {
        const secs = row.original.expires_in_seconds
        return (
          <Badge variant={secs > 60 ? 'success' : secs > 0 ? 'warning' : 'destructive'}>
            {formatExpiry(secs)}
          </Badge>
        )
      },
    },
    {
      accessorKey: 'registered_at',
      header: 'Registered At',
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {formatTimestamp(row.original.registered_at)}
        </span>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <Button
          variant="ghost"
          size="icon"
          className="text-destructive"
          onClick={() => {
            if (confirm(`Unregister all contacts for "${extractAorUser(row.original.aor)}"?`)) {
              deleteMutation.mutate(row.original.aor)
            }
          }}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Registrations</h1>
          <p className="text-muted-foreground">
            Active SIP registrations ({items.length} contact{items.length !== 1 ? 's' : ''})
          </p>
        </div>
        <Button
          variant="outline"
          onClick={() => queryClient.invalidateQueries({ queryKey: ['registrations'] })}
        >
          <RefreshCw className="mr-2 h-4 w-4" /> Refresh
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="aor" searchPlaceholder="Search by AOR..." />
      )}

      {dataUpdatedAt > 0 && (
        <p className="text-xs text-muted-foreground text-right">
          Last updated: {new Date(dataUpdatedAt).toLocaleTimeString()}
        </p>
      )}
    </div>
  )
}
