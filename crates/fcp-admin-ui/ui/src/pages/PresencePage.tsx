import { useQuery } from '@tanstack/react-query'
import { Eye, RefreshCw } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { useState } from 'react'
import { presence } from '@/api/endpoints'
import type { PresenceResponse } from '@/types'

const presenceStateBadge = (state: string) => {
  switch (state) {
    case 'open': case 'available': return 'success' as const
    case 'busy': case 'on-the-phone': return 'warning' as const
    case 'away': return 'secondary' as const
    case 'dnd': case 'do-not-disturb': return 'destructive' as const
    case 'closed': case 'offline': return 'outline' as const
    default: return 'outline' as const
  }
}

const presenceStateLabel = (state: string) => {
  switch (state) {
    case 'open': return 'Available'
    case 'closed': return 'Offline'
    case 'on-the-phone': return 'On Phone'
    case 'dnd': return 'DND'
    default: return state.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
  }
}

const presenceStateColor = (state: string) => {
  switch (state) {
    case 'open': case 'available': return 'bg-green-500'
    case 'busy': case 'on-the-phone': return 'bg-yellow-500'
    case 'away': return 'bg-orange-500'
    case 'dnd': case 'do-not-disturb': return 'bg-red-500'
    default: return 'bg-muted-foreground'
  }
}

export function PresencePage() {
  const [search, setSearch] = useState('')

  const { data: presenceList = [], isLoading, refetch } = useQuery({
    queryKey: ['presence'],
    queryFn: presence.list,
    refetchInterval: 5000,
  })

  const { data: stats } = useQuery({
    queryKey: ['presence-stats'],
    queryFn: presence.stats,
    refetchInterval: 10000,
  })

  const filtered = search
    ? presenceList.filter((p) =>
        p.aor.toLowerCase().includes(search.toLowerCase()) ||
        (p.note?.toLowerCase().includes(search.toLowerCase()))
      )
    : presenceList

  // Group by state for summary
  const stateCounts: Record<string, number> = {}
  presenceList.forEach((p) => {
    const key = presenceStateLabel(p.state)
    stateCounts[key] = (stateCounts[key] || 0) + 1
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Presence</h1>
          <p className="text-muted-foreground">
            Real-time user presence status
            {stats && (
              <span className="ml-2 text-xs">
                ({stats.total_presentities} users, {stats.total_watchers} watchers)
              </span>
            )}
          </p>
        </div>
        <Button variant="outline" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {/* Summary Cards */}
      <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
        {Object.entries(stateCounts).map(([state, count]) => (
          <Card key={state}>
            <CardContent className="p-4 text-center">
              <p className="text-2xl font-bold">{count}</p>
              <p className="text-xs text-muted-foreground">{state}</p>
            </CardContent>
          </Card>
        ))}
      </div>

      {/* Search */}
      <div className="max-w-sm">
        <Input
          placeholder="Search by address..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {/* Presence Grid */}
      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : filtered.length === 0 ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">
          {search ? 'No matching users' : 'No presence data available'}
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
          {filtered.map((p) => (
            <PresenceCard key={p.aor} presence={p} />
          ))}
        </div>
      )}
    </div>
  )
}

function PresenceCard({ presence: p }: { presence: PresenceResponse }) {
  // Extract user part from AOR (e.g., "alice@example.com" → "alice")
  const userName = p.aor.split('@')[0] || p.aor
  const label = p.display_name || userName

  return (
    <Card className="hover:border-primary/30 transition-colors">
      <CardContent className="p-4">
        <div className="flex items-start gap-3">
          <div className="relative">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-sm font-bold">
              {p.extension ? `x${p.extension.slice(-2)}` : label.charAt(0).toUpperCase()}
            </div>
            <span
              className={`absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-background ${presenceStateColor(p.state)}`}
            />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium truncate">{label}</p>
            {p.extension && (
              <p className="text-xs text-muted-foreground truncate">Ext {p.extension}</p>
            )}
            {!p.extension && (
              <p className="text-xs text-muted-foreground truncate">{p.aor}</p>
            )}
            <div className="flex items-center gap-2 mt-1">
              <Badge variant={presenceStateBadge(p.state)} className="text-xs">
                {presenceStateLabel(p.state)}
              </Badge>
              {p.watcher_count > 0 && (
                <span className="text-xs text-muted-foreground flex items-center gap-0.5">
                  <Eye className="h-3 w-3" />
                  {p.watcher_count}
                </span>
              )}
            </div>
            {p.note && (
              <p className="text-xs text-muted-foreground mt-1 truncate">{p.note}</p>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
