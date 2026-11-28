// Backups from the admin (docs/SCHEDULED_BACKUPS.md B2, B3a): where snapshots
// go (SFTP or an S3-compatible bucket), when, how many are kept, a test and
// "Back up now", every run, the master key downloaded once, and a snapshot
// downloaded for restore.sh. Platform
// administrators only; fcp-backup does the work.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Archive, Download, History, KeyRound, Pencil, Play, Plus, RotateCcw, ShieldCheck, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { backups } from '@/api/endpoints'
import type { BackupDestination, BackupDestinationRequest, BackupDownload, BackupRun, LostFile } from '@/types'
import { formatDateTime } from '@/lib/utils'
import { useIsPlatformAdmin } from '@/components/TenantSelector'

const bytes = (n: number) => {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let v = n
  let u = 0
  while (v >= 1024 && u < units.length - 1) { v /= 1024; u++ }
  return u === 0 ? `${n} B` : `${v.toFixed(1)} ${units[u]}`
}

const runBadge = (r?: BackupRun | null) => {
  if (!r) return <Badge variant="outline">Never run</Badge>
  switch (r.state) {
    case 'succeeded': return <Badge variant="success">Succeeded</Badge>
    case 'failed': return <Badge variant="destructive">Failed</Badge>
    case 'running': return <Badge variant="warning">Running</Badge>
    default: return <Badge variant="secondary">Waiting</Badge>
  }
}

/** A destination's snapshots, each downloadable in backup.sh's layout for
 *  restore.sh on any host (B3a). A point that does not hold the master key,
 *  restored where this system's key is another, needs the key file. */
function SnapshotsDialog({ destination, onClose }: { destination: BackupDestination | null; onClose: () => void }) {
  const snaps = useQuery({
    queryKey: ['backups', 'snapshots', destination?.id],
    queryFn: () => backups.snapshots(destination!.id),
    enabled: destination != null,
    retry: false,
  })
  const [run, setRun] = useState<string | null>(null)
  const [keyFile, setKeyFile] = useState('')
  const [passphrase, setPassphrase] = useState('')
  const [building, setBuilding] = useState<BackupDownload | null>(null)
  const [error, setError] = useState<string | null>(null)
  const follow = useQuery({
    queryKey: ['backups', 'download', building?.id],
    queryFn: () => backups.downloadState(building!.id),
    enabled: building != null,
    refetchInterval: (q) => (q.state.data?.state ?? 'building') === 'building' ? 2000 : false,
  })
  const state = follow.data ?? building
  const start = useMutation({
    mutationFn: () => backups.download(destination!.id, run!, keyFile.trim() ? { key_file: keyFile, passphrase } : {}),
    onSuccess: (d) => { setError(null); setBuilding(d) },
    onError: (e) => setError((e as Error).message),
  })
  const caps = useQuery({ queryKey: ['backups', 'capabilities'], queryFn: backups.capabilities, enabled: destination != null })
  const [putBack, setPutBack] = useState<string | null>(null)
  const restoreVolume = useMutation({
    mutationFn: (volume: string) => backups.restoreVolume(destination!.id, run!, volume),
    onSuccess: (r, volume) => { setError(null); setPutBack(`${volume}: missing files put back (${r.restored.filter((f) => f.present).length} checked)`) },
    onError: (e) => setError((e as Error).message),
  })
  const chosen = (snaps.data ?? []).find((p) => p.run === run)
  const close = () => { setRun(null); setBuilding(null); setKeyFile(''); setPassphrase(''); setError(null); setPutBack(null); onClose() }
  return (
    <Dialog open={destination != null} onOpenChange={(o) => { if (!o) close() }}>
      <DialogContent className="max-h-[90vh] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Snapshots in {destination?.name}</DialogTitle>
          <DialogDescription>
            Download one to restore with <code>restore.sh</code> on any host: the database, every volume and the
            key, checksummed, as <code>backup.sh</code> writes them.
          </DialogDescription>
        </DialogHeader>
        {snaps.isLoading && <p className="text-sm text-muted-foreground">Asking the destination…</p>}
        {snaps.error && <p className="text-sm text-destructive">{(snaps.error as Error).message}</p>}
        {(snaps.data ?? []).length === 0 && snaps.isSuccess && <p className="text-sm text-muted-foreground">No snapshots yet.</p>}
        {(snaps.data ?? []).length > 0 && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Taken</TableHead>
                <TableHead>Release</TableHead>
                <TableHead>Holds</TableHead>
                <TableHead className="text-right" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {(snaps.data ?? []).map((p) => (
                <TableRow key={p.run} data-state={run === p.run ? 'selected' : undefined}>
                  <TableCell>{formatDateTime(p.time)}</TableCell>
                  <TableCell>{p.release ?? '—'}</TableCell>
                  <TableCell className="text-xs">{['database', ...p.volumes].join(', ')}</TableCell>
                  <TableCell className="text-right">
                    <Button variant={run === p.run ? 'default' : 'outline'} size="sm" aria-label={`Choose the snapshot of ${p.time}`}
                      onClick={() => { setRun(p.run); setBuilding(null) }}>
                      Choose
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
        {run && (
          <div className="space-y-3 rounded border p-3">
            <div className="space-y-1">
              <Label htmlFor="bk-keyfile">Key file (if this system's master key is not the snapshot's)</Label>
              <textarea id="bk-keyfile" className="h-24 w-full rounded border bg-background p-2 font-mono text-xs"
                placeholder="-----BEGIN AGE ENCRYPTED FILE-----" value={keyFile} onChange={(e) => setKeyFile(e.target.value)} />
              <Input type="file" aria-label="Key file to read" onChange={async (e) => {
                const f = e.target.files?.[0]
                if (f) setKeyFile(await f.text())
              }} />
            </div>
            {keyFile.trim() && (
              <div className="space-y-1">
                <Label htmlFor="bk-pass">Its passphrase</Label>
                <Input id="bk-pass" type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
              </div>
            )}
            {chosen && chosen.volumes.length > 0 && (
              <div className="space-y-1">
                <Label>Put back missing files</Label>
                <p className="text-xs text-muted-foreground">
                  {caps.data?.file_restores
                    ? 'Files the volume no longer has come back; nothing there now is overwritten. Files removed on purpose come back too.'
                    : 'Restores are off: start fcp-backup with docker-compose.restore.yml to put files back here.'}
                </p>
                <div className="flex flex-wrap gap-1">
                  {chosen.volumes.filter((v) => v !== 'fcp-certs').map((v) => (
                    <Button key={v} variant="outline" size="sm" aria-label={`Put back ${v}`}
                      disabled={!caps.data?.file_restores || restoreVolume.isPending}
                      onClick={() => { if (confirm(`Put back every file ${v} is missing from this backup?`)) restoreVolume.mutate(v) }}>
                      <RotateCcw className="mr-1 h-3 w-3" /> {v}
                    </Button>
                  ))}
                </div>
                {putBack && <p className="text-sm">{putBack}</p>}
              </div>
            )}
            {error && <p className="text-sm text-destructive">{error}</p>}
            {state?.state === 'building' && <p className="text-sm text-muted-foreground">Putting the download together…</p>}
            {state?.state === 'failed' && <p className="text-sm text-destructive">{state.error}</p>}
            {state?.state === 'ready' && state.file_url && (
              <p className="text-sm">
                Ready{state.size != null && ` (${bytes(state.size)})`}:{' '}
                <a className="font-medium underline" href={state.file_url} download>save it</a>. The link works once.
              </p>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={close}>Close</Button>
          <Button disabled={!run || start.isPending || state?.state === 'building'} onClick={() => start.mutate()}>
            <Download className="mr-2 h-4 w-4" /> Make the download
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** The master key, downloaded once wrapped in a passphrase (Q4). */
function KeyCard() {
  const queryClient = useQueryClient()
  const status = useQuery({ queryKey: ['backups', 'key'], queryFn: backups.keyStatus })
  const [open, setOpen] = useState(false)
  const [phrase, setPhrase] = useState('')
  const [again, setAgain] = useState('')
  const [error, setError] = useState<string | null>(null)
  const download = useMutation({
    mutationFn: () => backups.exportKey(phrase),
    onSuccess: (url) => {
      const a = document.createElement('a')
      a.href = url
      a.download = `fcp-master-key-${(status.data?.kek_id ?? 'key').replace(/:/g, '-')}.age`
      a.click()
      URL.revokeObjectURL(url)
      setOpen(false)
      setPhrase('')
      setAgain('')
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['backups', 'key'] })
    },
    onError: (e) => setError((e as Error).message),
  })
  const s = status.data
  if (!s) return null
  if (!s.exportable) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2"><KeyRound className="h-5 w-5" /> The master key</CardTitle>
          <CardDescription>
            {s.provider === 'none'
              ? 'Encryption at rest is off: nothing sealed needs a key to restore.'
              : 'The master key is kept by the KMS and never leaves it; a restore uses the same KMS key.'}
          </CardDescription>
        </CardHeader>
      </Card>
    )
  }
  const needed = !s.last_exported_at
  return (
    <Card className={needed ? 'border-amber-500' : undefined}>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div>
          <CardTitle className="flex items-center gap-2"><KeyRound className="h-5 w-5" /> The master key</CardTitle>
          <CardDescription>
            Snapshots do not hold the key that opens their sealed secrets. Keep a copy off this machine, apart from
            the backups: a restore needs it.
          </CardDescription>
        </div>
        <Button variant={needed ? 'default' : 'outline'} size="sm" onClick={() => setOpen(true)}>
          <Download className="mr-2 h-4 w-4" /> {needed ? 'Download your master key' : 'Download again'}
        </Button>
      </CardHeader>
      <CardContent className="text-sm text-muted-foreground">
        {needed
          ? 'Not downloaded yet.'
          : `Downloaded ${formatDateTime(s.last_exported_at!)} by ${s.last_exported_by}.`}
        {' '}Key {s.kek_id}.
      </CardContent>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Download the master key</DialogTitle>
            <DialogDescription>
              The file is wrapped in this passphrase (an age file: <code>age -d</code> opens it too). The passphrase is
              kept nowhere; without it the file is useless.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); download.mutate() }}>
            <div className="space-y-1">
              <Label htmlFor="key-phrase">Passphrase</Label>
              <Input id="key-phrase" type="password" autoComplete="new-password" value={phrase} onChange={(e) => setPhrase(e.target.value)} />
            </div>
            <div className="space-y-1">
              <Label htmlFor="key-phrase-again">Passphrase again</Label>
              <Input id="key-phrase-again" type="password" autoComplete="new-password" value={again} onChange={(e) => setAgain(e.target.value)} />
            </div>
            {phrase && again && phrase !== again && <p className="text-sm text-destructive">The passphrases differ.</p>}
            {error && <p className="text-sm text-destructive">{error}</p>}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={phrase.length < 12 || phrase !== again || download.isPending}>Download</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </Card>
  )
}

interface Draft {
  id?: string
  name: string
  type: 'sftp' | 's3'
  host: string
  port: string
  user: string
  path: string
  private_key: string
  has_private_key: boolean
  endpoint: string
  bucket: string
  region: string
  prefix: string
  access_key_id: string
  secret_access_key: string
  has_secret: boolean
  password: string
  time_of_day: string
  daily: string
  weekly: string
  monthly: string
  include_key: boolean
  enabled: boolean
}

const empty: Draft = {
  name: '', type: 'sftp', host: '', port: '22', user: '', path: '', private_key: '', has_private_key: false,
  endpoint: '', bucket: '', region: '', prefix: '', access_key_id: '', secret_access_key: '', has_secret: false,
  password: '', time_of_day: '02:00', daily: '7', weekly: '4', monthly: '12', include_key: false, enabled: true,
}

const draftOf = (d: BackupDestination): Draft => {
  const t = d.target
  return {
    ...empty,
    id: d.id,
    name: d.name,
    type: t.type,
    ...(t.type === 'sftp'
      ? { host: t.host, port: String(t.port), user: t.user, path: t.path, has_private_key: t.has_private_key }
      : { endpoint: t.endpoint, bucket: t.bucket, region: t.region ?? '', prefix: t.prefix ?? '',
          access_key_id: t.access_key_id, has_secret: t.has_secret_access_key }),
    time_of_day: d.schedule.time_of_day ?? '',
    daily: String(d.keep.daily), weekly: String(d.keep.weekly), monthly: String(d.keep.monthly),
    include_key: d.include_key,
    enabled: d.enabled,
  }
}

const requestOf = (d: Draft): BackupDestinationRequest => ({
  name: d.name,
  target: d.type === 'sftp'
    ? { type: 'sftp', host: d.host, port: Number(d.port) || 22, user: d.user, path: d.path, private_key: d.private_key || null }
    : { type: 's3', endpoint: d.endpoint, bucket: d.bucket, region: d.region || null, prefix: d.prefix || null,
        access_key_id: d.access_key_id, secret_access_key: d.secret_access_key || null },
  password: d.password || null,
  schedule: { time_of_day: d.time_of_day || null },
  keep: { daily: Number(d.daily) || 0, weekly: Number(d.weekly) || 0, monthly: Number(d.monthly) || 0 },
  include_key: d.include_key,
  enabled: d.enabled,
})

function DestinationDialog({ draft, setDraft, onSaved }: {
  draft: Draft | null
  setDraft: (d: Draft | null) => void
  onSaved: (made: BackupDestination) => void
}) {
  const [error, setError] = useState<string | null>(null)
  const save = useMutation({
    mutationFn: (d: Draft) => (d.id ? backups.update(d.id, requestOf(d)) : backups.create(requestOf(d))),
    onSuccess: (made) => { setError(null); setDraft(null); onSaved(made) },
    onError: (e) => setError((e as Error).message),
  })
  const set = (patch: Partial<Draft>) => draft && setDraft({ ...draft, ...patch })
  return (
    <Dialog open={draft != null} onOpenChange={(o) => { if (!o) setDraft(null) }}>
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{draft?.id ? 'Edit destination' : 'Add a destination'}</DialogTitle>
          <DialogDescription>Snapshots are encrypted with the repository password before they leave this machine.</DialogDescription>
        </DialogHeader>
        {draft && (
          <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); save.mutate(draft) }}>
            <div className="space-y-1">
              <Label htmlFor="dest-name">Name</Label>
              <Input id="dest-name" value={draft.name} onChange={(e) => set({ name: e.target.value })} required />
            </div>
            <div className="space-y-1">
              <Label htmlFor="dest-type">Where</Label>
              <Select value={draft.type} onValueChange={(v) => set({ type: v as Draft['type'] })} disabled={!!draft.id}>
                <SelectTrigger id="dest-type"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="sftp">SFTP server</SelectItem>
                  <SelectItem value="s3">S3-compatible bucket</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {draft.type === 'sftp' ? (
              <>
                <div className="grid gap-2 sm:grid-cols-[1fr_6rem]">
                  <div className="space-y-1">
                    <Label htmlFor="dest-host">Host</Label>
                    <Input id="dest-host" value={draft.host} onChange={(e) => set({ host: e.target.value })} required />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="dest-port">Port</Label>
                    <Input id="dest-port" inputMode="numeric" value={draft.port} onChange={(e) => set({ port: e.target.value })} />
                  </div>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  <div className="space-y-1">
                    <Label htmlFor="dest-user">User</Label>
                    <Input id="dest-user" value={draft.user} onChange={(e) => set({ user: e.target.value })} required />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="dest-path">Directory</Label>
                    <Input id="dest-path" placeholder="/srv/fcp" value={draft.path} onChange={(e) => set({ path: e.target.value })} required />
                  </div>
                </div>
                <div className="space-y-1">
                  <Label htmlFor="dest-key">Private key</Label>
                  <textarea id="dest-key" className="min-h-24 w-full rounded-md border bg-background p-2 font-mono text-xs"
                    placeholder={draft.has_private_key ? 'Kept — paste a new one to replace it' : '-----BEGIN OPENSSH PRIVATE KEY-----'}
                    value={draft.private_key} onChange={(e) => set({ private_key: e.target.value })} required={!draft.has_private_key} />
                  <p className="text-xs text-muted-foreground">The server's host key is pinned by the first test.</p>
                </div>
              </>
            ) : (
              <>
                <div className="space-y-1">
                  <Label htmlFor="dest-endpoint">Endpoint</Label>
                  <Input id="dest-endpoint" placeholder="https://s3.us-east-1.amazonaws.com" value={draft.endpoint}
                    onChange={(e) => set({ endpoint: e.target.value })} required />
                </div>
                <div className="grid gap-2 sm:grid-cols-3">
                  <div className="space-y-1">
                    <Label htmlFor="dest-bucket">Bucket</Label>
                    <Input id="dest-bucket" value={draft.bucket} onChange={(e) => set({ bucket: e.target.value })} required />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="dest-region">Region</Label>
                    <Input id="dest-region" value={draft.region} onChange={(e) => set({ region: e.target.value })} />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="dest-prefix">Folder</Label>
                    <Input id="dest-prefix" value={draft.prefix} onChange={(e) => set({ prefix: e.target.value })} />
                  </div>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  <div className="space-y-1">
                    <Label htmlFor="dest-key-id">Access key id</Label>
                    <Input id="dest-key-id" value={draft.access_key_id} onChange={(e) => set({ access_key_id: e.target.value })} required />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="dest-secret">Secret access key</Label>
                    <Input id="dest-secret" type="password" autoComplete="off" value={draft.secret_access_key}
                      placeholder={draft.has_secret ? 'Kept' : ''} required={!draft.has_secret}
                      onChange={(e) => set({ secret_access_key: e.target.value })} />
                  </div>
                </div>
              </>
            )}
            <div className="space-y-1">
              <Label htmlFor="dest-password">Repository password</Label>
              <Input id="dest-password" type="password" autoComplete="new-password" value={draft.password}
                placeholder={draft.id ? 'Kept' : 'Leave empty to have one made'} onChange={(e) => set({ password: e.target.value })} />
            </div>
            <div className="grid gap-2 sm:grid-cols-4">
              <div className="space-y-1">
                <Label htmlFor="dest-time">Daily at (UTC)</Label>
                <Input id="dest-time" placeholder="02:00" value={draft.time_of_day} onChange={(e) => set({ time_of_day: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="dest-daily">Keep daily</Label>
                <Input id="dest-daily" inputMode="numeric" value={draft.daily} onChange={(e) => set({ daily: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="dest-weekly">Weekly</Label>
                <Input id="dest-weekly" inputMode="numeric" value={draft.weekly} onChange={(e) => set({ weekly: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="dest-monthly">Monthly</Label>
                <Input id="dest-monthly" inputMode="numeric" value={draft.monthly} onChange={(e) => set({ monthly: e.target.value })} />
              </div>
            </div>
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" checked={draft.include_key} onChange={(e) => set({ include_key: e.target.checked })} />
              <span>Include the master key in each snapshot. One file then restores everything; whoever has the
                snapshots and the repository password has every tenant's secrets.</span>
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={draft.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
              <span>Enabled (scheduled backups run)</span>
            </label>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDraft(null)}>Cancel</Button>
              <Button type="submit" disabled={save.isPending}>Save destination</Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

/** Whether restores are on, and how to turn them on. */
function RestoresCard() {
  const caps = useQuery({ queryKey: ['backups', 'capabilities'], queryFn: backups.capabilities })
  if (!caps.data) return null
  const c = caps.data
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2"><RotateCcw className="h-5 w-5" /> Restores</CardTitle>
        <CardDescription>
          {!c.service ? 'The backup service is not reachable from the admin ([backup] url).'
            : c.file_restores ? 'On: files can be put back from a snapshot, here and by tenant administrators.'
              : 'Off: the backup service reads every volume read-only, so nothing can be put back yet.'}
        </CardDescription>
      </CardHeader>
      {c.service && !c.file_restores && (
        <CardContent className="space-y-2 text-sm">
          <p>Switch them on for a restore, and off after:</p>
          <pre className="overflow-x-auto rounded bg-muted p-2 text-xs">docker compose -f docker-compose.yml -f docker-compose.restore.yml up -d fcp-backup</pre>
          {c.read_only.length > 0 && <p className="text-xs text-muted-foreground">Read-only now: {c.read_only.join(', ')}</p>}
        </CardContent>
      )}
    </Card>
  )
}

const CLASS_NAMES: Record<string, string> = {
  call_recording: 'Call recording',
  conference_recording: 'Conference recording',
  voicemail: 'Voicemail',
  voicemail_greeting: 'Voicemail greeting',
  prompt: 'Audio prompt',
}

/** A tenant administrator's view (Q9): the platform's schedule, the last
 *  good backup, and the tenant's own lost files put back. */
function TenantBackupsView() {
  const queryClient = useQueryClient()
  const status = useQuery({ queryKey: ['backups', 'tenant'], queryFn: backups.tenantStatus })
  const missing = useQuery({ queryKey: ['backups', 'missing'], queryFn: backups.tenantMissing, retry: false })
  const [note, setNote] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const restore = useMutation({
    mutationFn: async (files: LostFile[]) => {
      // One request per backup, each file from its newest.
      const byPoint = new Map<string, { class: string; item: string }[]>()
      for (const f of files) {
        const p = f.held_in[0]?.point
        if (!p) continue
        byPoint.set(p, [...(byPoint.get(p) ?? []), { class: f.class, item: f.item }])
      }
      let back = 0
      for (const [point, items] of byPoint) back += (await backups.tenantRestore(point, items)).restored.filter((r) => r.present).length
      return back
    },
    onSuccess: (n) => { setError(null); setNote(`${n} file${n === 1 ? '' : 's'} put back.`); queryClient.invalidateQueries({ queryKey: ['backups', 'missing'] }) },
    onError: (e) => setError((e as Error).message),
  })
  const list = missing.data ?? []
  const restorable = list.filter((f) => f.held_in.length > 0)
  const s = status.data
  return (
    <div className="space-y-6">
      <div>
        <h1 className="flex items-center gap-2 text-3xl font-bold tracking-tight"><Archive className="h-7 w-7" /> Backups</h1>
        <p className="text-muted-foreground">The platform backs up your recordings, voicemail and prompts with everything else.</p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Schedule</CardTitle>
          <CardDescription>
            {s?.times_of_day.length ? `Daily at ${s.times_of_day.join(', ')} UTC.` : 'No scheduled backups.'}{' '}
            {s?.last_good_backup_at ? `The last good backup finished ${formatDateTime(s.last_good_backup_at)}.` : 'No backup has finished yet.'}
          </CardDescription>
        </CardHeader>
      </Card>
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <CardTitle>Lost files</CardTitle>
              <CardDescription>Files your records still name that are gone. Each comes back from the newest backup holding it; nothing there now is overwritten.</CardDescription>
            </div>
            <Button disabled={!s?.restores_available || restorable.length === 0 || restore.isPending} onClick={() => restore.mutate(restorable)}>
              <RotateCcw className="mr-2 h-4 w-4" /> Restore all
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {s && !s.restores_available && <p className="text-sm text-muted-foreground">Restores are not available right now; your platform administrator can switch them on.</p>}
          {note && <p className="text-sm">{note}</p>}
          {error && <p className="text-sm text-destructive">{error}</p>}
          {missing.error && <p className="text-sm text-destructive">{(missing.error as Error).message}</p>}
          {missing.isSuccess && list.length === 0 && <p className="text-sm text-muted-foreground">Nothing is missing.</p>}
          {list.length > 0 && (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>What</TableHead>
                    <TableHead>Made</TableHead>
                    <TableHead>In a backup of</TableHead>
                    <TableHead className="text-right" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {list.map((f) => (
                    <TableRow key={`${f.class}:${f.item}`}>
                      <TableCell>{CLASS_NAMES[f.class] ?? f.class} <span className="font-mono text-xs text-muted-foreground">{f.item}</span></TableCell>
                      <TableCell>{f.created_at ? formatDateTime(f.created_at) : '—'}</TableCell>
                      <TableCell>{f.held_in[0] ? formatDateTime(f.held_in[0].time) : <span className="text-muted-foreground">No backup holds it</span>}</TableCell>
                      <TableCell className="text-right">
                        <Button variant="outline" size="sm" aria-label={`Restore ${f.item}`}
                          disabled={!s?.restores_available || f.held_in.length === 0 || restore.isPending}
                          onClick={() => restore.mutate([f])}>
                          Restore
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

export function BackupsPage() {
  const platform = useIsPlatformAdmin()
  return platform ? <PlatformBackups /> : <TenantBackupsView />
}

function PlatformBackups() {
  const queryClient = useQueryClient()
  const destinations = useQuery({ queryKey: ['backups', 'destinations'], queryFn: backups.destinations })
  const runs = useQuery({
    queryKey: ['backups', 'runs'],
    queryFn: () => backups.runs(),
    // Follow a run that is waiting or going.
    refetchInterval: (q) => (q.state.data ?? []).some((r) => r.state === 'requested' || r.state === 'running') ? 3000 : false,
  })
  const [draft, setDraft] = useState<Draft | null>(null)
  const [password, setPassword] = useState<string | null>(null)
  const [browsing, setBrowsing] = useState<BackupDestination | null>(null)
  const [error, setError] = useState<string | null>(null)
  const changed = () => queryClient.invalidateQueries({ queryKey: ['backups'] })
  const act = useMutation({
    mutationFn: async ({ id, what }: { id: string; what: 'test' | 'run' | 'delete' }) => {
      if (what === 'test') await backups.test(id)
      else if (what === 'run') await backups.run(id)
      else await backups.remove(id)
    },
    onSuccess: () => { setError(null); changed() },
    onError: (e) => setError((e as Error).message),
  })
  const names = new Map((destinations.data ?? []).map((d) => [d.id, d.name]))
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="flex items-center gap-2 text-3xl font-bold tracking-tight"><Archive className="h-7 w-7" /> Backups</h1>
          <p className="text-muted-foreground">
            The database and every recording, voicemail and prompt, off this machine on a schedule. Each run sends only
            what the destination does not have yet.
          </p>
        </div>
        <Button onClick={() => setDraft({ ...empty })}><Plus className="mr-2 h-4 w-4" /> Add a destination</Button>
      </div>
      {error && <p className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{error}</p>}

      <KeyCard />
      <RestoresCard />

      <Card>
        <CardHeader>
          <CardTitle>Destinations</CardTitle>
          <CardDescription>Test a new destination first: it pins an SFTP server's host key and makes the repository.</CardDescription>
        </CardHeader>
        <CardContent>
          {(destinations.data ?? []).length === 0 ? (
            <p className="text-sm text-muted-foreground">None yet: nothing is backed up off this machine.</p>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Where</TableHead>
                    <TableHead>Schedule</TableHead>
                    <TableHead>Keeps</TableHead>
                    <TableHead>Last backup</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {(destinations.data ?? []).map((d) => (
                    <TableRow key={d.id}>
                      <TableCell className="font-medium">
                        {d.name}
                        {!d.enabled && <Badge variant="outline" className="ml-2">Off</Badge>}
                        {d.include_key && <Badge variant="warning" className="ml-2">Holds the key</Badge>}
                      </TableCell>
                      <TableCell className="font-mono text-xs">{d.repository}</TableCell>
                      <TableCell>{d.schedule.time_of_day ? `Daily at ${d.schedule.time_of_day} UTC` : 'By hand'}</TableCell>
                      <TableCell>{d.keep.daily} daily, {d.keep.weekly} weekly, {d.keep.monthly} monthly</TableCell>
                      <TableCell>
                        <div className="flex flex-col gap-1">
                          {runBadge(d.last_run)}
                          {d.last_run?.finished_at && <span className="text-xs text-muted-foreground">{formatDateTime(d.last_run.finished_at)}</span>}
                        </div>
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-1">
                          <Button variant="outline" size="sm" aria-label={`Test ${d.name}`} onClick={() => act.mutate({ id: d.id, what: 'test' })}>
                            <ShieldCheck className="mr-1 h-4 w-4" /> Test
                          </Button>
                          <Button variant="outline" size="sm" aria-label={`Back up ${d.name} now`} onClick={() => act.mutate({ id: d.id, what: 'run' })}>
                            <Play className="mr-1 h-4 w-4" /> Back up now
                          </Button>
                          <Button variant="outline" size="sm" aria-label={`Snapshots in ${d.name}`} onClick={() => setBrowsing(d)}>
                            <History className="mr-1 h-4 w-4" /> Snapshots
                          </Button>
                          <Button variant="ghost" size="icon" aria-label={`Edit ${d.name}`} onClick={() => setDraft(draftOf(d))}>
                            <Pencil className="h-4 w-4" />
                          </Button>
                          <Button variant="ghost" size="icon" aria-label={`Delete ${d.name}`}
                            onClick={() => { if (confirm(`Delete ${d.name}? The snapshots stored there are not touched.`)) act.mutate({ id: d.id, what: 'delete' }) }}>
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Runs</CardTitle>
          <CardDescription>Every test and backup, newest first.</CardDescription>
        </CardHeader>
        <CardContent>
          {(runs.data ?? []).length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing has run yet.</p>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Destination</TableHead>
                    <TableHead>What</TableHead>
                    <TableHead>State</TableHead>
                    <TableHead>Asked for</TableHead>
                    <TableHead>Finished</TableHead>
                    <TableHead className="text-right">Added</TableHead>
                    <TableHead>Said</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {(runs.data ?? []).map((r) => (
                    <TableRow key={r.id}>
                      <TableCell>{names.get(r.destination_id) ?? r.destination_id}</TableCell>
                      <TableCell>{r.kind === 'test' ? 'Test' : 'Backup'} <span className="text-muted-foreground">· {r.trigger}</span></TableCell>
                      <TableCell>{runBadge(r)}</TableCell>
                      <TableCell>{formatDateTime(r.requested_at)}</TableCell>
                      <TableCell>{r.finished_at ? formatDateTime(r.finished_at) : ''}</TableCell>
                      <TableCell className="text-right tabular-nums">{r.kind === 'backup' && r.state === 'succeeded' ? bytes(r.bytes_added) : ''}</TableCell>
                      <TableCell className="max-w-md truncate text-sm" title={r.message ?? ''}>{r.message ?? ''}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      <SnapshotsDialog destination={browsing} onClose={() => setBrowsing(null)} />
      <DestinationDialog draft={draft} setDraft={setDraft}
        onSaved={(made) => { changed(); if (made.password) setPassword(made.password) }} />

      <Dialog open={password != null} onOpenChange={(o) => { if (!o) setPassword(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Keep the repository password</DialogTitle>
            <DialogDescription>
              It was made for this destination and is shown this once. Every snapshot there is encrypted with it; restic
              needs it to restore without this admin.
            </DialogDescription>
          </DialogHeader>
          <code className="block break-all rounded border bg-muted p-3 text-sm" aria-label="Repository password">{password}</code>
          <DialogFooter>
            <Button onClick={() => { void navigator.clipboard?.writeText(password ?? ''); setPassword(null) }}>Copy and close</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
