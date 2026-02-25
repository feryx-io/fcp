import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  PhoneOff, Pause, Play, ArrowRightLeft, ArrowUpDown,
  Phone, RefreshCw, FileText,
} from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { useNavigate } from 'react-router-dom'
import { calls } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { CallInfo } from '@/types'
import { useWebSocketStore } from '@/stores/websocket'

const callStateBadge = (state: string) => {
  switch (state) {
    case 'ringing': return 'warning' as const
    case 'active': case 'confirmed': return 'success' as const
    case 'held': return 'secondary' as const
    case 'terminated': case 'failed': return 'destructive' as const
    default: return 'outline' as const
  }
}

export function ActiveCallsPage() {
  const navigate = useNavigate()
  const [transferDialog, setTransferDialog] = useState<string | null>(null)
  const [transferTarget, setTransferTarget] = useState('')
  const [selectedCall, setSelectedCall] = useState<CallInfo | null>(null)
  const queryClient = useQueryClient()
  const { connected } = useWebSocketStore()

  const { data: callList = [], isLoading } = useQuery({
    queryKey: ['calls'],
    queryFn: calls.list,
    refetchInterval: connected ? false : 3000, // Poll if WS disconnected
  })

  const hangupMutation = useMutation({
    mutationFn: calls.hangup,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['calls'] }),
  })

  const holdMutation = useMutation({
    mutationFn: calls.hold,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['calls'] }),
  })

  const resumeMutation = useMutation({
    mutationFn: calls.resume,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['calls'] }),
  })

  const transferMutation = useMutation({
    mutationFn: ({ callId, target }: { callId: string; target: string }) =>
      calls.transfer(callId, target),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['calls'] })
      setTransferDialog(null)
      setTransferTarget('')
    },
  })

  const columns: ColumnDef<CallInfo>[] = [
    {
      accessorKey: 'id',
      header: 'Call ID',
      cell: ({ row }) => (
        <button
          className="text-primary hover:underline font-mono text-xs"
          onClick={() => setSelectedCall(row.original)}
        >
          {row.original.id.slice(0, 12)}...
        </button>
      ),
    },
    {
      accessorKey: 'state',
      header: 'State',
      cell: ({ row }) => (
        <Badge variant={callStateBadge(row.original.state)}>
          {row.original.state}
        </Badge>
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
      accessorKey: 'duration_seconds',
      header: 'Duration',
      cell: ({ row }) => {
        const d = row.original.duration_seconds
        return d != null ? formatDuration(d) : '-'
      },
    },
    {
      accessorKey: 'media_anchored',
      header: 'Media',
      cell: ({ row }) => (
        <Badge variant={row.original.media_anchored ? 'success' : 'outline'}>
          {row.original.media_anchored ? 'Anchored' : 'Direct'}
        </Badge>
      ),
    },
    {
      id: 'actions',
      header: 'Actions',
      cell: ({ row }) => {
        const call = row.original
        const isActive = call.state === 'active' || call.state === 'confirmed'
        const isHeld = call.state === 'held'

        return (
          <div className="flex items-center gap-1">
            {isActive && (
              <Button
                variant="ghost"
                size="icon"
                title="Hold"
                onClick={() => holdMutation.mutate(call.id)}
              >
                <Pause className="h-4 w-4" />
              </Button>
            )}
            {isHeld && (
              <Button
                variant="ghost"
                size="icon"
                title="Resume"
                onClick={() => resumeMutation.mutate(call.id)}
              >
                <Play className="h-4 w-4" />
              </Button>
            )}
            {(isActive || isHeld) && (
              <Button
                variant="ghost"
                size="icon"
                title="Transfer"
                onClick={() => {
                  setTransferDialog(call.id)
                  setTransferTarget('')
                }}
              >
                <ArrowRightLeft className="h-4 w-4" />
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              title="SIP Trace"
              onClick={() => navigate(`/admin/calls/${call.id}/sip-trace`)}
            >
              <FileText className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              title="Hangup"
              className="text-destructive hover:text-destructive"
              onClick={() => {
                if (confirm(`Hang up call ${call.id.slice(0, 12)}?`)) {
                  hangupMutation.mutate(call.id)
                }
              }}
            >
              <PhoneOff className="h-4 w-4" />
            </Button>
          </div>
        )
      },
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Active Calls</h1>
          <p className="text-muted-foreground">
            Monitor and manage live calls
            <span className={`ml-2 inline-flex items-center gap-1 text-xs ${connected ? 'text-green-600' : 'text-muted-foreground'}`}>
              <span className={`h-2 w-2 rounded-full ${connected ? 'bg-green-500' : 'bg-muted-foreground'}`} />
              {connected ? 'Live' : 'Polling'}
            </span>
          </p>
        </div>
        <Button
          variant="outline"
          onClick={() => queryClient.invalidateQueries({ queryKey: ['calls'] })}
        >
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={callList} searchKey="from_uri" searchPlaceholder="Search by caller..." />
      )}

      {/* Transfer Dialog */}
      <Dialog open={!!transferDialog} onOpenChange={() => setTransferDialog(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Transfer Call</DialogTitle>
            <DialogDescription>Enter the target extension or SIP URI.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="transfer-target">Transfer To</Label>
              <Input
                id="transfer-target"
                placeholder="e.g., 1001 or sip:user@domain"
                value={transferTarget}
                onChange={(e) => setTransferTarget(e.target.value)}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setTransferDialog(null)}>Cancel</Button>
              <Button
                disabled={!transferTarget || transferMutation.isPending}
                onClick={() => {
                  if (transferDialog) {
                    transferMutation.mutate({ callId: transferDialog, target: transferTarget })
                  }
                }}
              >
                {transferMutation.isPending ? 'Transferring...' : 'Transfer'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* Call Detail Drawer */}
      <Dialog open={!!selectedCall} onOpenChange={() => setSelectedCall(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Phone className="h-5 w-5" />
              Call Details
            </DialogTitle>
            <DialogDescription>Full information about this call.</DialogDescription>
          </DialogHeader>
          {selectedCall && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <p className="text-muted-foreground">Call ID</p>
                  <p className="font-mono text-xs break-all">{selectedCall.id}</p>
                </div>
                <div>
                  <p className="text-muted-foreground">State</p>
                  <Badge variant={callStateBadge(selectedCall.state)}>{selectedCall.state}</Badge>
                </div>
                <div>
                  <p className="text-muted-foreground">From</p>
                  <p className="font-mono">{selectedCall.from_uri}</p>
                  {selectedCall.from_display_name && (
                    <p className="text-muted-foreground text-xs">{selectedCall.from_display_name}</p>
                  )}
                </div>
                <div>
                  <p className="text-muted-foreground">To</p>
                  <p className="font-mono">{selectedCall.to_uri}</p>
                  {selectedCall.to_display_name && (
                    <p className="text-muted-foreground text-xs">{selectedCall.to_display_name}</p>
                  )}
                </div>
                <div>
                  <p className="text-muted-foreground">Started</p>
                  <p>{new Date(selectedCall.created_at).toLocaleString()}</p>
                </div>
                <div>
                  <p className="text-muted-foreground">Duration</p>
                  <p>{selectedCall.duration_seconds != null ? formatDuration(selectedCall.duration_seconds) : '-'}</p>
                </div>
                <div>
                  <p className="text-muted-foreground">Media</p>
                  <Badge variant={selectedCall.media_anchored ? 'success' : 'outline'}>
                    {selectedCall.media_anchored ? 'Anchored' : 'Direct'}
                  </Badge>
                </div>
                {selectedCall.established_at && (
                  <div>
                    <p className="text-muted-foreground">Established</p>
                    <p>{new Date(selectedCall.established_at).toLocaleString()}</p>
                  </div>
                )}
                {selectedCall.failure_reason && (
                  <div className="col-span-2">
                    <p className="text-muted-foreground">Failure Reason</p>
                    <p className="text-destructive">{selectedCall.failure_reason}</p>
                  </div>
                )}
              </div>
              <div className="pt-2 border-t">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => navigate(`/admin/calls/${selectedCall.id}/sip-trace`)}
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
