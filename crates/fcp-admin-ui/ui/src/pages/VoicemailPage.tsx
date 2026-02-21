import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  Voicemail, Inbox, Trash2, Check, MoreHorizontal, ArrowUpDown,
} from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { voicemail } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { MailboxResponse, VoicemailMessage } from '@/types'

const messagePriorityBadge = (priority: string) => {
  switch (priority) {
    case 'urgent': return 'destructive' as const
    case 'normal': return 'outline' as const
    default: return 'secondary' as const
  }
}

const messageStatusBadge = (status: string) => {
  switch (status) {
    case 'new': return 'default' as const
    case 'heard': return 'secondary' as const
    case 'saved': return 'success' as const
    default: return 'outline' as const
  }
}

export function VoicemailPage() {
  const [selectedMailbox, setSelectedMailbox] = useState<MailboxResponse | null>(null)
  const queryClient = useQueryClient()

  const { data: mailboxes = [], isLoading } = useQuery({
    queryKey: ['voicemail-mailboxes'],
    queryFn: voicemail.listMailboxes,
  })

  const { data: stats } = useQuery({
    queryKey: ['voicemail-stats', selectedMailbox?.id],
    queryFn: () => voicemail.getMailboxStats(selectedMailbox!.id),
    enabled: !!selectedMailbox,
  })

  const { data: messages = [] } = useQuery({
    queryKey: ['voicemail-messages', selectedMailbox?.id],
    queryFn: () => voicemail.listMessages(selectedMailbox!.id),
    enabled: !!selectedMailbox,
  })

  const markHeardMutation = useMutation({
    mutationFn: (messageId: string) =>
      voicemail.markHeard(selectedMailbox!.id, messageId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-messages'] })
      queryClient.invalidateQueries({ queryKey: ['voicemail-stats'] })
    },
  })

  const deleteMessageMutation = useMutation({
    mutationFn: (messageId: string) =>
      voicemail.deleteMessage(selectedMailbox!.id, messageId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-messages'] })
      queryClient.invalidateQueries({ queryKey: ['voicemail-stats'] })
    },
  })

  const mailboxColumns: ColumnDef<MailboxResponse>[] = [
    {
      accessorKey: 'extension',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Extension <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'success' : 'destructive'}>
          {row.original.enabled ? 'Enabled' : 'Disabled'}
        </Badge>
      ),
    },
    {
      accessorKey: 'email_notification',
      header: 'Email Notify',
      cell: ({ row }) => row.original.email_notification ? (
        <span className="text-sm">{row.original.email_address || 'Yes'}</span>
      ) : (
        <span className="text-muted-foreground text-sm">Off</span>
      ),
    },
    {
      accessorKey: 'transcription_enabled',
      header: 'Transcription',
      cell: ({ row }) => (
        <Badge variant={row.original.transcription_enabled ? 'default' : 'outline'}>
          {row.original.transcription_enabled ? 'On' : 'Off'}
        </Badge>
      ),
    },
    {
      accessorKey: 'max_messages',
      header: 'Max Messages',
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <Button
          variant="outline"
          size="sm"
          onClick={() => setSelectedMailbox(row.original)}
        >
          <Inbox className="mr-2 h-4 w-4" />
          Messages
        </Button>
      ),
    },
  ]

  const messageColumns: ColumnDef<VoicemailMessage>[] = [
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={messageStatusBadge(row.original.status)}>
          {row.original.status}
        </Badge>
      ),
    },
    {
      accessorKey: 'caller',
      header: 'Caller',
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.caller}</span>
          {row.original.caller_name && (
            <p className="text-xs text-muted-foreground">{row.original.caller_name}</p>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'priority',
      header: 'Priority',
      cell: ({ row }) => (
        <Badge variant={messagePriorityBadge(row.original.priority)}>
          {row.original.priority}
        </Badge>
      ),
    },
    {
      accessorKey: 'duration',
      header: 'Duration',
      cell: ({ row }) => formatDuration(row.original.duration),
    },
    {
      accessorKey: 'created_at',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Date <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => new Date(row.original.created_at * 1000).toLocaleString(),
    },
    {
      accessorKey: 'transcription',
      header: 'Transcription',
      cell: ({ row }) => (
        <span className="text-xs max-w-[200px] truncate block">
          {row.original.transcription ?? '-'}
        </span>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon">
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {row.original.status === 'new' && (
              <DropdownMenuItem onClick={() => markHeardMutation.mutate(row.original.id)}>
                <Check className="mr-2 h-4 w-4" />
                Mark Heard
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm('Delete this voicemail message?')) {
                  deleteMessageMutation.mutate(row.original.id)
                }
              }}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Voicemail</h1>
        <p className="text-muted-foreground">Manage mailboxes and messages</p>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable
          columns={mailboxColumns}
          data={mailboxes}
          searchKey="extension"
          searchPlaceholder="Search mailboxes..."
        />
      )}

      {/* Mailbox Messages Dialog */}
      <Dialog open={!!selectedMailbox} onOpenChange={() => setSelectedMailbox(null)}>
        <DialogContent className="max-w-4xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Voicemail className="h-5 w-5" />
              Mailbox: {selectedMailbox?.extension}
            </DialogTitle>
            <DialogDescription>Messages and statistics for this mailbox.</DialogDescription>
          </DialogHeader>

          {/* Stats */}
          {stats && (
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-4">
              <Card>
                <CardHeader className="p-3 pb-1">
                  <CardTitle className="text-xs text-muted-foreground">Total</CardTitle>
                </CardHeader>
                <CardContent className="p-3 pt-0">
                  <p className="text-2xl font-bold">{stats.total_messages}</p>
                </CardContent>
              </Card>
              <Card>
                <CardHeader className="p-3 pb-1">
                  <CardTitle className="text-xs text-muted-foreground">New</CardTitle>
                </CardHeader>
                <CardContent className="p-3 pt-0">
                  <p className="text-2xl font-bold text-primary">{stats.new_messages}</p>
                </CardContent>
              </Card>
              <Card>
                <CardHeader className="p-3 pb-1">
                  <CardTitle className="text-xs text-muted-foreground">Heard</CardTitle>
                </CardHeader>
                <CardContent className="p-3 pt-0">
                  <p className="text-2xl font-bold">{stats.heard_messages}</p>
                </CardContent>
              </Card>
              <Card>
                <CardHeader className="p-3 pb-1">
                  <CardTitle className="text-xs text-muted-foreground">Avg Duration</CardTitle>
                </CardHeader>
                <CardContent className="p-3 pt-0">
                  <p className="text-2xl font-bold">{formatDuration(Math.round(stats.avg_duration))}</p>
                </CardContent>
              </Card>
            </div>
          )}

          {/* Messages Table */}
          <DataTable
            columns={messageColumns}
            data={messages}
            searchKey="caller"
            searchPlaceholder="Search by caller..."
          />
        </DialogContent>
      </Dialog>
    </div>
  )
}
