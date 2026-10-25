import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download, Music, Plus, Save, Trash2, Upload } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { branding } from '@/api/endpoints'
import type { Brand, BrandAsset, BrandColors, BrandRequest } from '@/types'

const SLOTS: { key: keyof BrandColors; label: string }[] = [
  { key: 'primary', label: 'Primary' },
  { key: 'accent', label: 'Accent' },
  { key: 'background', label: 'Background' },
  { key: 'surface', label: 'Surface' },
  { key: 'on_primary', label: 'On primary' },
  { key: 'on_surface', label: 'On surface' },
  { key: 'error', label: 'Error' },
]

const LINKS = ['support', 'privacy', 'terms', 'website'] as const

interface Ringtone {
  name: string
  default: boolean
  sha256: string
}

interface Form {
  app_name: string
  colors: BrandColors
  dark: boolean
  dark_colors: BrandColors
  logo: string
  logo_dark: string
  icon: string
  ringtones: Ringtone[]
  links: Record<(typeof LINKS)[number], string>
  prefix_strip: string
  short_codes: string
}

const blank: Form = {
  app_name: '',
  colors: {},
  dark: false,
  dark_colors: {},
  logo: '',
  logo_dark: '',
  icon: '',
  ringtones: [],
  links: { support: '', privacy: '', terms: '', website: '' },
  prefix_strip: '',
  short_codes: '',
}

function formOf(b: Brand): Form {
  return {
    app_name: b.app_name,
    colors: b.colors,
    dark: b.dark_colors != null,
    dark_colors: b.dark_colors ?? {},
    logo: b.logo ?? '',
    logo_dark: b.logo_dark ?? '',
    icon: b.icon ?? '',
    ringtones: b.ringtones.map((r) => ({ name: r.name, default: r.default ?? false, sha256: r.sha256 })),
    links: {
      support: b.links.support ?? '',
      privacy: b.links.privacy ?? '',
      terms: b.links.terms ?? '',
      website: b.links.website ?? '',
    },
    prefix_strip: (b.dial_plan.prefix_strip ?? []).join(', '),
    short_codes: Object.entries(b.dial_plan.short_codes ?? {})
      .map(([code, uri]) => `${code} ${uri}`)
      .join('\n'),
  }
}

/** Colors with the empty slots left out: the app's own. */
function colorsOf(c: BrandColors): BrandColors {
  return Object.fromEntries(Object.entries(c).filter(([, v]) => v && String(v).trim())) as BrandColors
}

function requestOf(f: Form): BrandRequest {
  const short_codes: Record<string, string> = {}
  for (const line of f.short_codes.split('\n').map((l) => l.trim()).filter(Boolean)) {
    const [code, uri] = line.split(/\s+/, 2)
    if (!uri) throw new Error(`"${line}": write the code, a space, then the address`)
    short_codes[code] = uri
  }
  const links = Object.fromEntries(Object.entries(f.links).filter(([, v]) => v.trim()))
  return {
    app_name: f.app_name,
    colors: colorsOf(f.colors),
    dark_colors: f.dark ? colorsOf(f.dark_colors) : null,
    logo: f.logo || null,
    logo_dark: f.logo_dark || null,
    icon: f.icon || null,
    ringtones: f.ringtones,
    links,
    dial_plan: {
      prefix_strip: f.prefix_strip.split(',').map((p) => p.trim()).filter(Boolean),
      short_codes,
    },
  }
}

/** Where the admin UI shows an asset: the public route, on this origin. */
function assetSrc(sha256: string) {
  return `/brand/v1/assets/${sha256}`
}

function ColorField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const valid = /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/.test(value)
  return (
    <div className="flex items-center gap-2">
      <input
        type="color"
        aria-label={label}
        className="h-9 w-9 cursor-pointer rounded border bg-transparent p-0.5"
        value={valid ? value.slice(0, 7) : '#000000'}
        onChange={(e) => onChange(e.target.value.toUpperCase())}
      />
      <div className="flex-1">
        <Label className="text-xs text-muted-foreground">{label}</Label>
        <Input
          value={value}
          placeholder="the app's own"
          className={`h-8 font-mono text-xs ${value && !valid ? 'border-destructive' : ''}`}
          onChange={(e) => onChange(e.target.value.trim())}
        />
      </div>
    </div>
  )
}

function AssetPicker({
  label, hint, value, assets, accept, onChange, onUpload,
}: {
  label: string
  hint: string
  value: string
  assets: BrandAsset[]
  accept: string
  onChange: (sha256: string) => void
  onUpload: (file: File) => Promise<BrandAsset>
}) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <div className="flex h-20 items-center justify-center rounded border bg-muted/40">
        {value ? (
          <img src={assetSrc(value)} alt={label} className="max-h-16 max-w-full object-contain" />
        ) : (
          <span className="text-xs text-muted-foreground">{hint}</span>
        )}
      </div>
      <div className="flex gap-2">
        <select
          className="h-8 flex-1 rounded-md border bg-background px-2 text-xs"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">None</option>
          {assets.map((a) => (
            <option key={a.sha256} value={a.sha256}>
              {a.file_name ?? a.sha256.slice(0, 12)}{a.width ? ` (${a.width}×${a.height})` : ''}
            </option>
          ))}
        </select>
        <Button size="sm" variant="outline" type="button" title={`Upload the ${label.toLowerCase()}`} onClick={() => input.current?.click()}>
          <Upload className="h-4 w-4" />
        </Button>
        <input
          ref={input}
          type="file"
          accept={accept}
          className="hidden"
          onChange={async (e) => {
            const file = e.target.files?.[0]
            e.target.value = ''
            if (!file) return
            try {
              onChange((await onUpload(file)).sha256)
            } catch (err) {
              alert((err as Error).message)
            }
          }}
        />
      </div>
    </div>
  )
}

/** Branding (docs/BRANDING.md): the name, colors, logos, ringtones and
 * links the tenant's softphones take on when its people sign in. */
export function BrandingPage() {
  const queryClient = useQueryClient()
  const { data: brand, isLoading } = useQuery({ queryKey: ['branding'], queryFn: branding.get })
  const { data: assets = [] } = useQuery({ queryKey: ['branding-assets'], queryFn: branding.assets })
  const [form, setForm] = useState<Form>(blank)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const audioInput = useRef<HTMLInputElement>(null)

  useEffect(() => {
    setForm(brand ? formOf(brand) : blank)
  }, [brand])

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ['branding'] })
    queryClient.invalidateQueries({ queryKey: ['branding-assets'] })
  }
  const upload = async (file: File) => {
    const asset = await branding.upload(file)
    queryClient.invalidateQueries({ queryKey: ['branding-assets'] })
    return asset
  }
  const save = useMutation({
    mutationFn: () => branding.put(requestOf(form)),
    onSuccess: () => {
      setError(null)
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
      refresh()
    },
    onError: (e) => setError((e as Error).message),
  })
  const remove = useMutation({ mutationFn: branding.remove, onSuccess: refresh })
  const removeAsset = useMutation({
    mutationFn: branding.deleteAsset,
    onSuccess: refresh,
    onError: (e) => alert((e as Error).message),
  })

  const images = assets.filter((a) => a.kind === 'image')
  const rasters = images.filter((a) => a.mime !== 'image/svg+xml')
  const audio = assets.filter((a) => a.kind === 'audio')
  const set = (patch: Partial<Form>) => setForm((f) => ({ ...f, ...patch }))
  const setRingtone = (i: number, patch: Partial<Ringtone>) =>
    set({
      ringtones: form.ringtones.map((r, j) =>
        j === i ? { ...r, ...patch } : patch.default ? { ...r, default: false } : r,
      ),
    })

  if (isLoading) return null
  const primary = form.colors.primary || '#1A73E8'

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Branding</h1>
          <p className="text-muted-foreground">
            What the tenant's softphone looks like once its people sign in. Unset, the app shows its own.
          </p>
        </div>
        <div className="flex gap-2">
          {brand && (
            <Button
              variant="outline"
              onClick={() => { if (confirm('Remove the brand? The apps show their own again.')) remove.mutate() }}
            >
              <Trash2 className="mr-2 h-4 w-4" /> Remove
            </Button>
          )}
          <Button onClick={() => save.mutate()} disabled={save.isPending || !form.app_name.trim()}>
            <Save className="mr-2 h-4 w-4" /> {saved ? 'Saved' : 'Save'}
          </Button>
        </div>
      </div>
      {error && <p className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{error}</p>}
      {brand?.brand_url && (
        <p className="text-sm text-muted-foreground">
          Served to the apps at <span className="font-mono">{brand.brand_url}</span>
        </p>
      )}

      <div className="grid gap-6 lg:grid-cols-[1fr_280px]">
        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>Name and links</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-1">
                <Label htmlFor="app-name">App name</Label>
                <Input id="app-name" value={form.app_name} maxLength={64} placeholder="Acme Voice"
                  onChange={(e) => set({ app_name: e.target.value })} />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                {LINKS.map((l) => (
                  <div key={l} className="space-y-1">
                    <Label className="capitalize">{l}</Label>
                    <Input value={form.links[l]} placeholder="https://"
                      onChange={(e) => set({ links: { ...form.links, [l]: e.target.value } })} />
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Colors</CardTitle>
              <CardDescription>#RRGGBB or #RRGGBBAA; an empty slot is the app's own.</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-6 sm:grid-cols-2">
              <div className="space-y-3">
                <p className="text-sm font-medium">Light</p>
                {SLOTS.map((s) => (
                  <ColorField key={s.key} label={s.label} value={form.colors[s.key] ?? ''}
                    onChange={(v) => set({ colors: { ...form.colors, [s.key]: v } })} />
                ))}
              </div>
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <p className="text-sm font-medium">Dark</p>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    Its own colors
                    <Switch aria-label="Dark colors of its own" checked={form.dark} onCheckedChange={(dark) => set({ dark })} />
                  </div>
                </div>
                {form.dark ? (
                  SLOTS.map((s) => (
                    <ColorField key={s.key} label={s.label} value={form.dark_colors[s.key] ?? ''}
                      onChange={(v) => set({ dark_colors: { ...form.dark_colors, [s.key]: v } })} />
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">The app derives the dark theme itself.</p>
                )}
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Images</CardTitle>
              <CardDescription>PNG, JPEG, WebP or SVG, at most 5 MB. The icon is a raster image, 512×512 at best.</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-4 sm:grid-cols-3">
              <AssetPicker label="Logo" hint="No logo" value={form.logo} assets={images}
                accept="image/png,image/jpeg,image/webp,image/svg+xml" onChange={(logo) => set({ logo })} onUpload={upload} />
              <AssetPicker label="Logo on dark" hint="The logo" value={form.logo_dark} assets={images}
                accept="image/png,image/jpeg,image/webp,image/svg+xml" onChange={(logo_dark) => set({ logo_dark })} onUpload={upload} />
              <AssetPicker label="Icon" hint="No icon" value={form.icon} assets={rasters}
                accept="image/png,image/jpeg,image/webp" onChange={(icon) => set({ icon })} onUpload={upload} />
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="flex flex-row items-center justify-between space-y-0">
              <div>
                <CardTitle>Ringtones</CardTitle>
                <CardDescription>Ogg, MP3 or WAV, at most 5 MB each and eight in all.</CardDescription>
              </div>
              <Button size="sm" variant="outline" onClick={() => audioInput.current?.click()}
                disabled={form.ringtones.length >= 8}>
                <Plus className="mr-2 h-4 w-4" /> Add
              </Button>
              <input ref={audioInput} type="file" accept="audio/ogg,audio/mpeg,audio/wav" className="hidden"
                onChange={async (e) => {
                  const file = e.target.files?.[0]
                  e.target.value = ''
                  if (!file) return
                  try {
                    const asset = await upload(file)
                    set({
                      ringtones: [...form.ringtones, {
                        name: file.name.replace(/\.[^.]+$/, '').slice(0, 64),
                        default: form.ringtones.length === 0,
                        sha256: asset.sha256,
                      }],
                    })
                  } catch (err) {
                    alert((err as Error).message)
                  }
                }} />
            </CardHeader>
            <CardContent className="space-y-2">
              {form.ringtones.length === 0 && (
                <p className="text-sm text-muted-foreground">None: the app's own ringtones.</p>
              )}
              {form.ringtones.map((r, i) => (
                <div key={i} className="flex items-center gap-2">
                  <Music className="h-4 w-4 text-muted-foreground" />
                  <Input className="h-8 flex-1" value={r.name} maxLength={64}
                    onChange={(e) => setRingtone(i, { name: e.target.value })} />
                  <select className="h-8 w-40 rounded-md border bg-background px-2 text-xs" value={r.sha256}
                    onChange={(e) => setRingtone(i, { sha256: e.target.value })}>
                    {audio.map((a) => (
                      <option key={a.sha256} value={a.sha256}>{a.file_name ?? a.sha256.slice(0, 12)}</option>
                    ))}
                  </select>
                  <audio controls preload="none" src={assetSrc(r.sha256)} className="h-8 w-40" />
                  <label className="flex items-center gap-1 text-xs">
                    <input type="radio" name="default-ringtone" checked={r.default}
                      onChange={() => setRingtone(i, { default: true })} />
                    Default
                  </label>
                  <Button size="sm" variant="ghost" aria-label="Remove"
                    onClick={() => set({ ringtones: form.ringtones.filter((_, j) => j !== i) })}>
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Dial plan hints</CardTitle>
              <CardDescription>What the app's dialer does before it calls; the call manager still routes the call.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-1">
                <Label>Prefixes taken off, comma-separated</Label>
                <Input value={form.prefix_strip} placeholder="+1" className="font-mono"
                  onChange={(e) => set({ prefix_strip: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label>Short codes: the code, a space, the address; one per line</Label>
                <textarea
                  className="min-h-24 w-full rounded-md border bg-background p-2 font-mono text-xs"
                  value={form.short_codes}
                  placeholder="1000 sip:frontdesk@acme.example"
                  onChange={(e) => set({ short_codes: e.target.value })}
                />
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="space-y-6">
          <Card className="sticky top-4">
            <CardHeader>
              <CardTitle>Preview</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="overflow-hidden rounded-2xl border shadow-sm"
                style={{ background: form.colors.background || '#FFFFFF', color: form.colors.on_surface || '#202124' }}>
                <div className="flex items-center gap-2 p-3" style={{ background: primary, color: form.colors.on_primary || '#FFFFFF' }}>
                  {form.icon && <img src={assetSrc(form.icon)} alt="" className="h-6 w-6 rounded" />}
                  <span className="text-sm font-semibold">{form.app_name || 'App name'}</span>
                </div>
                <div className="space-y-3 p-4">
                  <div className="flex h-12 items-center justify-center">
                    {form.logo ? (
                      <img src={assetSrc(form.logo)} alt="" className="max-h-12 max-w-full object-contain" />
                    ) : (
                      <span className="text-xs opacity-60">No logo</span>
                    )}
                  </div>
                  <div className="rounded-lg p-3 text-xs" style={{ background: form.colors.surface || '#F5F5F5' }}>
                    sip:alice@acme.example
                  </div>
                  <button type="button" className="w-full rounded-full py-2 text-sm font-medium"
                    style={{ background: form.colors.accent || primary, color: form.colors.on_primary || '#FFFFFF' }}>
                    Call
                  </button>
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Uploaded files</CardTitle>
          <CardDescription>Public by their hash. One the brand names cannot be deleted.</CardDescription>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>File</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Size</TableHead>
                <TableHead>SHA-256</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {assets.length === 0 && (
                <TableRow><TableCell colSpan={5} className="text-muted-foreground">Nothing uploaded.</TableCell></TableRow>
              )}
              {assets.map((a) => (
                <TableRow key={a.sha256}>
                  <TableCell className="flex items-center gap-2">
                    {a.kind === 'image' ? (
                      <img src={assetSrc(a.sha256)} alt="" className="h-8 w-8 rounded object-contain" />
                    ) : (
                      <Music className="h-4 w-4 text-muted-foreground" />
                    )}
                    {a.file_name ?? '-'}
                  </TableCell>
                  <TableCell className="text-xs">{a.mime}{a.width ? ` ${a.width}×${a.height}` : ''}</TableCell>
                  <TableCell className="text-xs">{(a.size / 1024).toFixed(1)} KB</TableCell>
                  <TableCell className="font-mono text-xs">{a.sha256.slice(0, 16)}…</TableCell>
                  <TableCell className="text-right">
                    <Button size="sm" variant="ghost" aria-label="Delete"
                      onClick={() => { if (confirm('Delete this file?')) removeAsset.mutate(a.sha256) }}>
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <DeploymentCard />
    </div>
  )
}

/** `acme-voice`, as the server names the downloads. */
function fileSlug(name: string): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '')
  return slug || 'anvil'
}

/**
 * The softphone deployed with nobody typing anything (Anvil's
 * MANAGED_INSTALLS.md): the profile for its Windows setup, or the same
 * settings as policy.
 */
function DeploymentCard() {
  const [signIn, setSignIn] = useState('')
  const [updates, setUpdates] = useState('auto')
  const [lockServer, setLockServer] = useState(true)
  const [fromSystem, setFromSystem] = useState('')
  const query =
    (signIn ? `&sign_in=${signIn}` : '') +
    `&updates=${updates}&lock_server=${lockServer}` +
    (fromSystem ? `&username_from_system=${fromSystem}` : '')
  const profile = useQuery({
    queryKey: ['branding-deployment', query],
    queryFn: () => branding.deploymentProfile(query),
  })
  const brand = profile.data?.brand as { app_name?: string } | undefined
  const slug = fileSlug(brand?.app_name ?? 'anvil')
  const download = async (format: 'zip' | 'reg') => {
    try {
      const url = await branding.deploymentFile(format, query)
      const a = document.createElement('a')
      a.href = url
      a.download = format === 'zip' ? `${slug}-profile.zip` : `${slug}-policy.reg`
      a.click()
      URL.revokeObjectURL(url)
    } catch (err) {
      alert((err as Error).message)
    }
  }
  const select = 'h-9 w-full rounded-md border bg-background px-2 text-sm'
  return (
    <Card>
      <CardHeader>
        <CardTitle>Softphone deployment</CardTitle>
        <CardDescription>
          Install the softphone with this server, sign-in and brand already set. Give the profile to its
          Windows setup, or set the same settings as policy.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-4 md:grid-cols-4">
          <div className="space-y-1">
            <Label>Sign-in</Label>
            <select className={select} value={signIn} onChange={(e) => setSignIn(e.target.value)}>
              <option value="">The tenant's (asked at sign-in)</option>
              <option value="sso">Single sign-on</option>
              <option value="password">Password</option>
              <option value="auto">Ask the server</option>
            </select>
          </div>
          <div className="space-y-1">
            <Label>Updates</Label>
            <select className={select} value={updates} onChange={(e) => setUpdates(e.target.value)}>
              <option value="auto">Installed by the app</option>
              <option value="notify">Offered, installed by IT</option>
              <option value="off">Never looked for</option>
            </select>
          </div>
          <div className="space-y-1">
            <Label>Username from Windows</Label>
            <select className={select} value={fromSystem} onChange={(e) => setFromSystem(e.target.value)}>
              <option value="">When the tenant has a login domain</option>
              <option value="true">Yes</option>
              <option value="false">No</option>
            </select>
          </div>
          <div className="flex items-center gap-2 pt-6">
            <Switch checked={lockServer} onCheckedChange={setLockServer} id="lock-server" />
            <Label htmlFor="lock-server">Server locked</Label>
          </div>
        </div>
        {profile.error && <p className="text-sm text-destructive">{(profile.error as Error).message}</p>}
        {profile.data && (
          <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 font-mono text-xs">
            {JSON.stringify(profile.data, null, 2)}
          </pre>
        )}
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => download('zip')} disabled={!profile.data}>
            <Download className="mr-2 h-4 w-4" /> Profile (.zip)
          </Button>
          <Button variant="outline" onClick={() => download('reg')} disabled={!profile.data}>
            <Download className="mr-2 h-4 w-4" /> Windows policy (.reg)
          </Button>
        </div>
        <div className="space-y-1">
          <Label>Intune or a script</Label>
          <pre className="overflow-auto rounded-md bg-muted p-3 font-mono text-xs">
            {`Anvil-<version>-windows-x86_64-setup.exe /VERYSILENT /PROFILE=${slug}-profile.zip`}
          </pre>
          <p className="text-xs text-muted-foreground">
            The setup keeps the profile when the app updates itself. Policy (the .reg, or Group Policy writing the
            same values under HKLM\Software\Policies\Anvil) goes over the profile but carries no brand.
          </p>
        </div>
      </CardContent>
    </Card>
  )
}
