import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Plus, Rocket, Trash2, Upload } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { devices, firmware } from '@/api/endpoints'
import { useIsPlatformAdmin } from '@/components/TenantSelector'
import type { FirmwareTarget } from '@/types'

const VENDORS = ['yealink', 'polycom', 'grandstream', 'cisco', 'fanvil', 'snom']

function megabytes(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/** Firmware (docs/PROVISIONING.md §11): images the platform administrator
 * uploads, the tenant's targets, and who is behind. */
export function FirmwarePage() {
  const queryClient = useQueryClient()
  const platformAdmin = useIsPlatformAdmin()
  const [uploading, setUploading] = useState(false)
  const [editing, setEditing] = useState<FirmwareTarget | 'new' | null>(null)
  const { data: images = [] } = useQuery({ queryKey: ['firmware-images'], queryFn: firmware.images })
  const { data: targets = [] } = useQuery({ queryKey: ['firmware-targets'], queryFn: firmware.targets })
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ['firmware-images'] })
    queryClient.invalidateQueries({ queryKey: ['firmware-targets'] })
  }
  const removeImage = useMutation({ mutationFn: firmware.deleteImage, onSuccess: refresh, onError: (e) => alert((e as Error).message) })
  const removeTarget = useMutation({ mutationFn: firmware.deleteTarget, onSuccess: refresh })
  const rollOut = useMutation({
    mutationFn: (t: FirmwareTarget) =>
      firmware.updateTarget(t.id, {
        vendor: t.vendor,
        model_pattern: t.model_pattern ?? undefined,
        image_id: t.image_id,
        stage: 'all',
        pilot_devices: t.pilot_devices,
      }),
    onSuccess: refresh,
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Firmware</h1>
          <p className="text-muted-foreground">
            Point a vendor's models at an image, for a pilot set of phones first; their templates' firmware URL follows.
          </p>
        </div>
        <Button onClick={() => setEditing('new')} disabled={images.length === 0}>
          <Plus className="mr-2 h-4 w-4" /> New target
        </Button>
      </div>

      <Card>
        <CardContent className="space-y-3 p-4">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium">Targets</p>
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Phones</TableHead>
                <TableHead>Version</TableHead>
                <TableHead>Stage</TableHead>
                <TableHead>Current</TableHead>
                <TableHead>Behind</TableHead>
                <TableHead>Unknown</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {targets.length === 0 && (
                <TableRow><TableCell colSpan={7} className="text-muted-foreground">No targets: phones are told of no firmware.</TableCell></TableRow>
              )}
              {targets.map((t) => (
                <TableRow key={t.id}>
                  <TableCell>{t.vendor} <span className="font-mono text-xs">{t.model_pattern || 'any model'}</span></TableCell>
                  <TableCell className="font-mono text-xs">{t.version ?? '-'}</TableCell>
                  <TableCell>
                    <Badge variant={t.stage === 'all' ? 'success' : 'secondary'}>
                      {t.stage === 'all' ? 'every phone' : `pilot (${t.pilot_devices.length})`}
                    </Badge>
                  </TableCell>
                  <TableCell>{t.current} / {t.devices}</TableCell>
                  <TableCell>{t.behind}</TableCell>
                  <TableCell>{t.unknown}</TableCell>
                  <TableCell className="whitespace-nowrap text-right">
                    {t.stage === 'pilot' && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={rollOut.isPending}
                        onClick={() => { if (confirm('Roll this image out to every matching phone?')) rollOut.mutate(t) }}
                      >
                        <Rocket className="mr-2 h-4 w-4" /> Roll out
                      </Button>
                    )}{' '}
                    <Button size="sm" variant="ghost" onClick={() => setEditing(t)}>Edit</Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label="Delete"
                      onClick={() => { if (confirm('Delete the target? Phones keep what they run.')) removeTarget.mutate(t.id) }}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="space-y-3 p-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Images</p>
              <p className="text-sm text-muted-foreground">Uploaded by the platform administrator; every tenant can target them.</p>
            </div>
            {platformAdmin && (
              <Button size="sm" variant="outline" onClick={() => setUploading(true)}>
                <Upload className="mr-2 h-4 w-4" /> Upload image
              </Button>
            )}
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Vendor</TableHead>
                <TableHead>Version</TableHead>
                <TableHead>Models</TableHead>
                <TableHead>File</TableHead>
                <TableHead>Size</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {images.length === 0 && (
                <TableRow><TableCell colSpan={6} className="text-muted-foreground">No images yet.</TableCell></TableRow>
              )}
              {images.map((i) => (
                <TableRow key={i.id}>
                  <TableCell>{i.vendor}</TableCell>
                  <TableCell className="font-mono text-xs">{i.version}</TableCell>
                  <TableCell className="font-mono text-xs">{i.models.join(', ') || 'any'}</TableCell>
                  <TableCell className="font-mono text-xs" title={`SHA-256 ${i.sha256}`}>{i.file_name}</TableCell>
                  <TableCell>{megabytes(i.size)}</TableCell>
                  <TableCell className="text-right">
                    {platformAdmin && (
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label="Remove"
                        onClick={() => { if (confirm(`Remove ${i.file_name}?`)) removeImage.mutate(i.id) }}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <UploadDialog open={uploading} onClose={() => setUploading(false)} onDone={refresh} />
      <TargetDialog target={editing} onClose={() => setEditing(null)} onDone={refresh} />
    </div>
  )
}

function UploadDialog({ open, onClose, onDone }: { open: boolean; onClose: () => void; onDone: () => void }) {
  const [vendor, setVendor] = useState('yealink')
  const [version, setVersion] = useState('')
  const [models, setModels] = useState('')
  const [notes, setNotes] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const upload = useMutation({
    mutationFn: () => firmware.upload({ vendor, version, models, notes, file: file! }),
    onSuccess: () => {
      onDone()
      onClose()
      setVersion('')
      setModels('')
      setNotes('')
      setFile(null)
    },
  })
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Upload a firmware image</DialogTitle>
          <DialogDescription>
            The version as the phones report it in their User-Agent; the file under the vendor's own name.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <Label>Vendor</Label>
            <Select value={vendor} onValueChange={setVendor}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>{VENDORS.map((v) => <SelectItem key={v} value={v}>{v}</SelectItem>)}</SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label htmlFor="fw-version">Version</Label>
            <Input id="fw-version" value={version} placeholder="96.86.0.80" onChange={(e) => setVersion(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="fw-models">Models</Label>
            <Input id="fw-models" value={models} placeholder="T5*, T4* — empty for any" onChange={(e) => setModels(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="fw-notes">Notes</Label>
            <Input id="fw-notes" value={notes} onChange={(e) => setNotes(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="fw-file">File</Label>
            <Input id="fw-file" type="file" onChange={(e) => setFile(e.target.files?.[0] ?? null)} />
          </div>
          {upload.isError && <p className="text-sm text-destructive">{(upload.error as Error).message}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>Cancel</Button>
            <Button disabled={upload.isPending || !file || !version.trim()} onClick={() => upload.mutate()}>
              {upload.isPending ? 'Uploading…' : 'Upload'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function TargetDialog({
  target,
  onClose,
  onDone,
}: {
  target: FirmwareTarget | 'new' | null
  onClose: () => void
  onDone: () => void
}) {
  const { data: images = [] } = useQuery({ queryKey: ['firmware-images'], queryFn: firmware.images, enabled: !!target })
  const { data: deviceList = [] } = useQuery({ queryKey: ['devices'], queryFn: devices.list, enabled: !!target })
  const existing = target && target !== 'new' ? target : null
  const [shownFor, setShownFor] = useState<string | null>(null)
  const [vendor, setVendor] = useState('yealink')
  const [pattern, setPattern] = useState('')
  const [imageId, setImageId] = useState('')
  const [pilot, setPilot] = useState<string[]>([])
  const key = target === 'new' ? 'new' : existing?.id ?? null
  if (target && shownFor !== key) {
    setShownFor(key)
    setVendor(existing?.vendor ?? 'yealink')
    setPattern(existing?.model_pattern ?? '')
    setImageId(existing?.image_id ?? '')
    setPilot(existing?.pilot_devices ?? [])
  }
  const save = useMutation({
    mutationFn: () => {
      const body = {
        vendor,
        model_pattern: pattern.trim() || undefined,
        image_id: imageId,
        stage: existing?.stage ?? ('pilot' as const),
        pilot_devices: pilot,
      }
      return existing ? firmware.updateTarget(existing.id, body) : firmware.createTarget(body)
    },
    onSuccess: () => {
      onDone()
      setShownFor(null)
      onClose()
    },
  })
  const vendorImages = images.filter((i) => i.vendor === vendor)
  const vendorPhones = deviceList.filter((d) => d.vendor === vendor)
  return (
    <Dialog open={!!target} onOpenChange={(o) => { if (!o) { setShownFor(null); onClose() } }}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{existing ? 'Edit target' : 'New target'}</DialogTitle>
          <DialogDescription>
            A new target starts with its pilot phones only; roll it out when they are happy.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <Label>Vendor</Label>
              <Select value={vendor} onValueChange={(v) => { setVendor(v); setImageId('') }}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>{VENDORS.map((v) => <SelectItem key={v} value={v}>{v}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label htmlFor="ft-pattern">Models</Label>
              <Input id="ft-pattern" value={pattern} placeholder="T5* — empty for any" onChange={(e) => setPattern(e.target.value)} />
            </div>
          </div>
          <div className="space-y-1">
            <Label>Image</Label>
            <Select value={imageId} onValueChange={setImageId}>
              <SelectTrigger><SelectValue placeholder="Choose an image" /></SelectTrigger>
              <SelectContent>
                {vendorImages.map((i) => (
                  <SelectItem key={i.id} value={i.id}>{i.version} ({i.file_name})</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>Pilot phones</Label>
            <div className="max-h-40 space-y-1 overflow-auto rounded-md border p-2">
              {vendorPhones.length === 0 && <p className="text-xs text-muted-foreground">No phones of this vendor.</p>}
              {vendorPhones.map((d) => (
                <label key={d.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={pilot.includes(d.id)}
                    onChange={(e) => setPilot(e.target.checked ? [...pilot, d.id] : pilot.filter((p) => p !== d.id))}
                  />
                  {d.name} <span className="text-xs text-muted-foreground">{d.model} {d.firmware_version}</span>
                </label>
              ))}
            </div>
          </div>
          {save.isError && <p className="text-sm text-destructive">{(save.error as Error).message}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>Cancel</Button>
            <Button disabled={save.isPending || !imageId} onClick={() => save.mutate()}>Save</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
