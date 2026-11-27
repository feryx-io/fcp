// Speech spending caps (docs/SPEECH_CAPS.md): characters a provider may
// synthesize for the tenant each month. The tenant caps its own accounts
// and the platform's providers; the platform's cap on the tenant is shown,
// and a platform administrator sets or lifts it here for the tenant picked.
// At a cap the default provider speaks instead until the month turns.
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
import type { SpeechCap } from '@/types'

const n = (v: number) => v.toLocaleString()
const month = (iso: string) =>
  new Date(iso).toLocaleDateString(undefined, { day: 'numeric', month: 'long', timeZone: 'UTC' })
/** A whole number of characters, or null for an empty or bad field. */
const count = (s: string): number | null => {
  const v = Number(s.replace(/[,\s]/g, ''))
  return s.trim() !== '' && Number.isInteger(v) && v >= 0 ? v : null
}

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
      return v == null ? speech.removeCap(cap.provider) : speech.saveCap(cap.provider, v)
    },
    onSuccess: done, onError: fail,
  })
  const savePlatform = useMutation({
    mutationFn: (v: number | null | 'default') =>
      v === 'default' ? speech.removeCap(cap.provider, true) : speech.saveCap(cap.provider, v, true),
    onSuccess: done, onError: fail,
  })
  const account = cap.provider.startsWith('account:')
  const pct = cap.cap ? Math.min(100, Math.round((cap.used / cap.cap) * 100)) : 0
  return (
    <div className="space-y-2 rounded-md border p-3" data-testid={`cap-${cap.provider}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{cap.label}</span>
        {cap.state === 'reached' && <Badge variant="destructive">Cap reached</Badge>}
        {cap.state === 'near' && <Badge variant="secondary">Near its cap</Badge>}
      </div>
      <div className="text-sm text-muted-foreground">
        {cap.cap != null ? `${n(cap.used)} of ${n(cap.cap)} characters this month` : `${n(cap.used)} characters this month, no cap`}
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
          The default provider speaks instead of {cap.label} until the month turns or the cap is raised.
        </p>
      )}
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor={`own-${cap.provider}`}>Your cap</Label>
          <div className="flex gap-2">
            <Input id={`own-${cap.provider}`} inputMode="numeric" placeholder="None" value={own}
              onChange={(e) => setOwn(e.target.value)} />
            <Button variant="outline" size="sm" disabled={saveOwn.isPending || (own.trim() !== '' && count(own) == null)}
              onClick={() => saveOwn.mutate()} aria-label={`Save your cap on ${cap.label}`}>Save</Button>
          </div>
        </div>
        {!account && (
          <div className="space-y-1">
            <Label htmlFor={`platform-${cap.provider}`}>The platform's cap</Label>
            {platformAdmin ? (
              <div className="flex flex-wrap gap-2">
                <Input id={`platform-${cap.provider}`} inputMode="numeric" placeholder="None for this tenant" value={platform}
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
              <p id={`platform-${cap.provider}`} className="text-sm">
                {cap.platform_cap != null ? `${n(cap.platform_cap)} characters` : 'None'}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

export function SpeechCapsCard({ platformAdmin }: { platformAdmin: boolean }) {
  const queryClient = useQueryClient()
  const caps = useQuery({ queryKey: ['speech', 'caps'], queryFn: speech.caps })
  const providers = useQuery({ queryKey: ['speech', 'providers'], queryFn: speech.providers })
  // The tenant's accounts too: listed even where no TTS server answers.
  const accounts = useQuery({ queryKey: ['speech', 'accounts'], queryFn: speech.accounts })
  const [error, setError] = useState<string | null>(null)
  const [adding, setAdding] = useState<{ provider: string; value: string } | null>(null)
  const add = useMutation({
    mutationFn: ({ provider, value }: { provider: string; value: number }) => speech.saveCap(provider, value),
    onSuccess: () => { setAdding(null); setError(null); queryClient.invalidateQueries({ queryKey: ['speech', 'caps'] }) },
    onError: (e) => setError((e as Error).message),
  })
  const rows = caps.data ?? []
  // What can be capped and is not yet: never the default, which speaks at a cap.
  const choices = [
    ...(providers.data ?? []).map((p) => ({ name: p.name, label: p.label, is_default: p.is_default })),
    ...(accounts.data ?? []).map((a) => ({ name: a.provider, label: a.name, is_default: false })),
  ].filter((p, i, all) => all.findIndex((q) => q.name === p.name) === i)
  const open = choices.filter((p) => !p.is_default && !rows.some((r) => r.provider === p.name))
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle>Caps</CardTitle>
          <CardDescription>
            Characters a provider may speak for you each month (UTC); the cache is free. At a cap the
            default provider speaks instead.
          </CardDescription>
        </div>
        {open.length > 0 && !adding && (
          <Button variant="outline" size="sm" onClick={() => setAdding({ provider: open[0].name, value: '' })}>
            <Plus className="mr-2 h-4 w-4" /> Add a cap
          </Button>
        )}
      </CardHeader>
      <CardContent className="space-y-3">
        {rows.length === 0 && !adding && <p className="text-sm text-muted-foreground">No caps: speech is not limited.</p>}
        {rows.map((c) => <CapRow key={`${c.provider}-${c.own_cap}-${c.platform_cap}`} cap={c} platformAdmin={platformAdmin} onError={setError} />)}
        {adding && (
          <form className="flex flex-col gap-2 rounded-md border p-3 sm:flex-row sm:items-end"
            onSubmit={(e) => {
              e.preventDefault()
              const v = count(adding.value)
              if (v != null) add.mutate({ provider: adding.provider, value: v })
            }}>
            <div className="min-w-0 flex-1 space-y-1">
              <Label htmlFor="cap-provider">Provider</Label>
              <Select value={adding.provider} onValueChange={(provider) => setAdding({ ...adding, provider })}>
                <SelectTrigger id="cap-provider"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {open.map((p) => <SelectItem key={p.name} value={p.name}>{p.label}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-0 flex-1 space-y-1">
              <Label htmlFor="cap-value">Characters a month</Label>
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

/** The platform's default cap and price per provider, for every tenant. */
export function PlatformCaps({ providers, prices, onPrices }: {
  providers: { name: string; is_default: boolean }[]
  prices: Record<string, number>
  onPrices: (prices: Record<string, number>) => void
}) {
  const queryClient = useQueryClient()
  const caps = useQuery({ queryKey: ['speech', 'platform-caps'], queryFn: speech.platformCaps })
  const [error, setError] = useState<string | null>(null)
  const save = useMutation({
    mutationFn: ({ provider, value }: { provider: string; value: number | null }) =>
      value == null ? speech.removePlatformCap(provider) : speech.savePlatformCap(provider, value),
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['speech'] })
    },
    onError: (e) => setError((e as Error).message),
  })
  const capOf = (p: string) => (caps.data ?? []).find((c) => c.provider === p)?.monthly_characters
  return (
    <div className="space-y-2">
      {providers.filter((p) => !p.is_default).map((p) => (
        <form key={`${p.name}-${capOf(p.name)}-${prices[p.name]}`} className="grid gap-2 sm:grid-cols-[1fr_1fr_1fr_auto] sm:items-end"
          onSubmit={(e) => {
            e.preventDefault()
            const form = new FormData(e.currentTarget)
            const cap = count(String(form.get('cap') ?? ''))
            const price = String(form.get('price') ?? '').trim()
            save.mutate({ provider: p.name, value: cap })
            const next = { ...prices }
            if (price === '') delete next[p.name]
            else next[p.name] = Number(price)
            if (next[p.name] !== prices[p.name]) onPrices(next)
          }}>
          <span className="text-sm font-medium sm:self-center">{p.name}</span>
          <div className="space-y-1">
            <Label htmlFor={`default-cap-${p.name}`}>Default cap a month</Label>
            <Input id={`default-cap-${p.name}`} name="cap" inputMode="numeric" placeholder="None"
              defaultValue={capOf(p.name) != null ? String(capOf(p.name)) : ''} />
          </div>
          <div className="space-y-1">
            <Label htmlFor={`price-${p.name}`}>Price per million</Label>
            <Input id={`price-${p.name}`} name="price" inputMode="decimal" placeholder="None"
              defaultValue={prices[p.name] != null ? String(prices[p.name]) : ''} />
          </div>
          <Button type="submit" variant="outline" size="sm" disabled={save.isPending}
            aria-label={`Save ${p.name}'s default cap and price`}>Save</Button>
        </form>
      ))}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}
