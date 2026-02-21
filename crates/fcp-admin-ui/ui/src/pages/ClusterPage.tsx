import { useQuery } from '@tanstack/react-query'
import {
  Server, Activity, Clock, Users, RefreshCw,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { cluster, dashboard } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'

export function ClusterPage() {
  const { data: status, isLoading, refetch } = useQuery({
    queryKey: ['cluster-status'],
    queryFn: cluster.status,
    refetchInterval: 10000,
  })

  const { data: health } = useQuery({
    queryKey: ['cluster-health'],
    queryFn: cluster.health,
    refetchInterval: 10000,
  })

  const { data: summary } = useQuery({
    queryKey: ['dashboard-summary'],
    queryFn: dashboard.summary,
    refetchInterval: 10000,
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Cluster Monitor</h1>
          <p className="text-muted-foreground">System health and node status</p>
        </div>
        <Button variant="outline" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <>
          {/* Health Status */}
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
                  <Activity className="h-4 w-4" />
                  System Status
                </CardTitle>
              </CardHeader>
              <CardContent>
                <Badge
                  variant={health?.status === 'ok' ? 'success' : 'destructive'}
                  className="text-lg px-3 py-1"
                >
                  {health?.status ?? 'unknown'}
                </Badge>
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
                <p className="text-2xl font-bold font-mono">{health?.version ?? '-'}</p>
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
                <p className="text-2xl font-bold">
                  {status?.uptime_seconds != null ? formatDuration(status.uptime_seconds) : '-'}
                </p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm text-muted-foreground flex items-center gap-2">
                  <Users className="h-4 w-4" />
                  Registrations
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">{status?.active_registrations_count ?? '-'}</p>
              </CardContent>
            </Card>
          </div>

          {/* Resource Summary */}
          {summary && (
            <div>
              <h2 className="text-lg font-semibold mb-3">Resource Summary</h2>
              <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
                <StatCard label="Active Calls" value={summary.active_calls} />
                <StatCard label="Registered Devices" value={summary.registered_devices} />
                <StatCard label="Total Users" value={summary.total_users} />
                <StatCard label="Total Extensions" value={summary.total_extensions} />
                <StatCard label="Phone Numbers" value={summary.total_phone_numbers} />
                <StatCard label="Calls Today" value={summary.calls_today} />
              </div>
            </div>
          )}

          {/* System Details */}
          {status && (
            <div>
              <h2 className="text-lg font-semibold mb-3">System Details</h2>
              <Card>
                <CardContent className="p-4">
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                    <div>
                      <p className="text-muted-foreground text-xs">Status</p>
                      <p className="font-medium">{status.status}</p>
                    </div>
                    <div>
                      <p className="text-muted-foreground text-xs">Version</p>
                      <p className="font-mono">{status.version}</p>
                    </div>
                    <div>
                      <p className="text-muted-foreground text-xs">Users</p>
                      <p className="font-medium">{status.users_count}</p>
                    </div>
                    <div>
                      <p className="text-muted-foreground text-xs">Active Registrations</p>
                      <p className="font-medium">{status.active_registrations_count}</p>
                    </div>
                  </div>
                </CardContent>
              </Card>
            </div>
          )}
        </>
      )}
    </div>
  )
}

function StatCard({ label, value }: { label: string; value: number }) {
  return (
    <Card>
      <CardContent className="p-4 text-center">
        <p className="text-2xl font-bold">{value}</p>
        <p className="text-xs text-muted-foreground">{label}</p>
      </CardContent>
    </Card>
  )
}
