import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download, Lock, Trash2 } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { captures, tenants, users } from '@/api/endpoints'
import type { CaptureResponse, Device } from '@/types'

/** How long an allow may run: at most 30 days (docs/CALL_DIAGNOSTICS.md §3). */
const DURATIONS = [
  { days: 1, label: '1 day' },
  { days: 7, label: '7 days' },
  { days: 30, label: '30 days' },
]

const size = (bytes: number) =>
  bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`

/** A device's captures and who may take them: allow the app or its owner
 * until a date, end an allow, download (audited) or remove a capture. */
export function DeviceCapturesDialog({ device, onClose }: { device: Device | null; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [days, setDays] = useState('7')
  const [reason, setReason] = useState('')
  const [error, setError] = useState<string | null>(null)
  const open = !!device
  const { data: tenant } = useQuery({
    queryKey: ['tenant', device?.tenant_id],
    queryFn: () => tenants.get(device!.tenant_id),
    enabled: open,
  })
  const { data: owner } = useQuery({
    queryKey: ['user-by-id', device?.owner_user_id],
    queryFn: () => users.byId(device!.owner_user_id!),
    enabled: open && !!device?.owner_user_id,
  })
  const { data: allows = [] } = useQuery({
    queryKey: ['capture-allows'],
    queryFn: captures.allows,
    enabled: open,
  })
  const { data: list = [], isLoading } = useQuery({
    queryKey: ['captures', device?.id],
    queryFn: () => captures.list({ device_id: device!.id }),
    enabled: open,
  })
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ['capture-allows'] })
    queryClient.invalidateQueries({ queryKey: ['captures', device?.id] })
  }
  const failed = (e: Error) => setError(e.message)
  const allow = useMutation({
    mutationFn: (who: 'device' | 'owner') =>
      captures.allow({
        ...(who === 'device' ? { device_id: device!.id } : { username: owner!.username }),
        until: new Date(Date.now() + Number(days) * 86_400_000).toISOString(),
        reason: reason.trim() || null,
      }),
    onSuccess: () => {
      setError(null)
      setReason('')
      refresh()
    },
    onError: failed,
  })
  const end = useMutation({ mutationFn: captures.endAllow, onSuccess: refresh, onError: failed })
  const remove = useMutation({ mutationFn: captures.delete, onSuccess: refresh, onError: failed })
  const download = useMutation({ mutationFn: (c: CaptureResponse) => captures.download(c), onError: failed })

  const on = tenant?.diagnostics?.capture_allowed ?? false
  const mine = allows.filter(
    (a) => a.active && (a.device_id === device?.id || (!!device?.owner_user_id && a.user_id === device.owner_user_id)),
  )

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Captures: {device?.name}</DialogTitle>
          <DialogDescription>
            A capture holds the app's calls as it saw them: SIP before encryption, the audio, and the keys to play it.
            The app uploads each one when the call ends.
          </DialogDescription>
        </DialogHeader>

        {!on ? (
          <p className="text-sm text-muted-foreground">
            The tenant has captures off. Turn on “Apps may capture calls” on the Tenants page first.
          </p>
        ) : (
          <div className="space-y-3">
            <p className="text-sm font-medium">Who may capture</p>
            {mine.length === 0 && <p className="text-sm text-muted-foreground">Nobody: this app may not capture.</p>}
            {mine.map((a) => (
              <div key={a.id} className="flex items-center justify-between gap-2 text-sm">
                <span>
                  {a.kind === 'device' ? 'This app' : `${a.username ?? 'Its owner'}, on every app`} until{' '}
                  {new Date(a.until).toLocaleString()}
                  {a.reason && <span className="text-muted-foreground"> — {a.reason}</span>}
                </span>
                <Button variant="outline" size="sm" disabled={end.isPending} onClick={() => end.mutate(a.id)}>
                  End
                </Button>
              </div>
            ))}
            <div className="flex flex-wrap items-end gap-2">
              <div className="space-y-1">
                <Label htmlFor="capture-days">For</Label>
                <Select value={days} onValueChange={setDays}>
                  <SelectTrigger id="capture-days" className="w-28">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DURATIONS.map((d) => (
                      <SelectItem key={d.days} value={String(d.days)}>
                        {d.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="min-w-48 flex-1 space-y-1">
                <Label htmlFor="capture-reason">Why</Label>
                <Input id="capture-reason" value={reason} onChange={(e) => setReason(e.target.value)} placeholder="One-way audio from home" />
              </div>
              <Button disabled={allow.isPending} onClick={() => allow.mutate('device')}>
                Allow this app
              </Button>
              {owner && (
                <Button variant="outline" disabled={allow.isPending} onClick={() => allow.mutate('owner')}>
                  Allow {owner.username} on every app
                </Button>
              )}
            </div>
          </div>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}

        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Uploaded</TableHead>
              <TableHead>User</TableHead>
              <TableHead>Call</TableHead>
              <TableHead>Size</TableHead>
              <TableHead>Kept until</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={6}>Loading…</TableCell>
              </TableRow>
            )}
            {!isLoading && list.length === 0 && (
              <TableRow>
                <TableCell colSpan={6} className="text-muted-foreground">
                  No captures.
                </TableCell>
              </TableRow>
            )}
            {list.map((c) => (
              <TableRow key={c.id}>
                <TableCell className="whitespace-nowrap text-sm">{new Date(c.uploaded_at).toLocaleString()}</TableCell>
                <TableCell className="text-sm">{c.username ?? c.user_id}</TableCell>
                <TableCell className="font-mono text-xs">{c.call_id ?? '-'}</TableCell>
                <TableCell className="text-sm">{size(c.size)}</TableCell>
                <TableCell className="whitespace-nowrap text-sm">
                  {c.held ? (
                    <Badge variant="secondary">
                      <Lock className="mr-1 h-3 w-3" />
                      Held
                    </Badge>
                  ) : (
                    new Date(c.expires_at).toLocaleDateString()
                  )}
                </TableCell>
                <TableCell className="whitespace-nowrap text-right">
                  <Button variant="ghost" size="icon" aria-label="Download" onClick={() => download.mutate(c)}>
                    <Download className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="Remove"
                    disabled={c.held || remove.isPending}
                    onClick={() => remove.mutate(c.id)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </DialogContent>
    </Dialog>
  )
}
