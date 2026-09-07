import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Eye, EyeOff, Pin, PinOff, Star, StarOff, Video, VideoOff, Mic, MicOff } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { ApiRequestError } from '@/api/client'
import { conferenceLive, VIDEO_LAYOUT_LABELS, type LiveVideoParticipant } from '@/api/endpoints'

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
  const busy = layoutMutation.isPending || pinMutation.isPending || spotlightMutation.isPending || participantVideoMutation.isPending
  const actionError = layoutMutation.error || pinMutation.error || spotlightMutation.error || participantVideoMutation.error

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
                <div className="col-span-2 text-xs text-muted-foreground md:col-span-4">
                  Speaking now: {nameOf(video.active_speaker) ?? '—'} · Node {data.node_id}
                  {video.overruns > 0 && <> · {video.overruns} compositor overrun{video.overruns === 1 ? '' : 's'}</>}
                </div>
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
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1">
                          <Badge variant={stateVariant(v.state)} className="text-xs">{v.state}</Badge>
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
                          </>
                        ) : 'audio only'}
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
