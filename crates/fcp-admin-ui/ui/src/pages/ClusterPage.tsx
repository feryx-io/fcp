import { useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  Server, Activity, Clock, AlertTriangle, Phone,
  RefreshCw, ArrowUpDown, Info,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { DataTable } from '@/components/DataTable'
import { cluster, dashboard } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { ClusterNodeInfo, DashboardSummary } from '@/types'

const statusBadge = (status: string) => {
  switch (status) {
    case 'healthy': return 'success' as const
    case 'degraded': return 'warning' as const
    case 'offline': return 'destructive' as const
    default: return 'outline' as const
  }
}

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
      <Badge variant={statusBadge(row.original.status)}>
        {row.original.status}
      </Badge>
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

          {/* Summary Stats Cards */}
          {stats && (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-5">
              <StatCard icon={Server} label="Total Nodes" value={stats.total_nodes} />
              <StatCard icon={Activity} label="Healthy" value={stats.healthy_nodes} color="text-green-600" />
              <StatCard icon={AlertTriangle} label="Degraded" value={stats.degraded_nodes} color="text-yellow-600" />
              <StatCard icon={AlertTriangle} label="Offline" value={stats.offline_nodes} color="text-red-600" />
              <StatCard icon={Phone} label="Total Active Calls" value={stats.total_active_calls} />
            </div>
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
