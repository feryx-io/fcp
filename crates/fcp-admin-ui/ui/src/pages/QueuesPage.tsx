import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  Users, Phone, Clock, TrendingUp, ChevronRight,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { queues } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { QueueStatsResponse } from '@/types'

const agentStateBadge = (state: string) => {
  switch (state) {
    case 'available': return 'success' as const
    case 'on_call': case 'busy': return 'warning' as const
    case 'paused': case 'wrap_up': return 'secondary' as const
    case 'offline': return 'destructive' as const
    default: return 'outline' as const
  }
}

export function QueuesPage() {
  const [selectedQueue, setSelectedQueue] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: queueList = [], isLoading } = useQuery({
    queryKey: ['queues'],
    queryFn: queues.list,
    refetchInterval: 5000,
  })

  const { data: queueStats } = useQuery({
    queryKey: ['queue-stats', selectedQueue],
    queryFn: () => queues.stats(selectedQueue!),
    enabled: !!selectedQueue,
    refetchInterval: 3000,
  })

  const { data: agents = [] } = useQuery({
    queryKey: ['queue-agents', selectedQueue],
    queryFn: () => queues.agents(selectedQueue!),
    enabled: !!selectedQueue,
    refetchInterval: 5000,
  })

  const pauseAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) =>
      queues.pauseAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  const resumeAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) =>
      queues.resumeAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  const logoutAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) =>
      queues.logoutAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  if (isLoading) {
    return <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Queues</h1>
        <p className="text-muted-foreground">Monitor ACD queues and manage agents</p>
      </div>

      {/* Queue Cards */}
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        {queueList.map((q) => (
          <Card
            key={q.name}
            className="cursor-pointer hover:border-primary/50 transition-colors"
            onClick={() => setSelectedQueue(q.name)}
          >
            <CardHeader className="pb-2">
              <CardTitle className="flex items-center justify-between text-base">
                <span>{q.display_name || q.name}</span>
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              </CardTitle>
              <p className="text-xs text-muted-foreground font-mono">{q.extension}</p>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-3 gap-2 text-center">
                <div>
                  <p className="text-2xl font-bold">{q.calls_waiting}</p>
                  <p className="text-xs text-muted-foreground">Waiting</p>
                </div>
                <div>
                  <p className="text-2xl font-bold">{q.agents_configured}</p>
                  <p className="text-xs text-muted-foreground">Agents</p>
                </div>
                <div>
                  <Badge variant="outline" className="text-xs">{q.strategy}</Badge>
                  <p className="text-xs text-muted-foreground mt-1">Strategy</p>
                </div>
              </div>
            </CardContent>
          </Card>
        ))}

        {queueList.length === 0 && (
          <div className="col-span-full flex h-32 items-center justify-center text-muted-foreground">
            No queues configured
          </div>
        )}
      </div>

      {/* Queue Detail Dialog */}
      <Dialog open={!!selectedQueue} onOpenChange={() => setSelectedQueue(null)}>
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Queue: {selectedQueue}</DialogTitle>
            <DialogDescription>Real-time queue statistics and agent management.</DialogDescription>
          </DialogHeader>

          {queueStats && <QueueStatsPanel stats={queueStats} />}

          {/* Agents */}
          <div className="space-y-3 mt-4">
            <h3 className="font-semibold text-sm">Agents ({agents.length})</h3>
            {agents.length === 0 ? (
              <p className="text-sm text-muted-foreground">No agents logged in</p>
            ) : (
              <div className="space-y-2">
                {agents.map((agent) => (
                  <div key={agent.id} className="flex items-center justify-between p-3 rounded-lg border">
                    <div className="flex items-center gap-3">
                      <Badge variant={agentStateBadge(agent.state)}>{agent.state}</Badge>
                      <div>
                        <p className="text-sm font-medium">{agent.extension}</p>
                        <p className="text-xs text-muted-foreground">
                          {agent.calls_handled} calls handled | Priority: {agent.priority}
                        </p>
                      </div>
                    </div>
                    <div className="flex gap-1">
                      {agent.state === 'available' && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => pauseAgentMutation.mutate({
                            name: selectedQueue!, agentId: agent.id,
                          })}
                        >
                          Pause
                        </Button>
                      )}
                      {agent.state === 'paused' && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => resumeAgentMutation.mutate({
                            name: selectedQueue!, agentId: agent.id,
                          })}
                        >
                          Resume
                        </Button>
                      )}
                      <Button
                        variant="outline"
                        size="sm"
                        className="text-destructive"
                        onClick={() => {
                          if (confirm(`Log out agent ${agent.extension}?`)) {
                            logoutAgentMutation.mutate({
                              name: selectedQueue!, agentId: agent.id,
                            })
                          }
                        }}
                      >
                        Logout
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function QueueStatsPanel({ stats }: { stats: QueueStatsResponse }) {
  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
      <StatCard
        icon={Phone}
        label="Calls Waiting"
        value={stats.calls_waiting}
        highlight={stats.calls_waiting > 0}
      />
      <StatCard icon={Users} label="Agents Available" value={stats.agents_available} />
      <StatCard icon={Clock} label="Avg Wait" value={formatDuration(Math.round(stats.avg_wait_time_secs))} />
      <StatCard icon={Clock} label="Longest Wait" value={formatDuration(Math.round(stats.longest_wait_secs))} />
      <StatCard icon={TrendingUp} label="Handled Today" value={stats.calls_handled_today} />
      <StatCard icon={Phone} label="Abandoned Today" value={stats.calls_abandoned_today} />
      <StatCard icon={Users} label="On Call" value={stats.agents_on_call} />
      <StatCard
        icon={TrendingUp}
        label="Service Level"
        value={`${Math.round(stats.service_level_pct)}%`}
      />
    </div>
  )
}

function StatCard({ icon: Icon, label, value, highlight }: {
  icon: React.ElementType
  label: string
  value: string | number
  highlight?: boolean
}) {
  return (
    <div className="rounded-lg border p-3 text-center">
      <Icon className={`h-4 w-4 mx-auto mb-1 ${highlight ? 'text-warning' : 'text-muted-foreground'}`} />
      <p className="text-xl font-bold">{value}</p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  )
}
