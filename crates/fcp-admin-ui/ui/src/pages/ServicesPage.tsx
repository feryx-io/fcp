// Every running service, the platform's and its sites' (docs/SERVICES.md):
// what it is, where, which release, how it is, what it carries; a service
// that stopped stays here, down since then, until it returns or is
// forgotten. Platform administrators only.
import { useEffect, useMemo } from 'react'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { AlertTriangle, ArrowLeft, ArrowUpDown, CheckCircle2, CircleHelp, Server, XCircle } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { DataTable } from '@/components/DataTable'
import { services, type ServiceView } from '@/api/endpoints'
import { formatDateTime, formatDuration } from '@/lib/utils'
import { useWebSocketStore } from '@/stores/websocket'

const STATES = ['serving', 'degraded', 'draining', 'stopping', 'down', 'stopped', 'unknown'] as const

const STATE_HELP: Record<string, string> = {
  serving: 'Taking work, every check passing',
  degraded: 'Taking work, a check failing',
  draining: 'Finishing what it has, taking nothing new',
  stopping: 'Shutting down on purpose',
  down: 'Its heartbeat stopped: lost, not shut down',
  stopped: 'It said it was stopping',
  unknown: 'Redis, or the site’s link, cannot be read now',
}

export const stateBadge = (state: string) => {
  switch (state) {
    case 'serving': return 'success' as const
    case 'degraded':
    case 'draining':
    case 'stopping': return 'warning' as const
    case 'down': return 'destructive' as const
    default: return 'outline' as const
  }
}

export const kindLabel = (kind: string) =>
  kind.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())

const since = (at: string | null | undefined) => {
  if (!at) return null
  const secs = Math.max(0, Math.floor((Date.now() - new Date(at).getTime()) / 1000))
  const days = Math.floor(secs / 86400)
  return days > 0 ? `${days}d ${formatDuration(secs % 86400)}` : formatDuration(secs)
}

const servicePath = (s: ServiceView) =>
  `/admin/services/${encodeURIComponent(s.id)}${s.site_id ? `?site_id=${encodeURIComponent(s.site_id)}` : ''}`

const isDown = (s: ServiceView) => s.state === 'down' || s.state === 'stopped'

/** Refetch the services when one comes up or goes down. */
function useServiceEvents() {
  const queryClient = useQueryClient()
  const latest = useWebSocketStore((s) => s.events[0])
  useEffect(() => {
    if (latest?.event_type === 'system.service_up' || latest?.event_type === 'system.service_down') {
      void queryClient.invalidateQueries({ queryKey: ['services'] })
    }
  }, [latest, queryClient])
}

const columns: ColumnDef<ServiceView>[] = [
  {
    accessorKey: 'state',
    header: 'State',
    cell: ({ row }) => (
      <Badge variant={stateBadge(row.original.state)} title={STATE_HELP[row.original.state]}>
        {row.original.state}
      </Badge>
    ),
  },
  {
    accessorKey: 'id',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Service <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) => (
      <Link to={servicePath(row.original)} className="font-mono text-xs hover:underline">
        {row.original.id}
      </Link>
    ),
  },
  {
    accessorKey: 'kind',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Kind <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) => <Badge variant="secondary">{kindLabel(row.original.kind)}</Badge>,
  },
  {
    id: 'site',
    header: 'Where',
    accessorFn: (s) => s.site_name ?? s.site_id ?? '',
    cell: ({ row }) => (
      <div className="text-xs">
        <div>{row.original.site_name ?? row.original.site_id ?? 'Platform'}</div>
        <div className="font-mono text-muted-foreground">{row.original.host}</div>
      </div>
    ),
  },
  {
    accessorKey: 'version',
    header: 'Version',
    cell: ({ row }) => <span className="font-mono text-xs">{row.original.version}</span>,
  },
  {
    id: 'for',
    header: 'For',
    cell: ({ row }) => {
      const s = row.original
      if (isDown(s)) {
        return <span className="text-xs text-destructive">down {since(s.down_since)}</span>
      }
      return <span className="text-xs text-muted-foreground">up {since(s.started_at) ?? '—'}</span>
    },
  },
  {
    id: 'checks',
    header: 'Checks',
    cell: ({ row }) => {
      const failing = row.original.checks.filter((c) => !c.ok)
      if (row.original.checks.length === 0) return <span className="text-xs text-muted-foreground">—</span>
      if (failing.length === 0) return <span className="text-xs text-muted-foreground">all passing</span>
      return (
        <div className="flex flex-wrap gap-1">
          {failing.map((c) => (
            <Badge key={c.name} variant={c.required ? 'destructive' : 'warning'} title={c.error ?? undefined}>
              {c.name}
            </Badge>
          ))}
        </div>
      )
    },
  },
  {
    id: 'load',
    header: 'Load',
    cell: ({ row }) => {
      const load = Object.entries(row.original.load ?? {})
      if (load.length === 0) return <span className="text-xs text-muted-foreground">—</span>
      return (
        <span className="text-xs">
          {load.map(([k, v]) => `${v} ${k.replace(/_/g, ' ')}`).join(', ')}
        </span>
      )
    },
  },
]

export function ServicesPage() {
  useServiceEvents()
  const [params, setParams] = useSearchParams()
  const kind = params.get('kind') ?? 'all'
  const state = params.get('state') ?? 'all'
  const where = params.get('site') ?? 'all'

  const { data: list = [], isLoading, error } = useQuery({
    queryKey: ['services', 'list'],
    queryFn: services.list,
    refetchInterval: 5_000,
  })
  const { data: summary } = useQuery({
    queryKey: ['services', 'summary'],
    queryFn: services.summary,
    refetchInterval: 5_000,
  })

  const kinds = useMemo(() => [...new Set(list.map((s) => s.kind))].sort(), [list])
  const sites = useMemo(() => {
    const seen = new Map<string, string>()
    for (const s of list) if (s.site_id) seen.set(s.site_id, s.site_name ?? s.site_id)
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]))
  }, [list])

  const shown = list.filter((s) =>
    (kind === 'all' || s.kind === kind)
    && (state === 'all'
      || (state === 'attention' ? !['serving'].includes(s.state) : s.state === state))
    && (where === 'all' || (where === 'platform' ? !s.site_id : s.site_id === where)))

  const set = (key: string, value: string) => {
    const next = new URLSearchParams(params)
    if (value === 'all') next.delete(key)
    else next.set(key, value)
    setParams(next, { replace: true })
  }

  const cards = [
    { title: 'Serving', value: summary?.serving, icon: CheckCircle2, filter: 'serving' },
    { title: 'Needs attention', value: summary?.attention, icon: AlertTriangle, filter: 'degraded' },
    { title: 'Down', value: summary?.down, icon: XCircle, filter: 'down' },
    { title: 'Unknown', value: summary?.unknown, icon: CircleHelp, filter: 'unknown' },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Services</h1>
        <p className="text-muted-foreground">
          Every running process, the platform's and its sites', from the heartbeat each one writes
        </p>
      </div>

      {summary && !summary.redis_reachable && (
        <Card className="border-yellow-500">
          <CardContent className="flex items-start gap-3 pt-6">
            <AlertTriangle className="h-5 w-5 text-yellow-600 shrink-0" />
            <div className="text-sm">
              <p className="font-medium">Redis cannot be read: the platform's services are shown as last seen</p>
              <p className="text-muted-foreground">
                {summary.redis_error ?? 'Not read yet.'} Nothing is marked down while this lasts.
              </p>
            </div>
          </CardContent>
        </Card>
      )}

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        {cards.map((card) => {
          const Icon = card.icon
          return (
            <Card
              key={card.title}
              className="cursor-pointer hover:bg-accent/50"
              onClick={() => set('state', state === card.filter ? 'all' : card.filter)}
            >
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">{card.title}</CardTitle>
                <Icon className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{card.value ?? '-'}</div>
              </CardContent>
            </Card>
          )
        })}
      </div>

      <Card>
        <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-2">
          <CardTitle className="flex items-center gap-2">
            <Server className="h-5 w-5" /> {shown.length} of {list.length}
          </CardTitle>
          <div className="flex flex-wrap gap-2">
            <Select value={kind} onValueChange={(v) => set('kind', v)}>
              <SelectTrigger className="w-40" aria-label="Kind"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">Every kind</SelectItem>
                {kinds.map((k) => <SelectItem key={k} value={k}>{kindLabel(k)}</SelectItem>)}
              </SelectContent>
            </Select>
            <Select value={state} onValueChange={(v) => set('state', v)}>
              <SelectTrigger className="w-40" aria-label="State"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">Every state</SelectItem>
                <SelectItem value="attention">Not serving</SelectItem>
                {STATES.map((s) => <SelectItem key={s} value={s}>{s}</SelectItem>)}
              </SelectContent>
            </Select>
            <Select value={where} onValueChange={(v) => set('site', v)}>
              <SelectTrigger className="w-44" aria-label="Where"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">Everywhere</SelectItem>
                <SelectItem value="platform">The platform</SelectItem>
                {sites.map(([id, name]) => <SelectItem key={id} value={id}>{name}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
        </CardHeader>
        <CardContent>
          {error ? (
            <p className="text-sm text-destructive">Services could not be read: {String((error as Error).message)}</p>
          ) : isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : list.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No service has said it is there yet. Every FCP process with a Redis writes a heartbeat;
              the admin keeps them with <code>[cluster] redis_url</code>.
            </p>
          ) : (
            <DataTable columns={columns} data={shown} searchKey="id" searchPlaceholder="Search services…" />
          )}
        </CardContent>
      </Card>
    </div>
  )
}

/** One service: its checks, load, addresses and times; Forget once it is down. */
export function ServicePage() {
  useServiceEvents()
  const { id = '' } = useParams()
  const [params] = useSearchParams()
  const siteId = params.get('site_id')
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: s, error, isLoading } = useQuery({
    queryKey: ['services', 'one', id, siteId],
    queryFn: () => services.get(id, siteId),
    refetchInterval: 5_000,
  })
  const forget = useMutation({
    mutationFn: () => services.forget(id, siteId),
    onSuccess: () => {
      // Gone: asking for it again would be a 404.
      queryClient.removeQueries({ queryKey: ['services', 'one', id, siteId] })
      void queryClient.invalidateQueries({ queryKey: ['services', 'list'] })
      void queryClient.invalidateQueries({ queryKey: ['services', 'summary'] })
      navigate('/admin/services')
    },
  })

  const back = (
    <Button variant="ghost" asChild>
      <Link to="/admin/services"><ArrowLeft className="mr-2 h-4 w-4" />Services</Link>
    </Button>
  )
  if (isLoading) return <div className="space-y-4">{back}<p className="text-sm text-muted-foreground">Loading…</p></div>
  if (error || !s) {
    return (
      <div className="space-y-4">
        {back}
        <p className="text-sm text-destructive">No such service{error ? `: ${(error as Error).message}` : ''}</p>
      </div>
    )
  }

  const rows: [string, React.ReactNode][] = [
    ['Kind', kindLabel(s.kind)],
    ['Where', s.site_name ?? s.site_id ?? 'The platform'],
    ['Host', <span className="font-mono">{s.host}</span>],
    ['Version', <span className="font-mono">{s.version}</span>],
    ['Region', s.region ?? '—'],
    ['Cluster member', s.member ? 'Yes' : 'No'],
    ['HTTP', s.http_address ? <span className="font-mono">{s.http_address}</span> : '—'],
    ['SIP', s.sip_address ? <span className="font-mono">{s.sip_address}</span> : '—'],
    ['Public URL', s.public_url ?? '—'],
    ['Started', s.started_at ? formatDateTime(s.started_at) : '—'],
    ['First seen', formatDateTime(s.first_seen)],
    ['Last seen', formatDateTime(s.last_seen)],
    ['Down since', s.down_since ? formatDateTime(s.down_since) : '—'],
  ]

  return (
    <div className="space-y-6">
      {back}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-3xl font-bold tracking-tight font-mono">{s.id}</h1>
          <p className="text-muted-foreground">{STATE_HELP[s.state] ?? s.state}</p>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant={stateBadge(s.state)} className="text-sm">{s.state}</Badge>
          {isDown(s) && (
            <Button
              variant="destructive"
              disabled={forget.isPending}
              onClick={() => {
                if (window.confirm(`Forget ${s.id}? It returns to the list if it starts again.`)) forget.mutate()
              }}
            >
              Forget
            </Button>
          )}
        </div>
      </div>
      {forget.error && <p className="text-sm text-destructive">{(forget.error as Error).message}</p>}

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Checks</CardTitle></CardHeader>
          <CardContent>
            {s.checks.length === 0 ? (
              <p className="text-sm text-muted-foreground">None reported{isDown(s) ? ' since it went down' : ''}.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow><TableHead>Check</TableHead><TableHead>Answer</TableHead><TableHead>Since</TableHead></TableRow>
                </TableHeader>
                <TableBody>
                  {s.checks.map((c) => (
                    <TableRow key={c.name}>
                      <TableCell>
                        {c.name}
                        {c.required && <span className="ml-1 text-xs text-muted-foreground">(required)</span>}
                      </TableCell>
                      <TableCell>
                        <Badge variant={c.ok ? 'success' : c.required ? 'destructive' : 'warning'}>
                          {c.ok ? 'ok' : 'failing'}
                        </Badge>
                        {c.error && <div className="mt-1 text-xs text-muted-foreground">{c.error}</div>}
                      </TableCell>
                      <TableCell className="text-xs">{formatDateTime(c.since)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Load</CardTitle></CardHeader>
          <CardContent>
            {Object.keys(s.load ?? {}).length === 0 ? (
              <p className="text-sm text-muted-foreground">This kind reports no load.</p>
            ) : (
              <dl className="grid grid-cols-2 gap-2 text-sm">
                {Object.entries(s.load).map(([k, v]) => (
                  <div key={k}>
                    <dt className="text-muted-foreground">{k.replace(/_/g, ' ')}</dt>
                    <dd className="text-2xl font-bold">{v}</dd>
                  </div>
                ))}
              </dl>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader><CardTitle>Details</CardTitle></CardHeader>
        <CardContent>
          <dl className="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
            {rows.map(([label, value]) => (
              <div key={label} className="flex justify-between gap-4 border-b py-1">
                <dt className="text-muted-foreground">{label}</dt>
                <dd className="text-right">{value}</dd>
              </div>
            ))}
          </dl>
        </CardContent>
      </Card>
    </div>
  )
}
