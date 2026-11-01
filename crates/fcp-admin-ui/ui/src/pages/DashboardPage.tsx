import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Users, Phone, MonitorSmartphone, PhoneCall, HeartPulse } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { dashboard, services, system } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth'

export function DashboardPage() {
  const { data: summary } = useQuery({
    queryKey: ['dashboard', 'summary'],
    queryFn: dashboard.summary,
    refetchInterval: 10_000,
  })

  const { data: status } = useQuery({
    queryKey: ['system', 'status'],
    queryFn: system.status,
    refetchInterval: 30_000,
  })

  // The platform's services (docs/SERVICES.md §3), for its administrators.
  const platformAdmin = useAuthStore((s) => s.user?.role === 'super_admin')
  const { data: serviceSummary } = useQuery({
    queryKey: ['services', 'summary'],
    queryFn: services.summary,
    refetchInterval: 10_000,
    enabled: platformAdmin,
  })

  const cards = [
    {
      title: 'Active Calls',
      value: summary?.active_calls ?? '-',
      icon: PhoneCall,
      description: `${summary?.calls_today ?? 0} calls today`,
    },
    {
      title: 'Registered Devices',
      value: summary?.registered_devices ?? '-',
      icon: MonitorSmartphone,
      description: 'Currently online',
    },
    {
      title: 'Total Users',
      value: summary?.total_users ?? '-',
      icon: Users,
      description: `${summary?.total_extensions ?? 0} extensions`,
    },
    {
      title: 'Phone Numbers',
      value: summary?.total_phone_numbers ?? '-',
      icon: Phone,
      description: summary?.avg_call_duration_secs
        ? `Avg duration: ${formatDuration(summary.avg_call_duration_secs)}`
        : 'No calls yet',
    },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">
          System overview and key metrics
          {status && (
            <span className="ml-2">
              &middot; Uptime: {formatDuration(status.uptime_seconds)} &middot; v{status.version}
            </span>
          )}
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        {cards.map((card) => {
          const Icon = card.icon
          return (
            <Card key={card.title}>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">{card.title}</CardTitle>
                <Icon className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{card.value}</div>
                <p className="text-xs text-muted-foreground">{card.description}</p>
              </CardContent>
            </Card>
          )
        })}
      </div>

      {platformAdmin && serviceSummary && serviceSummary.total > 0 && (
        <ServicesTile summary={serviceSummary} />
      )}
    </div>
  )
}

function ServicesTile({ summary }: { summary: import('@/api/endpoints').ServicesSummary }) {
  const notServing = summary.total - summary.serving
  const href = notServing > 0 ? '/admin/services?state=attention' : '/admin/services'
  return (
    <Link to={href} className="block max-w-sm">
      <Card className={notServing > 0 ? 'border-yellow-500 hover:bg-accent/50' : 'hover:bg-accent/50'}>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">Services</CardTitle>
          <HeartPulse className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold">
            {summary.serving} of {summary.total} serving
          </div>
          <p className="text-xs text-muted-foreground">
            {notServing === 0
              ? 'Every service is serving'
              : [
                  summary.down > 0 && `${summary.down} down`,
                  summary.attention > 0 && `${summary.attention} need attention`,
                  summary.unknown > 0 && `${summary.unknown} unknown`,
                ].filter(Boolean).join(', ')}
            {!summary.redis_reachable && ' · Redis cannot be read'}
          </p>
        </CardContent>
      </Card>
    </Link>
  )
}
