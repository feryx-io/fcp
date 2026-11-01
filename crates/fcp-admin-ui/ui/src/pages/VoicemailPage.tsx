import { useState, useMemo } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import {
  Voicemail, Inbox, Trash2, Check, MoreHorizontal, ArrowUpDown, Plus, Pencil, Upload,
} from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { voicemail, users, lineBindings, phoneNumbers, flows } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { MailboxResponse, VoicemailMessage, CreateMailboxRequest, UpdateMailboxRequest } from '@/types'

const createSchema = z.object({
  user_id: z.string().min(1, 'User is required'),
  line_binding_id: z.string().optional(),
  pin: z.string().min(1, 'PIN is required'),
  max_message_duration: z.coerce.number().min(1).max(3600).optional(),
  max_messages: z.coerce.number().min(1).max(10000).optional(),
})

const editSchema = z.object({
  pin: z.string().optional(),
  enabled: z.boolean().optional(),
  max_message_duration: z.coerce.number().min(1).max(3600).optional(),
  max_messages: z.coerce.number().min(1).max(10000).optional(),
  email_notification: z.boolean().optional(),
  email_address: z.string().optional(),
  attach_audio: z.boolean().optional(),
  include_transcription: z.boolean().optional(),
  transcription_enabled: z.boolean().optional(),
  retention_days: z.union([z.coerce.number().min(1).max(365), z.null()]).optional(),
  flow_id: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

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
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<MailboxResponse | null>(null)
  const queryClient = useQueryClient()

  const { data: mailboxes = [], isLoading } = useQuery({
    queryKey: ['voicemail-mailboxes'],
    queryFn: voicemail.listMailboxes,
  })

  // Flows a mailbox can hand its calls to.
  const { data: flowList = [] } = useQuery({
    queryKey: ['flows'],
    queryFn: () => flows.list(),
  })
  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const { data: lbList = [] } = useQuery({
    queryKey: ['line-bindings'],
    queryFn: lineBindings.list,
  })

  const { data: pnList = [] } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const userMap = useMemo(() => {
    const m = new Map<string, string>()
    for (const u of userList) m.set(u.id, u.display_name || u.username)
    return m
  }, [userList])

  const phoneNumberMap = useMemo(() => {
    const m = new Map<string, string>()
    for (const p of pnList) m.set(p.id, p.e164)
    return m
  }, [pnList])

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

  const createMutation = useMutation({
    mutationFn: voicemail.createMailbox,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-mailboxes'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateMailboxRequest }) =>
      voicemail.updateMailbox(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-mailboxes'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: voicemail.deleteMailbox,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['voicemail-mailboxes'] }),
  })

  const markHeardMutation = useMutation({
    mutationFn: (messageId: string) =>
      voicemail.markHeard(messageId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-messages'] })
      queryClient.invalidateQueries({ queryKey: ['voicemail-stats'] })
    },
  })

  const deleteMessageMutation = useMutation({
    mutationFn: (messageId: string) =>
      voicemail.deleteMessage(messageId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['voicemail-messages'] })
      queryClient.invalidateQueries({ queryKey: ['voicemail-stats'] })
    },
  })

  // --- Greeting management ---
  const { data: greetings = [], refetch: refetchGreetings } = useQuery({
    queryKey: ['voicemail-greetings', editItem?.id],
    queryFn: () => voicemail.listGreetings(editItem!.id),
    enabled: !!editItem,
  })

  const [tempDuration, setTempDuration] = useState<number>(168) // default 1 week (168 hours)

  const uploadGreetingMutation = useMutation({
    mutationFn: ({ mailboxId, greetingType, file, expiresInHours }: { mailboxId: string; greetingType: string; file: File; expiresInHours?: number }) =>
      voicemail.uploadGreeting(mailboxId, greetingType, file, expiresInHours),
    onSuccess: () => refetchGreetings(),
  })

  const deleteGreetingMutation = useMutation({
    mutationFn: ({ mailboxId, greetingType }: { mailboxId: string; greetingType: string }) =>
      voicemail.deleteGreeting(mailboxId, greetingType),
    onSuccess: () => refetchGreetings(),
  })

  const toggleGreetingMutation = useMutation({
    mutationFn: ({ mailboxId, greetingType, active }: { mailboxId: string; greetingType: string; active: boolean }) =>
      voicemail.setGreetingActive(mailboxId, greetingType, active),
    onSuccess: () => refetchGreetings(),
  })

  const GREETING_TYPES = ['unavailable', 'temporary', 'busy', 'name'] as const
  const GREETING_LABELS: Record<string, string> = {
    unavailable: 'Standard',
    temporary: 'Alternate',
    busy: 'Busy',
    name: 'Name',
  }

  const handleGreetingUpload = (greetingType: string, file: File) => {
    if (!editItem) return
    const expiresInHours = greetingType === 'temporary' ? tempDuration : undefined
    uploadGreetingMutation.mutate({ mailboxId: editItem.id, greetingType, file, expiresInHours })
  }

  const handleGreetingDelete = (greetingType: string) => {
    if (!editItem) return
    deleteGreetingMutation.mutate({ mailboxId: editItem.id, greetingType })
  }

  const mailboxColumns: ColumnDef<MailboxResponse>[] = [
    {
      id: 'owner',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          User <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      accessorFn: (row) => userMap.get(row.user_id) || row.extension,
      cell: ({ row }) => userMap.get(row.original.user_id) || row.original.extension,
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
      accessorKey: 'retention_days',
      header: 'Retention',
      cell: ({ row }) => (
        <span className="text-sm">
          {row.original.retention_days ? `${row.original.retention_days} days` : 'No limit'}
        </span>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="Actions"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setSelectedMailbox(row.original)}>
              <Inbox className="mr-2 h-4 w-4" />
              Messages
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete mailbox for "${userMap.get(row.original.user_id) || row.original.extension}"?`)) {
                  deleteMutation.mutate(row.original.id)
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
      cell: ({ row }) => new Date(row.original.created_at).toLocaleString(),
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
            <Button variant="ghost" size="icon" aria-label="Actions">
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

  // --- Create form ---
  const createForm = useForm<CreateForm>({ resolver: zodResolver(createSchema), defaultValues: { max_message_duration: 300, max_messages: 100 } })

  const onCreateSubmit = (data: CreateForm) => {
    const tenantId = userList.find(u => u.id === data.user_id)?.tenant_id || ''
    const payload: CreateMailboxRequest = {
      tenant_id: tenantId,
      user_id: data.user_id,
      line_binding_id: data.line_binding_id && data.line_binding_id !== 'none' ? data.line_binding_id : null,
      pin: data.pin,
      max_message_duration: data.max_message_duration,
      max_messages: data.max_messages,
    }
    createMutation.mutate(payload)
  }

  // --- Edit form ---
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      pin: '',
      enabled: editItem.enabled,
      max_message_duration: editItem.max_message_duration,
      max_messages: editItem.max_messages,
      email_notification: editItem.email_notification,
      email_address: editItem.email_address || '',
      attach_audio: editItem.attach_audio,
      include_transcription: editItem.include_transcription,
      transcription_enabled: editItem.transcription_enabled,
      retention_days: editItem.retention_days,
      flow_id: editItem.flow_id ?? '',
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    const payload: UpdateMailboxRequest = {
      enabled: data.enabled,
      max_message_duration: data.max_message_duration,
      max_messages: data.max_messages,
      email_notification: data.email_notification,
      email_address: data.email_address || undefined,
      attach_audio: data.attach_audio,
      include_transcription: data.include_transcription,
      transcription_enabled: data.transcription_enabled,
      retention_days: data.retention_days,
      // Empty: the mailbox takes its calls itself.
      flow_id: data.flow_id ?? '',
    }
    if (data.pin) payload.pin = data.pin
    updateMutation.mutate({ id: editItem.id, data: payload })
  }

  // Checkbox helper — simple controlled checkbox using native input
  const Checkbox = ({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) => (
    <label className="flex items-center gap-2 text-sm cursor-pointer">
      <input type="checkbox" checked={checked} onChange={e => onChange(e.target.checked)} className="rounded border-input" />
      {label}
    </label>
  )

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Voicemail</h1>
          <p className="text-muted-foreground">Manage mailboxes and messages</p>
        </div>
        <Button onClick={() => { createForm.reset({ max_message_duration: 300, max_messages: 100 }); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Mailbox
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable
          columns={mailboxColumns}
          data={mailboxes}
          searchKey="owner"
          searchPlaceholder="Search mailboxes..."
        />
      )}

      {/* Create Mailbox Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Mailbox</DialogTitle>
            <DialogDescription>Create a new voicemail mailbox.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label>User *</Label>
              <Select onValueChange={(v) => createForm.setValue('user_id', v, { shouldValidate: true })}>
                <SelectTrigger><SelectValue placeholder="Select user" /></SelectTrigger>
                <SelectContent>
                  {userList.map(u => (
                    <SelectItem key={u.id} value={u.id}>
                      {u.display_name || u.username} ({u.username})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {createForm.formState.errors.user_id && <p className="text-sm text-destructive">{createForm.formState.errors.user_id.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Line</Label>
              <Select onValueChange={(v) => createForm.setValue('line_binding_id', v, { shouldValidate: true })}>
                <SelectTrigger><SelectValue placeholder="None — the user's extension" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None — the user's extension</SelectItem>
                  {lbList.map(lb => {
                    const pn = phoneNumberMap.get(lb.phone_number_id) || lb.phone_number_id.slice(0, 8)
                    const owner = lb.owner_user_id ? (userMap.get(lb.owner_user_id) || '') : ''
                    return (
                      <SelectItem key={lb.id} value={lb.id}>
                        {pn}{owner ? ` - ${owner}` : ''}
                      </SelectItem>
                    )
                  })}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">The mailbox is the user's and answers their extension. Pick a line only to tie it to one of the user's numbers as well.</p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-pin">PIN *</Label>
              <Input id="c-pin" type="password" placeholder="Numeric PIN" {...createForm.register('pin')} />
              {createForm.formState.errors.pin && <p className="text-sm text-destructive">{createForm.formState.errors.pin.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-max-dur">Max Duration (s)</Label>
                <Input id="c-max-dur" type="number" {...createForm.register('max_message_duration')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-max-msg">Max Messages</Label>
                <Input id="c-max-msg" type="number" {...createForm.register('max_messages')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Mailbox'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Mailbox Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); editForm.reset() } }}>
        <DialogContent className="max-w-md max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Mailbox: {editItem ? (userMap.get(editItem.user_id) || editItem.extension) : ''}</DialogTitle>
            <DialogDescription>
              Owner: {editItem ? (userMap.get(editItem.user_id) || editItem.user_id.slice(0, 8)) : ''}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-pin">New PIN</Label>
              <Input id="e-pin" type="password" placeholder="Leave empty to keep current" {...editForm.register('pin')} />
            </div>
            <div className="space-y-2">
              <Checkbox checked={editForm.watch('enabled') ?? true} onChange={v => editForm.setValue('enabled', v)} label="Enabled" />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-max-dur">Max Duration (s)</Label>
                <Input id="e-max-dur" type="number" {...editForm.register('max_message_duration')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-max-msg">Max Messages</Label>
                <Input id="e-max-msg" type="number" {...editForm.register('max_messages')} />
              </div>
            </div>
            <div className="border-t pt-3 space-y-2">
              <Label htmlFor="e-flow" className="text-xs text-muted-foreground uppercase tracking-wider">Call handling</Label>
              <select
                id="e-flow"
                className="w-full border rounded px-3 py-2 text-sm bg-background"
                value={editForm.watch('flow_id') ?? ''}
                onChange={e => editForm.setValue('flow_id', e.target.value)}
              >
                <option value="">Take a message (the greeting, then record)</option>
                {flowList.map(f => (
                  <option key={f.id} value={f.id}>Run the flow: {f.name}</option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground">
                A flow can say something, ring someone else, send the call to a queue, or still take a
                message with its Voicemail node.
              </p>
            </div>
            <div className="border-t pt-3 space-y-2">
              <Label className="text-xs text-muted-foreground uppercase tracking-wider">Message Retention</Label>
              <select
                className="w-full border rounded px-3 py-2 text-sm bg-background"
                value={editForm.watch('retention_days') ?? ''}
                onChange={e => {
                  const v = e.target.value
                  editForm.setValue('retention_days', v === '' ? null : Number(v))
                }}
              >
                <option value="">No limit</option>
                <option value="7">7 days</option>
                <option value="14">14 days</option>
                <option value="30">30 days</option>
                <option value="60">60 days</option>
                <option value="90">90 days</option>
                <option value="180">180 days</option>
                <option value="365">365 days</option>
              </select>
              <p className="text-xs text-muted-foreground">New and heard messages will expire after this period. Saved messages are exempt.</p>
            </div>
            <div className="border-t pt-3 space-y-2">
              <Label className="text-xs text-muted-foreground uppercase tracking-wider">Email Settings</Label>
              <Checkbox checked={editForm.watch('email_notification') ?? false} onChange={v => editForm.setValue('email_notification', v)} label="Email notification" />
              {editForm.watch('email_notification') && (
                <div className="space-y-2 pl-6">
                  <Input placeholder="user@example.com" {...editForm.register('email_address')} />
                  <Checkbox checked={editForm.watch('attach_audio') ?? false} onChange={v => editForm.setValue('attach_audio', v)} label="Attach audio file" />
                  <Checkbox checked={editForm.watch('include_transcription') ?? false} onChange={v => editForm.setValue('include_transcription', v)} label="Include transcription in email" />
                </div>
              )}
            </div>
            <div className="border-t pt-3 space-y-2">
              <Checkbox checked={editForm.watch('transcription_enabled') ?? false} onChange={v => editForm.setValue('transcription_enabled', v)} label="Transcription enabled" />
            </div>
            <div className="border-t pt-3 space-y-3">
              <Label className="text-xs text-muted-foreground uppercase tracking-wider">Greetings</Label>
              {GREETING_TYPES.map(type_ => {
                const existing = greetings.find(g => g.greeting_type === type_)
                const showRadio = (type_ === 'unavailable' || type_ === 'temporary') && existing
                const expiresAt = existing && type_ === 'temporary' && existing.expires_at
                  ? new Date(existing.expires_at) : null
                const isExpired = expiresAt ? expiresAt.getTime() < Date.now() : false
                return (
                  <div key={type_} className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 min-w-0">
                      {showRadio && (
                        <input
                          type="radio"
                          name="active-greeting"
                          checked={existing.is_active}
                          onChange={() => {
                            if (!editItem || existing.is_active) return
                            toggleGreetingMutation.mutate({ mailboxId: editItem.id, greetingType: type_, active: true })
                          }}
                          disabled={toggleGreetingMutation.isPending}
                          className="border-input"
                          title={existing.is_active ? 'Active — will be played' : 'Click to activate'}
                        />
                      )}
                      <Badge variant="outline" className="shrink-0">{GREETING_LABELS[type_]}</Badge>
                      {existing && type_ === 'temporary' && expiresAt ? (
                        <span className={`text-xs truncate ${isExpired ? 'text-destructive' : 'text-green-600'}`}>
                          {isExpired ? 'Expired' : 'Expires'} {expiresAt.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                        </span>
                      ) : existing ? (
                        <span className="text-xs text-muted-foreground truncate">
                          {new Date(existing.created_at).toLocaleDateString()}
                        </span>
                      ) : null}
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      {existing ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() => handleGreetingDelete(type_)}
                          disabled={deleteGreetingMutation.isPending}
                        >
                          <Trash2 className="h-3.5 w-3.5 text-destructive" />
                        </Button>
                      ) : (
                        <>
                          {type_ === 'temporary' && (
                            <select
                              className="text-xs border rounded px-1.5 py-1 bg-background"
                              value={tempDuration}
                              onChange={e => setTempDuration(Number(e.target.value))}
                            >
                              <option value={24}>24 hours</option>
                              <option value={72}>3 days</option>
                              <option value={168}>1 week</option>
                              <option value={336}>2 weeks</option>
                              <option value={720}>1 month</option>
                            </select>
                          )}
                          <label className="cursor-pointer">
                            <input
                              type="file"
                              accept=".wav,.mp3,audio/*"
                              className="hidden"
                              onChange={e => {
                                const file = e.target.files?.[0]
                                if (file) handleGreetingUpload(type_, file)
                                e.target.value = ''
                              }}
                            />
                            <Button type="button" variant="outline" size="sm" asChild>
                              <span><Upload className="h-3.5 w-3.5 mr-1" /> Upload</span>
                            </Button>
                          </label>
                        </>
                      )}
                    </div>
                  </div>
                )
              })}
              {uploadGreetingMutation.isError && (
                <p className="text-sm text-destructive">{(uploadGreetingMutation.error as Error).message}</p>
              )}
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setEditItem(null)}>Cancel</Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {updateMutation.isPending ? 'Saving...' : 'Save Changes'}
              </Button>
            </div>
            {updateMutation.isError && (
              <p className="text-sm text-destructive">{(updateMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Mailbox Messages Dialog */}
      <Dialog open={!!selectedMailbox} onOpenChange={() => setSelectedMailbox(null)}>
        <DialogContent className="max-w-4xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Voicemail className="h-5 w-5" />
              Mailbox: {selectedMailbox ? (userMap.get(selectedMailbox.user_id) || selectedMailbox.extension) : ''}
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
