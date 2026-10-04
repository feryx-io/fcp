import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, KeyRound, Smartphone, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { configTemplates, devices, provisioning, users } from '@/api/endpoints'
import type { Device, PendingDevice } from '@/types'

/** The tenant's provisioning URL (docs/PROVISIONING.md): shown, copied, and
 * replaced — which stops the old one working. */
export function ProvisioningUrlCard() {
  const queryClient = useQueryClient()
  const [copied, setCopied] = useState(false)
  const { data: key, isLoading } = useQuery({ queryKey: ['provisioning-key'], queryFn: provisioning.getKey })
  const rotate = useMutation({
    mutationFn: provisioning.rotateKey,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['provisioning-key'] }),
  })
  const shown = key ? (key.url ?? key.path) : null
  return (
    <Card>
      <CardContent className="flex flex-wrap items-center justify-between gap-4 p-4">
        <div className="min-w-0">
          <p className="text-sm font-medium">Provisioning URL</p>
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : shown ? (
            <p className="break-all font-mono text-sm">{shown}</p>
          ) : (
            <p className="text-sm text-muted-foreground">
              None yet. Phones fetch their files under it, as their vendor names them (e.g.{' '}
              <span className="font-mono">&lt;mac&gt;.cfg</span>); HTTPS only.
            </p>
          )}
        </div>
        <div className="flex gap-2">
          {shown && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard?.writeText(shown)
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              }}
            >
              <Copy className="mr-2 h-4 w-4" /> {copied ? 'Copied' : 'Copy'}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={rotate.isPending}
            onClick={() => {
              if (!key || confirm('Make a new provisioning URL? Phones using the old one stop getting their files until they are given the new one.')) {
                rotate.mutate()
              }
            }}
          >
            <KeyRound className="mr-2 h-4 w-4" /> {key ? 'New URL' : 'Make URL'}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

const OUTCOMES: Record<string, string> = {
  served: 'Served',
  unauthorized: 'Wrong or missing device credentials',
  network_refused: 'From outside the tenant’s networks',
  not_served: 'Disabled, no template, or the template fails',
}

/** A device's provisioning fetches, newest first. */
export function DeviceFetchesDialog({ device, onClose }: { device: { id: string; name: string } | null; onClose: () => void }) {
  const { data: fetches = [], isLoading } = useQuery({
    queryKey: ['device-fetches', device?.id],
    queryFn: () => devices.fetches(device!.id),
    enabled: !!device,
  })
  return (
    <Dialog open={!!device} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Provisioning fetches: {device?.name}</DialogTitle>
          <DialogDescription>What the phone asked for, from where, and what it was told.</DialogDescription>
        </DialogHeader>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>When</TableHead>
              <TableHead>File</TableHead>
              <TableHead>From</TableHead>
              <TableHead>Outcome</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && <TableRow><TableCell colSpan={4}>Loading…</TableCell></TableRow>}
            {!isLoading && fetches.length === 0 && (
              <TableRow><TableCell colSpan={4} className="text-muted-foreground">No fetches yet.</TableCell></TableRow>
            )}
            {fetches.map((f, i) => (
              <TableRow key={i}>
                <TableCell className="whitespace-nowrap text-sm">{new Date(f.at).toLocaleString()}</TableCell>
                <TableCell className="font-mono text-xs">{f.file}</TableCell>
                <TableCell className="text-xs">
                  {f.address ?? '-'}
                  {f.user_agent && <div className="text-muted-foreground">{f.user_agent}</div>}
                </TableCell>
                <TableCell className="text-sm">{OUTCOMES[f.outcome] ?? f.outcome}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </DialogContent>
    </Dialog>
  )
}

const NONE = '__none'

/** Phones that fetched with a MAC no device has (docs/PROVISIONING.md §6):
 * claimed in one step, or dismissed. */
export function PendingPhonesCard({ onClaimed }: { onClaimed: (device: Device) => void }) {
  const queryClient = useQueryClient()
  const [claiming, setClaiming] = useState<PendingDevice | null>(null)
  const { data: pending = [] } = useQuery({ queryKey: ['provisioning-pending'], queryFn: provisioning.pending })
  const dismiss = useMutation({
    mutationFn: provisioning.dismiss,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['provisioning-pending'] }),
  })
  if (pending.length === 0) return null
  return (
    <Card>
      <CardContent className="space-y-3 p-4">
        <div>
          <p className="text-sm font-medium">Phones waiting to be claimed</p>
          <p className="text-sm text-muted-foreground">
            These asked for their files under the provisioning URL with a MAC no device has.
          </p>
        </div>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>MAC</TableHead>
              <TableHead>Phone</TableHead>
              <TableHead>Last seen</TableHead>
              <TableHead>Asked</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {pending.map((p) => (
              <TableRow key={p.mac_address}>
                <TableCell className="font-mono text-xs">{p.mac_address}</TableCell>
                <TableCell className="text-sm">
                  {[p.vendor, p.model, p.firmware].filter(Boolean).join(' ') || '-'}
                  {p.address && <div className="text-xs text-muted-foreground">{p.address}</div>}
                </TableCell>
                <TableCell className="whitespace-nowrap text-sm">{new Date(p.last_seen).toLocaleString()}</TableCell>
                <TableCell className="text-sm">{p.fetches}</TableCell>
                <TableCell className="text-right">
                  <Button size="sm" onClick={() => setClaiming(p)}>
                    <Smartphone className="mr-2 h-4 w-4" /> Claim
                  </Button>{' '}
                  <Button size="sm" variant="ghost" aria-label="Dismiss" onClick={() => dismiss.mutate(p.mac_address)}>
                    <X className="h-4 w-4" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        <ClaimDialog
          pending={claiming}
          onClose={() => setClaiming(null)}
          onClaimed={(device) => {
            setClaiming(null)
            queryClient.invalidateQueries({ queryKey: ['provisioning-pending'] })
            queryClient.invalidateQueries({ queryKey: ['devices'] })
            onClaimed(device)
          }}
        />
      </CardContent>
    </Card>
  )
}

function ClaimDialog({
  pending,
  onClose,
  onClaimed,
}: {
  pending: PendingDevice | null
  onClose: () => void
  onClaimed: (device: Device) => void
}) {
  const [name, setName] = useState('')
  const [owner, setOwner] = useState(NONE)
  const [template, setTemplate] = useState(NONE)
  const { data: userList = [] } = useQuery({ queryKey: ['users'], queryFn: users.list, enabled: !!pending })
  const { data: templateList = [] } = useQuery({
    queryKey: ['config-templates'],
    queryFn: configTemplates.list,
    enabled: !!pending,
  })
  const claim = useMutation({
    mutationFn: () =>
      provisioning.claim(pending!.mac_address, {
        name: name.trim() || undefined,
        owner_user_id: owner === NONE ? undefined : owner,
        config_template_id: template === NONE ? undefined : template,
      }),
    onSuccess: (device) => {
      setName('')
      setOwner(NONE)
      setTemplate(NONE)
      onClaimed(device)
    },
  })
  return (
    <Dialog open={!!pending} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Claim {pending?.mac_address}</DialogTitle>
          <DialogDescription>
            It becomes a device with the vendor and model it gave
            {pending?.model ? ` (${[pending.vendor, pending.model].filter(Boolean).join(' ')})` : ''}.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="claim-name">Name</Label>
            <Input id="claim-name" value={name} placeholder="Its vendor, model and MAC" onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label>Owner</Label>
            <Select value={owner} onValueChange={setOwner}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>Nobody</SelectItem>
                {userList.map((u) => (
                  <SelectItem key={u.id} value={u.id}>{u.display_name ? `${u.display_name} (${u.username})` : u.username}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>Template</Label>
            <Select value={template} onValueChange={setTemplate}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>Picked by vendor and model</SelectItem>
                {templateList.map((t) => (
                  <SelectItem key={t.id} value={t.id}>{t.name} ({t.vendor})</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {claim.isError && <p className="text-sm text-destructive">{(claim.error as Error).message}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>Cancel</Button>
            <Button disabled={claim.isPending} onClick={() => claim.mutate()}>Claim</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/** What a device's phone would be given now (credentials included). */
export function DeviceConfigDialog({ device, onClose }: { device: { id: string; name: string } | null; onClose: () => void }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ['device-config', device?.id],
    queryFn: () => devices.render(device!.id),
    enabled: !!device,
    retry: false,
  })
  return (
    <Dialog open={!!device} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Configuration: {device?.name}</DialogTitle>
          <DialogDescription>
            {data?.config_template_name
              ? `From the template “${data.config_template_name}” (${data.vendor}). It holds the phone's credentials.`
              : 'What the phone is given when it fetches.'}
          </DialogDescription>
        </DialogHeader>
        {isLoading && <p className="text-sm text-muted-foreground">Rendering…</p>}
        {error && <p className="text-sm text-destructive">{(error as Error).message}</p>}
        {data && (
          <pre className="max-h-[60vh] overflow-auto rounded-md bg-muted p-3 font-mono text-xs">{data.content}</pre>
        )}
      </DialogContent>
    </Dialog>
  )
}
