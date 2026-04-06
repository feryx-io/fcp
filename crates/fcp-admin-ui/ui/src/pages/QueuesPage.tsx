import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useForm } from 'react-hook-form'
import {
  Users, Phone, Clock, TrendingUp, ChevronRight, Plus, Pencil, Trash2, RefreshCw,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { queues, queueConfigs } from '@/api/endpoints'
import { formatDuration } from '@/lib/utils'
import type { QueueStatsResponse, QueueConfigResponse, CreateQueueConfigRequest } from '@/types'

const agentStateBadge = (state: string) => {
  switch (state) {
    case 'available': return 'success' as const
    case 'on_call': case 'busy': return 'warning' as const
    case 'paused': case 'wrap_up': return 'secondary' as const
    case 'offline': return 'destructive' as const
    default: return 'outline' as const
  }
}

interface QueueFormData {
  name: string
  extension: string
  strategy: string
  max_wait_time_secs: number
  max_queue_depth: number
  overflow_destination: string
  moh_class: string
  announce_position: boolean
  announce_interval_secs: number
  retry_on_no_answer: boolean
  agents: string
  priority: number
  required_skills: string
  record_calls: boolean
  callback_enabled: boolean
  timezone: string
  enabled: boolean
}

const defaultFormValues: QueueFormData = {
  name: '',
  extension: '',
  strategy: 'round_robin',
  max_wait_time_secs: 300,
  max_queue_depth: 50,
  overflow_destination: '',
  moh_class: '',
  announce_position: true,
  announce_interval_secs: 30,
  retry_on_no_answer: true,
  agents: '',
  priority: 0,
  required_skills: '',
  record_calls: false,
  callback_enabled: false,
  timezone: 'UTC',
  enabled: true,
}

function configToForm(c: QueueConfigResponse): QueueFormData {
  return {
    name: c.name,
    extension: c.extension,
    strategy: c.strategy,
    max_wait_time_secs: c.max_wait_time_secs,
    max_queue_depth: c.max_queue_depth,
    overflow_destination: c.overflow_destination ?? '',
    moh_class: c.moh_class ?? '',
    announce_position: c.announce_position,
    announce_interval_secs: c.announce_interval_secs,
    retry_on_no_answer: c.retry_on_no_answer,
    agents: c.agents.join(', '),
    priority: c.priority,
    required_skills: c.required_skills.join(', '),
    record_calls: c.record_calls,
    callback_enabled: c.callback_enabled,
    timezone: c.timezone,
    enabled: c.enabled,
  }
}

function formToRequest(f: QueueFormData): CreateQueueConfigRequest {
  return {
    name: f.name,
    extension: f.extension,
    strategy: f.strategy,
    max_wait_time_secs: f.max_wait_time_secs,
    max_queue_depth: f.max_queue_depth,
    overflow_destination: f.overflow_destination || undefined,
    moh_class: f.moh_class || undefined,
    announce_position: f.announce_position,
    announce_interval_secs: f.announce_interval_secs,
    retry_on_no_answer: f.retry_on_no_answer,
    agents: f.agents ? f.agents.split(',').map((s) => s.trim()).filter(Boolean) : [],
    priority: f.priority,
    required_skills: f.required_skills ? f.required_skills.split(',').map((s) => s.trim()).filter(Boolean) : [],
    record_calls: f.record_calls,
    callback_enabled: f.callback_enabled,
    timezone: f.timezone,
    enabled: f.enabled,
  }
}

export function QueuesPage() {
  const queryClient = useQueryClient()
  const [selectedQueue, setSelectedQueue] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [editConfig, setEditConfig] = useState<QueueConfigResponse | null>(null)
  const [deleteConfig, setDeleteConfig] = useState<QueueConfigResponse | null>(null)
  const [tab, setTab] = useState<'live' | 'config'>('config')

  const createForm = useForm<QueueFormData>({ defaultValues: defaultFormValues })
  const editForm = useForm<QueueFormData>({ defaultValues: defaultFormValues })

  // Live queue data (runtime)
  const { data: queueList = [], isLoading: liveLoading } = useQuery({
    queryKey: ['queues'],
    queryFn: queues.list,
    refetchInterval: 5000,
  })

  // Queue configs (database)
  const { data: configs = [], isLoading: configLoading, isFetching, refetch } = useQuery({
    queryKey: ['queue-configs'],
    queryFn: queueConfigs.list,
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

  const createMutation = useMutation({
    mutationFn: (data: CreateQueueConfigRequest) => queueConfigs.create(data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['queue-configs'] }); setCreateOpen(false); createForm.reset() },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateQueueConfigRequest> }) => queueConfigs.update(id, data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['queue-configs'] }); setEditConfig(null) },
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => queueConfigs.delete(id),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['queue-configs'] }); setDeleteConfig(null) },
  })

  const pauseAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) => queues.pauseAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  const resumeAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) => queues.resumeAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  const logoutAgentMutation = useMutation({
    mutationFn: ({ name, agentId }: { name: string; agentId: string }) => queues.logoutAgent(name, agentId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['queue-agents'] }),
  })

  const openEdit = (config: QueueConfigResponse) => {
    editForm.reset(configToForm(config))
    setEditConfig(config)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Queues</h1>
          <p className="text-muted-foreground">Configure and monitor ACD queues</p>
        </div>
        <div className="flex gap-2">
          <div className="flex border rounded-lg">
            <Button variant={tab === 'config' ? 'default' : 'ghost'} size="sm" onClick={() => setTab('config')}>Configuration</Button>
            <Button variant={tab === 'live' ? 'default' : 'ghost'} size="sm" onClick={() => setTab('live')}>Live Status</Button>
          </div>
          {tab === 'config' && (
            <>
              <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
                <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} /> Refresh
              </Button>
              <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
                <Plus className="mr-2 h-4 w-4" /> Add Queue
              </Button>
            </>
          )}
        </div>
      </div>

      {/* Config Tab */}
      {tab === 'config' && (
        configLoading ? (
          <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
        ) : configs.length === 0 ? (
          <div className="flex flex-col h-48 items-center justify-center text-muted-foreground gap-4">
            <Users className="h-12 w-12 opacity-30" />
            <p>No queues configured yet</p>
            <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
              <Plus className="mr-2 h-4 w-4" /> Create Your First Queue
            </Button>
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {configs.map((c) => (
              <Card key={c.id} className={`hover:border-primary/30 transition-colors ${!c.enabled ? 'opacity-60' : ''}`}>
                <CardHeader className="pb-2">
                  <CardTitle className="flex items-center justify-between text-base">
                    <span>{c.name}</span>
                    <div className="flex items-center gap-1">
                      <Badge variant={c.enabled ? 'success' : 'secondary'}>{c.enabled ? 'Active' : 'Disabled'}</Badge>
                    </div>
                  </CardTitle>
                  <p className="text-xs text-muted-foreground font-mono">Ext: {c.extension}</p>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="grid grid-cols-3 gap-2 text-center text-sm">
                    <div>
                      <p className="text-lg font-bold">{c.agents.length}</p>
                      <p className="text-xs text-muted-foreground">Agents</p>
                    </div>
                    <div>
                      <Badge variant="outline" className="text-xs">{c.strategy.replace('_', ' ')}</Badge>
                      <p className="text-xs text-muted-foreground mt-1">Strategy</p>
                    </div>
                    <div>
                      <p className="text-lg font-bold">{c.max_queue_depth}</p>
                      <p className="text-xs text-muted-foreground">Max Depth</p>
                    </div>
                  </div>
                  <div className="flex items-center justify-between text-xs text-muted-foreground">
                    <div className="flex items-center gap-2">
                      <span>Wait: {c.max_wait_time_secs}s</span>
                      {c.record_calls && <Badge variant="secondary" className="text-xs">REC</Badge>}
                      {c.callback_enabled && <Badge variant="secondary" className="text-xs">CB</Badge>}
                    </div>
                    <div className="flex items-center gap-1">
                      <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => openEdit(c)}>
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                      <Button variant="ghost" size="icon" className="h-7 w-7 text-destructive" onClick={() => setDeleteConfig(c)}>
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        )
      )}

      {/* Live Tab */}
      {tab === 'live' && (
        liveLoading ? (
          <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
        ) : queueList.length === 0 ? (
          <div className="flex h-32 items-center justify-center text-muted-foreground">No queues active</div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {queueList.map((q) => (
              <Card key={q.name} className="cursor-pointer hover:border-primary/50 transition-colors" onClick={() => setSelectedQueue(q.name)}>
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
          </div>
        )
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Create Queue</DialogTitle>
            <DialogDescription>Configure a new ACD queue.</DialogDescription>
          </DialogHeader>
          <QueueForm form={createForm} onSubmit={(d) => createMutation.mutate(formToRequest(d))} isSubmitting={createMutation.isPending} />
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editConfig} onOpenChange={() => setEditConfig(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Queue: {editConfig?.name}</DialogTitle>
            <DialogDescription>Update queue configuration.</DialogDescription>
          </DialogHeader>
          <QueueForm
            form={editForm}
            onSubmit={(d) => editConfig && updateMutation.mutate({ id: editConfig.id, data: formToRequest(d) })}
            isSubmitting={updateMutation.isPending}
            submitLabel="Save Changes"
          />
        </DialogContent>
      </Dialog>

      {/* Delete Dialog */}
      <Dialog open={!!deleteConfig} onOpenChange={() => setDeleteConfig(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Queue</DialogTitle>
            <DialogDescription>Delete <strong>{deleteConfig?.name}</strong>? This cannot be undone.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteConfig(null)}>Cancel</Button>
            <Button variant="destructive" onClick={() => deleteConfig && deleteMutation.mutate(deleteConfig.id)}>Delete</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Live Queue Detail Dialog */}
      <Dialog open={!!selectedQueue} onOpenChange={() => setSelectedQueue(null)}>
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Queue: {selectedQueue}</DialogTitle>
            <DialogDescription>Real-time queue statistics and agent management.</DialogDescription>
          </DialogHeader>

          {queueStats && <QueueStatsPanel stats={queueStats} />}

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
                        <p className="text-xs text-muted-foreground">{agent.calls_handled} calls | Priority: {agent.priority}</p>
                      </div>
                    </div>
                    <div className="flex gap-1">
                      {agent.state === 'available' && (
                        <Button variant="outline" size="sm" onClick={() => pauseAgentMutation.mutate({ name: selectedQueue!, agentId: agent.id })}>Pause</Button>
                      )}
                      {agent.state === 'paused' && (
                        <Button variant="outline" size="sm" onClick={() => resumeAgentMutation.mutate({ name: selectedQueue!, agentId: agent.id })}>Resume</Button>
                      )}
                      <Button variant="outline" size="sm" className="text-destructive" onClick={() => {
                        if (confirm(`Log out agent ${agent.extension}?`)) logoutAgentMutation.mutate({ name: selectedQueue!, agentId: agent.id })
                      }}>Logout</Button>
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

function QueueForm({ form, onSubmit, isSubmitting, submitLabel = 'Create Queue' }: {
  form: ReturnType<typeof useForm<QueueFormData>>
  onSubmit: (data: QueueFormData) => void
  isSubmitting: boolean
  submitLabel?: string
}) {
  const { register, handleSubmit, watch, setValue } = form

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
      {/* Basic */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Basic</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="name">Queue Name</Label>
            <Input id="name" placeholder="e.g. Sales" {...register('name')} />
          </div>
          <div>
            <Label htmlFor="extension">Extension</Label>
            <Input id="extension" placeholder="e.g. 6000" {...register('extension')} />
          </div>
          <div>
            <Label>Distribution Strategy</Label>
            <Select value={watch('strategy')} onValueChange={(v) => setValue('strategy', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="round_robin">Round Robin</SelectItem>
                <SelectItem value="longest_idle">Longest Idle</SelectItem>
                <SelectItem value="fewest_calls">Fewest Calls</SelectItem>
                <SelectItem value="random">Random</SelectItem>
                <SelectItem value="linear">Linear</SelectItem>
                <SelectItem value="skills_based">Skills Based</SelectItem>
                <SelectItem value="weighted">Weighted</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-end">
            <div className="flex items-center gap-2">
              <Switch id="enabled" checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
              <Label htmlFor="enabled">Enabled</Label>
            </div>
          </div>
        </div>
      </div>

      {/* Agents */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Agents</h3>
        <div>
          <Label htmlFor="agents">Agent Extensions (comma-separated)</Label>
          <Input id="agents" placeholder="e.g. 1001, 1002, 1003" {...register('agents')} />
        </div>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="priority">Queue Priority (lower = higher)</Label>
            <Input id="priority" type="number" {...register('priority', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="required_skills">Required Skills (comma-separated)</Label>
            <Input id="required_skills" placeholder="e.g. billing, english" {...register('required_skills')} />
          </div>
        </div>
      </div>

      {/* Queue Behavior */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Queue Behavior</h3>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label htmlFor="max_wait_time_secs">Max Wait (seconds)</Label>
            <Input id="max_wait_time_secs" type="number" {...register('max_wait_time_secs', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="max_queue_depth">Max Queue Depth</Label>
            <Input id="max_queue_depth" type="number" {...register('max_queue_depth', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="overflow_destination">Overflow Destination</Label>
            <Input id="overflow_destination" placeholder="e.g. voicemail:6000" {...register('overflow_destination')} />
          </div>
        </div>
        <div className="flex items-center gap-6">
          <div className="flex items-center gap-2">
            <Switch id="retry_on_no_answer" checked={watch('retry_on_no_answer')} onCheckedChange={(v) => setValue('retry_on_no_answer', v)} />
            <Label htmlFor="retry_on_no_answer">Retry on No Answer</Label>
          </div>
          <div className="flex items-center gap-2">
            <Switch id="record_calls" checked={watch('record_calls')} onCheckedChange={(v) => setValue('record_calls', v)} />
            <Label htmlFor="record_calls">Record Calls</Label>
          </div>
          <div className="flex items-center gap-2">
            <Switch id="callback_enabled" checked={watch('callback_enabled')} onCheckedChange={(v) => setValue('callback_enabled', v)} />
            <Label htmlFor="callback_enabled">Callback</Label>
          </div>
        </div>
      </div>

      {/* Announcements */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Announcements</h3>
        <div className="grid grid-cols-3 gap-4">
          <div className="flex items-end">
            <div className="flex items-center gap-2">
              <Switch id="announce_position" checked={watch('announce_position')} onCheckedChange={(v) => setValue('announce_position', v)} />
              <Label htmlFor="announce_position">Announce Position</Label>
            </div>
          </div>
          <div>
            <Label htmlFor="announce_interval_secs">Announce Interval (secs)</Label>
            <Input id="announce_interval_secs" type="number" {...register('announce_interval_secs', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="moh_class">Music on Hold Class</Label>
            <Input id="moh_class" placeholder="default" {...register('moh_class')} />
          </div>
        </div>
      </div>

      <DialogFooter>
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}

function QueueStatsPanel({ stats }: { stats: QueueStatsResponse }) {
  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
      <StatCard icon={Phone} label="Calls Waiting" value={stats.calls_waiting} highlight={stats.calls_waiting > 0} />
      <StatCard icon={Users} label="Agents Available" value={stats.agents_available} />
      <StatCard icon={Clock} label="Avg Wait" value={formatDuration(Math.round(stats.avg_wait_time_secs))} />
      <StatCard icon={Clock} label="Longest Wait" value={formatDuration(Math.round(stats.longest_wait_secs))} />
      <StatCard icon={TrendingUp} label="Handled Today" value={stats.calls_handled_today} />
      <StatCard icon={Phone} label="Abandoned Today" value={stats.calls_abandoned_today} />
      <StatCard icon={Users} label="On Call" value={stats.agents_on_call} />
      <StatCard icon={TrendingUp} label="Service Level" value={`${Math.round(stats.service_level_pct)}%`} />
    </div>
  )
}

function StatCard({ icon: Icon, label, value, highlight }: {
  icon: React.ElementType; label: string; value: string | number; highlight?: boolean
}) {
  return (
    <div className="rounded-lg border p-3 text-center">
      <Icon className={`h-4 w-4 mx-auto mb-1 ${highlight ? 'text-warning' : 'text-muted-foreground'}`} />
      <p className="text-xl font-bold">{value}</p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  )
}
