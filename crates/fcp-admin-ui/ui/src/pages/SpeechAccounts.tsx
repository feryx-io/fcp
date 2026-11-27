// A tenant's own TTS vendor accounts (docs/TENANT_SPEECH.md, S3): bring your
// own key. The key is written here and never shown again; an account is
// then a provider the tenant's speech can choose.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { KeyRound, Pencil, Play, Plus, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { speech } from '@/api/endpoints'
import type { SpeechAccount, SpeechAccountKind } from '@/types'

const KINDS: { value: SpeechAccountKind; label: string; key: string }[] = [
  { value: 'openai', label: 'OpenAI', key: 'API key' },
  { value: 'elevenlabs', label: 'ElevenLabs', key: 'API key' },
  { value: 'azure', label: 'Azure AI Speech', key: 'Subscription key' },
  { value: 'aws_polly', label: 'Amazon Polly', key: 'Secret access key' },
  { value: 'google_cloud', label: 'Google Cloud', key: 'API key' },
]
const kindLabel = (k: SpeechAccountKind) => KINDS.find((x) => x.value === k)?.label ?? k

interface Draft {
  id?: string
  name: string
  kind: SpeechAccountKind
  region: string
  model: string
  endpoint: string
  access_key_id: string
  secret: string
  has_key: boolean
}

const empty: Draft = {
  name: '', kind: 'openai', region: '', model: '', endpoint: '', access_key_id: '', secret: '', has_key: false,
}

const draftOf = (a: SpeechAccount): Draft => ({
  id: a.id,
  name: a.name,
  kind: a.kind,
  region: a.region ?? '',
  model: a.model ?? '',
  endpoint: a.endpoint ?? '',
  access_key_id: a.access_key_id ?? '',
  secret: '',
  has_key: a.has_key,
})

export function SpeechAccounts() {
  const queryClient = useQueryClient()
  const accounts = useQuery({ queryKey: ['speech', 'accounts'], queryFn: speech.accounts })
  const [draft, setDraft] = useState<Draft | null>(null)
  const [error, setError] = useState<string | null>(null)
  const changed = () => queryClient.invalidateQueries({ queryKey: ['speech'] })

  const save = useMutation({
    mutationFn: (d: Draft) => {
      const body = {
        name: d.name,
        kind: d.kind,
        region: d.region || null,
        model: d.model || null,
        endpoint: d.endpoint || null,
        access_key_id: d.access_key_id || null,
        secret: d.secret || null,
      }
      return d.id ? speech.updateAccount(d.id, body) : speech.createAccount(body)
    },
    onSuccess: () => { setDraft(null); setError(null); changed() },
    onError: (e) => setError((e as Error).message),
  })
  const remove = useMutation({
    mutationFn: (id: string) => speech.deleteAccount(id),
    onSuccess: changed,
  })
  const [tested, setTested] = useState<{ id: string; ok: boolean; message: string } | null>(null)
  const test = async (a: SpeechAccount) => {
    setTested(null)
    try {
      const url = await speech.testAccount(a.id)
      const audio = new Audio(url)
      audio.onended = () => URL.revokeObjectURL(url)
      setTested({ id: a.id, ok: true, message: `${a.name} spoke.` })
      await audio.play().catch(() => undefined)
    } catch (e) {
      setTested({ id: a.id, ok: false, message: `${a.name} could not speak: ${(e as Error).message}` })
    }
  }

  const kind = draft ? KINDS.find((k) => k.value === draft.kind) : undefined
  const set = (patch: Partial<Draft>) => setDraft((d) => (d ? { ...d, ...patch } : d))

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle>Your accounts</CardTitle>
          <CardDescription>
            Speak with your own account at a vendor. Its key is kept encrypted and never shown again.
          </CardDescription>
        </div>
        <Button variant="outline" size="sm" onClick={() => { setError(null); setDraft({ ...empty }) }}>
          <Plus className="mr-2 h-4 w-4" /> Add account
        </Button>
      </CardHeader>
      <CardContent>
        {(accounts.data ?? []).length === 0 ? (
          <p className="text-sm text-muted-foreground">None: the platform's providers speak for you.</p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Vendor</TableHead>
                  <TableHead>Region</TableHead>
                  <TableHead>Key</TableHead>
                  <TableHead className="w-32" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {(accounts.data ?? []).map((a) => (
                  <TableRow key={a.id}>
                    <TableCell className="font-medium">{a.name}</TableCell>
                    <TableCell>{kindLabel(a.kind)}</TableCell>
                    <TableCell>{a.region ?? '—'}</TableCell>
                    <TableCell>{a.has_key ? <KeyRound className="h-4 w-4" aria-label="Key kept" /> : '—'}</TableCell>
                    <TableCell className="text-right">
                      <Button variant="ghost" size="icon" aria-label={`Test ${a.name}`} onClick={() => test(a)}>
                        <Play className="h-4 w-4" />
                      </Button>
                      <Button variant="ghost" size="icon" aria-label={`Edit ${a.name}`}
                        onClick={() => { setError(null); setDraft(draftOf(a)) }}>
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button variant="ghost" size="icon" aria-label={`Delete ${a.name}`}
                        onClick={() => {
                          if (confirm(`Delete ${a.name}? Speech that chose it goes back to the platform's provider.`)) {
                            remove.mutate(a.id)
                          }
                        }}>
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
        {tested && (
          <p role="status" className={`mt-3 text-sm ${tested.ok ? 'text-muted-foreground' : 'text-destructive'}`}>
            {tested.message}
          </p>
        )}
      </CardContent>

      <Dialog open={draft !== null} onOpenChange={(open) => { if (!open) setDraft(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{draft?.id ? 'Edit account' : 'Add account'}</DialogTitle>
            <DialogDescription>Once saved, choose it as the provider above.</DialogDescription>
          </DialogHeader>
          {draft && (
            <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); save.mutate(draft) }}>
              <div className="space-y-1">
                <Label htmlFor="account-name">Name</Label>
                <Input id="account-name" value={draft.name} maxLength={100} required
                  onChange={(e) => set({ name: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="account-kind">Vendor</Label>
                <Select value={draft.kind} onValueChange={(v) => set({ kind: v as SpeechAccountKind })}>
                  <SelectTrigger id="account-kind" aria-label="Vendor"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {KINDS.map((k) => <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
              {(draft.kind === 'azure' || draft.kind === 'aws_polly') && (
                <div className="space-y-1">
                  <Label htmlFor="account-region">Region</Label>
                  <Input id="account-region" value={draft.region} required
                    placeholder={draft.kind === 'azure' ? 'eastus' : 'us-east-1'}
                    onChange={(e) => set({ region: e.target.value })} />
                </div>
              )}
              {draft.kind === 'aws_polly' && (
                <div className="space-y-1">
                  <Label htmlFor="account-key-id">Access key id</Label>
                  <Input id="account-key-id" value={draft.access_key_id} required
                    onChange={(e) => set({ access_key_id: e.target.value })} />
                </div>
              )}
              <div className="space-y-1">
                <Label htmlFor="account-secret">{kind?.key ?? 'Key'}</Label>
                <Input id="account-secret" type="password" autoComplete="off" value={draft.secret}
                  required={!draft.has_key}
                  placeholder={draft.has_key ? 'Kept — type a new one to replace it' : ''}
                  onChange={(e) => set({ secret: e.target.value })} />
              </div>
              {draft.kind !== 'google_cloud' && draft.kind !== 'azure' && (
                <div className="space-y-1">
                  <Label htmlFor="account-model">{draft.kind === 'aws_polly' ? 'Engine' : 'Model'} (optional)</Label>
                  <Input id="account-model" value={draft.model}
                    placeholder={draft.kind === 'openai' ? 'tts-1' : draft.kind === 'aws_polly' ? 'neural' : 'eleven_multilingual_v2'}
                    onChange={(e) => set({ model: e.target.value })} />
                </div>
              )}
              <div className="space-y-1">
                <Label htmlFor="account-endpoint">Endpoint (optional, https)</Label>
                <Input id="account-endpoint" value={draft.endpoint} placeholder="The vendor's own"
                  onChange={(e) => set({ endpoint: e.target.value })} />
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setDraft(null)}>Cancel</Button>
                <Button type="submit" disabled={save.isPending}>Save account</Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </Card>
  )
}

/** The platform administrator's choice of which of the TTS server's
 *  providers tenants may choose. */
export function PlatformOffer() {
  const queryClient = useQueryClient()
  const offer = useQuery({ queryKey: ['speech', 'platform'], queryFn: speech.platform })
  const [error, setError] = useState<string | null>(null)
  const save = useMutation({
    mutationFn: (held: string[]) => speech.savePlatform(held),
    onSuccess: () => { setError(null); queryClient.invalidateQueries({ queryKey: ['speech'] }) },
    onError: (e) => setError((e as Error).message),
  })
  const providers = offer.data?.providers ?? []
  const held = offer.data?.held_back ?? []
  return (
    <Card>
      <CardHeader>
        <CardTitle>Offered to tenants</CardTitle>
        <CardDescription>
          The platform's providers every tenant may choose. A held-back provider speaks for no tenant
          that chose it: the default does.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {providers.length === 0 && <p className="text-sm text-muted-foreground">No TTS service answers this admin.</p>}
        {providers.map((p) => (
          <label key={p.name} className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={p.offered} disabled={p.is_default || save.isPending}
              aria-label={`Offer ${p.name}`}
              onChange={(e) => save.mutate(e.target.checked ? held.filter((h) => h !== p.name) : [...held, p.name])} />
            <span className="font-medium">{p.name}</span>
            {p.is_default && <span className="text-muted-foreground">(the default, always offered)</span>}
          </label>
        ))}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>
    </Card>
  )
}

/** The tenant's speech by month and provider (docs/TENANT_SPEECH.md S4). */
export function SpeechUsageCard() {
  const usage = useQuery({ queryKey: ['speech', 'usage'], queryFn: speech.usage })
  const rows = [...(usage.data ?? [])].sort((a, b) => b.period.localeCompare(a.period) || a.label.localeCompare(b.label))
  const n = (v: number) => v.toLocaleString()
  return (
    <Card>
      <CardHeader>
        <CardTitle>Usage</CardTitle>
        <CardDescription>
          Characters spoken for you by month (UTC). A provider bills for what it spoke; the rest came from the cache.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {rows.length === 0 ? (
          <p className="text-sm text-muted-foreground">Nothing spoken in the last twelve months.</p>
        ) : (
          <div className="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Month</TableHead>
                  <TableHead>Provider</TableHead>
                  <TableHead className="text-right">Requests</TableHead>
                  <TableHead className="text-right">Characters</TableHead>
                  <TableHead className="text-right">Spoken by the provider</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((r) => (
                  <TableRow key={`${r.period}-${r.provider}`}>
                    <TableCell>{r.period}</TableCell>
                    <TableCell>{r.label}</TableCell>
                    <TableCell className="text-right tabular-nums">{n(r.requests)}</TableCell>
                    <TableCell className="text-right tabular-nums">{n(r.characters)}</TableCell>
                    <TableCell className="text-right tabular-nums">{n(r.synthesized_characters)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
