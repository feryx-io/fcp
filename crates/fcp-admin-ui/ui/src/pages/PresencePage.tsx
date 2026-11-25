// Presence (docs/PRESENCE.md): every person in the tenant and what others
// see of them — what they chose, a call, DND, idle, offline when none of
// their phones is registered — as the admin works it out. Until v0.25.645
// this page listed the admin process's own presence table, which phones
// subscribing to the call managers never fill (D8 finding 69).
import { useQuery } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { useState } from 'react'
import { presence } from '@/api/endpoints'
import type { PersonPresence } from '@/types'

const STATES: Record<string, { label: string; badge: 'success' | 'warning' | 'secondary' | 'destructive' | 'outline'; dot: string }> = {
  available: { label: 'Available', badge: 'success', dot: 'bg-green-500' },
  away: { label: 'Away', badge: 'secondary', dot: 'bg-orange-500' },
  busy: { label: 'Busy', badge: 'warning', dot: 'bg-yellow-500' },
  dnd: { label: 'Do not disturb', badge: 'destructive', dot: 'bg-red-500' },
  ringing: { label: 'Ringing', badge: 'warning', dot: 'bg-yellow-500' },
  on_call: { label: 'On a call', badge: 'warning', dot: 'bg-yellow-500' },
  offline: { label: 'Offline', badge: 'outline', dot: 'bg-muted-foreground' },
}
const ORDER = ['available', 'on_call', 'ringing', 'busy', 'away', 'dnd', 'offline']

const stateOf = (p: PersonPresence) => STATES[p.state ?? ''] ?? { label: 'Unknown', badge: 'outline' as const, dot: 'bg-muted-foreground' }

export function PresencePage() {
  const [search, setSearch] = useState('')
  const [only, setOnly] = useState<string | null>(null)

  const { data: people = [], isLoading, refetch } = useQuery({
    queryKey: ['presence', 'people'],
    queryFn: presence.people,
    refetchInterval: 5000,
  })

  const needle = search.trim().toLowerCase()
  const filtered = people
    .filter((p) => !only || p.state === only)
    .filter((p) => !needle || [p.username, p.display_name, p.extension, p.label, p.note]
      .some((f) => f?.toLowerCase().includes(needle)))
    .sort((a, b) =>
      ORDER.indexOf(a.state ?? '') - ORDER.indexOf(b.state ?? '')
      || (a.display_name ?? a.username).localeCompare(b.display_name ?? b.username))

  const counts: Record<string, number> = {}
  for (const p of people) counts[p.state ?? 'unknown'] = (counts[p.state ?? 'unknown'] ?? 0) + 1

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Presence</h1>
          <p className="text-muted-foreground">
            What everyone in the tenant shows to the others: their status, calls, do not disturb and
            whether a phone of theirs is registered
          </p>
        </div>
        <Button variant="outline" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      <div className="grid gap-4 md:grid-cols-4 lg:grid-cols-7">
        {ORDER.filter((s) => counts[s]).map((s) => (
          <Card
            key={s}
            className={`cursor-pointer hover:bg-accent/50 ${only === s ? 'border-primary' : ''}`}
            onClick={() => setOnly(only === s ? null : s)}
          >
            <CardContent className="p-4 text-center">
              <p className="text-2xl font-bold">{counts[s]}</p>
              <p className="text-xs text-muted-foreground">{STATES[s].label}</p>
            </CardContent>
          </Card>
        ))}
      </div>

      <div className="max-w-sm">
        <Input
          placeholder="Search by name, extension or note..."
          aria-label="Search people"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : filtered.length === 0 ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">
          {people.length === 0 ? 'Nobody in this tenant yet' : 'Nobody matches'}
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
          {filtered.map((p) => <PersonCard key={p.user_id} person={p} />)}
        </div>
      )}
    </div>
  )
}

function PersonCard({ person: p }: { person: PersonPresence }) {
  const name = p.display_name || p.username
  const s = stateOf(p)
  return (
    <Card className="hover:border-primary/30 transition-colors" data-testid="person-presence">
      <CardContent className="p-4">
        <div className="flex items-start gap-3">
          <div className="relative">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-sm font-bold">
              {name.charAt(0).toUpperCase()}
            </div>
            <span className={`absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-background ${s.dot}`} />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium truncate">{name}</p>
            <p className="text-xs text-muted-foreground truncate">
              {p.extension ? `Ext ${p.extension}` : p.username}
            </p>
            <div className="flex flex-wrap items-center gap-1 mt-1">
              <Badge variant={s.badge} className="text-xs">{s.label}</Badge>
              {p.idle && <Badge variant="outline" className="text-xs">Idle</Badge>}
              {p.label && <span className="text-xs text-muted-foreground truncate">{p.label}</span>}
            </div>
            {p.note && <p className="text-xs text-muted-foreground mt-1 truncate">{p.note}</p>}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
