import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  Server, Activity, Clock, AlertTriangle, Phone,
  RefreshCw, ArrowUpDown, Info, LogOut,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { DataTable } from '@/components/DataTable'
import { cluster, dashboard } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { ClusterNodeInfo, DashboardSummary, SiteInfo } from '@/types'

const statusBadge = (status: string) => {
  switch (status) {
    case 'healthy': return 'success' as const
    case 'degraded': return 'warning' as const
    case 'offline': return 'destructive' as const
    default: return 'outline' as const
  }
}

// A node leaving service takes no new work and keeps what it has until
// its deadline (docs/CAPACITY_AND_SURVIVABILITY.md §6). It is still
// healthy, so it needs saying separately.
const DRAINING_TITLE =
  'Leaving service: taking no new calls, keeping the ones it has until its drain deadline'

// A node that has lost every other region at once cannot tell a lost
// region from a cut link, so it says its own view may be what is broken
// (docs/MULTI_REGION_FOLLOWUPS.md §2.3). Nothing acts on it.
const viewLabel = (state: string) =>
  state === 'remote_regions_lost' ? 'lost other regions' : state.replace(/_/g, ' ')

const formatNodeType = (t: string) =>
  t.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())

const formatUptime = (seconds: number) => {
  const days = Math.floor(seconds / 86400)
  const remaining = seconds % 86400
  if (days > 0) return `${days}d ${formatDuration(remaining)}`
  return formatDuration(remaining)
}

const columns: ColumnDef<ClusterNodeInfo>[] = [
  {
    accessorKey: 'status',
    header: 'Status',
    cell: ({ row }) => (
      <div className="flex flex-wrap items-center gap-1">
        <Badge variant={statusBadge(row.original.status)}>
          {row.original.status}
        </Badge>
        {row.original.draining && (
          <Badge variant="warning" title={DRAINING_TITLE}>draining</Badge>
        )}
        {row.original.view && row.original.view.state !== 'whole' && (
          <Badge variant="warning" title={row.original.view.summary}>
            {viewLabel(row.original.view.state)}
          </Badge>
        )}
      </div>
    ),
  },
  {
    accessorKey: 'node_id',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Node ID <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) => (
      <span className="font-mono text-xs">{row.original.node_id}</span>
    ),
  },
  {
    accessorKey: 'node_type',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Type <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) => (
      <Badge variant="secondary">{formatNodeType(row.original.node_type)}</Badge>
    ),
  },
  {
    accessorKey: 'region',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Region <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) =>
      row.original.region ? (
        <Badge variant="outline">{row.original.region}</Badge>
      ) : (
        <span className="text-xs text-muted-foreground">—</span>
      ),
  },
  {
    accessorKey: 'version',
    header: 'Version',
    cell: ({ row }) => (
      <span className="font-mono text-xs">{row.original.version}</span>
    ),
  },
  {
    accessorKey: 'active_calls',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Active Calls <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
  },
  {
    id: 'headroom',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Headroom <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    // What the node will still take, against what it allows: a node
    // carrying more calls can have more room than one carrying fewer,
    // and placement prefers room (docs/CAPACITY_AND_SURVIVABILITY.md §7).
    accessorFn: (row) => row.calls_left ?? -1,
    cell: ({ row }) => {
      const node = row.original
      const capacity = node.capacity
      if (!capacity || capacity.max_concurrent_calls === 0) {
        return (
          <span
            className="text-xs text-muted-foreground"
            title="This node advertises no call limit, so its room is unknown; placement ranks it on its calls."
          >
            no limit
          </span>
        )
      }
      const left = node.calls_left ?? 0
      const used = capacity.max_concurrent_calls - left
      const pct = Math.round((used / capacity.max_concurrent_calls) * 100)
      const tone = pct >= 90 ? 'text-destructive' : pct >= 70 ? 'text-yellow-600' : ''
      return (
        <div className="text-xs space-y-0.5">
          <div className={tone}>
            <span className="font-mono">{left}</span> free of {capacity.max_concurrent_calls}
          </div>
          <div className="text-muted-foreground">
            {capacity.max_cps > 0 && <span className="mr-2">{capacity.cps}/{capacity.max_cps} cps</span>}
            {capacity.max_concurrent_registrations > 0 && (
              <span>
                {capacity.registrations}/{capacity.max_concurrent_registrations} reg
              </span>
            )}
          </div>
        </div>
      )
    },
  },
  {
    id: 'video_budget',
    header: 'Video Budget',
    cell: ({ row }) => {
      const budget = row.original.video_budget
      if (!budget || budget.length === 0) {
        return <span className="text-xs text-muted-foreground">—</span>
      }
      return (
        <div className="text-xs space-y-0.5">
          {budget.map((b) => {
            const pct = b.total > 0 ? Math.round((b.left / b.total) * 100) : 0
            const tone = (p: number) => (p < 15 ? 'text-destructive' : p < 40 ? 'text-yellow-600' : '')
            return (
              <div key={b.device} title={`encodes ${b.codecs.join(', ')} · ${b.left} of ${b.total} work units left${b.engines ? ' (the tightest engine)' : ''}`}>
                <span className="font-mono">{b.device}</span>{' '}
                <span className={tone(pct)}>
                  {b.left}/{b.total} ({pct}%)
                </span>
                {b.engines && (
                  <div className="pl-3 text-muted-foreground">
                    {(['encode', 'decode', 'compose'] as const).map((e) => {
                      const eng = b.engines![e]
                      const ep = eng.total > 0 ? Math.round((eng.left / eng.total) * 100) : 0
                      return (
                        <span key={e} className="mr-2" title={`${e}: ${eng.left} of ${eng.total} units left`}>
                          {e} <span className={tone(ep)}>{ep}%</span>
                        </span>
                      )
                    })}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )
    },
  },
  {
    accessorKey: 'uptime_seconds',
    header: ({ column }) => (
      <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
        Uptime <ArrowUpDown className="ml-2 h-4 w-4" />
      </Button>
    ),
    cell: ({ row }) => formatUptime(row.original.uptime_seconds),
  },
  {
    accessorKey: 'capabilities',
    header: 'Capabilities',
    cell: ({ row }) => (
      <div className="flex flex-wrap gap-1">
        {row.original.capabilities.map((cap) => (
          <Badge key={cap} variant="outline" className="text-xs">{cap}</Badge>
        ))}
      </div>
    ),
  },
  {
    id: 'addresses',
    header: 'Addresses',
    cell: ({ row }) => (
      <div className="text-xs space-y-0.5">
        <div><span className="text-muted-foreground">HTTP:</span> {row.original.http_address}</div>
        <div><span className="text-muted-foreground">SIP:</span> {row.original.sip_address}</div>
      </div>
    ),
  },
]

export function ClusterPage() {
  const { data: nodesData, isLoading: nodesLoading, refetch: refetchNodes } = useQuery({
    queryKey: ['cluster-nodes'],
    queryFn: cluster.nodes,
    refetchInterval: 5000,
  })

  const { data: sitesData } = useQuery({
    queryKey: ['cluster-sites'],
    queryFn: cluster.sites,
    refetchInterval: 10000,
  })

  const { data: statsData, isLoading: statsLoading } = useQuery({
    queryKey: ['cluster-stats'],
    queryFn: cluster.stats,
    refetchInterval: 5000,
  })

  const { data: systemStatus } = useQuery({
    queryKey: ['cluster-status'],
    queryFn: cluster.status,
    refetchInterval: 10000,
  })

  const { data: summary } = useQuery({
    queryKey: ['dashboard-summary'],
    queryFn: dashboard.summary,
    refetchInterval: 10000,
  })

  const isLoading = nodesLoading || statsLoading

  // Not configured state
  if (!isLoading && nodesData && !nodesData.available) {
    return (
      <div className="space-y-6">
        <PageHeader onRefresh={() => refetchNodes()} />
        <NotConfiguredBanner message={nodesData.message} />
        <LocalNodeInfo status={systemStatus} summary={summary} />
      </div>
    )
  }

  const stats = statsData?.stats
  const nodes = nodesData?.nodes ?? []

  return (
    <div className="space-y-6">
      <PageHeader onRefresh={() => refetchNodes()} />

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading cluster data...</div>
      ) : (
        <>
          {nodesData?.message && (
            <div className="rounded-md border border-yellow-200 bg-yellow-50 dark:border-yellow-900 dark:bg-yellow-950 p-4 text-sm text-yellow-800 dark:text-yellow-200">
              <AlertTriangle className="inline mr-2 h-4 w-4" />
              {nodesData.message}
            </div>
          )}

          {/* Nodes that doubt their own view of the cluster */}
          <PartialViewBanner nodes={nodes} />

          {/* More than one release running (docs/BACKUP_AND_RESTORE.md §5) */}
          {stats?.version_skew && stats.versions && (
            <VersionSkewBanner versions={stats.versions} />
          )}

          {/* Summary Stats Cards */}
          {stats && (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-6">
              <StatCard icon={Server} label="Total Nodes" value={stats.total_nodes} />
              <StatCard icon={Activity} label="Healthy" value={stats.healthy_nodes} color="text-green-600" />
              <StatCard icon={AlertTriangle} label="Degraded" value={stats.degraded_nodes} color="text-yellow-600" />
              <StatCard icon={AlertTriangle} label="Offline" value={stats.offline_nodes} color="text-red-600" />
              <StatCard icon={Phone} label="Total Active Calls" value={stats.total_active_calls} />
              <StatCard
                icon={LogOut}
                label="Draining"
                value={nodes.filter((n) => n.draining).length}
                color={nodes.some((n) => n.draining) ? 'text-yellow-600' : undefined}
              />
            </div>
          )}

          {/* Sites: tenants' own stacks enrolled here (docs/HYBRID_CONTROL_PLANE.md) */}
          {sitesData?.available && sitesData.sites.length > 0 && (
            <SitesSection sites={sitesData.sites} />
          )}

          {/* Nodes by Type */}
          {stats && Object.keys(stats.nodes_by_type).length > 0 && (
            <div>
              <h2 className="text-lg font-semibold mb-3">Nodes by Type</h2>
              <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
                {Object.entries(stats.nodes_by_type).map(([type_, count]) => (
                  <Card key={type_}>
                    <CardContent className="p-4 text-center">
                      <p className="text-2xl font-bold">{count}</p>
                      <p className="text-xs text-muted-foreground">{formatNodeType(type_)}</p>
                    </CardContent>
                  </Card>
                ))}
              </div>
            </div>
          )}

          {/* Nodes by Region */}
          {stats && Object.keys(stats.nodes_by_region).length > 0 && (
            <div>
              <h2 className="text-lg font-semibold mb-3">Nodes by Region</h2>
              <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
                {Object.entries(stats.nodes_by_region).map(([region, count]) => (
                  <Card key={region}>
                    <CardContent className="p-4 text-center">
                      <p className="text-2xl font-bold">{count}</p>
                      <p className="text-xs text-muted-foreground">{region}</p>
                    </CardContent>
                  </Card>
                ))}
                {stats.nodes_without_region > 0 && (
                  <Card key="__none">
                    <CardContent className="p-4 text-center">
                      <p className="text-2xl font-bold">{stats.nodes_without_region}</p>
                      <p className="text-xs text-muted-foreground">No region</p>
                    </CardContent>
                  </Card>
                )}
              </div>
            </div>
          )}

          {/* Node Table */}
          <div>
            <h2 className="text-lg font-semibold mb-3">Cluster Nodes</h2>
            <DataTable
              columns={columns}
              data={nodes}
              searchKey="node_id"
              searchPlaceholder="Search nodes..."
            />
          </div>

          {/* Local system info (preserved from original) */}
          <LocalNodeInfo status={systemStatus} summary={summary} />
        </>
      )}
    </div>
  )
}

/**
 * Nodes whose own view of the cluster does not look whole. Every node of a
 * region cut off from the others says the same thing, so they are grouped
 * by what they lost. A node that cannot reach the cluster store at all
 * cannot write its heartbeat either; it vanishes from this list, and its
 * own /status is where it says why.
 */
// An upgrade is a skew on purpose: the first new node makes it, the
// last old one ends it. One that stays is an upgrade that stopped half
// way, and an old binary against the new schema refuses to start
// (docs/BACKUP_AND_RESTORE.md §5), so the banner names who is on what.
function VersionSkewBanner({ versions }: { versions: Record<string, string[]> }) {
  const groups = Object.entries(versions).sort((a, b) => b[1].length - a[1].length)
  return (
    <div className="rounded-md border border-yellow-200 bg-yellow-50 dark:border-yellow-900 dark:bg-yellow-950 p-4 text-sm text-yellow-800 dark:text-yellow-200">
      <div className="flex items-center gap-2 font-medium">
        <AlertTriangle className="h-4 w-4" />
        The cluster runs {groups.length} releases
      </div>
      <ul className="mt-2 space-y-1">
        {groups.map(([version, nodeIds]) => (
          <li key={version}>
            <span className="font-mono text-xs">{version}</span>
            {': '}
            {nodeIds.length} {nodeIds.length === 1 ? 'node' : 'nodes'}
            {' ('}
            {nodeIds.join(', ')}
            {')'}
          </li>
        ))}
      </ul>
      <div className="mt-2 text-xs">
        Expected during a rolling upgrade, one node at a time; a skew that stays is an upgrade
        that stopped half way. Finish it, node by node — an old binary does not start against the
        new schema, so the way back is the backup taken before it.
      </div>
    </div>
  )
}

// A site is a tenant's own stack on the tenant's network, enrolled to
// this cloud and kept by fcp-link; the cloud sees it through its link's
// heartbeats, not through cluster membership.
function SitesSection({ sites }: { sites: SiteInfo[] }) {
  const ago = (secs?: number) =>
    secs === undefined ? 'never' : formatDuration(Math.max(0, Math.floor(Date.now() / 1000) - secs)) + ' ago'
  return (
    <div>
      <h2 className="text-lg font-semibold mb-3">Sites</h2>
      <div className="rounded-md border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b bg-muted/50 text-left">
              <th className="p-2">Site</th>
              <th className="p-2">Tenant</th>
              <th className="p-2">Status</th>
              <th className="p-2">Link</th>
              <th className="p-2">Last seen</th>
              <th className="p-2">Nodes</th>
              <th className="p-2">Sync</th>
              <th className="p-2">Releases</th>
            </tr>
          </thead>
          <tbody>
            {sites.map((site) => (
              <tr key={site.id} className="border-b last:border-0">
                <td className="p-2 font-medium">{site.name}</td>
                <td className="p-2 font-mono text-xs">{site.tenant_id}</td>
                <td className="p-2">
                  <Badge variant={site.status === 'enrolled' ? 'success' : site.status === 'disabled' ? 'destructive' : 'outline'}>
                    {site.status}
                  </Badge>
                </td>
                <td className="p-2">
                  <Badge variant={site.link_up ? 'success' : 'warning'}>{site.link_up ? 'up' : 'down'}</Badge>
                </td>
                <td className="p-2 text-muted-foreground">{ago(site.last_seen_at)}</td>
                <td className="p-2">
                  {site.nodes.length}
                  {site.nodes.some((n) => !n.healthy) && (
                    <span className="ml-1 text-yellow-600">({site.nodes.filter((n) => !n.healthy).length} unhealthy)</span>
                  )}
                </td>
                <td className="p-2">
                  {site.sync_cursor === undefined ? (
                    <span className="text-muted-foreground">—</span>
                  ) : site.sync_error ? (
                    <Badge variant="destructive" title={site.sync_error}>error</Badge>
                  ) : site.sync_behind ? (
                    <Badge variant="warning">{site.sync_behind} behind</Badge>
                  ) : (
                    <Badge variant="success">current</Badge>
                  )}
                </td>
                <td className="p-2 font-mono text-xs">{Object.keys(site.versions).join(', ') || '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function PartialViewBanner({ nodes }: { nodes: ClusterNodeInfo[] }) {
  const doubting = nodes.filter((n) => n.view && n.view.state !== 'whole')
  if (doubting.length === 0) return null
  const groups = new Map<string, ClusterNodeInfo[]>()
  for (const n of doubting) {
    const key = `${n.region ?? '—'}|${(n.view!.lost_regions ?? []).join(', ')}`
    groups.set(key, [...(groups.get(key) ?? []), n])
  }
  return (
    <div className="rounded-md border border-yellow-200 bg-yellow-50 dark:border-yellow-900 dark:bg-yellow-950 p-4 text-sm text-yellow-800 dark:text-yellow-200 space-y-2">
      <div className="font-semibold">
        <AlertTriangle className="inline mr-2 h-4 w-4" />
        {doubting.length === 1 ? 'A node doubts' : `${doubting.length} nodes doubt`} its view of the cluster
      </div>
      {[...groups.entries()].map(([key, members]) => {
        const [region, lost] = key.split('|')
        const since = members[0].view?.since
        return (
          <div key={key}>
            {region !== '—' ? <>In <strong>{region}</strong>, </> : null}
            {members.length === 1 ? '1 node' : `${members.length} nodes`} lost every node
            {lost ? <> in <strong>{lost}</strong></> : null} at once
            {since ? <> at {new Date(since * 1000).toLocaleTimeString()}</> : null}.
            {' '}Either {lost || 'that region'} is down, or the link to it is: from inside, a node
            cannot tell which. Nothing changes behaviour because of this.
            <div className="text-xs opacity-80">{members.map((m) => m.node_id).join(', ')}</div>
          </div>
        )
      })}
    </div>
  )
}

function PageHeader({ onRefresh }: { onRefresh: () => void }) {
  return (
    <div className="flex items-center justify-between">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Cluster Monitor</h1>
        <p className="text-muted-foreground">Cluster-wide node status and health monitoring</p>
      </div>
      <Button variant="outline" onClick={onRefresh}>
        <RefreshCw className="mr-2 h-4 w-4" />
        Refresh
      </Button>
    </div>
  )
}

function NotConfiguredBanner({ message }: { message?: string }) {
  return (
    <Card className="border-dashed">
      <CardContent className="p-6">
        <div className="flex items-start gap-4">
          <Info className="h-6 w-6 text-muted-foreground mt-0.5 flex-shrink-0" />
          <div className="space-y-2">
            <h3 className="font-semibold">Cluster Monitoring Not Configured</h3>
            <p className="text-sm text-muted-foreground">
              {message || 'To enable cluster-wide node monitoring, configure Redis in your admin config file.'}
            </p>
            <div className="rounded-md bg-muted p-3 font-mono text-sm">
              <p className="text-muted-foreground"># Add to your fcp-admin config file:</p>
              <p>[cluster]</p>
              <p>redis_url = "redis://localhost:6379"</p>
              <p>key_prefix = "fcp:"</p>
            </div>
            <p className="text-sm text-muted-foreground">
              When other FCP services (call-manager, SBC, etc.) are running with cluster mode enabled,
              they write heartbeats to Redis. This view reads those heartbeats to show all running instances.
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function StatCard({
  icon: Icon,
  label,
  value,
  color,
}: {
  icon: React.ComponentType<{ className?: string }>
  label: string
  value: number
  color?: string
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
          <Icon className="h-4 w-4" />
          {label}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <p className={`text-2xl font-bold ${color ?? ''}`}>{value}</p>
      </CardContent>
    </Card>
  )
}

function LocalNodeInfo({
  status,
  summary,
}: {
  status?: { status: string; version: string; uptime_seconds: number; users_count: number; active_registrations_count: number }
  summary?: DashboardSummary
}) {
  return (
    <>
      {/* Local System Status */}
      {status && (
        <div>
          <h2 className="text-lg font-semibold mb-3">Local Admin Node</h2>
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
                  <Activity className="h-4 w-4" />
                  Status
                </CardTitle>
              </CardHeader>
              <CardContent>
                <Badge variant="success" className="text-lg px-3 py-1">{status.status}</Badge>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
                  <Server className="h-4 w-4" />
                  Version
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold font-mono">{status.version}</p>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
                  <Clock className="h-4 w-4" />
                  Uptime
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">{formatUptime(status.uptime_seconds)}</p>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground">Registrations</CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">{status.active_registrations_count}</p>
              </CardContent>
            </Card>
          </div>
        </div>
      )}

      {/* Resource Summary */}
      {summary && (
        <div>
          <h2 className="text-lg font-semibold mb-3">Resource Summary</h2>
          <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
            <MiniStatCard label="Active Calls" value={summary.active_calls} />
            <MiniStatCard label="Registered Devices" value={summary.registered_devices} />
            <MiniStatCard label="Total Users" value={summary.total_users} />
            <MiniStatCard label="Total Extensions" value={summary.total_extensions} />
            <MiniStatCard label="Phone Numbers" value={summary.total_phone_numbers} />
            <MiniStatCard label="Calls Today" value={summary.calls_today} />
          </div>
        </div>
      )}
    </>
  )
}

function MiniStatCard({ label, value }: { label: string; value: number }) {
  return (
    <Card>
      <CardContent className="p-4 text-center">
        <p className="text-2xl font-bold">{value}</p>
        <p className="text-xs text-muted-foreground">{label}</p>
      </CardContent>
    </Card>
  )
}
