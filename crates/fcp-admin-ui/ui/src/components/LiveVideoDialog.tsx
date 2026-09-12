import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Eye, EyeOff, Pin, PinOff, ScreenShare, ScreenShareOff, Star, StarOff, Video, VideoOff, Mic, MicOff } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { ApiRequestError } from '@/api/client'
import { conferenceLive, CONTENT_LAYOUT_LABELS, VIDEO_LAYOUT_LABELS, type LiveRoomVideo, type LiveVideoParticipant } from '@/api/endpoints'

/** How often the live view refreshes while open. */
const REFRESH_MS = 3000

function stateVariant(state: string): 'success' | 'warning' | 'secondary' | 'destructive' | 'outline' {
  switch (state) {
    case 'on': return 'success'
    case 'lost': return 'warning'
    case 'disabled': return 'secondary'
    case 'failed': return 'destructive'
    default: return 'outline'
  }
}

function participantLabel(p: LiveVideoParticipant): string {
  return p.display_name || p.caller_id || p.participant_id
}

function errorText(err: unknown): string {
  if (err instanceof ApiRequestError) {
    if (err.status === 404) return 'This room is not active on any conference node right now.'
    if (err.status === 503) return err.message || 'No conference server is configured for live controls.'
    return err.message
  }
  return (err as Error)?.message || 'Request failed'
}

/**
 * Live video controls for one room (video conferencing phase 3c): the
 * layout, the pinned and spotlighted participants, each participant's
 * video state, and the host controls the conference server exposes.
 */
/** The stages of a room's pipeline that run off its device (§15.7): a
 * codec the device cannot encode, an output composed on the host. */
function fallbacks(video: LiveRoomVideo): string[] {
  if (!video.device) return []
  const out = new Set<string>()
  for (const o of video.outputs ?? []) {
    if (o.device && o.device !== video.device) out.add(`${o.view ?? 'composite'} composite`)
    for (const f of o.flavors ?? []) {
      if (f.device && f.device !== video.device) out.add(`${f.codec} encode`)
    }
  }
  return [...out]
}

export function LiveVideoDialog({ room, onOpenChange }: {
  room: string | null
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const queryKey = ['live-video', room]

  const { data, error, isLoading } = useQuery({
    queryKey,
    queryFn: () => conferenceLive.video(room!),
    enabled: !!room,
    refetchInterval: REFRESH_MS,
    retry: false,
  })

  const refresh = () => queryClient.invalidateQueries({ queryKey })
  const layoutMutation = useMutation({
    mutationFn: (layout: string) => conferenceLive.setLayout(room!, layout),
    onSuccess: refresh,
  })
  const pinMutation = useMutation({
    mutationFn: (pid: string | null) => (pid ? conferenceLive.pin(room!, pid) : conferenceLive.unpin(room!)),
    onSuccess: refresh,
  })
  const spotlightMutation = useMutation({
    mutationFn: (pid: string | null) => (pid ? conferenceLive.spotlight(room!, pid) : conferenceLive.unspotlight(room!)),
    onSuccess: refresh,
  })
  const participantVideoMutation = useMutation({
    mutationFn: ({ pid, on }: { pid: string; on: boolean }) => conferenceLive.setParticipantVideo(room!, pid, on),
    onSuccess: refresh,
  })
  const stopContentMutation = useMutation({
    mutationFn: () => conferenceLive.stopContent(room!),
    onSuccess: refresh,
  })
  const contentLayoutMutation = useMutation({
    mutationFn: (layout: string) => conferenceLive.setContentLayout(room!, layout),
    onSuccess: refresh,
  })
  const busy = layoutMutation.isPending || pinMutation.isPending || spotlightMutation.isPending || participantVideoMutation.isPending
    || stopContentMutation.isPending || contentLayoutMutation.isPending
  const actionError = layoutMutation.error || pinMutation.error || spotlightMutation.error || participantVideoMutation.error
    || stopContentMutation.error || contentLayoutMutation.error

  const video = data?.video ?? null
  const participants = data?.participants ?? []
  const nameOf = (pid: string | null) => {
    if (!pid) return null
    const p = participants.find(x => x.participant_id === pid)
    return p ? participantLabel(p) : pid
  }

  return (
    <Dialog open={!!room} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Video className="h-5 w-5" /> Live Video — Room {room}
          </DialogTitle>
          <DialogDescription>
            The room's composite as the conference server builds it, refreshed every {REFRESH_MS / 1000} seconds.
            Layout, pin and spotlight apply to everyone; a participant's video can be turned off for the room.
          </DialogDescription>
        </DialogHeader>

        {isLoading && <div className="py-8 text-center text-muted-foreground">Loading...</div>}

        {error && !data && (
          <div className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
            {errorText(error)}
          </div>
        )}

        {data && (
          <div className="space-y-4">
            {video ? (
              <div className="grid grid-cols-2 gap-4 rounded-md border p-4 md:grid-cols-4">
                <div className="col-span-2">
                  <Label className="text-xs">Layout</Label>
                  <Select
                    value={video.layout}
                    onValueChange={(v) => layoutMutation.mutate(v)}
                    disabled={busy}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {Object.entries(VIDEO_LAYOUT_LABELS).map(([value, label]) => (
                        <SelectItem key={value} value={value}>{label}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label className="text-xs">Canvas</Label>
                  <p className="text-sm">{video.resolution} @ {video.fps} fps{video.fps !== video.target_fps && (
                    <span className="text-yellow-600" title="Frame rate halved: the compositor overran its budget"> (target {video.target_fps})</span>
                  )}</p>
                </div>
                <div>
                  <Label className="text-xs">Pipeline</Label>
                  <p className="text-sm">{video.sources} source{video.sources === 1 ? '' : 's'}, {video.encoders} encoder{video.encoders === 1 ? '' : 's'}</p>
                </div>
                {video.device && (
                  <div>
                    <Label className="text-xs">Device</Label>
                    <p className="text-sm">
                      <span className="font-mono">{video.device}</span>
                      {fallbacks(video).length > 0 && (
                        <span className="text-muted-foreground" title="Stages the device lacks run on the host, one copy per frame">
                          {' '}({fallbacks(video).join(', ')} on host)
                        </span>
                      )}
                    </p>
                  </div>
                )}
                <div className="col-span-2 flex items-center gap-2 text-sm">
                  <Pin className="h-4 w-4 text-muted-foreground" />
                  <span className="text-muted-foreground">Pinned:</span>
                  <span>{nameOf(video.pinned) ?? '—'}</span>
                  {video.pinned && (
                    <Button variant="ghost" size="sm" disabled={busy} onClick={() => pinMutation.mutate(null)} title="Unpin">
                      <PinOff className="h-4 w-4" />
                    </Button>
                  )}
                </div>
                <div className="col-span-2 flex items-center gap-2 text-sm">
                  <Star className="h-4 w-4 text-muted-foreground" />
                  <span className="text-muted-foreground">Spotlight:</span>
                  <span>{nameOf(video.spotlight) ?? '—'}</span>
                  {video.spotlight && (
                    <Button variant="ghost" size="sm" disabled={busy} onClick={() => spotlightMutation.mutate(null)} title="Clear spotlight (the layout stays; pick another to change it)">
                      <StarOff className="h-4 w-4" />
                    </Button>
                  )}
                </div>
                {/* The shared screen (§15.6): who holds the floor, how the
                    composite shows it, and the host's way to end it. */}
                <div className="col-span-2 flex items-center gap-2 text-sm">
                  <ScreenShare className="h-4 w-4 text-muted-foreground" />
                  <span className="text-muted-foreground">Sharing:</span>
                  <span>
                    {video.content
                      ? <>{video.content.display_name || nameOf(video.content.participant_id) || video.content.participant_id}
                          <span className="text-xs text-muted-foreground"> · {video.content.codec} {video.content.resolution}{video.content.live ? '' : ' · waiting for a frame'}</span></>
                      : '—'}
                  </span>
                  {video.content && (
                    <Button variant="ghost" size="sm" disabled={busy} onClick={() => stopContentMutation.mutate()} title="End the share (the presenter cannot resume it by just sending)">
                      <ScreenShareOff className="h-4 w-4" />
                    </Button>
                  )}
                </div>
                <div className="col-span-2">
                  <Label className="text-xs">Shared screen layout</Label>
                  <Select
                    value={video.content_layout}
                    onValueChange={(v) => contentLayoutMutation.mutate(v)}
                    disabled={busy}
                  >
                    <SelectTrigger className="text-sm"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {Object.entries(CONTENT_LAYOUT_LABELS).map(([value, label]) => (
                        <SelectItem key={value} value={value}>{label}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="col-span-2 text-xs text-muted-foreground md:col-span-4">
                  Speaking now: {nameOf(data.active_speaker ?? video.active_speaker) ?? '—'}
                  {/* A room can be hosted on several nodes; they agree on
                      one speaker within a take interval. */}
                  {data.nodes && data.nodes.length > 1
                    ? <> · {data.nodes.length} nodes{data.agreed === false && <> · agreeing…</>}</>
                    : <> · Node {data.node_id}</>}
                  {video.overruns > 0 && <> · {video.overruns} compositor overrun{video.overruns === 1 ? '' : 's'}</>}
                </div>
                {data.trunks && data.trunks.length > 0 && (
                  <div className="col-span-2 text-xs text-muted-foreground md:col-span-4">
                    Trunks: {data.trunks.map((t) => `${t.peer}${t.video ? ` (video ${t.video_codec ?? ''})` : ' (audio only)'}`).join(', ')}
                  </div>
                )}
              </div>
            ) : (
              <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                Nobody in this room has video yet. Layout controls appear once a participant sends video.
              </div>
            )}

            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Participant</TableHead>
                  <TableHead>Video</TableHead>
                  <TableHead>Stream</TableHead>
                  <TableHead className="text-right">Controls</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {participants.length === 0 && (
                  <TableRow><TableCell colSpan={4} className="text-center text-muted-foreground">No participants</TableCell></TableRow>
                )}
                {participants.map((p) => {
                  const v = p.video
                  const hasVideo = v.state !== 'off'
                  const pinned = video?.pinned === p.participant_id
                  const spotlit = video?.spotlight === p.participant_id
                  return (
                    <TableRow key={p.participant_id}>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          {p.muted ? <MicOff className="h-3.5 w-3.5 text-muted-foreground" /> : <Mic className={`h-3.5 w-3.5 ${p.is_speaking ? 'text-green-600' : 'text-muted-foreground'}`} />}
                          <span className="font-medium">{participantLabel(p)}</span>
                          {p.is_host && <Badge variant="secondary" className="text-xs">Host</Badge>}
                          {p.is_dial_out && <Badge variant="outline" className="text-xs">Dial-out</Badge>}
                        </div>
                        {p.display_name && <div className="font-mono text-xs text-muted-foreground">{p.participant_id}</div>}
                        {p.node_id && (data.nodes?.length ?? 0) > 1 && (
                          <div className="text-xs text-muted-foreground">on {p.node_id}</div>
                        )}
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1">
                          <Badge variant={stateVariant(v.state)} className="text-xs">{v.state}</Badge>
                          {v.content && (
                            <Badge variant={v.presenting ? 'success' : stateVariant(v.content.state)} className="text-xs" title={v.presenting ? 'Their screen is what the room shows' : `Shared screen section: ${v.content.state}`}>
                              <ScreenShare className="mr-1 h-3 w-3" />{v.presenting ? 'presenting' : v.content.state}
                            </Badge>
                          )}
                          {pinned && <Badge variant="outline" className="text-xs"><Pin className="mr-1 h-3 w-3" />pinned</Badge>}
                          {spotlit && <Badge variant="outline" className="text-xs"><Star className="mr-1 h-3 w-3" />spotlight</Badge>}
                        </div>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {hasVideo ? (
                          <>
                            {v.codec}{v.resolution ? ` ${v.resolution}` : ''}{typeof v.fps === 'number' ? ` @ ${v.fps} fps` : ''}
                            {typeof v.bitrate_kbps === 'number' && <> · {v.bitrate_kbps} kb/s</>}
                            {typeof v.frames_lost === 'number' && v.frames_lost > 0 && <> · {v.frames_lost} lost</>}
                            {/* The rung they are being sent, when the ladder
                                has moved them off the room's own size. */}
                            {v.rung && video?.resolution && v.rung !== video.resolution && (
                              <> · <span title={`Their link is carrying ${v.remb_kbps ?? '?'} kb/s`}>sending {v.rung}</span></>
                            )}
                            {v.forwarding && <> · direct from {nameOf(v.forwarding.replace(/#content$/, '')) ?? v.forwarding}{v.forwarding.endsWith('#content') ? '\u2019s screen' : ''}</>}
                          </>
                        ) : 'audio only'}
                        {v.content && v.content.state !== 'off' && (
                          <div>
                            screen: {v.content.codec}{v.content.resolution ? ` ${v.content.resolution}` : ''}{typeof v.content.fps === 'number' ? ` @ ${v.content.fps} fps` : ''}
                            {v.content.channel && <> · receives the room&apos;s screen</>}
                          </div>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-1">
                          <Button variant="ghost" size="icon" disabled={busy || !video} title={pinned ? 'Unpin' : 'Pin to the first tile'}
                            onClick={() => pinMutation.mutate(pinned ? null : p.participant_id)}>
                            {pinned ? <PinOff className="h-4 w-4" /> : <Pin className="h-4 w-4" />}
                          </Button>
                          <Button variant="ghost" size="icon" disabled={busy || !video} title={spotlit ? 'Clear spotlight' : 'Spotlight (full canvas)'}
                            onClick={() => spotlightMutation.mutate(spotlit ? null : p.participant_id)}>
                            {spotlit ? <StarOff className="h-4 w-4" /> : <Star className="h-4 w-4" />}
                          </Button>
                          <Button variant="ghost" size="icon" disabled={busy || !video || !hasVideo}
                            title={v.state === 'disabled' ? 'Show this participant\'s video again' : 'Hide this participant\'s video from the room'}
                            onClick={() => participantVideoMutation.mutate({ pid: p.participant_id, on: v.state === 'disabled' })}>
                            {v.state === 'disabled' ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>

            {actionError && (
              <p className="text-sm text-destructive">{errorText(actionError)}</p>
            )}
            {!video && participants.some(p => p.video.state !== 'off') && (
              <p className="text-xs text-muted-foreground"><VideoOff className="mr-1 inline h-3 w-3" />Video is starting; controls follow on the next refresh.</p>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
