import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { useForm } from 'react-hook-form'
import {
  Plus, MoreHorizontal, ArrowUpDown, Shield, ShieldOff, ExternalLink,
  Pencil, Trash2, RefreshCw, Route, Siren,
} from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { DataTable } from '@/components/DataTable'
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
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { routing, trunks } from '@/api/endpoints'
import type { RoutingRule, CreateRoutingRuleRequest, RoutePreview, RoutePreviewTrunk, SipTrunk } from '@/types'

const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']

const actionBadge = (action: string) => {
  switch (action) {
    case 'allow': return 'success' as const
    case 'deny': return 'destructive' as const
    case 'route_to_trunk': return 'default' as const
    default: return 'outline' as const
  }
}

const actionLabel = (action: string) => {
  switch (action) {
    case 'allow': return 'Allow'
    case 'deny': return 'Deny'
    case 'route_to_trunk': return 'Route to Trunk'
    default: return action
  }
}

interface RuleFormData {
  name: string
  description: string
  priority: number
  enabled: boolean
  destination_pattern: string
  source_pattern: string
  source_ip_ranges: string
  time_window_days: string[]
  time_window_start: string
  time_window_end: string
  time_window_timezone: string
  action: string
  target_trunk_id: string
  target_trunk_group: string
  reason: string
}

const defaultValues: RuleFormData = {
  name: '',
  description: '',
  priority: 100,
  enabled: true,
  destination_pattern: '',
  source_pattern: '',
  source_ip_ranges: '',
  time_window_days: [],
  time_window_start: '',
  time_window_end: '',
  time_window_timezone: 'UTC',
  action: 'allow',
  target_trunk_id: '',
  target_trunk_group: '',
  reason: '',
}

function ruleToForm(r: RoutingRule): RuleFormData {
  return {
    name: r.name,
    description: r.description ?? '',
    priority: r.priority,
    enabled: r.enabled,
    destination_pattern: r.destination_pattern ?? '',
    source_pattern: r.source_pattern ?? '',
    source_ip_ranges: r.source_ip_ranges.join(', '),
    time_window_days: r.time_window_days,
    time_window_start: r.time_window_start ?? '',
    time_window_end: r.time_window_end ?? '',
    time_window_timezone: r.time_window_timezone,
    action: r.action,
    target_trunk_id: r.target_trunk_id ?? '',
    target_trunk_group: r.target_trunk_group ?? '',
    reason: r.reason ?? '',
  }
}

/**
 * An edit sends every field, empty ones as "" — the API clears a field it
 * is sent empty, and leaves alone one it is not sent, so an edit that
 * omitted an emptied field could never remove a pattern or a target.
 */
function formToUpdate(f: RuleFormData): Partial<CreateRoutingRuleRequest> {
  const routes = f.action === 'route_to_trunk'
  return {
    ...formToRequest(f),
    description: f.description,
    destination_pattern: f.destination_pattern,
    source_pattern: f.source_pattern,
    time_window_start: f.time_window_start,
    time_window_end: f.time_window_end,
    // A rule names one target: a trunk, or else a group.
    target_trunk_id: routes ? f.target_trunk_id : '',
    target_trunk_group: routes && !f.target_trunk_id ? f.target_trunk_group : '',
    reason: f.reason,
  }
}

function formToRequest(f: RuleFormData): CreateRoutingRuleRequest {
  return {
    name: f.name,
    description: f.description || undefined,
    priority: f.priority,
    enabled: f.enabled,
    destination_pattern: f.destination_pattern || undefined,
    source_pattern: f.source_pattern || undefined,
    source_ip_ranges: f.source_ip_ranges ? f.source_ip_ranges.split(',').map((s) => s.trim()).filter(Boolean) : [],
    time_window_days: f.time_window_days.length > 0 ? f.time_window_days : [],
    time_window_start: f.time_window_start || undefined,
    time_window_end: f.time_window_end || undefined,
    time_window_timezone: f.time_window_timezone,
    action: f.action,
    target_trunk_id: f.target_trunk_id || undefined,
    target_trunk_group: f.target_trunk_group || undefined,
    reason: f.reason || undefined,
  }
}

export function RoutingPage() {
  const queryClient = useQueryClient()
  const [createOpen, setCreateOpen] = useState(false)
  const [editRule, setEditRule] = useState<RoutingRule | null>(null)
  const [deleteRule, setDeleteRule] = useState<RoutingRule | null>(null)

  const [error, setError] = useState<string | null>(null)

  const createForm = useForm<RuleFormData>({ defaultValues })
  const editForm = useForm<RuleFormData>({ defaultValues })

  const { data: rules = [], isLoading, isFetching, refetch } = useQuery({
    queryKey: ['routing-rules'],
    queryFn: routing.list,
  })

  const { data: trunkList = [] } = useQuery({
    queryKey: ['trunks'],
    queryFn: trunks.list,
  })

  const createMutation = useMutation({
    mutationFn: (data: CreateRoutingRuleRequest) => routing.create(data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['routing-rules'] }); setCreateOpen(false); createForm.reset() },
    onError: (e: Error) => setError(e.message),
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateRoutingRuleRequest> }) => routing.update(id, data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['routing-rules'] }); setEditRule(null) },
    onError: (e: Error) => setError(e.message),
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => routing.delete(id),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['routing-rules'] }); setDeleteRule(null) },
    onError: (e: Error) => setError(e.message),
  })

  const openEdit = (rule: RoutingRule) => {
    setError(null)
    editForm.reset(ruleToForm(rule))
    setEditRule(rule)
  }

  const columns: ColumnDef<RoutingRule>[] = [
    {
      accessorKey: 'priority',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Priority <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => <span className="font-mono text-sm">{row.original.priority}</span>,
    },
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <p className="font-medium">{row.original.name}</p>
          {row.original.description && <p className="text-xs text-muted-foreground">{row.original.description}</p>}
        </div>
      ),
    },
    {
      accessorKey: 'action',
      header: 'Action',
      cell: ({ row }) => {
        const r = row.original
        return (
          <div className="flex flex-col gap-1">
            <Badge variant={actionBadge(r.action)}>
              {r.action === 'allow' && <Shield className="h-3 w-3 mr-1" />}
              {r.action === 'deny' && <ShieldOff className="h-3 w-3 mr-1" />}
              {r.action === 'route_to_trunk' && <ExternalLink className="h-3 w-3 mr-1" />}
              {actionLabel(r.action)}
            </Badge>
            {r.action === 'route_to_trunk' && (r.target_trunk_id || r.target_trunk_group) && (
              <span className="text-xs text-muted-foreground">
                {r.target_trunk_group ? `Group: ${r.target_trunk_group}` : trunkList.find((t) => t.id === r.target_trunk_id)?.name ?? r.target_trunk_id}
              </span>
            )}
          </div>
        )
      },
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'default' : 'secondary'}>
          {row.original.enabled ? 'Active' : 'Disabled'}
        </Badge>
      ),
    },
    {
      accessorKey: 'destination_pattern',
      header: 'Destination',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.destination_pattern ?? 'Any'}</span>
      ),
    },
    {
      accessorKey: 'source_pattern',
      header: 'Source',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.source_pattern ?? 'Any'}</span>
      ),
    },
    {
      accessorKey: 'time_window_days',
      header: 'Schedule',
      cell: ({ row }) => {
        const r = row.original
        if (!r.time_window_days.length && !r.time_window_start) return <span className="text-muted-foreground text-xs">Always</span>
        const days = r.time_window_days.map((d) => d.slice(0, 3)).join(', ')
        const time = r.time_window_start && r.time_window_end ? `${r.time_window_start}-${r.time_window_end}` : ''
        return <span className="text-xs">{[days, time].filter(Boolean).join(' ')}</span>
      },
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="Actions"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => openEdit(row.original)}>
              <Pencil className="h-3.5 w-3.5 mr-2" /> Edit
            </DropdownMenuItem>
            <DropdownMenuItem className="text-destructive" onClick={() => setDeleteRule(row.original)}>
              <Trash2 className="h-3.5 w-3.5 mr-2" /> Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Routing Rules</h1>
          <p className="text-muted-foreground">
            Your call policy: refuse calls, or send them to a trunk, by number, caller, network and time
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} /> Refresh
          </Button>
          <Button onClick={() => { setError(null); createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Rule
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">How rules apply</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-1">
          <p>
            Every call manager applies these rules to your calls as they arrive, lowest priority number first; the
            first rule that matches decides. <strong>Deny</strong> refuses the call (403). <strong>Allow</strong> lets it
            route normally. <strong>Route to Trunk</strong> sends it out only on the trunk or trunk group named — if
            none of those can take it, the call is refused (503) rather than sent elsewhere; name a group for failover.
          </p>
          <p>
            A call no rule matches routes normally. The platform's own policy runs before these rules, and{' '}
            <strong>no rule applies to an emergency call</strong>. Patterns are regular expressions over the dialled
            number or caller (<span className="font-mono">^\+44</span> works as written).
          </p>
        </CardContent>
      </Card>

      <RoutePreviewPanel />

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={rules} searchKey="name" searchPlaceholder="Search rules..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Create Routing Rule</DialogTitle>
            <DialogDescription>Define how calls are matched and where they're routed.</DialogDescription>
          </DialogHeader>
          <RuleForm form={createForm} trunkList={trunkList} error={error} onSubmit={(d) => createMutation.mutate(formToRequest(d))} isSubmitting={createMutation.isPending} />
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editRule} onOpenChange={() => setEditRule(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Rule: {editRule?.name}</DialogTitle>
            <DialogDescription>Update routing rule configuration.</DialogDescription>
          </DialogHeader>
          <RuleForm
            form={editForm}
            trunkList={trunkList}
            error={error}
            onSubmit={(d) => editRule && updateMutation.mutate({ id: editRule.id, data: formToUpdate(d) })}
            isSubmitting={updateMutation.isPending}
            submitLabel="Save Changes"
          />
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={!!deleteRule} onOpenChange={() => setDeleteRule(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Rule</DialogTitle>
            <DialogDescription>
              Delete <strong>{deleteRule?.name}</strong>? This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRule(null)}>Cancel</Button>
            <Button variant="destructive" onClick={() => deleteRule && deleteMutation.mutate(deleteRule.id)}>Delete</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function RuleForm({ form, trunkList, error, onSubmit, isSubmitting, submitLabel = 'Create Rule' }: {
  form: ReturnType<typeof useForm<RuleFormData>>
  trunkList: SipTrunk[]
  error: string | null
  onSubmit: (data: RuleFormData) => void
  isSubmitting: boolean
  submitLabel?: string
}) {
  const { register, handleSubmit, watch, setValue } = form
  const action = watch('action')
  const selectedDays = watch('time_window_days')

  const toggleDay = (day: string) => {
    const current = selectedDays || []
    if (current.includes(day)) {
      setValue('time_window_days', current.filter((d) => d !== day))
    } else {
      setValue('time_window_days', [...current, day])
    }
  }

  // Get unique trunk groups from trunk list
  const trunkGroups = [...new Set(trunkList.filter((t) => t.trunk_group).map((t) => t.trunk_group!))]

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
      {/* Basic */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Basic</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="name">Rule Name</Label>
            <Input id="name" placeholder="e.g. us-domestic-outbound" {...register('name')} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <Label htmlFor="priority">Priority</Label>
              <Input id="priority" type="number" {...register('priority', { valueAsNumber: true })} />
            </div>
            <div className="flex items-end">
              <div className="flex items-center gap-2">
                <Switch id="enabled" checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
                <Label htmlFor="enabled">Enabled</Label>
              </div>
            </div>
          </div>
          <div className="col-span-2">
            <Label htmlFor="description">Description</Label>
            <Input id="description" placeholder="What this rule does" {...register('description')} />
          </div>
        </div>
      </div>

      {/* Match Conditions */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Match Conditions</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="destination_pattern">Destination Pattern (regex)</Label>
            <Input id="destination_pattern" className="font-mono" placeholder={'e.g. ^\\+1[2-9]\\d{9}$'} {...register('destination_pattern')} />
          </div>
          <div>
            <Label htmlFor="source_pattern">Source/Caller Pattern (regex)</Label>
            <Input id="source_pattern" className="font-mono" placeholder="e.g. ^1[0-9]{3}$" {...register('source_pattern')} />
          </div>
          <div className="col-span-2">
            <Label htmlFor="source_ip_ranges">Source IP Ranges (comma-separated CIDR)</Label>
            <Input id="source_ip_ranges" className="font-mono" placeholder="e.g. 192.168.1.0/24, 10.0.0.0/8" {...register('source_ip_ranges')} />
          </div>
        </div>
      </div>

      {/* Time Window */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Time Window (optional)</h3>
        <div>
          <Label className="mb-2 block">Days</Label>
          <div className="flex flex-wrap gap-1">
            {DAYS.map((day) => (
              <Button
                key={day}
                type="button"
                variant={selectedDays?.includes(day) ? 'default' : 'outline'}
                size="sm"
                onClick={() => toggleDay(day)}
              >
                {day.slice(0, 3)}
              </Button>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label htmlFor="time_window_start">Start Time</Label>
            <Input id="time_window_start" type="time" {...register('time_window_start')} />
          </div>
          <div>
            <Label htmlFor="time_window_end">End Time</Label>
            <Input id="time_window_end" type="time" {...register('time_window_end')} />
          </div>
          <div>
            <Label htmlFor="time_window_timezone">Timezone</Label>
            <Input id="time_window_timezone" placeholder="UTC" {...register('time_window_timezone')} />
          </div>
        </div>
      </div>

      {/* Action */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Action</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label>Action</Label>
            <Select value={action} onValueChange={(v) => setValue('action', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="allow">Allow (proceed normally)</SelectItem>
                <SelectItem value="deny">Deny (block call)</SelectItem>
                <SelectItem value="route_to_trunk">Route to Trunk</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="reason">Reason</Label>
            <Input id="reason" placeholder="Why this rule exists" {...register('reason')} />
          </div>
        </div>

        {action === 'route_to_trunk' && (
          <div className="grid grid-cols-2 gap-4 p-3 bg-muted rounded-lg">
            <div>
              <Label>Target Trunk</Label>
              <Select value={watch('target_trunk_id') || '__none__'} onValueChange={(v) => setValue('target_trunk_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select trunk..." /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None (use group)</SelectItem>
                  {trunkList.filter((t) => t.enabled).map((t) => (
                    <SelectItem key={t.id} value={t.id}>{t.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor="target_trunk_group">Or Trunk Group (failover)</Label>
              {/* Free text: a group may also name the platform's trunks, which are not listed here. */}
              <Input
                id="target_trunk_group"
                list="trunk-groups"
                placeholder="e.g. intl"
                disabled={!!watch('target_trunk_id')}
                {...register('target_trunk_group')}
              />
              <datalist id="trunk-groups">
                {trunkGroups.map((g) => <option key={g} value={g} />)}
              </datalist>
            </div>
            <p className="col-span-2 text-xs text-muted-foreground">
              A trunk is tried alone. A group's members are tried one at a time — your own first, by your least-cost
              routes and then priority, then the platform's in the same group unless you refuse the platform's trunks.
            </p>
          </div>
        )}
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <DialogFooter>
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}

const outcomeText: Record<RoutePreview['outcome'], string> = {
  routes: 'Goes out',
  refused: 'Refused',
  unavailable: 'No trunk can take it now',
  no_route: 'Nothing is configured to carry it',
}

function decisionText(label: string, d: RoutePreview['rule']): string | null {
  switch (d.decision) {
    case 'not_applied': return null
    case 'no_match': return `${label}: no rule matched`
    case 'allow': return `${label}: allowed${d.rule ? ` by ${d.rule}` : ''}`
    case 'deny': return `${label}: denied${d.rule ? ` by ${d.rule}` : ''}${d.reason ? ` — ${d.reason}` : ''}`
    case 'route': return `${label}: ${d.rule} sends it to ${d.target}`
    case 'error': return `${label}: could not be evaluated${d.reason ? ` — ${d.reason}` : ''}`
    default: return `${label}: ${d.decision}`
  }
}

function PreviewTrunkRow({ trunk, index }: { trunk: RoutePreviewTrunk; index?: number }) {
  return (
    <div className="flex flex-wrap items-center gap-2 text-sm">
      {index !== undefined && <span className="w-5 text-right font-mono text-muted-foreground">{index + 1}.</span>}
      <span className="font-medium">{trunk.name}</span>
      <Badge variant={trunk.owner === 'tenant' ? 'default' : 'secondary'}>
        {trunk.owner === 'tenant' ? 'Yours' : 'Platform'}
      </Badge>
      <Badge variant={trunk.health === 'healthy' ? 'success' : trunk.health === 'unknown' ? 'outline' : 'destructive'}>
        {trunk.health}
      </Badge>
      {trunk.price ? (
        <span className="text-xs text-muted-foreground">
          {trunk.price.prefix} at {trunk.price.cost_per_minute}/min ({trunk.price.source === 'tenant' ? 'your routes' : "the node's table"})
        </span>
      ) : (
        <span className="text-xs text-muted-foreground">priority {trunk.priority}, weight {trunk.weight}</span>
      )}
    </div>
  )
}

/**
 * What a live call manager would do with a call (`GET /v1/routing/preview`):
 * the node's answer, from its own policy, rules, least-cost routes and
 * trunks — nothing is dialled.
 */
function RoutePreviewPanel() {
  const [to, setTo] = useState('')
  const [from, setFrom] = useState('')
  const preview = useMutation({
    mutationFn: () => routing.preview(to.trim(), from.trim() || undefined),
  })
  const result = preview.data
  const lines = result
    ? [decisionText("Platform policy", result.policy), decisionText('Your rules', result.rule)].filter(Boolean)
    : []

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-base flex items-center gap-2"><Route className="h-4 w-4" /> Test routing</CardTitle>
        <CardDescription>Ask a live call manager what it would do with a call — nothing is dialled.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => { e.preventDefault(); if (to.trim()) preview.mutate() }}
        >
          <div>
            <Label htmlFor="preview-to">Number dialled</Label>
            <Input id="preview-to" className="font-mono w-56" placeholder="+442079460000" value={to} onChange={(e) => setTo(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="preview-from">Caller (optional)</Label>
            <Input id="preview-from" className="font-mono w-56" placeholder="1001" value={from} onChange={(e) => setFrom(e.target.value)} />
          </div>
          <Button type="submit" disabled={!to.trim() || preview.isPending}>
            {preview.isPending ? 'Asking...' : 'Test'}
          </Button>
        </form>

        {preview.error && <p className="text-sm text-destructive">{(preview.error as Error).message}</p>}

        {result && (
          <div className="space-y-3 rounded-lg border p-3">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={result.outcome === 'routes' ? 'success' : 'destructive'}>
                {outcomeText[result.outcome]}{result.status ? ` (${result.status})` : ''}
              </Badge>
              {result.emergency && (
                <Badge variant="destructive"><Siren className="h-3 w-3 mr-1" /> Emergency number: no rule applies</Badge>
              )}
              <span className="text-xs text-muted-foreground">answered by {result.node_id}</span>
            </div>
            {lines.map((line) => <p key={line} className="text-sm">{line}</p>)}
            {result.routes.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Tried in this order, one at a time</p>
                {result.routes.map((t, i) => <PreviewTrunkRow key={t.trunk_id} trunk={t} index={i} />)}
              </div>
            )}
            {result.not_offered.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Not offered this call</p>
                {result.not_offered.map((t) => <PreviewTrunkRow key={t.trunk_id} trunk={t} />)}
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
