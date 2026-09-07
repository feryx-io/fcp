import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useForm } from 'react-hook-form'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Pencil, Phone, PhoneOutgoing, Users } from 'lucide-react'
import { me } from '@/api/endpoints'
import type { MeetingResponse, UpdateMeetingRequest } from '@/types'

export function UserConferencesPage() {
  const [editItem, setEditItem] = useState<MeetingResponse | null>(null)
  const [dialOutConf, setDialOutConf] = useState<MeetingResponse | null>(null)
  const [dialOutDest, setDialOutDest] = useState('')
  const [dialOutGreeting, setDialOutGreeting] = useState('')
  const [dialOutPending, setDialOutPending] = useState(false)
  const [dialOutResult, setDialOutResult] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: conferences = [], isLoading } = useQuery({
    queryKey: ['my-conferences'],
    queryFn: me.listConferences,
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateMeetingRequest }) =>
      me.updateConference(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['my-conferences'] })
      setEditItem(null)
    },
  })

  if (isLoading) {
    return <div className="flex items-center justify-center p-8 text-muted-foreground">Loading...</div>
  }

  if (conferences.length === 0) {
    return (
      <div className="space-y-4">
        <h1 className="text-2xl font-bold">My Conference Bridges</h1>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12 text-center">
            <Users className="h-12 w-12 text-muted-foreground mb-4" />
            <p className="text-lg font-medium">No conference bridges assigned</p>
            <p className="text-sm text-muted-foreground mt-1">Contact your administrator to get a conference bridge assigned to your account.</p>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <h1 className="text-2xl font-bold">My Conference Bridges</h1>

      <div className="grid gap-4 md:grid-cols-2">
        {conferences.map(conf => (
          <Card key={conf.id}>
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Phone className="h-5 w-5 text-primary" />
                  <CardTitle className="text-lg">{conf.name}</CardTitle>
                </div>
                <div className="flex gap-1">
                  <Button variant="ghost" size="icon" onClick={() => { setDialOutConf(conf); setDialOutDest(''); setDialOutResult(null) }} title="Dial Out">
                    <PhoneOutgoing className="h-4 w-4" />
                  </Button>
                  <Button variant="ghost" size="icon" onClick={() => setEditItem(conf)} title="Edit">
                    <Pencil className="h-4 w-4" />
                  </Button>
                </div>
              </div>
              <CardDescription>Room: <span className="font-mono font-medium">{conf.room_number}</span></CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex flex-wrap gap-1.5 mb-3">
                <Badge variant={conf.enabled ? 'success' : 'outline'}>
                  {conf.enabled ? 'Enabled' : 'Disabled'}
                </Badge>
                {conf.require_guest_pin && <Badge variant="secondary">PIN Required</Badge>}
                {conf.wait_for_moderator && <Badge variant="secondary">Wait for Host</Badge>}
                {conf.auto_record && <Badge variant="secondary">Auto Record</Badge>}
                {conf.default_locked && <Badge variant="secondary">Locked</Badge>}
              </div>
              <div className="grid grid-cols-2 gap-2 text-sm text-muted-foreground">
                <div>Guest PIN: <span className="font-mono">{conf.guest_pin ? '****' : 'None'}</span></div>
                <div>Host PIN: <span className="font-mono">{conf.host_pin ? '****' : 'None'}</span></div>
                <div>Max: {conf.max_participants === 0 ? 'Unlimited' : conf.max_participants}</div>
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      {/* Edit Dialog */}
      <EditConferenceDialog
        item={editItem}
        onOpenChange={(open) => { if (!open) setEditItem(null) }}
        onSubmit={(data) => { if (editItem) updateMutation.mutate({ id: editItem.id, data }) }}
        isPending={updateMutation.isPending}
      />

      {/* Dial-Out Dialog */}
      <Dialog open={!!dialOutConf} onOpenChange={(open) => { if (!open) setDialOutConf(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Invite Participant — Room {dialOutConf?.room_number}</DialogTitle>
            <DialogDescription>
              Call someone and connect them to your conference.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>Phone Number or SIP Address</Label>
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
                placeholder="e.g. You are being invited to my conference"
                disabled={dialOutPending}
              />
              <p className="text-xs text-muted-foreground mt-1">TTS text played to the callee before connecting. Leave blank for default.</p>
            </div>
            {dialOutResult && (
              <p className={`text-sm ${dialOutResult.startsWith('Error') ? 'text-destructive' : 'text-green-600'}`}>
                {dialOutResult}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setDialOutConf(null)} disabled={dialOutPending}>Cancel</Button>
              <Button
                disabled={!dialOutDest.trim() || dialOutPending}
                onClick={async () => {
                  setDialOutPending(true)
                  setDialOutResult(null)
                  try {
                    const opts: { greeting_tts?: string } = {}
                    if (dialOutGreeting.trim()) opts.greeting_tts = dialOutGreeting.trim()
                    const res = await me.dialOutConference(dialOutConf!.id, dialOutDest.trim(), opts)
                    setDialOutResult(`Connected: ${res.participant_id}`)
                    setTimeout(() => setDialOutConf(null), 2000)
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

function EditConferenceDialog({ item, onOpenChange, onSubmit, isPending }: {
  item: MeetingResponse | null
  onOpenChange: (open: boolean) => void
  onSubmit: (data: UpdateMeetingRequest) => void
  isPending: boolean
}) {
  const { register, handleSubmit, reset, setValue, watch } = useForm<{
    name: string
    guest_pin: string
    host_pin: string
    require_guest_pin: boolean
    max_participants: number
    wait_for_moderator: boolean
    auto_record: boolean
    default_locked: boolean
    play_entry_tone: boolean
    play_exit_tone: boolean
    welcome_tts: string
  }>()

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
      play_entry_tone: item.play_entry_tone,
      play_exit_tone: item.play_exit_tone,
      welcome_tts: item.welcome_tts || '',
    })
  }

  const onFormSubmit = handleSubmit((data) => {
    const update: UpdateMeetingRequest = {}
    if (data.name) update.name = data.name
    if (data.guest_pin !== undefined) update.guest_pin = data.guest_pin || null
    if (data.host_pin !== undefined) update.host_pin = data.host_pin || null
    if (data.require_guest_pin !== undefined) update.require_guest_pin = data.require_guest_pin
    if (data.max_participants !== undefined) update.max_participants = data.max_participants
    if (data.wait_for_moderator !== undefined) update.wait_for_moderator = data.wait_for_moderator
    if (data.auto_record !== undefined) update.auto_record = data.auto_record
    if (data.default_locked !== undefined) update.default_locked = data.default_locked
    if (data.play_entry_tone !== undefined) update.play_entry_tone = data.play_entry_tone
    if (data.play_exit_tone !== undefined) update.play_exit_tone = data.play_exit_tone
    if (data.welcome_tts !== undefined) update.welcome_tts = data.welcome_tts || null
    onSubmit(update)
  })

  return (
    <Dialog open={!!item} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Edit Conference Bridge</DialogTitle>
          <DialogDescription>
            Room {item?.room_number} — {item?.name}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={onFormSubmit} className="space-y-4">
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

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label>Require Guest PIN</Label>
              <Switch checked={watch('require_guest_pin')} onCheckedChange={(v) => setValue('require_guest_pin', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Wait for Host</Label>
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
              <Label>Entry Tone</Label>
              <Switch checked={watch('play_entry_tone') !== false} onCheckedChange={(v) => setValue('play_entry_tone', v)} />
            </div>
            <div className="flex items-center justify-between">
              <Label>Exit Tone</Label>
              <Switch checked={watch('play_exit_tone') !== false} onCheckedChange={(v) => setValue('play_exit_tone', v)} />
            </div>
          </div>

          <div>
            <Label className="text-xs">Welcome TTS Text</Label>
            <Input {...register('welcome_tts')} placeholder="e.g. Welcome to the meeting." className="text-sm" />
          </div>

          <Button type="submit" className="w-full" disabled={isPending}>
            {isPending ? 'Saving...' : 'Save Changes'}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  )
}
