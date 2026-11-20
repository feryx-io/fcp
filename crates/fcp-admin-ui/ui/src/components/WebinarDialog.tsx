import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, Download, Radio, Square, UserPlus, Ban, Link as LinkIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { webinars, type AttendanceResponse, type JoinTokenResponse } from '@/api/endpoints'
import type { MeetingResponse } from '@/types'

type Tab = 'live' | 'registrants' | 'attendance'

const when = (at: string | null | undefined) => (at ? new Date(at).toLocaleString() : '—')
const minutes = (secs: number) => `${Math.round(secs / 60)} min`

/** Copy `text`, saying so for a moment. */
function useCopy(): [string | null, (key: string, text: string) => void] {
  const [copied, setCopied] = useState<string | null>(null)
  return [copied, (key, text) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(key)
      setTimeout(() => setCopied(null), 1500)
    }).catch(() => setCopied(null))
  }]
}

/** Save `body` as a file named `name`. */
function save(name: string, type: string, body: string) {
  const url = URL.createObjectURL(new Blob([body], { type }))
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  URL.revokeObjectURL(url)
}

const csvCell = (v: unknown) => {
  const s = v === null || v === undefined ? '' : String(v)
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

/** A sitting's legs as CSV, one row a leg. */
function legsCsv(report: AttendanceResponse): string {
  const head = ['name', 'from', 'role', 'registrant', 'joined_at', 'left_at', 'duration_secs', 'lobby', 'questions', 'messages']
  const rows = report.legs.map((l) => [l.name, l.from, l.role, l.registrant, l.joined_at, l.left_at, l.duration_secs, l.lobby, l.questions, l.messages])
  return [head, ...rows].map((r) => r.map(csvCell).join(',')).join('\n') + '\n'
}

/**
 * A webinar's life (`docs/WEBINARS.md` §2.6): its sitting now — practice,
 * live or ended, the lobby, going live and ending, the audience by page —
 * its registrations, and who came to each sitting.
 */
export function WebinarDialog({ meeting, onOpenChange }: {
  meeting: MeetingResponse | null
  onOpenChange: (open: boolean) => void
}) {
  const [tab, setTab] = useState<Tab>('live')
  useEffect(() => { if (meeting) setTab('live') }, [meeting])
  return (
    <Dialog open={!!meeting} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Webinar — {meeting?.name}</DialogTitle>
          <DialogDescription>Room {meeting?.room_number}</DialogDescription>
        </DialogHeader>
        <div className="flex gap-2 border-b pb-2">
          {(['live', 'registrants', 'attendance'] as Tab[]).map((t) => (
            <Button key={t} size="sm" variant={tab === t ? 'default' : 'ghost'} onClick={() => setTab(t)}>
              {t === 'live' ? 'Live' : t === 'registrants' ? 'Registrants' : 'Attendance'}
            </Button>
          ))}
        </div>
        {meeting && tab === 'live' && <LiveTab meeting={meeting} />}
        {meeting && tab === 'registrants' && <RegistrantsTab meeting={meeting} />}
        {meeting && tab === 'attendance' && <AttendanceTab meeting={meeting} />}
      </DialogContent>
    </Dialog>
  )
}

function LiveTab({ meeting }: { meeting: MeetingResponse }) {
  const queryClient = useQueryClient()
  const room = meeting.room_number
  const [cursors, setCursors] = useState<(string | null)[]>([null])
  const cursor = cursors[cursors.length - 1]
  const [error, setError] = useState<string | null>(null)
  const sitting = useQuery({
    queryKey: ['webinar-sitting', room],
    queryFn: () => webinars.sitting(room),
    // Asked again only while there is a sitting to follow.
    refetchInterval: (q) => (q.state.status === 'success' ? 3000 : false),
    retry: false,
  })
  const audience = useQuery({
    queryKey: ['webinar-audience', room, cursor],
    queryFn: () => webinars.audience(room, cursor),
    refetchInterval: (q) => (q.state.status === 'success' ? 5000 : false),
    retry: false,
    enabled: sitting.isSuccess,
  })
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ['webinar-sitting', room] })
    queryClient.invalidateQueries({ queryKey: ['webinar-audience', room] })
  }
  const live = useMutation({ mutationFn: () => webinars.goLive(room), onSuccess: refresh, onError: (e) => setError((e as Error).message) })
  const end = useMutation({ mutationFn: () => webinars.end(room), onSuccess: refresh, onError: (e) => setError((e as Error).message) })

  if (sitting.isError) {
    return <p className="text-sm text-muted-foreground">Nobody is in the room: the webinar begins in practice when its first caller joins.</p>
  }
  if (!sitting.data) return <p className="text-sm text-muted-foreground">Loading…</p>
  const s = sitting.data
  const phase = s.phase
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Badge variant={phase === 'live' ? 'success' : 'secondary'}>
          {phase === 'practice' ? 'Practice' : phase === 'live' ? 'Live' : 'Ended'}
        </Badge>
        <span className="text-sm text-muted-foreground">
          Started {when(s.started_at)}{s.live_at ? ` · live since ${when(s.live_at)}` : ''} · {s.lobby} waiting in the lobby
        </span>
      </div>
      <div className="flex gap-2">
        <Button disabled={phase !== 'practice' || live.isPending} onClick={() => { setError(null); live.mutate() }}>
          <Radio className="mr-2 h-4 w-4" /> Go live
        </Button>
        <Button
          variant="destructive"
          disabled={phase === 'ended' || end.isPending}
          onClick={() => { if (confirm('End the webinar? Everyone in it is released.')) { setError(null); end.mutate() } }}
        >
          <Square className="mr-2 h-4 w-4" /> End
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        In practice the stage hears itself and attendees wait in the lobby; going live lets them in, a few a second, and starts recording if the meeting records.
      </p>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <div>
        <Label>Audience ({audience.data?.total ?? 0})</Label>
        <table className="w-full text-sm mt-2">
          <thead className="text-left text-muted-foreground">
            <tr><th>Name</th><th>From</th><th>Joined</th><th>Where</th><th>Hand</th></tr>
          </thead>
          <tbody>
            {(audience.data?.data ?? []).map((a) => (
              <tr key={a.participant_id} className="border-t">
                <td>{a.display_name ?? '—'}</td>
                <td className="font-mono text-xs">{a.caller_id ?? '—'}</td>
                <td>{when(a.joined_at)}</td>
                <td>{a.lobby ? 'Lobby' : a.on_stage ? 'Stage' : 'Audience'}</td>
                <td>{a.hand ? 'Raised' : ''}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="flex justify-end gap-2 mt-2">
          <Button size="sm" variant="outline" disabled={cursors.length < 2} onClick={() => setCursors(cursors.slice(0, -1))}>Previous</Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!audience.data?.next_cursor}
            onClick={() => audience.data?.next_cursor && setCursors([...cursors, audience.data.next_cursor])}
          >
            Next
          </Button>
        </div>
      </div>
    </div>
  )
}

function RegistrantsTab({ meeting }: { meeting: MeetingResponse }) {
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [minted, setMinted] = useState<{ who: string; token: JoinTokenResponse } | null>(null)
  const [copied, copy] = useCopy()
  const list = useQuery({ queryKey: ['registrants', meeting.id], queryFn: () => webinars.registrants(meeting.id) })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['registrants', meeting.id] })
  const register = useMutation({
    mutationFn: () => webinars.register(meeting.id, { name: name.trim(), email: email.trim() || undefined }),
    onSuccess: (r) => { setMinted({ who: r.registrant.name, token: r.token }); setName(''); setEmail(''); refresh() },
    onError: (e) => setError((e as Error).message),
  })
  const revoke = useMutation({ mutationFn: (rid: string) => webinars.revoke(meeting.id, rid), onSuccess: refresh })
  const remint = useMutation({
    mutationFn: ({ rid }: { rid: string; who: string }) => webinars.token(meeting.id, rid),
    onSuccess: (token, { who }) => setMinted({ who, token }),
    onError: (e) => setError((e as Error).message),
  })
  const shareable = minted ? minted.token.join_url ?? minted.token.token : ''
  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        {meeting.registration
          ? 'This webinar requires registration: an attendee joins only by the link minted for their registration.'
          : 'Registration is optional for this webinar: attendees may also join by the guest PIN. Turn on "Require registration" to admit registrants alone.'}
      </p>
      <div className="grid grid-cols-[1fr_1fr_auto] gap-2 items-end">
        <div><Label>Name</Label><Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Ann Lee" /></div>
        <div><Label>Email</Label><Input value={email} onChange={(e) => setEmail(e.target.value)} placeholder="Optional" /></div>
        <Button disabled={!name.trim() || register.isPending} onClick={() => { setError(null); register.mutate() }}>
          <UserPlus className="mr-2 h-4 w-4" /> Register
        </Button>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
      {minted && (
        <div className="rounded border p-3 space-y-2">
          <p className="text-sm">{minted.token.join_url ? 'Join link' : 'Join token'} for {minted.who}, good until {new Date(minted.token.expires_at).toLocaleString()}:</p>
          <div className="flex gap-2">
            <Input readOnly value={shareable} className="font-mono text-xs" />
            <Button size="icon" variant="outline" aria-label="Copy" onClick={() => copy('minted', shareable)}><Copy className="h-4 w-4" /></Button>
          </div>
          {copied === 'minted' && <p className="text-xs text-green-600">Copied</p>}
        </div>
      )}
      <table className="w-full text-sm">
        <thead className="text-left text-muted-foreground">
          <tr><th>Name</th><th>Email</th><th>Registered</th><th>Status</th><th /></tr>
        </thead>
        <tbody>
          {(list.data ?? []).map((r) => (
            <tr key={r.id} className="border-t">
              <td>{r.name}</td>
              <td>{r.email ?? '—'}</td>
              <td>{when(r.created_at)}</td>
              <td>{r.revoked_at ? <Badge variant="outline">Revoked</Badge> : <Badge variant="secondary">Registered</Badge>}</td>
              <td className="text-right space-x-1">
                {!r.revoked_at && (
                  <>
                    <Button size="sm" variant="ghost" onClick={() => remint.mutate({ rid: r.id, who: r.name })}>
                      <LinkIcon className="mr-1 h-3 w-3" /> New link
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-destructive"
                      onClick={() => { if (confirm(`Revoke ${r.name}'s registration? Their link stops working.`)) revoke.mutate(r.id) }}
                    >
                      <Ban className="mr-1 h-3 w-3" /> Revoke
                    </Button>
                  </>
                )}
              </td>
            </tr>
          ))}
          {list.data?.length === 0 && (
            <tr><td colSpan={5} className="text-muted-foreground py-2">Nobody is registered.</td></tr>
          )}
        </tbody>
      </table>
    </div>
  )
}

function AttendanceTab({ meeting }: { meeting: MeetingResponse }) {
  const sittings = useQuery({ queryKey: ['sittings', meeting.id], queryFn: () => webinars.sittings(meeting.id) })
  const [chosen, setChosen] = useState<string | null>(null)
  const sitting = chosen ?? sittings.data?.[0]?.id ?? null
  const report = useQuery({
    queryKey: ['attendance', meeting.id, sitting],
    queryFn: () => webinars.attendance(meeting.id, sitting ?? undefined),
    enabled: !!sitting,
  })
  if (sittings.data?.length === 0) {
    return <p className="text-sm text-muted-foreground">The room has not been used yet.</p>
  }
  const r = report.data
  const came = r?.registrants.filter((x) => x.attended).length ?? 0
  return (
    <div className="space-y-4">
      <div className="flex items-end gap-2">
        <div className="flex-1">
          <Label>Sitting</Label>
          <Select value={sitting ?? ''} onValueChange={setChosen}>
            <SelectTrigger><SelectValue placeholder="Choose a sitting" /></SelectTrigger>
            <SelectContent>
              {(sittings.data ?? []).map((s) => (
                <SelectItem key={s.id} value={s.id}>
                  {when(s.started_at)}{s.live_at ? '' : ' (never live)'}{s.ended_at ? '' : ' (running)'}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {r && (
          <>
            <Button variant="outline" onClick={() => save(`attendance-${meeting.room_number}-${r.sitting.id}.csv`, 'text/csv', legsCsv(r))}>
              <Download className="mr-2 h-4 w-4" /> CSV
            </Button>
            <Button variant="outline" onClick={() => save(`attendance-${meeting.room_number}-${r.sitting.id}.json`, 'application/json', JSON.stringify(r, null, 2))}>
              <Download className="mr-2 h-4 w-4" /> JSON
            </Button>
          </>
        )}
      </div>
      {r && (
        <>
          <div className="grid grid-cols-4 gap-2 text-sm">
            <div><div className="text-muted-foreground">Attendees</div><div className="text-lg font-semibold">{r.attendees}</div></div>
            <div><div className="text-muted-foreground">Peak</div><div className="text-lg font-semibold">{r.peak}</div></div>
            <div><div className="text-muted-foreground">Registered / came</div><div className="text-lg font-semibold">{r.registrants.length} / {came}</div></div>
            <div><div className="text-muted-foreground">Questions / messages</div><div className="text-lg font-semibold">{r.questions.length} / {r.messages.length}</div></div>
          </div>
          <p className="text-xs text-muted-foreground">
            Live {when(r.sitting.live_at)} · ended {when(r.sitting.ended_at)}{r.sitting.ended_by ? ` by ${r.sitting.ended_by}` : ''}
          </p>
          <div>
            <Label>Legs</Label>
            <table className="w-full text-sm mt-1">
              <thead className="text-left text-muted-foreground">
                <tr><th>Name</th><th>Role</th><th>Joined</th><th>Time</th><th>Q</th><th>Chat</th></tr>
              </thead>
              <tbody>
                {r.legs.map((l, i) => (
                  <tr key={i} className="border-t">
                    <td>{l.name ?? l.from}{l.lobby && <Badge variant="outline" className="ml-2">Lobby only</Badge>}</td>
                    <td>{l.role ?? '—'}</td>
                    <td>{when(l.joined_at)}</td>
                    <td>{minutes(l.duration_secs)}</td>
                    <td>{l.questions}</td>
                    <td>{l.messages}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {r.registrants.length > 0 && (
            <div>
              <Label>Registrants</Label>
              <table className="w-full text-sm mt-1">
                <tbody>
                  {r.registrants.map((x) => (
                    <tr key={x.id} className="border-t">
                      <td>{x.name}</td>
                      <td>{x.email ?? ''}</td>
                      <td>{x.attended ? `Came, ${minutes(x.attended_secs)}` : 'Did not come'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {r.questions.length > 0 && (
            <div>
              <Label>Questions</Label>
              <ul className="text-sm mt-1 space-y-1">
                {r.questions.map((q) => (
                  <li key={q.id} className="border-t pt-1">
                    <span className="font-medium">{q.asker ?? 'Anonymous'}:</span> {q.text}
                    <span className="text-muted-foreground"> · {q.status}{q.answered_live ? ', answered live' : ''} · {q.upvotes} up</span>
                    {q.answer && <div className="text-muted-foreground">↳ {q.answer}</div>}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {r.messages.length > 0 && (
            <div>
              <Label>Chat</Label>
              <ul className="text-sm mt-1 space-y-1">
                {r.messages.map((m) => (
                  <li key={m.id} className="border-t pt-1">
                    <span className="font-medium">{m.sender ?? '—'}</span>
                    {m.audience === 'hosts' && <Badge variant="outline" className="ml-1">to hosts</Badge>}: {m.deleted_at ? <em className="text-muted-foreground">deleted</em> : m.text}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </>
      )}
    </div>
  )
}
