import { useQuery } from '@tanstack/react-query'
import { Users, Phone, MonitorSmartphone, PhoneCall } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { dashboard, system } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'

export function DashboardPage() {
  const { data: summary } = useQuery({
    queryKey: ['dashboard', 'summary'],
    queryFn: dashboard.summary,
    refetchInterval: 10_000,
  })

  const { data: health } = useQuery({
    queryKey: ['system', 'health'],
    queryFn: system.health,
    refetchInterval: 30_000,
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
          {health && (
            <span className="ml-2">
              &middot; Uptime: {formatDuration(health.uptime_secs)} &middot; v{health.version}
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
    </div>
  )
}
