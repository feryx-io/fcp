// Speech spending caps (docs/SPEECH_CAPS.md; minutes, docs/TENANT_RECOGNITION.md
// R3): characters a voice may synthesize, or minutes a recognizer may hear,
// for the tenant each month. The tenant caps its own accounts and the
// platform's providers; the platform's cap on the tenant is shown, and a
// platform administrator sets or lifts it here for the tenant picked. At a
// cap the default voice or recognizer steps in until the month turns.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Plus, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { speech } from '@/api/endpoints'
import type { CapUnit, SpeechCap } from '@/types'

const n = (v: number) => v.toLocaleString()
const month = (iso: string) =>
  new Date(iso).toLocaleDateString(undefined, { day: 'numeric', month: 'long', timeZone: 'UTC' })
/** A whole number, or null for an empty or bad field. */
const count = (s: string): number | null => {
  const v = Number(s.replace(/[,\s]/g, ''))
  return s.trim() !== '' && Number.isInteger(v) && v >= 0 ? v : null
}
const unitWord = (u: CapUnit) => (u === 'minutes' ? 'minutes heard' : 'characters')
/** The testid of a cap's row: characters keep the provider's alone. */
const rowId = (provider: string, unit: CapUnit) => (unit === 'minutes' ? `cap-${provider}-minutes` : `cap-${provider}`)

function CapRow({ cap, platformAdmin, onError }: {
  cap: SpeechCap; platformAdmin: boolean; onError: (m: string | null) => void
}) {
  const queryClient = useQueryClient()
  const [own, setOwn] = useState(cap.own_cap != null ? String(cap.own_cap) : '')
  const [platform, setPlatform] = useState(cap.platform_cap != null ? String(cap.platform_cap) : '')
  const done = () => { onError(null); queryClient.invalidateQueries({ queryKey: ['speech', 'caps'] }) }
  const fail = (e: unknown) => onError((e as Error).message)
  const saveOwn = useMutation({
    mutationFn: () => {
      const v = count(own)
      return v == null ? speech.removeCap(cap.provider, false, cap.unit) : speech.saveCap(cap.provider, v, false, cap.unit)
    },
    onSuccess: done, onError: fail,
  })
  const savePlatform = useMutation({
    mutationFn: (v: number | null | 'default') =>
      v === 'default' ? speech.removeCap(cap.provider, true, cap.unit) : speech.saveCap(cap.provider, v, true, cap.unit),
    onSuccess: done, onError: fail,
  })
  const account = cap.provider.startsWith('account:')
  const pct = cap.cap ? Math.min(100, Math.round((cap.used / cap.cap) * 100)) : 0
  const what = unitWord(cap.unit)
  const stepsIn = cap.unit === 'minutes' ? 'The default recognizer hears' : 'The default provider speaks'
  return (
    <div className="space-y-2 rounded-md border p-3" data-testid={rowId(cap.provider, cap.unit)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{cap.label} <span className="font-normal text-muted-foreground">· {what}</span></span>
        {cap.state === 'reached' && <Badge variant="destructive">Cap reached</Badge>}
        {cap.state === 'near' && <Badge variant="secondary">Near its cap</Badge>}
      </div>
      <div className="text-sm text-muted-foreground">
        {cap.cap != null ? `${n(cap.used)} of ${n(cap.cap)} ${what} this month` : `${n(cap.used)} ${what} this month, no cap`}
        {' · '}counts again on {month(cap.resets_at)}
      </div>
      {cap.cap != null && (
        <div className="h-2 w-full overflow-hidden rounded bg-muted" role="progressbar" aria-label={`${cap.label} this month`}
          aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
          <div className={`h-full ${cap.state === 'reached' ? 'bg-destructive' : cap.state === 'near' ? 'bg-amber-500' : 'bg-primary'}`}
            style={{ width: `${pct}%` }} />
        </div>
      )}
      {cap.state === 'reached' && (
        <p className="text-sm text-destructive">
          {stepsIn} instead of {cap.label} until the month turns or the cap is raised.
        </p>
      )}
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor={`own-${cap.provider}-${cap.unit}`}>Your cap</Label>
          <div className="flex gap-2">
            <Input id={`own-${cap.provider}-${cap.unit}`} inputMode="numeric" placeholder="None" value={own}
              onChange={(e) => setOwn(e.target.value)} />
            <Button variant="outline" size="sm" disabled={saveOwn.isPending || (own.trim() !== '' && count(own) == null)}
              onClick={() => saveOwn.mutate()} aria-label={`Save your cap on ${cap.label}`}>Save</Button>
          </div>
        </div>
        {!account && (
          <div className="space-y-1">
            <Label htmlFor={`platform-${cap.provider}-${cap.unit}`}>The platform's cap</Label>
            {platformAdmin ? (
              <div className="flex flex-wrap gap-2">
                <Input id={`platform-${cap.provider}-${cap.unit}`} inputMode="numeric" placeholder="None for this tenant" value={platform}
                  onChange={(e) => setPlatform(e.target.value)} className="min-w-0 flex-1" />
                <Button variant="outline" size="sm" disabled={savePlatform.isPending}
                  aria-label={`Save the platform's cap on ${cap.label}`}
                  onClick={() => savePlatform.mutate(count(platform))}>Save</Button>
                {cap.platform_override && (
                  <Button variant="ghost" size="sm" disabled={savePlatform.isPending}
                    onClick={() => savePlatform.mutate('default')}>Use the default</Button>
                )}
              </div>
            ) : (
              <p id={`platform-${cap.provider}-${cap.unit}`} className="text-sm">
                {cap.platform_cap != null ? `${n(cap.platform_cap)} ${what}` : 'None'}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

/** What may be capped: a voice's characters, a recognizer's minutes. */
interface Choice { key: string; provider: string; unit: CapUnit; label: string }

export function SpeechCapsCard({ platformAdmin }: { platformAdmin: boolean }) {
  const queryClient = useQueryClient()
  const caps = useQuery({ queryKey: ['speech', 'caps'], queryFn: speech.caps })
  const providers = useQuery({ queryKey: ['speech', 'providers'], queryFn: speech.providers })
  // The tenant's accounts too: listed even where no TTS server answers.
  const accounts = useQuery({ queryKey: ['speech', 'accounts'], queryFn: speech.accounts })
  const [error, setError] = useState<string | null>(null)
  const [adding, setAdding] = useState<{ key: string; value: string } | null>(null)
  const add = useMutation({
    mutationFn: ({ c, value }: { c: Choice; value: number }) => speech.saveCap(c.provider, value, false, c.unit),
    onSuccess: () => { setAdding(null); setError(null); queryClient.invalidateQueries({ queryKey: ['speech', 'caps'] }) },
    onError: (e) => setError((e as Error).message),
  })
  const rows = caps.data ?? []
  const choices: Choice[] = []
  const offer = (provider: string, label: string, unit: CapUnit) => {
    const key = `${provider}|${unit}`
    if (!choices.some((c) => c.key === key) && !rows.some((r) => r.provider === provider && r.unit === unit)) {
      choices.push({ key, provider, unit, label: `${label} (${unitWord(unit)})` })
    }
  }
  // Never the defaults, which step in at a cap.
  for (const p of providers.data ?? []) {
    if (p.speaks && !p.is_default) offer(p.name, p.label, 'characters')
    if (p.hears && !p.is_default_recognizer) offer(p.name, p.label, 'minutes')
  }
  for (const a of accounts.data ?? []) {
    if (a.kind !== 'deepgram') offer(a.provider, a.name, 'characters')
    if (a.kind === 'openai' || a.kind === 'deepgram') offer(a.provider, a.name, 'minutes')
  }
  const chosen = adding ? choices.find((c) => c.key === adding.key) : undefined
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle>Caps</CardTitle>
          <CardDescription>
            Characters a voice may speak, or minutes a recognizer may hear, for you each month (UTC); the
            cache is free. At a cap the default steps in.
          </CardDescription>
        </div>
        {choices.length > 0 && !adding && (
          <Button variant="outline" size="sm" onClick={() => setAdding({ key: choices[0].key, value: '' })}>
            <Plus className="mr-2 h-4 w-4" /> Add a cap
          </Button>
        )}
      </CardHeader>
      <CardContent className="space-y-3">
        {rows.length === 0 && !adding && <p className="text-sm text-muted-foreground">No caps: speech is not limited.</p>}
        {rows.map((c) => <CapRow key={`${c.provider}-${c.unit}-${c.own_cap}-${c.platform_cap}`} cap={c} platformAdmin={platformAdmin} onError={setError} />)}
        {adding && (
          <form className="flex flex-col gap-2 rounded-md border p-3 sm:flex-row sm:items-end"
            onSubmit={(e) => {
              e.preventDefault()
              const v = count(adding.value)
              if (v != null && chosen) add.mutate({ c: chosen, value: v })
            }}>
            <div className="min-w-0 flex-1 space-y-1">
              <Label htmlFor="cap-provider">Provider</Label>
              <Select value={adding.key} onValueChange={(key) => setAdding({ ...adding, key })}>
                <SelectTrigger id="cap-provider"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {choices.map((c) => <SelectItem key={c.key} value={c.key}>{c.label}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-0 flex-1 space-y-1">
              <Label htmlFor="cap-value">{chosen?.unit === 'minutes' ? 'Minutes a month' : 'Characters a month'}</Label>
              <Input id="cap-value" inputMode="numeric" value={adding.value}
                onChange={(e) => setAdding({ ...adding, value: e.target.value })} />
            </div>
            <div className="flex gap-2">
              <Button type="submit" size="sm" disabled={add.isPending || count(adding.value) == null}>Save cap</Button>
              <Button type="button" variant="ghost" size="icon" aria-label="Cancel" onClick={() => setAdding(null)}>
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </form>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>
    </Card>
  )
}

/** The platform's default caps and prices per provider, for every tenant:
 *  a voice's characters, a recognizer's minutes. */
export function PlatformCaps({ providers, prices, minutePrices, onPrices }: {
  providers: { name: string; is_default: boolean; speaks: boolean; hears: boolean; is_default_recognizer: boolean }[]
  prices: Record<string, number>
  minutePrices: Record<string, number>
  onPrices: (prices: Record<string, number>, minutePrices: Record<string, number>) => void
}) {
  const queryClient = useQueryClient()
  const caps = useQuery({ queryKey: ['speech', 'platform-caps'], queryFn: speech.platformCaps })
  const [error, setError] = useState<string | null>(null)
  const save = useMutation({
    mutationFn: ({ provider, unit, value }: { provider: string; unit: CapUnit; value: number | null }) =>
      value == null ? speech.removePlatformCap(provider, unit) : speech.savePlatformCap(provider, value, unit),
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['speech'] })
    },
    onError: (e) => setError((e as Error).message),
  })
  const capOf = (p: string, unit: CapUnit) => (caps.data ?? []).find((c) => c.provider === p && c.unit === unit)?.monthly
  const rows: { name: string; unit: CapUnit }[] = []
  for (const p of providers) {
    if (p.speaks && !p.is_default) rows.push({ name: p.name, unit: 'characters' })
    if (p.hears && !p.is_default_recognizer) rows.push({ name: p.name, unit: 'minutes' })
  }
  return (
    <div className="space-y-2">
      {rows.map(({ name, unit }) => {
        const table = unit === 'minutes' ? minutePrices : prices
        const id = `${name}-${unit}`
        return (
          <form key={`${id}-${capOf(name, unit)}-${table[name]}`} className="grid gap-2 sm:grid-cols-[1fr_1fr_1fr_auto] sm:items-end"
            onSubmit={(e) => {
              e.preventDefault()
              const form = new FormData(e.currentTarget)
              const cap = count(String(form.get('cap') ?? ''))
              const price = String(form.get('price') ?? '').trim()
              save.mutate({ provider: name, unit, value: cap })
              const next = { ...table }
              if (price === '') delete next[name]
              else next[name] = Number(price)
              if (next[name] !== table[name]) {
                if (unit === 'minutes') onPrices(prices, next)
                else onPrices(next, minutePrices)
              }
            }}>
            <span className="text-sm font-medium sm:self-center">{name} <span className="font-normal text-muted-foreground">· {unitWord(unit)}</span></span>
            <div className="space-y-1">
              <Label htmlFor={`default-cap-${id}`}>{unit === 'minutes' ? 'Default minutes a month' : 'Default cap a month'}</Label>
              <Input id={`default-cap-${id}`} name="cap" inputMode="numeric" placeholder="None"
                defaultValue={capOf(name, unit) != null ? String(capOf(name, unit)) : ''} />
            </div>
            <div className="space-y-1">
              <Label htmlFor={`price-${id}`}>{unit === 'minutes' ? 'Price per minute' : 'Price per million'}</Label>
              <Input id={`price-${id}`} name="price" inputMode="decimal" placeholder="None"
                defaultValue={table[name] != null ? String(table[name]) : ''} />
            </div>
            <Button type="submit" variant="outline" size="sm" disabled={save.isPending}
              aria-label={`Save ${name}'s default ${unitWord(unit)} cap and price`}>Save</Button>
          </form>
        )
      })}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}
