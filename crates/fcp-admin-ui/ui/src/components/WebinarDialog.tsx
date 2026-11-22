import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, Download, Radio, Square, UserPlus, Ban, Link as LinkIcon, Tv } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { recordings, webinars, type AttendanceResponse, type JoinTokenResponse, type RtmpPush, type StreamLinkResponse } from '@/api/endpoints'
import type { MeetingResponse } from '@/types'

type Tab = 'live' | 'stream' | 'registrants' | 'attendance'

const TAB_NAMES: Record<Tab, string> = { live: 'Live', stream: 'Stream', registrants: 'Registrants', attendance: 'Attendance' }

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
 * its stream out and its push (§2.8), its registrations, and who came to
 * each sitting.
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
          {(['live', 'stream', 'registrants', 'attendance'] as Tab[]).map((t) => (
            <Button key={t} size="sm" variant={tab === t ? 'default' : 'ghost'} onClick={() => setTab(t)}>
              {TAB_NAMES[t]}
            </Button>
          ))}
        </div>
        {meeting && tab === 'live' && <LiveTab meeting={meeting} />}
        {meeting && tab === 'stream' && <StreamTab meeting={meeting} />}
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

const STREAM_STATES: Record<string, { label: string; variant: 'success' | 'secondary' | 'outline' }> = {
  off: { label: 'Not streamed', variant: 'outline' },
  waiting: { label: 'Waiting to go live', variant: 'secondary' },
  live: { label: 'Live', variant: 'success' },
  ended: { label: 'Ended', variant: 'secondary' },
}

const LINK_LIFETIMES: { label: string; secs: number }[] = [
  { label: 'An hour', secs: 3600 },
  { label: 'A day', secs: 86_400 },
  { label: 'A week', secs: 7 * 86_400 },
]

/**
 * The webinar out to an audience not on FCP (`docs/WEBINARS.md` §2.8): its
 * HLS stream (W6.3d) and its push to a live service over RTMP (W7.3).
 */
function StreamTab({ meeting }: { meeting: MeetingResponse }) {
  return (
    <div className="space-y-6">
      <HlsSection meeting={meeting} />
      <div className="border-t pt-4">
        <RtmpSection meeting={meeting} />
      </div>
    </div>
  )
}

const PUSH_STATES: Record<RtmpPush['state'], { label: string; variant: 'success' | 'secondary' | 'outline' | 'destructive' }> = {
  connecting: { label: 'Connecting', variant: 'secondary' },
  live: { label: 'Pushing', variant: 'success' },
  retrying: { label: 'Retrying', variant: 'destructive' },
  stopped: { label: 'Stopped', variant: 'outline' },
}

const megabytes = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`

/**
 * Where the webinar is pushed over RTMP — a service's ingest URL and the
 * stream key it gave, the key never shown again — and the push while live.
 */
function RtmpSection({ meeting }: { meeting: MeetingResponse }) {
  const queryClient = useQueryClient()
  const [url, setUrl] = useState('')
  const [key, setKey] = useState('')
  const [error, setError] = useState<string | null>(null)
  const destination = useQuery({ queryKey: ['webinar-rtmp', meeting.id], queryFn: () => webinars.rtmp(meeting.id) })
  useEffect(() => { setUrl(destination.data?.url ?? '') }, [destination.data?.url])
  const sitting = useQuery({
    queryKey: ['webinar-sitting', meeting.room_number],
    queryFn: () => webinars.sitting(meeting.room_number),
    refetchInterval: (q) => (q.state.status === 'success' ? 3000 : false),
    retry: false,
  })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['webinar-rtmp', meeting.id] })
  const save = useMutation({
    mutationFn: () => webinars.setRtmp(meeting.id, { url: url.trim(), key: key.trim() || undefined }),
    onSuccess: () => { setKey(''); refresh() },
    onError: (e) => setError((e as Error).message),
  })
  const remove = useMutation({
    mutationFn: () => webinars.deleteRtmp(meeting.id),
    onSuccess: () => { setUrl(''); setKey(''); refresh() },
    onError: (e) => setError((e as Error).message),
  })
  const d = destination.data
  const push = sitting.data?.rtmp
  const shown = push ? PUSH_STATES[push.state] : null
  return (
    <div className="space-y-3">
      <div>
        <Label>Push to a live service (RTMP)</Label>
        <p className="text-xs text-muted-foreground">
          While the webinar is live its stage pushes it to the service's ingest — YouTube Live, Twitch, any RTMP server — under the stream key the service gave. The key is kept sealed and never shown again.
        </p>
      </div>
      <div className="grid grid-cols-[1fr_1fr] gap-2">
        <div>
          <Label>Ingest URL</Label>
          <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="rtmp://a.rtmp.youtube.com/live2" aria-label="Ingest URL" />
        </div>
        <div>
          <Label>Stream key</Label>
          <Input
            type="password"
            autoComplete="off"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder={d?.key_set ? 'Set — leave empty to keep it' : 'From the service'}
            aria-label="Stream key"
          />
        </div>
      </div>
      <div className="flex gap-2 items-center">
        <Button
          disabled={!url.trim() || (!key.trim() && !d?.key_set) || save.isPending}
          onClick={() => { setError(null); save.mutate() }}
        >
          Save destination
        </Button>
        {d?.url && (
          <Button
            variant="ghost"
            className="text-destructive"
            disabled={remove.isPending}
            onClick={() => { if (confirm('Remove the RTMP destination? The webinar will not be pushed.')) { setError(null); remove.mutate() } }}
          >
            Remove
          </Button>
        )}
        {d?.key_set && <span className="text-xs text-muted-foreground">Stream key set{d.updated_at ? ` · saved ${when(d.updated_at)}` : ''}</span>}
      </div>
      {push && shown && (
        <div className="rounded border p-3 space-y-1 text-sm">
          <div className="flex items-center gap-3">
            <Badge variant={shown.variant}>{shown.label}</Badge>
            <span className="text-muted-foreground">to {push.ingest} since {when(push.since)}</span>
          </div>
          <div className="text-muted-foreground">
            {megabytes(push.bytes_sent)} sent · {push.connects} connection{push.connects === 1 ? '' : 's'}
            {push.dropped_frames > 0 ? ` · ${push.dropped_frames} frames dropped while behind` : ''}
          </div>
          {push.error && push.state !== 'live' && <div className="text-destructive">{push.error}</div>}
        </div>
      )}
      {!push && d?.url && (
        <p className="text-xs text-muted-foreground">Nothing is being pushed now: the push starts when the webinar goes live.</p>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

/**
 * The webinar's HLS stream (`docs/WEBINARS.md` §2.8, W6.3d): its state,
 * a link minted for watching it, and the stream kept as a recording for
 * replay.
 */
function HlsSection({ meeting }: { meeting: MeetingResponse }) {
  const [ttl, setTtl] = useState(String(86_400))
  const [link, setLink] = useState<StreamLinkResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [copied, copy] = useCopy()
  const state = useQuery({
    queryKey: ['webinar-stream', meeting.id],
    queryFn: () => webinars.stream(meeting.id),
    refetchInterval: 5000,
  })
  const mint = useMutation({
    mutationFn: () => webinars.streamLink(meeting.id, Number(ttl)),
    onSuccess: setLink,
    onError: (e) => setError((e as Error).message),
  })
  const download = async (recordingId: string, sitting: string) => {
    setError(null)
    try {
      const url = await recordings.fetchMedia(recordingId)
      const a = document.createElement('a')
      a.href = url
      a.download = `${sitting}.mp4`
      a.click()
      setTimeout(() => URL.revokeObjectURL(url), 30_000)
    } catch {
      setError('Download failed; the node holding the stream may be unreachable.')
    }
  }
  if (!state.data) return <p className="text-sm text-muted-foreground">Loading…</p>
  const s = state.data
  const shown = STREAM_STATES[s.state] ?? { label: s.state, variant: 'outline' as const }
  if (s.state === 'off') {
    return (
      <div className="space-y-2">
        <Badge variant={shown.variant}>{shown.label}</Badge>
        <p className="text-sm text-muted-foreground">
          This webinar is not streamed out as HLS. Turn on "Stream out as HLS" in its form to stream it from going live to the end.
        </p>
      </div>
    )
  }
  const playlist = link ? link.playlist_url ?? `${window.location.origin}${link.playlist_path}` : ''
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Badge variant={shown.variant}>{shown.label}</Badge>
        <span className="text-sm text-muted-foreground">
          {s.state === 'waiting'
            ? 'The stream starts when the webinar goes live.'
            : `Started ${when(s.started_at)}${s.ended_at ? ` · ended ${when(s.ended_at)}` : ''}${s.duration_secs != null ? ` · ${minutes(s.duration_secs)}` : ''}`}
        </span>
      </div>
      <div className="space-y-2">
        <Label>Viewer link</Label>
        <p className="text-xs text-muted-foreground">
          Anyone holding the link can watch the stream in an HLS player, live and after it ends, until the link expires. It is the only credential: share it as you would a password.
        </p>
        <div className="flex gap-2 items-center">
          <Select value={ttl} onValueChange={setTtl}>
            <SelectTrigger className="w-40" aria-label="Link lifetime"><SelectValue /></SelectTrigger>
            <SelectContent>
              {LINK_LIFETIMES.map((l) => (
                <SelectItem key={l.secs} value={String(l.secs)}>{l.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button disabled={mint.isPending} onClick={() => { setError(null); mint.mutate() }}>
            <Tv className="mr-2 h-4 w-4" /> Make a link
          </Button>
        </div>
        {link && (
          <div className="rounded border p-3 space-y-2">
            <p className="text-sm">Playlist, good until {new Date(link.expires_at).toLocaleString()}:</p>
            <div className="flex gap-2">
              <Input readOnly value={playlist} className="font-mono text-xs" aria-label="Stream link" />
              <Button size="icon" variant="outline" aria-label="Copy" onClick={() => copy('stream', playlist)}><Copy className="h-4 w-4" /></Button>
            </div>
            {copied === 'stream' && <p className="text-xs text-green-600">Copied</p>}
          </div>
        )}
      </div>
      {s.recording_id && s.sitting_id && (
        <div className="space-y-2">
          <Label>Replay</Label>
          <p className="text-xs text-muted-foreground">
            The stream is kept as a recording, under the tenant's retention, and downloads as one MP4{s.state === 'live' ? ' as far as it has got' : ''}.
          </p>
          <Button variant="outline" onClick={() => download(s.recording_id!, s.sitting_id!)}>
            <Download className="mr-2 h-4 w-4" /> Download the stream
          </Button>
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
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
