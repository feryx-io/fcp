import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, Pencil, Trash2, Phone, PhoneOutgoing } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Switch } from '@/components/ui/switch'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { conferences, conferenceLive, prompts, users, type PromptResponse } from '@/api/endpoints'
import type { MeetingResponse, CreateMeetingRequest, UpdateMeetingRequest, User } from '@/types'

const createSchema = z.object({
  room_number: z.string().min(1, 'Room number is required').regex(/^\d+$/, 'Must be numeric'),
  name: z.string().min(1, 'Name is required'),
  guest_pin: z.string().optional(),
  host_pin: z.string().optional(),
  require_guest_pin: z.boolean().optional(),
  max_participants: z.coerce.number().int().min(0).optional(),
  wait_for_moderator: z.boolean().optional(),
  enabled: z.boolean().optional(),
  auto_record: z.boolean().optional(),
  default_locked: z.boolean().optional(),
  permanent: z.boolean().optional(),
  owner_id: z.string().optional(),
  pin_prompt_sound: z.string().optional(),
  invalid_pin_sound: z.string().optional(),
  moh_sound: z.string().optional(),
  pin_prompt_tts: z.string().optional(),
  invalid_pin_tts: z.string().optional(),
  welcome_tts: z.string().optional(),
})

const editSchema = z.object({
  name: z.string().min(1).optional(),
  guest_pin: z.string().optional(),
  host_pin: z.string().optional(),
  require_guest_pin: z.boolean().optional(),
  max_participants: z.coerce.number().int().min(0).optional(),
  wait_for_moderator: z.boolean().optional(),
  auto_record: z.boolean().optional(),
  default_locked: z.boolean().optional(),
  permanent: z.boolean().optional(),
  owner_id: z.string().optional(),
  pin_prompt_sound: z.string().optional(),
  invalid_pin_sound: z.string().optional(),
  moh_sound: z.string().optional(),
  pin_prompt_tts: z.string().optional(),
  invalid_pin_tts: z.string().optional(),
  welcome_tts: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

/** Dropdown that lists audio prompts with audio files, plus a "Default (none)" option. */
function AudioPromptSelect({ value, onChange, promptList, label }: {
  value: string | undefined
  onChange: (val: string) => void
  promptList: PromptResponse[]
  label: string
}) {
  // Only show prompts that actually have audio
  const available = promptList.filter(p => p.has_audio)
  return (
    <div>
      <Label className="text-xs">{label}</Label>
      <Select value={value || '__none__'} onValueChange={(v) => onChange(v === '__none__' ? '' : v)}>
        <SelectTrigger className="text-sm">
          <SelectValue placeholder="Default" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__none__">Default (server TTS)</SelectItem>
          {available.map(p => (
            <SelectItem key={p.id} value={p.audio_path || p.name}>
              {p.name}{p.description ? ` — ${p.description}` : ''}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

export function ConferencesPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<MeetingResponse | null>(null)
  const [dialOutRoom, setDialOutRoom] = useState<string | null>(null)
  const [dialOutDest, setDialOutDest] = useState('')
  const [dialOutGreeting, setDialOutGreeting] = useState('')
  const [dialOutPending, setDialOutPending] = useState(false)
  const [dialOutResult, setDialOutResult] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['meetings'],
    queryFn: conferences.listMeetings,
  })

  const { data: promptList = [] } = useQuery({
    queryKey: ['prompts'],
    queryFn: () => prompts.list(),
  })

  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const createMutation = useMutation({
    mutationFn: conferences.createMeeting,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['meetings'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateMeetingRequest }) =>
      conferences.updateMeeting(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['meetings'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: conferences.deleteMeeting,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['meetings'] }),
  })

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      enabled ? conferences.enableMeeting(id) : conferences.disableMeeting(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['meetings'] }),
  })

  const columns: ColumnDef<MeetingResponse>[] = [
    {
      accessorKey: 'room_number',
      header: 'Room #',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Phone className="h-4 w-4 text-muted-foreground" />
          <span className="font-mono font-medium">{row.original.room_number}</span>
        </div>
      ),
    },
    {
      accessorKey: 'name',
      header: 'Name',
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'success' : 'outline'}>
          {row.original.enabled ? 'Enabled' : 'Disabled'}
        </Badge>
      ),
    },
    {
      id: 'features',
      header: 'Features',
      cell: ({ row }) => {
        const m = row.original
        return (
          <div className="flex gap-1">
            {m.require_guest_pin && (
              <Badge variant="secondary" className="text-xs">PIN</Badge>
            )}
            {m.host_pin && (
              <Badge variant="secondary" className="text-xs">Host PIN</Badge>
            )}
            {m.wait_for_moderator && (
              <Badge variant="secondary" className="text-xs">Wait for Host</Badge>
            )}
            {m.auto_record && (
              <Badge variant="secondary" className="text-xs">Auto Record</Badge>
            )}
            {m.default_locked && (
              <Badge variant="secondary" className="text-xs">Locked</Badge>
            )}
            {m.permanent && (
              <Badge variant="secondary" className="text-xs">Permanent</Badge>
            )}
          </div>
        )
      },
    },
    {
      accessorKey: 'max_participants',
      header: 'Max',
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.max_participants === 0 ? 'Unlimited' : row.original.max_participants}
        </span>
      ),
    },
    {
      accessorKey: 'owner_id',
      header: 'Owner',
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.owner_id || '—'}
        </span>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" /> Edit
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => { setDialOutRoom(row.original.room_number); setDialOutDest(''); setDialOutGreeting(''); setDialOutResult(null) }}>
              <PhoneOutgoing className="mr-2 h-4 w-4" /> Dial Out
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => toggleMutation.mutate({ id: row.original.id, enabled: !row.original.enabled })}
            >
              {row.original.enabled ? 'Disable' : 'Enable'}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete conference bridge "${row.original.name}"?`))
                  deleteMutation.mutate(row.original.id)
              }}
            >
              <Trash2 className="mr-2 h-4 w-4" /> Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Conference Bridges</h1>
          <p className="text-muted-foreground">
            Manage conference rooms, PINs, recording, and room features
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="mr-2 h-4 w-4" /> Create Bridge
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search conferences..." />
      )}

      <CreateDialog open={createOpen} onOpenChange={setCreateOpen} onSubmit={(data) => createMutation.mutate(data)} isPending={createMutation.isPending} promptList={promptList} userList={userList} />
      <EditDialog item={editItem} onOpenChange={(open) => { if (!open) setEditItem(null) }} onSubmit={(data) => { if (editItem) updateMutation.mutate({ id: editItem.id, data }) }} isPending={updateMutation.isPending} promptList={promptList} userList={userList} />

      {/* Dial-Out Dialog */}
      <Dialog open={!!dialOutRoom} onOpenChange={(open) => { if (!open) setDialOutRoom(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Dial Out — Room {dialOutRoom}</DialogTitle>
            <DialogDescription>
              Invite a participant by calling them. Enter a SIP URI or phone number.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>Destination</Label>
              <Input
                value={dialOutDest}
                onChange={(e) => setDialOutDest(e.target.value)}
                placeholder="e.g. 1001, jerry@domain, or +15551234567"
                disabled={dialOutPending}
              />
            </div>
            <div>
              <Label>Greeting (optional)</Label>
              <Input
                value={dialOutGreeting}
                onChange={(e) => setDialOutGreeting(e.target.value)}
                placeholder="e.g. You are being invited to Jerry's conference"
                disabled={dialOutPending}
              />
              <p className="text-xs text-muted-foreground mt-1">TTS text played to the callee before connecting. Leave blank for default greeting.</p>
            </div>
            {dialOutResult && (
              <p className={`text-sm ${dialOutResult.startsWith('Error') ? 'text-destructive' : 'text-green-600'}`}>
                {dialOutResult}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setDialOutRoom(null)} disabled={dialOutPending}>Cancel</Button>
              <Button
                disabled={!dialOutDest.trim() || dialOutPending}
                onClick={async () => {
                  setDialOutPending(true)
                  setDialOutResult(null)
                  try {
                    const opts: { greeting_tts?: string } = {}
                    if (dialOutGreeting.trim()) opts.greeting_tts = dialOutGreeting.trim()
                    const res = await conferenceLive.dialOut(dialOutRoom!, dialOutDest.trim(), opts)
                    setDialOutResult(`Connected: ${res.participant_id}`)
                    setTimeout(() => setDialOutRoom(null), 2000)
                  } catch (e) {
                    setDialOutResult(`Error: ${(e as Error).message}`)
                  } finally {
                    setDialOutPending(false)
                  }
                }}
              >
                <PhoneOutgoing className="mr-2 h-4 w-4" />
                {dialOutPending ? 'Calling...' : 'Call'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function CreateDialog({ open, onOpenChange, onSubmit, isPending, promptList, userList }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (data: CreateMeetingRequest) => void
  isPending: boolean
  promptList: PromptResponse[]
  userList: User[]
}) {
  const { register, handleSubmit, formState: { errors }, reset, setValue, watch } = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
    defaultValues: { enabled: true, require_guest_pin: false, wait_for_moderator: false, auto_record: false, default_locked: false, permanent: false, max_participants: 0 },
  })

  const onFormSubmit = (data: CreateForm) => {
    onSubmit({
      room_number: data.room_number,
      name: data.name,
      guest_pin: data.guest_pin || undefined,
      host_pin: data.host_pin || undefined,
      require_guest_pin: data.require_guest_pin,
      max_participants: data.max_participants,
      wait_for_moderator: data.wait_for_moderator,
      enabled: data.enabled,
      auto_record: data.auto_record,
      default_locked: data.default_locked,
      permanent: data.permanent,
      owner_id: data.owner_id || undefined,
      pin_prompt_sound: data.pin_prompt_sound || undefined,
      invalid_pin_sound: data.invalid_pin_sound || undefined,
      moh_sound: data.moh_sound || undefined,
      pin_prompt_tts: data.pin_prompt_tts || undefined,
      invalid_pin_tts: data.invalid_pin_tts || undefined,
      welcome_tts: data.welcome_tts || undefined,
    })
    reset()
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Create Conference Bridge</DialogTitle>
          <DialogDescription>Configure a new conference room</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onFormSubmit)} className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>Room Number</Label>
              <Input {...register('room_number')} placeholder="8001" />
              {errors.room_number && <p className="text-xs text-destructive mt-1">{errors.room_number.message}</p>}
            </div>
            <div>
              <Label>Name</Label>
              <Input {...register('name')} placeholder="Main Conference" />
              {errors.name && <p className="text-xs text-destructive mt-1">{errors.name.message}</p>}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>Guest PIN</Label>
              <Input {...register('guest_pin')} placeholder="Optional" type="password" />
            </div>
            <div>
              <Label>Host PIN</Label>
              <Input {...register('host_pin')} placeholder="Optional" type="password" />
            </div>
          </div>

          <div>
            <Label>Max Participants</Label>
            <Input {...register('max_participants')} type="number" min="0" placeholder="0 = unlimited" />
          </div>

          <div>
            <Label>Owner</Label>
            <Select value={watch('owner_id') || '__none__'} onValueChange={(v) => setValue('owner_id', v === '__none__' ? '' : v)}>
              <SelectTrigger><SelectValue placeholder="No owner" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">No owner (admin only)</SelectItem>
                {userList.map(u => (
                  <SelectItem key={u.username} value={u.username}>
                    {u.display_name || u.username} ({u.username})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label>Enabled</Label>
              <Switch checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Require Guest PIN</Label>
              <Switch checked={watch('require_guest_pin')} onCheckedChange={(v) => setValue('require_guest_pin', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Wait for Moderator</Label>
              <Switch checked={watch('wait_for_moderator')} onCheckedChange={(v) => setValue('wait_for_moderator', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Auto Record</Label>
              <Switch checked={watch('auto_record')} onCheckedChange={(v) => setValue('auto_record', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Default Locked</Label>
              <Switch checked={watch('default_locked')} onCheckedChange={(v) => setValue('default_locked', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Permanent Room</Label>
              <Switch checked={watch('permanent')} onCheckedChange={(v) => setValue('permanent', v)} />
            </div>
          </div>

          <details className="border rounded-md p-3">
            <summary className="text-sm font-medium cursor-pointer">Audio Prompts (optional overrides)</summary>
            <p className="text-xs text-muted-foreground mt-1 mb-3">Leave blank to use server defaults. WAV path overrides take priority over TTS text.</p>
            <div className="space-y-3 mt-2">
              <AudioPromptSelect label="PIN Prompt Audio" value={watch('pin_prompt_sound')} onChange={(v) => setValue('pin_prompt_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="Invalid PIN Audio" value={watch('invalid_pin_sound')} onChange={(v) => setValue('invalid_pin_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="MOH / Hold Music" value={watch('moh_sound')} onChange={(v) => setValue('moh_sound', v)} promptList={promptList} />
              <div>
                <Label className="text-xs">PIN Prompt TTS Text</Label>
                <Input {...register('pin_prompt_tts')} placeholder="e.g. Please enter your PIN" className="text-sm" />
              </div>
              <div>
                <Label className="text-xs">Invalid PIN TTS Text</Label>
                <Input {...register('invalid_pin_tts')} placeholder="e.g. Incorrect PIN, try again" className="text-sm" />
              </div>
              <div>
                <Label className="text-xs">Welcome TTS Text</Label>
                <Input {...register('welcome_tts')} placeholder="e.g. Welcome to the meeting. Use {count} for participant count." className="text-sm" />
              </div>
            </div>
          </details>

          <Button type="submit" className="w-full" disabled={isPending}>
            {isPending ? 'Creating...' : 'Create Bridge'}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function EditDialog({ item, onOpenChange, onSubmit, isPending, promptList, userList }: {
  item: MeetingResponse | null
  onOpenChange: (open: boolean) => void
  onSubmit: (data: UpdateMeetingRequest) => void
  isPending: boolean
  promptList: PromptResponse[]
  userList: User[]
}) {
  const { register, handleSubmit, reset, setValue, watch } = useForm<EditForm>({
    resolver: zodResolver(editSchema),
  })

  // Reset form when item changes
  const [lastId, setLastId] = useState<string | null>(null)
  if (item && item.id !== lastId) {
    setLastId(item.id)
    reset({
      name: item.name,
      guest_pin: item.guest_pin || '',
      host_pin: item.host_pin || '',
      require_guest_pin: item.require_guest_pin,
      max_participants: item.max_participants,
      wait_for_moderator: item.wait_for_moderator,
      auto_record: item.auto_record,
      default_locked: item.default_locked,
      permanent: item.permanent,
      owner_id: item.owner_id || '',
      pin_prompt_sound: item.pin_prompt_sound || '',
      invalid_pin_sound: item.invalid_pin_sound || '',
      moh_sound: item.moh_sound || '',
      pin_prompt_tts: item.pin_prompt_tts || '',
      invalid_pin_tts: item.invalid_pin_tts || '',
      welcome_tts: item.welcome_tts || '',
    })
  }

  const onFormSubmit = (data: EditForm) => {
    const update: UpdateMeetingRequest = {}
    if (data.name) update.name = data.name
    if (data.guest_pin !== undefined) update.guest_pin = data.guest_pin || null
    if (data.host_pin !== undefined) update.host_pin = data.host_pin || null
    if (data.require_guest_pin !== undefined) update.require_guest_pin = data.require_guest_pin
    if (data.max_participants !== undefined) update.max_participants = data.max_participants
    if (data.wait_for_moderator !== undefined) update.wait_for_moderator = data.wait_for_moderator
    if (data.auto_record !== undefined) update.auto_record = data.auto_record
    if (data.default_locked !== undefined) update.default_locked = data.default_locked
    if (data.permanent !== undefined) update.permanent = data.permanent
    if (data.owner_id !== undefined) update.owner_id = data.owner_id || null
    if (data.pin_prompt_sound !== undefined) update.pin_prompt_sound = data.pin_prompt_sound || null
    if (data.invalid_pin_sound !== undefined) update.invalid_pin_sound = data.invalid_pin_sound || null
    if (data.moh_sound !== undefined) update.moh_sound = data.moh_sound || null
    if (data.pin_prompt_tts !== undefined) update.pin_prompt_tts = data.pin_prompt_tts || null
    if (data.invalid_pin_tts !== undefined) update.invalid_pin_tts = data.invalid_pin_tts || null
    if (data.welcome_tts !== undefined) update.welcome_tts = data.welcome_tts || null
    onSubmit(update)
  }

  return (
    <Dialog open={!!item} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Edit Conference Bridge</DialogTitle>
          <DialogDescription>
            Room {item?.room_number} — {item?.name}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onFormSubmit)} className="space-y-4">
          <div>
            <Label>Name</Label>
            <Input {...register('name')} />
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>Guest PIN</Label>
              <Input {...register('guest_pin')} placeholder="None" type="password" />
            </div>
            <div>
              <Label>Host PIN</Label>
              <Input {...register('host_pin')} placeholder="None" type="password" />
            </div>
          </div>

          <div>
            <Label>Max Participants</Label>
            <Input {...register('max_participants')} type="number" min="0" />
          </div>

          <div>
            <Label>Owner</Label>
            <Select value={watch('owner_id') || '__none__'} onValueChange={(v) => setValue('owner_id', v === '__none__' ? '' : v)}>
              <SelectTrigger><SelectValue placeholder="No owner" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">No owner (admin only)</SelectItem>
                {userList.map(u => (
                  <SelectItem key={u.username} value={u.username}>
                    {u.display_name || u.username} ({u.username})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label>Require Guest PIN</Label>
              <Switch checked={watch('require_guest_pin')} onCheckedChange={(v) => setValue('require_guest_pin', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Wait for Moderator</Label>
              <Switch checked={watch('wait_for_moderator')} onCheckedChange={(v) => setValue('wait_for_moderator', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Auto Record</Label>
              <Switch checked={watch('auto_record')} onCheckedChange={(v) => setValue('auto_record', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Default Locked</Label>
              <Switch checked={watch('default_locked')} onCheckedChange={(v) => setValue('default_locked', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Permanent Room</Label>
              <Switch checked={watch('permanent')} onCheckedChange={(v) => setValue('permanent', v)} />
            </div>
          </div>

          <details className="border rounded-md p-3">
            <summary className="text-sm font-medium cursor-pointer">Audio Prompts (optional overrides)</summary>
            <p className="text-xs text-muted-foreground mt-1 mb-3">Leave blank to use server defaults. WAV path overrides take priority over TTS text.</p>
            <div className="space-y-3 mt-2">
              <AudioPromptSelect label="PIN Prompt Audio" value={watch('pin_prompt_sound')} onChange={(v) => setValue('pin_prompt_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="Invalid PIN Audio" value={watch('invalid_pin_sound')} onChange={(v) => setValue('invalid_pin_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="MOH / Hold Music" value={watch('moh_sound')} onChange={(v) => setValue('moh_sound', v)} promptList={promptList} />
              <div>
                <Label className="text-xs">PIN Prompt TTS Text</Label>
                <Input {...register('pin_prompt_tts')} placeholder="e.g. Please enter your PIN" className="text-sm" />
              </div>
              <div>
                <Label className="text-xs">Invalid PIN TTS Text</Label>
                <Input {...register('invalid_pin_tts')} placeholder="e.g. Incorrect PIN, try again" className="text-sm" />
              </div>
              <div>
                <Label className="text-xs">Welcome TTS Text</Label>
                <Input {...register('welcome_tts')} placeholder="e.g. Welcome to the meeting. Use {count} for participant count." className="text-sm" />
              </div>
            </div>
          </details>

          <Button type="submit" className="w-full" disabled={isPending}>
            {isPending ? 'Saving...' : 'Save Changes'}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  )
}
