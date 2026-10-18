import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, Pencil, Trash2, Phone, PhoneOutgoing, ArrowUp, X, Video, Link as LinkIcon } from 'lucide-react'
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
import { conferences, conferenceLive, joinTokens, prompts, users, type PromptResponse } from '@/api/endpoints'
import type { MeetingResponse, CreateMeetingRequest, UpdateMeetingRequest, User } from '@/types'

/** A codec the conference server encodes: what the API accepts. */
type VideoCodecName = NonNullable<CreateMeetingRequest['video_codecs']>[number]
import { useTenantListFilter } from '@/components/TenantSelector'
import { LiveVideoDialog } from '@/components/LiveVideoDialog'
import { JoinLinkDialog } from '@/components/JoinLinkDialog'

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
  play_entry_tone: z.boolean().optional(),
  play_exit_tone: z.boolean().optional(),
  pin_prompt_sound: z.string().optional(),
  invalid_pin_sound: z.string().optional(),
  moh_sound: z.string().optional(),
  join_sound: z.string().optional(),
  exit_sound: z.string().optional(),
  pin_prompt_tts: z.string().optional(),
  invalid_pin_tts: z.string().optional(),
  welcome_tts: z.string().optional(),
  video_enabled: z.boolean().optional(),
  video_layout: z.string().optional(),
  video_max_participants: z.coerce.number().int().min(1, 'Must be 1–16').max(16, 'Must be 1–16').optional(),
  video_max_resolution: z.string().optional(),
  video_fps: z.coerce.number().int().min(1, 'Must be 1–30').max(30, 'Must be 1–30').optional(),
  video_exclude_self: z.boolean().optional(),
  video_passthrough: z.enum(['off', 'auto', 'on']).optional(),
  video_content_share: z.enum(['off', 'hosts', 'anyone']).optional(),
  video_content_layout: z.enum(['presentation', 'content_only']).optional(),
  video_codecs: z.array(z.string()).optional(),
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
  play_entry_tone: z.boolean().optional(),
  play_exit_tone: z.boolean().optional(),
  pin_prompt_sound: z.string().optional(),
  invalid_pin_sound: z.string().optional(),
  moh_sound: z.string().optional(),
  join_sound: z.string().optional(),
  exit_sound: z.string().optional(),
  pin_prompt_tts: z.string().optional(),
  invalid_pin_tts: z.string().optional(),
  welcome_tts: z.string().optional(),
  video_enabled: z.boolean().optional(),
  video_layout: z.string().optional(),
  video_max_participants: z.coerce.number().int().min(1, 'Must be 1–16').max(16, 'Must be 1–16').optional(),
  video_max_resolution: z.string().optional(),
  video_fps: z.coerce.number().int().min(1, 'Must be 1–30').max(30, 'Must be 1–30').optional(),
  video_exclude_self: z.boolean().optional(),
  video_passthrough: z.enum(['off', 'auto', 'on']).optional(),
  video_content_share: z.enum(['off', 'hosts', 'anyone']).optional(),
  video_content_layout: z.enum(['presentation', 'content_only']).optional(),
  video_codecs: z.array(z.string()).optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

// Mirrors fcp_core::meeting::{VIDEO_LAYOUTS, VIDEO_RESOLUTIONS} and VideoCodec::ALL.
const VIDEO_LAYOUTS = [
  { value: 'grid', label: 'Grid' },
  { value: 'active_speaker', label: 'Active speaker' },
  { value: 'spotlight', label: 'Spotlight' },
  { value: 'pip', label: 'Picture-in-picture' },
]
const VIDEO_RESOLUTIONS = ['180p', '360p', '720p', '1080p']
// Mirrors fcp_core::meeting::{VIDEO_CONTENT_SHARE_MODES, VIDEO_CONTENT_LAYOUTS} (§15.6).
const CONTENT_SHARE_MODES = [
  { value: 'anyone', label: 'Anyone' },
  { value: 'hosts', label: 'Hosts only' },
  { value: 'off', label: 'Off' },
]
const CONTENT_LAYOUTS = [
  { value: 'presentation', label: 'Presentation — screen large, cameras in a strip' },
  { value: 'content_only', label: 'Screen only' },
]
const VIDEO_CODECS = ['H264', 'H265', 'VP8', 'VP9', 'AV1']
const DEFAULT_VIDEO_CODECS = ['H264', 'VP8']

/** Ordered codec preference list: remove / move-up per entry, plus a dropdown to append. */
function CodecOrderPicker({ value, onChange }: {
  value: string[]
  onChange: (codecs: string[]) => void
}) {
  const remaining = VIDEO_CODECS.filter(c => !value.includes(c))
  const moveUp = (i: number) => {
    if (i === 0) return
    const next = [...value]
    const above = next[i - 1]
    next[i - 1] = next[i]
    next[i] = above
    onChange(next)
  }
  return (
    <div>
      <Label className="text-xs">Codecs (preference order)</Label>
      <div className="flex flex-wrap items-center gap-1 mt-1">
        {value.map((codec, i) => (
          <Badge key={codec} variant="secondary" className="text-xs gap-1 pr-1">
            <span className="text-muted-foreground">{i + 1}.</span> {codec}
            <button type="button" className="ml-1 hover:text-foreground disabled:opacity-30" disabled={i === 0} onClick={() => moveUp(i)} aria-label={`Move ${codec} up`}>
              <ArrowUp className="h-3 w-3" />
            </button>
            <button type="button" className="hover:text-destructive" onClick={() => onChange(value.filter(c => c !== codec))} aria-label={`Remove ${codec}`}>
              <X className="h-3 w-3" />
            </button>
          </Badge>
        ))}
        {remaining.length > 0 && (
          <Select value="" onValueChange={(v) => onChange([...value, v])}>
            <SelectTrigger className="h-7 w-28 text-xs"><SelectValue placeholder="Add codec" /></SelectTrigger>
            <SelectContent>
              {remaining.map(c => <SelectItem key={c} value={c}>{c}</SelectItem>)}
            </SelectContent>
          </Select>
        )}
      </div>
      {value.length === 0 && <p className="text-xs text-destructive mt-1">At least one codec is required</p>}
    </div>
  )
}

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
  const [liveVideoRoom, setLiveVideoRoom] = useState<string | null>(null)
  const [joinLinkMeeting, setJoinLinkMeeting] = useState<MeetingResponse | null>(null)
  const queryClient = useQueryClient()

  const { data: rawItems = [], isLoading } = useQuery({
    queryKey: ['meetings'],
    queryFn: conferences.listMeetings,
  })
  const items = useTenantListFilter(rawItems)

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
            {m.video_enabled && (
              <Badge variant="secondary" className="text-xs">Video</Badge>
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
            <Button variant="ghost" size="icon" aria-label="Actions"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" /> Edit
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => { setDialOutRoom(row.original.room_number); setDialOutDest(''); setDialOutGreeting(''); setDialOutResult(null) }}>
              <PhoneOutgoing className="mr-2 h-4 w-4" /> Dial Out
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setLiveVideoRoom(row.original.room_number)}>
              <Video className="mr-2 h-4 w-4" /> Live Video
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setJoinLinkMeeting(row.original)}>
              <LinkIcon className="mr-2 h-4 w-4" /> Join in browser
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

      <LiveVideoDialog room={liveVideoRoom} onOpenChange={(open) => { if (!open) setLiveVideoRoom(null) }} />
      <JoinLinkDialog meeting={joinLinkMeeting} onOpenChange={(open) => { if (!open) setJoinLinkMeeting(null) }} mint={joinTokens.mint} />

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
    defaultValues: {
      enabled: true, require_guest_pin: false, wait_for_moderator: false, auto_record: false, default_locked: false, permanent: false, max_participants: 0,
      video_enabled: false, video_layout: 'grid', video_max_participants: 16, video_max_resolution: '720p', video_fps: 15, video_exclude_self: false, video_passthrough: 'auto', video_content_share: 'anyone', video_content_layout: 'presentation', video_codecs: DEFAULT_VIDEO_CODECS,
    },
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
      play_entry_tone: data.play_entry_tone,
      play_exit_tone: data.play_exit_tone,
      pin_prompt_sound: data.pin_prompt_sound || undefined,
      invalid_pin_sound: data.invalid_pin_sound || undefined,
      moh_sound: data.moh_sound || undefined,
      join_sound: data.join_sound || undefined,
      exit_sound: data.exit_sound || undefined,
      pin_prompt_tts: data.pin_prompt_tts || undefined,
      invalid_pin_tts: data.invalid_pin_tts || undefined,
      welcome_tts: data.welcome_tts || undefined,
      video_enabled: data.video_enabled,
      video_layout: data.video_layout,
      video_max_participants: data.video_max_participants,
      video_max_resolution: data.video_max_resolution,
      video_fps: data.video_fps,
      video_exclude_self: data.video_exclude_self,
      video_passthrough: data.video_passthrough,
      video_content_share: data.video_content_share,
      video_content_layout: data.video_content_layout,
      video_codecs: data.video_codecs as VideoCodecName[] | undefined,
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
              <div className="flex items-center justify-between">
                <Label className="text-xs">Entry Tone</Label>
                <Switch checked={watch('play_entry_tone') !== false} onCheckedChange={(v) => setValue('play_entry_tone', v)} />
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-xs">Exit Tone</Label>
                <Switch checked={watch('play_exit_tone') !== false} onCheckedChange={(v) => setValue('play_exit_tone', v)} />
              </div>
              <AudioPromptSelect label="Join Tone" value={watch('join_sound')} onChange={(v) => setValue('join_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="Exit Tone" value={watch('exit_sound')} onChange={(v) => setValue('exit_sound', v)} promptList={promptList} />
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

          <details className="border rounded-md p-3">
            <summary className="text-sm font-medium cursor-pointer">Video</summary>
            <p className="text-xs text-muted-foreground mt-1 mb-3">Composite video for participants that offer it. Audio-only participants are unaffected.</p>
            <div className="space-y-3 mt-2">
              <div className="flex items-center justify-between">
                <Label className="text-xs">Video Enabled</Label>
                <Switch checked={watch('video_enabled') === true} onCheckedChange={(v) => setValue('video_enabled', v)} />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Layout</Label>
                  <Select value={watch('video_layout') || 'grid'} onValueChange={(v) => setValue('video_layout', v)}>
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {VIDEO_LAYOUTS.map(l => <SelectItem key={l.value} value={l.value}>{l.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label className="text-xs">Max Resolution</Label>
                  <Select value={watch('video_max_resolution') || '720p'} onValueChange={(v) => setValue('video_max_resolution', v)}>
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {VIDEO_RESOLUTIONS.map(r => <SelectItem key={r} value={r}>{r}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Frame Rate (1–30)</Label>
                  <Input {...register('video_fps')} type="number" min="1" max="30" className="text-sm" />
                  {errors.video_fps && <p className="text-xs text-destructive mt-1">{errors.video_fps.message}</p>}
                </div>
                <div>
                  <Label className="text-xs">Max Video Tiles (1–16)</Label>
                  <Input {...register('video_max_participants')} type="number" min="1" max="16" className="text-sm" />
                  {errors.video_max_participants && <p className="text-xs text-destructive mt-1">{errors.video_max_participants.message}</p>}
                </div>
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-xs">Exclude Own Tile</Label>
                <Switch checked={watch('video_exclude_self') === true} onCheckedChange={(v) => setValue('video_exclude_self', v)} />
              </div>
              <div>
                <Label className="text-xs">Direct Forwarding</Label>
                <Select
                  value={watch('video_passthrough') ?? 'auto'}
                  onValueChange={(v) => setValue('video_passthrough', v as 'off' | 'auto' | 'on')}
                >
                  <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto — when one person is spotlighted</SelectItem>
                    <SelectItem value="on">On — whenever one person is on screen</SelectItem>
                    <SelectItem value="off">Off — always compose</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground mt-1">
                  Sends a viewer the speaker&apos;s own stream instead of a composed one:
                  sharper and lower latency, but with no name banner or speaking border.
                </p>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Screen Sharing</Label>
                  <Select
                    value={watch('video_content_share') ?? 'anyone'}
                    onValueChange={(v) => setValue('video_content_share', v as 'off' | 'hosts' | 'anyone')}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {CONTENT_SHARE_MODES.map(m => <SelectItem key={m.value} value={m.value}>{m.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label className="text-xs">Shared Screen Layout</Label>
                  <Select
                    value={watch('video_content_layout') ?? 'presentation'}
                    onValueChange={(v) => setValue('video_content_layout', v as 'presentation' | 'content_only')}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {CONTENT_LAYOUTS.map(l => <SelectItem key={l.value} value={l.value}>{l.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                One screen at a time: the first to share holds it until they stop or a host ends it.
                Under &quot;Hosts only&quot; a guest&apos;s screen is never shown.
              </p>
              <CodecOrderPicker value={watch('video_codecs') ?? DEFAULT_VIDEO_CODECS} onChange={(v) => setValue('video_codecs', v)} />
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
  const { register, handleSubmit, formState: { errors }, reset, setValue, watch } = useForm<EditForm>({
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
      play_entry_tone: item.play_entry_tone,
      play_exit_tone: item.play_exit_tone,
      pin_prompt_sound: item.pin_prompt_sound || '',
      invalid_pin_sound: item.invalid_pin_sound || '',
      moh_sound: item.moh_sound || '',
      join_sound: item.join_sound || '',
      exit_sound: item.exit_sound || '',
      pin_prompt_tts: item.pin_prompt_tts || '',
      invalid_pin_tts: item.invalid_pin_tts || '',
      welcome_tts: item.welcome_tts || '',
      video_enabled: item.video_enabled ?? false,
      video_layout: item.video_layout ?? 'grid',
      video_max_participants: item.video_max_participants ?? 16,
      video_max_resolution: item.video_max_resolution ?? '720p',
      video_fps: item.video_fps ?? 15,
      video_exclude_self: item.video_exclude_self ?? false,
      video_passthrough: (item.video_passthrough as 'off' | 'auto' | 'on') ?? 'auto',
      video_content_share: (item.video_content_share as 'off' | 'hosts' | 'anyone') ?? 'anyone',
      video_content_layout: (item.video_content_layout as 'presentation' | 'content_only') ?? 'presentation',
      video_codecs: item.video_codecs ?? DEFAULT_VIDEO_CODECS,
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
    if (data.play_entry_tone !== undefined) update.play_entry_tone = data.play_entry_tone
    if (data.play_exit_tone !== undefined) update.play_exit_tone = data.play_exit_tone
    if (data.pin_prompt_sound !== undefined) update.pin_prompt_sound = data.pin_prompt_sound || null
    if (data.invalid_pin_sound !== undefined) update.invalid_pin_sound = data.invalid_pin_sound || null
    if (data.moh_sound !== undefined) update.moh_sound = data.moh_sound || null
    if (data.join_sound !== undefined) update.join_sound = data.join_sound || null
    if (data.exit_sound !== undefined) update.exit_sound = data.exit_sound || null
    if (data.pin_prompt_tts !== undefined) update.pin_prompt_tts = data.pin_prompt_tts || null
    if (data.invalid_pin_tts !== undefined) update.invalid_pin_tts = data.invalid_pin_tts || null
    if (data.welcome_tts !== undefined) update.welcome_tts = data.welcome_tts || null
    if (data.video_enabled !== undefined) update.video_enabled = data.video_enabled
    if (data.video_layout !== undefined) update.video_layout = data.video_layout
    if (data.video_max_participants !== undefined) update.video_max_participants = data.video_max_participants
    if (data.video_max_resolution !== undefined) update.video_max_resolution = data.video_max_resolution
    if (data.video_fps !== undefined) update.video_fps = data.video_fps
    if (data.video_exclude_self !== undefined) update.video_exclude_self = data.video_exclude_self
    if (data.video_passthrough !== undefined) update.video_passthrough = data.video_passthrough
    if (data.video_content_share !== undefined) update.video_content_share = data.video_content_share
    if (data.video_content_layout !== undefined) update.video_content_layout = data.video_content_layout
    if (data.video_codecs !== undefined) update.video_codecs = data.video_codecs as VideoCodecName[]
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
              <div className="flex items-center justify-between">
                <Label className="text-xs">Entry Tone</Label>
                <Switch checked={watch('play_entry_tone') !== false} onCheckedChange={(v) => setValue('play_entry_tone', v)} />
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-xs">Exit Tone</Label>
                <Switch checked={watch('play_exit_tone') !== false} onCheckedChange={(v) => setValue('play_exit_tone', v)} />
              </div>
              <AudioPromptSelect label="Join Tone" value={watch('join_sound')} onChange={(v) => setValue('join_sound', v)} promptList={promptList} />
              <AudioPromptSelect label="Exit Tone" value={watch('exit_sound')} onChange={(v) => setValue('exit_sound', v)} promptList={promptList} />
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

          <details className="border rounded-md p-3">
            <summary className="text-sm font-medium cursor-pointer">Video</summary>
            <p className="text-xs text-muted-foreground mt-1 mb-3">Composite video for participants that offer it. Audio-only participants are unaffected.</p>
            <div className="space-y-3 mt-2">
              <div className="flex items-center justify-between">
                <Label className="text-xs">Video Enabled</Label>
                <Switch checked={watch('video_enabled') === true} onCheckedChange={(v) => setValue('video_enabled', v)} />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Layout</Label>
                  <Select value={watch('video_layout') || 'grid'} onValueChange={(v) => setValue('video_layout', v)}>
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {VIDEO_LAYOUTS.map(l => <SelectItem key={l.value} value={l.value}>{l.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label className="text-xs">Max Resolution</Label>
                  <Select value={watch('video_max_resolution') || '720p'} onValueChange={(v) => setValue('video_max_resolution', v)}>
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {VIDEO_RESOLUTIONS.map(r => <SelectItem key={r} value={r}>{r}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Frame Rate (1–30)</Label>
                  <Input {...register('video_fps')} type="number" min="1" max="30" className="text-sm" />
                  {errors.video_fps && <p className="text-xs text-destructive mt-1">{errors.video_fps.message}</p>}
                </div>
                <div>
                  <Label className="text-xs">Max Video Tiles (1–16)</Label>
                  <Input {...register('video_max_participants')} type="number" min="1" max="16" className="text-sm" />
                  {errors.video_max_participants && <p className="text-xs text-destructive mt-1">{errors.video_max_participants.message}</p>}
                </div>
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-xs">Exclude Own Tile</Label>
                <Switch checked={watch('video_exclude_self') === true} onCheckedChange={(v) => setValue('video_exclude_self', v)} />
              </div>
              <div>
                <Label className="text-xs">Direct Forwarding</Label>
                <Select
                  value={watch('video_passthrough') ?? 'auto'}
                  onValueChange={(v) => setValue('video_passthrough', v as 'off' | 'auto' | 'on')}
                >
                  <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto — when one person is spotlighted</SelectItem>
                    <SelectItem value="on">On — whenever one person is on screen</SelectItem>
                    <SelectItem value="off">Off — always compose</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground mt-1">
                  Sends a viewer the speaker&apos;s own stream instead of a composed one:
                  sharper and lower latency, but with no name banner or speaking border.
                </p>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-xs">Screen Sharing</Label>
                  <Select
                    value={watch('video_content_share') ?? 'anyone'}
                    onValueChange={(v) => setValue('video_content_share', v as 'off' | 'hosts' | 'anyone')}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {CONTENT_SHARE_MODES.map(m => <SelectItem key={m.value} value={m.value}>{m.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label className="text-xs">Shared Screen Layout</Label>
                  <Select
                    value={watch('video_content_layout') ?? 'presentation'}
                    onValueChange={(v) => setValue('video_content_layout', v as 'presentation' | 'content_only')}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {CONTENT_LAYOUTS.map(l => <SelectItem key={l.value} value={l.value}>{l.label}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                One screen at a time: the first to share holds it until they stop or a host ends it.
                Under &quot;Hosts only&quot; a guest&apos;s screen is never shown.
              </p>
              <CodecOrderPicker value={watch('video_codecs') ?? DEFAULT_VIDEO_CODECS} onChange={(v) => setValue('video_codecs', v)} />
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
