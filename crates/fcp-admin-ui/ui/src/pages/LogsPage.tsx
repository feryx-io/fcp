// The services' logs (docs/LOG_VIEWER.md, L4): every service's recent
// lines, newest first, followed live or paged back, filtered by level,
// text, service, call and time; a tenant's administrator sees only lines
// naming their own tenant. A platform administrator also filters by tenant
// and raises a service's level for an hour.
import { useEffect, useMemo, useState } from 'react'
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download, Pause, Play, RotateCcw } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { logs, type LogFilter } from '@/api/endpoints'
import type { LogLine } from '@/types'
import { useAuthStore } from '@/stores/auth'

const LEVELS = ['error', 'warn', 'info', 'debug', 'trace'] as const
const SINCE: Record<string, number | null> = { '15m': 15, '1h': 60, '6h': 360, '24h': 1440, all: null }

const levelBadge = (level: string) => {
  switch (level.toUpperCase()) {
    case 'ERROR': return 'destructive' as const
    case 'WARN': return 'warning' as const
    case 'INFO': return 'secondary' as const
    default: return 'outline' as const
  }
}

const time = (iso: string) => {
  const d = new Date(iso)
  return `${d.toLocaleTimeString([], { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

export function LogsPage() {
  const { user } = useAuthStore()
  const platform = user?.role === 'super_admin'
  const [level, setLevel] = useState('info')
  const [q, setQ] = useState('')
  const [service, setService] = useState('')
  const [callId, setCallId] = useState('')
  const [tenant, setTenant] = useState('')
  const [since, setSince] = useState('1h')
  const [live, setLive] = useState(true)
  const [older, setOlder] = useState<LogLine[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [open, setOpen] = useState<string | null>(null)

  // Typing settles before a query goes out.
  const [typed, setTyped] = useState({ q: '', service: '', callId: '', tenant: '' })
  useEffect(() => {
    const t = setTimeout(() => setTyped({ q, service, callId, tenant }), 400)
    return () => clearTimeout(t)
  }, [q, service, callId, tenant])

  const filter: LogFilter = useMemo(() => {
    const minutes = SINCE[since]
    return {
      level,
      q: typed.q || undefined,
      service: typed.service || undefined,
      call_id: typed.callId || undefined,
      tenant: platform ? typed.tenant || undefined : undefined,
      since: minutes ? new Date(Date.now() - minutes * 60_000).toISOString() : undefined,
      limit: 200,
    }
    // `since` is fixed when the filter changes, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, typed, since, platform])

  // A new filter starts again from the newest lines.
  useEffect(() => {
    setOlder([])
    setCursor(null)
  }, [filter])

  const newest = useQuery({
    queryKey: ['logs', filter],
    queryFn: () => logs.page(filter),
    refetchInterval: live ? 2000 : false,
    placeholderData: keepPreviousData,
  })
  useEffect(() => {
    if (older.length === 0) setCursor(newest.data?.next_cursor ?? null)
  }, [newest.data, older.length])

  const loadOlder = useMutation({
    mutationFn: () => logs.page({ ...filter, cursor: cursor ?? undefined }),
    onSuccess: (page) => {
      setOlder((o) => [...o, ...page.data])
      setCursor(page.next_cursor ?? null)
      setLive(false)
    },
  })

  const lines = useMemo(() => {
    const seen = new Set<string>()
    return [...(newest.data?.data ?? []), ...older].filter((l) => {
      const key = `${l.service}/${l.id}`
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
  }, [newest.data, older])

  const download = async () => {
    const url = await logs.exportUrl({ ...filter, limit: undefined })
    const a = document.createElement('a')
    a.href = url
    a.download = 'fcp-logs.jsonl'
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Logs</h1>
          <p className="text-muted-foreground">
            {platform
              ? "Every service's recent lines, newest first: the last day, kept in the cluster's Redis"
              : "Your tenant's lines from every service, newest first"}
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => setLive((l) => !l)} aria-label={live ? 'Pause' : 'Follow'}>
            {live ? <Pause className="mr-2 h-4 w-4" /> : <Play className="mr-2 h-4 w-4" />}
            {live ? 'Pause' : 'Follow'}
          </Button>
          <Button variant="outline" onClick={download}>
            <Download className="mr-2 h-4 w-4" /> Export
          </Button>
        </div>
      </div>

      <Card>
        <CardContent className="grid gap-3 pt-6 sm:grid-cols-2 lg:grid-cols-6">
          <div className="space-y-1">
            <Label>Level and worse</Label>
            <Select value={level} onValueChange={setLevel}>
              <SelectTrigger aria-label="Level"><SelectValue /></SelectTrigger>
              <SelectContent>
                {LEVELS.map((l) => <SelectItem key={l} value={l}>{l}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>Since</Label>
            <Select value={since} onValueChange={setSince}>
              <SelectTrigger aria-label="Since"><SelectValue /></SelectTrigger>
              <SelectContent>
                {Object.keys(SINCE).map((s) => (
                  <SelectItem key={s} value={s}>{s === 'all' ? 'all kept' : `last ${s}`}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1 lg:col-span-2">
            <Label htmlFor="logs-q">Text</Label>
            <Input id="logs-q" placeholder="in the message or a field" value={q} onChange={(e) => setQ(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="logs-service">Service</Label>
            <Input id="logs-service" placeholder="its id" value={service} onChange={(e) => setService(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="logs-call">Call</Label>
            <Input id="logs-call" placeholder="call id" value={callId} onChange={(e) => setCallId(e.target.value)} />
          </div>
          {platform && (
            <div className="space-y-1">
              <Label htmlFor="logs-tenant">Tenant</Label>
              <Input id="logs-tenant" placeholder="id, or none for the platform's" value={tenant}
                onChange={(e) => setTenant(e.target.value)} />
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardContent className="p-0">
          {newest.error ? (
            <p className="p-6 text-sm text-destructive">{(newest.error as Error).message}</p>
          ) : lines.length === 0 ? (
            <p className="p-6 text-sm text-muted-foreground">
              {newest.isLoading ? 'Loading…' : 'No lines match.'}
            </p>
          ) : (
            <div className="divide-y font-mono text-xs" data-testid="log-lines">
              {lines.map((l) => {
                const key = `${l.service}/${l.id}`
                const fields = Object.entries(l.fields ?? {})
                return (
                  <div key={key}>
                    <button
                      type="button"
                      className="flex w-full min-w-0 items-start gap-2 px-3 py-1.5 text-left hover:bg-accent/50"
                      onClick={() => setOpen(open === key ? null : key)}
                    >
                      <span className="shrink-0 text-muted-foreground">{time(l.time)}</span>
                      <Badge variant={levelBadge(l.level)} className="shrink-0 px-1 py-0 text-[10px]">{l.level}</Badge>
                      <span className="shrink-0 text-muted-foreground">{l.service}</span>
                      <span className="min-w-0 flex-1 break-words">{l.message}</span>
                    </button>
                    {open === key && (
                      <div className="space-y-0.5 bg-muted/40 px-3 py-2">
                        <div><span className="text-muted-foreground">target</span> {l.target}</div>
                        {l.tenant_id && <div><span className="text-muted-foreground">tenant</span> {l.tenant_id}</div>}
                        {l.call_id && (
                          <div>
                            <span className="text-muted-foreground">call</span>{' '}
                            <button type="button" className="underline" onClick={() => { setCallId(l.call_id!); setLive(false) }}>
                              {l.call_id}
                            </button>
                          </div>
                        )}
                        {fields.map(([k, v]) => (
                          <div key={k} className="break-words"><span className="text-muted-foreground">{k}</span> {v}</div>
                        ))}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
          {cursor && (
            <div className="border-t p-3 text-center">
              <Button variant="outline" size="sm" onClick={() => loadOlder.mutate()} disabled={loadOlder.isPending}>
                Older lines
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {platform && <Levels />}
    </div>
  )
}

/** A service's level for an hour (platform administrators). */
function Levels() {
  const queryClient = useQueryClient()
  const levels = useQuery({ queryKey: ['logs', 'levels'], queryFn: logs.levels, refetchInterval: 10_000 })
  const set = useMutation({
    mutationFn: ({ service, level }: { service: string; level: string | null }) => logs.setLevel(service, level),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['logs', 'levels'] }),
    onError: (e) => alert((e as Error).message),
  })
  return (
    <Card>
      <CardHeader>
        <CardTitle>Levels</CardTitle>
        <CardDescription>
          Raise a service's level to see more of it for an hour; it goes back on its own.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {(levels.data ?? []).length === 0 && (
          <p className="text-sm text-muted-foreground">No service has shipped lines yet.</p>
        )}
        {(levels.data ?? []).map((l) => (
          <div key={l.service} className="flex flex-wrap items-center gap-2 text-sm">
            <span className="min-w-[12rem] font-mono">{l.service}</span>
            <Select value={l.level ?? 'configured'}
              onValueChange={(v) => set.mutate({ service: l.service, level: v === 'configured' ? null : v })}>
              <SelectTrigger className="h-8 w-36" aria-label={`Level of ${l.service}`}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="configured">as configured</SelectItem>
                {LEVELS.map((v) => <SelectItem key={v} value={v}>{v}</SelectItem>)}
              </SelectContent>
            </Select>
            {l.level && (
              <>
                <span className="text-xs text-muted-foreground">
                  for {Math.ceil((l.expires_in_secs ?? 0) / 60)} more minutes
                </span>
                <Button variant="ghost" size="sm" aria-label={`Back to configured for ${l.service}`}
                  onClick={() => set.mutate({ service: l.service, level: null })}>
                  <RotateCcw className="h-4 w-4" />
                </Button>
              </>
            )}
          </div>
        ))}
      </CardContent>
    </Card>
  )
}
