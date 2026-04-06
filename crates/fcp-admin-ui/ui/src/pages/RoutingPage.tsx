import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { useForm } from 'react-hook-form'
import {
  Plus, MoreHorizontal, ArrowUpDown, Shield, ShieldOff, ExternalLink,
  Pencil, Trash2, RefreshCw,
} from 'lucide-react'
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
import type { RoutingRule, CreateRoutingRuleRequest, SipTrunk } from '@/types'

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
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateRoutingRuleRequest> }) => routing.update(id, data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['routing-rules'] }); setEditRule(null) },
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => routing.delete(id),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['routing-rules'] }); setDeleteRule(null) },
  })

  const openEdit = (rule: RoutingRule) => {
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
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
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
          <p className="text-muted-foreground">Route calls to trunks based on patterns, schedules, and policies</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} /> Refresh
          </Button>
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Rule
          </Button>
        </div>
      </div>

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
          <RuleForm form={createForm} trunkList={trunkList} onSubmit={(d) => createMutation.mutate(formToRequest(d))} isSubmitting={createMutation.isPending} />
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
            onSubmit={(d) => editRule && updateMutation.mutate({ id: editRule.id, data: formToRequest(d) })}
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
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRule(null)}>Cancel</Button>
            <Button variant="destructive" onClick={() => deleteRule && deleteMutation.mutate(deleteRule.id)}>Delete</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function RuleForm({ form, trunkList, onSubmit, isSubmitting, submitLabel = 'Create Rule' }: {
  form: ReturnType<typeof useForm<RuleFormData>>
  trunkList: SipTrunk[]
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
              <Label>Or Trunk Group (failover)</Label>
              <Select value={watch('target_trunk_group') || '__none__'} onValueChange={(v) => setValue('target_trunk_group', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select group..." /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {trunkGroups.map((g) => (
                    <SelectItem key={g} value={g}>{g}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
        )}
      </div>

      <DialogFooter>
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}
