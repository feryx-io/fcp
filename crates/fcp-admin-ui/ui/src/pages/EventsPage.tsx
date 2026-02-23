import { useState, useEffect, useRef } from 'react'
import { Activity, Pause, Play, Trash2, Filter } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { useWebSocketStore } from '@/stores/websocket'
import type { PlatformEvent } from '@/types'

type BadgeVariant = 'default' | 'secondary' | 'destructive' | 'outline' | 'success' | 'warning'

const EVENT_TYPE_CONFIG: Record<string, { label: string; variant: BadgeVariant }> = {
  call: { label: 'Call', variant: 'default' },
  media: { label: 'Media', variant: 'secondary' },
  registration: { label: 'Registration', variant: 'success' },
  system: { label: 'System', variant: 'outline' },
  queue: { label: 'Queue', variant: 'warning' },
  presence: { label: 'Presence', variant: 'secondary' },
  voicemail: { label: 'Voicemail', variant: 'secondary' },
  conference: { label: 'Conference', variant: 'default' },
  recording: { label: 'Recording', variant: 'secondary' },
  flow: { label: 'Flow', variant: 'secondary' },
  ivr: { label: 'IVR', variant: 'secondary' },
  user: { label: 'User', variant: 'outline' },
}

function getEventConfig(eventType: string) {
  return EVENT_TYPE_CONFIG[eventType] ?? { label: eventType, variant: 'outline' as BadgeVariant }
}

function formatEventSummary(event: PlatformEvent): string {
  const e = event as Record<string, unknown>
  const type = (e.type as string) ?? ''

  switch (event.event_type) {
    case 'call': {
      const callId = ((e.call_id as string) ?? '').slice(0, 16)
      const from = (e.from as string) ?? ''
      const to = (e.to as string) ?? ''
      switch (type) {
        case 'initiated': return `${extractUser(from)} -> ${extractUser(to)} (${callId})`
        case 'ringing': return `Ringing: ${callId}`
        case 'answered': return `Answered: ${callId} [${e.codec ?? ''}]`
        case 'ended': {
          const dur = e.duration_ms ? `${Math.round((e.duration_ms as number) / 1000)}s` : ''
          return `Ended: ${callId} ${dur} (${e.hangup_cause ?? ''})`
        }
        case 'failed': return `Failed: ${callId} ${e.sip_code ?? ''} ${e.reason ?? ''}`
        default: return `${type}: ${callId}`
      }
    }
    case 'registration': {
      const aor = (e.aor as string) ?? ''
      const contact = (e.contact as string) ?? ''
      switch (type) {
        case 'registered': return `${extractUser(aor)} registered from ${extractAddr(contact)}`
        case 'unregistered': return `${extractUser(aor)} unregistered`
        case 'expired': return `${extractUser(aor)} registration expired`
        default: return `${type}: ${aor}`
      }
    }
    case 'system': {
      const nodeId = (e.node_id as string) ?? ''
      switch (type) {
        case 'started': return `Node ${nodeId} started`
        case 'stopped': return `Node ${nodeId} stopped`
        case 'health_check': return `Health check: ${nodeId}`
        default: return `${type}: ${nodeId}`
      }
    }
    case 'media': {
      const callId = ((e.call_id as string) ?? '').slice(0, 16)
      switch (type) {
        case 'session_started': return `Media session started: ${callId} [${e.codec ?? ''}]`
        case 'session_ended': return `Media session ended: ${callId}`
        case 'dtmf_received': return `DTMF '${e.digit ?? ''}' on ${callId}`
        case 'quality_warning': return `Quality warning: ${callId} (loss: ${e.packet_loss ?? '?'}%)`
        default: return `${type}: ${callId}`
      }
    }
    case 'queue': {
      const queue = (e.queue_name as string) ?? ''
      switch (type) {
        case 'call_enqueued': return `Call enqueued in ${queue}`
        case 'call_answered': return `Call answered from ${queue}`
        case 'agent_logged_in': return `Agent ${e.agent_id ?? ''} logged into ${queue}`
        case 'agent_logged_out': return `Agent ${e.agent_id ?? ''} logged out of ${queue}`
        default: return `${type}: ${queue}`
      }
    }
    case 'presence': {
      const aor = (e.aor as string) ?? ''
      return `${type}: ${extractUser(aor)}`
    }
    default: {
      // Generic: show type and any identifying field
      const id = (e.call_id as string) ?? (e.node_id as string) ?? (e.aor as string) ?? ''
      return `${type}${id ? ': ' + id.slice(0, 20) : ''}`
    }
  }
}

function extractUser(uri: string): string {
  const match = uri.match(/(?:sip:)?([^@;>]+)@/)
  return match ? match[1] : uri
}

function extractAddr(contact: string): string {
  const match = contact.match(/<sip:[^@]+@([^;>]+)/)
  return match ? match[1] : contact
}

function formatTimestamp(event: PlatformEvent): string {
  const e = event as Record<string, unknown>
  // Try various timestamp fields
  const ts = (e.timestamp as number) ?? (e.end_time as number) ?? (e.answer_time as number) ?? 0
  if (ts > 0) {
    return new Date(ts * 1000).toLocaleTimeString()
  }
  return new Date().toLocaleTimeString()
}

export function EventsPage() {
  const { events, connected } = useWebSocketStore()
  const [paused, setPaused] = useState(false)
  const [filters, setFilters] = useState<Set<string>>(new Set())
  const [showFilters, setShowFilters] = useState(false)
  const [pausedEvents, setPausedEvents] = useState<PlatformEvent[]>([])
  const scrollRef = useRef<HTMLDivElement>(null)

  // When paused, freeze the display; when unpaused, sync back
  const displayEvents = paused ? pausedEvents : events

  useEffect(() => {
    if (paused && pausedEvents.length === 0 && events.length > 0) {
      setPausedEvents(events)
    }
  }, [paused, pausedEvents, events])

  const handlePause = () => {
    if (!paused) {
      setPausedEvents(events)
    }
    setPaused(!paused)
  }

  const handleClear = () => {
    // Can't clear the store since other components use it,
    // but we can clear the paused snapshot
    setPausedEvents([])
    setPaused(true)
  }

  const toggleFilter = (eventType: string) => {
    const next = new Set(filters)
    if (next.has(eventType)) {
      next.delete(eventType)
    } else {
      next.add(eventType)
    }
    setFilters(next)
  }

  const filteredEvents = filters.size > 0
    ? displayEvents.filter((e) => filters.has(e.event_type))
    : displayEvents

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Events</h1>
          <p className="text-muted-foreground">
            Live event stream from the cluster
            <span className={`ml-2 inline-flex items-center gap-1 text-xs ${connected ? 'text-green-600' : 'text-muted-foreground'}`}>
              <span className={`h-2 w-2 rounded-full ${connected ? 'bg-green-500 animate-pulse' : 'bg-muted-foreground'}`} />
              {connected ? (paused ? 'Paused' : 'Live') : 'Disconnected'}
            </span>
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => setShowFilters(!showFilters)}>
            <Filter className="mr-2 h-4 w-4" />
            Filter {filters.size > 0 && `(${filters.size})`}
          </Button>
          <Button variant="outline" size="sm" onClick={handlePause}>
            {paused ? <Play className="mr-2 h-4 w-4" /> : <Pause className="mr-2 h-4 w-4" />}
            {paused ? 'Resume' : 'Pause'}
          </Button>
          <Button variant="outline" size="sm" onClick={handleClear}>
            <Trash2 className="mr-2 h-4 w-4" /> Clear
          </Button>
        </div>
      </div>

      {/* Filter bar */}
      {showFilters && (
        <div className="flex flex-wrap gap-2 p-3 border rounded-md bg-muted/50">
          {Object.entries(EVENT_TYPE_CONFIG).map(([type, config]) => {
            const active = filters.has(type)
            const count = events.filter((e) => e.event_type === type).length
            return (
              <button
                key={type}
                onClick={() => toggleFilter(type)}
                className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                  active
                    ? 'bg-primary text-primary-foreground'
                    : 'bg-background border hover:bg-accent'
                }`}
              >
                {config.label}
                {count > 0 && <span className="opacity-70">({count})</span>}
              </button>
            )
          })}
          {filters.size > 0 && (
            <button
              onClick={() => setFilters(new Set())}
              className="inline-flex items-center gap-1 rounded-full px-3 py-1 text-xs text-muted-foreground hover:text-foreground"
            >
              Clear filters
            </button>
          )}
        </div>
      )}

      {/* Event list */}
      <div
        ref={scrollRef}
        className="border rounded-md bg-card overflow-auto"
        style={{ maxHeight: 'calc(100vh - 240px)' }}
      >
        {filteredEvents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
            <Activity className="h-10 w-10 mb-3 opacity-40" />
            <p className="text-sm">
              {connected ? 'Waiting for events...' : 'WebSocket disconnected. Events will appear when connected.'}
            </p>
          </div>
        ) : (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-card border-b">
              <tr className="text-left text-muted-foreground">
                <th className="px-3 py-2 w-24">Time</th>
                <th className="px-3 py-2 w-28">Type</th>
                <th className="px-3 py-2 w-24">Action</th>
                <th className="px-3 py-2">Details</th>
                <th className="px-3 py-2 w-28">Node</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {filteredEvents.map((event, i) => {
                const e = event as Record<string, unknown>
                const config = getEventConfig(event.event_type)
                const type = (e.type as string) ?? ''
                return (
                  <tr key={i} className="hover:bg-muted/50 transition-colors">
                    <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground whitespace-nowrap">
                      {formatTimestamp(event)}
                    </td>
                    <td className="px-3 py-1.5">
                      <Badge variant={config.variant} className="text-xs">
                        {config.label}
                      </Badge>
                    </td>
                    <td className="px-3 py-1.5 text-xs font-medium">
                      {type.replace(/_/g, ' ')}
                    </td>
                    <td className="px-3 py-1.5 text-xs font-mono truncate max-w-md">
                      {formatEventSummary(event)}
                    </td>
                    <td className="px-3 py-1.5 text-xs text-muted-foreground">
                      {(e.node_id as string) ?? '-'}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* Footer stats */}
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          Showing {filteredEvents.length} of {displayEvents.length} events
          {paused && ` (paused, ${events.length - pausedEvents.length} new)`}
        </span>
        <span>Buffer: 200 max</span>
      </div>
    </div>
  )
}
