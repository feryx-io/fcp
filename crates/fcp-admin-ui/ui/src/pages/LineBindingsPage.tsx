import { useState, useMemo } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, X } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { lineBindings, users, devices, phoneNumbers, flows, tenants, schedules } from '@/api/endpoints'
import type { LineBinding, DeviceBinding, UpdateLineBindingRequest } from '@/types'
import { formatDateTime } from '@/lib/utils'
import { TenantField, useEffectiveTenantId, useTenantListFilter, useShowTenantColumn } from '@/components/TenantSelector'

const BINDING_TYPES = [
  { value: 'user', label: 'User' },
  { value: 'ivr', label: 'IVR' },
  { value: 'queue', label: 'Queue' },
  { value: 'hunt_group', label: 'Hunt Group' },
  { value: 'conference_bridge', label: 'Conference Bridge' },
  { value: 'voicemail', label: 'Voicemail' },
  { value: 'parking', label: 'Parking' },
  { value: 'paging', label: 'Paging' },
  { value: 'fax', label: 'Fax' },
  { value: 'emergency', label: 'Emergency' },
]

const RING_STRATEGIES = [
  { value: 'simultaneous', label: 'Simultaneous' },
  { value: 'sequential', label: 'Sequential' },
  { value: 'round_robin', label: 'Round Robin' },
  { value: 'longest_idle', label: 'Longest Idle' },
  { value: 'primary', label: 'Primary' },
]

function formatLabel(value: string): string {
  return value.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
}

function newDeviceRow(): DeviceBinding {
  return { device_id: '', priority: 1, line_appearance: 1, shared_line: false, enabled: true }
}

const createSchema = z.object({
  phone_number_id: z.string().min(1, 'Phone number is required'),
  owner_user_id: z.string().optional(),
  binding_type: z.string().min(1, 'Binding type is required'),
  ring_strategy: z.string().default('simultaneous'),
  tenant_id: z.string().optional(),
})

const editSchema = z.object({
  ring_strategy: z.string().optional(),
  outbound_caller_id: z.string().optional(),
  outbound_cnam: z.string().optional(),
  schedule_id: z.string().optional(),
  closed_target: z.string().optional(),
  holiday_target: z.string().optional(),
  shared: z.boolean().optional(),
  barge_in: z.boolean().optional(),
  max_appearances: z.coerce.number().int().min(1, 'At least 1').max(24, 'At most 24').optional(),
  tenant_id: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function LineBindingsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<LineBinding | null>(null)
  const [createDevices, setCreateDevices] = useState<DeviceBinding[]>([])
  const [editDevices, setEditDevices] = useState<DeviceBinding[]>([])
  const [createFlowId, setCreateFlowId] = useState<string>('')
  const [editFlowId, setEditFlowId] = useState<string>('')
  const [createBindingType, setCreateBindingType] = useState<string>('user')
  const queryClient = useQueryClient()

  const { data: rawItems = [], isLoading } = useQuery({
    queryKey: ['line-bindings'],
    queryFn: lineBindings.list,
  })
  const items = useTenantListFilter(rawItems)

  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })
  const { data: deviceList = [] } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })
  const { data: phoneNumberList = [] } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })
  const { data: flowsData } = useQuery({
    queryKey: ['flows'],
    queryFn: () => flows.list(),
  })
  const flowList = flowsData?.flows ?? []

  const showTenantColumn = useShowTenantColumn()
  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: showTenantColumn,
  })
  const tenantName = (id: string | null | undefined) => {
    if (!id) return '-'
    return tenantList.find((t) => t.id === id)?.name ?? id.slice(0, 8)
  }

  const userMap = useMemo(() => new Map(userList.map(u => [u.id, u])), [userList])
  const deviceMap = useMemo(() => new Map(deviceList.map(d => [d.id, d])), [deviceList])
  const phoneNumberMap = useMemo(() => new Map(phoneNumberList.map(p => [p.id, p])), [phoneNumberList])

  const createMutation = useMutation({
    mutationFn: lineBindings.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['line-bindings'] })
      setCreateOpen(false)
      setCreateDevices([])
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateLineBindingRequest }) =>
      lineBindings.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['line-bindings'] })
      setEditItem(null)
      setEditDevices([])
    },
  })

  const deleteMutation = useMutation({
    mutationFn: lineBindings.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['line-bindings'] }),
  })

  const columns: ColumnDef<LineBinding>[] = [
    {
      accessorKey: 'phone_number_id',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Phone Number <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => {
        const pn = phoneNumberMap.get(row.original.phone_number_id)
        return pn ? pn.e164 : row.original.phone_number_id
      },
    },
    {
      accessorKey: 'owner_user_id',
      header: 'Owner',
      cell: ({ row }) => {
        if (!row.original.owner_user_id) return <span className="text-muted-foreground">-</span>
        const user = userMap.get(row.original.owner_user_id)
        return user ? (user.display_name || user.username) : row.original.owner_user_id
      },
    },
    {
      accessorKey: 'binding_type',
      header: 'Type',
      cell: ({ row }) => (
        <div className="flex gap-1">
          <Badge variant="outline">{formatLabel(row.original.binding_type)}</Badge>
          {row.original.shared && (
            <Badge variant="secondary" title="Shared call appearance">
              Shared ×{row.original.max_appearances}
            </Badge>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'flow_id',
      header: 'Flow',
      cell: ({ row }) => {
        if (row.original.binding_type !== 'ivr' || !row.original.flow_id) return <span className="text-muted-foreground">-</span>
        const flow = flowList.find(f => f.id === row.original.flow_id)
        return flow ? flow.name : row.original.flow_id.slice(0, 8)
      },
    },
    {
      accessorKey: 'devices',
      header: 'Devices',
      cell: ({ row }) => {
        const devs = row.original.devices
        if (!devs || devs.length === 0) return <span className="text-muted-foreground">-</span>
        return devs
          .map(d => {
            const dev = deviceMap.get(d.device_id)
            return dev ? dev.name : d.device_id.slice(0, 8)
          })
          .join(', ')
      },
    },
    {
      accessorKey: 'ring_strategy',
      header: 'Ring Strategy',
      cell: ({ row }) => formatLabel(row.original.ring_strategy),
    },
    {
      accessorKey: 'created_at',
      header: 'Created',
      cell: ({ row }) => formatDateTime(row.original.created_at),
    },
    ...(showTenantColumn
      ? ([
          {
            id: 'tenant',
            header: 'Tenant',
            cell: ({ row }) => (
              <Badge variant="outline">{tenantName(row.original.tenant_id)}</Badge>
            ),
          },
        ] as ColumnDef<LineBinding>[])
      : []),
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => {
              setEditItem(row.original)
              setEditDevices(row.original.devices ? [...row.original.devices] : [])
              setEditFlowId(row.original.flow_id || '')
            }}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm('Delete this line binding?')) {
                  deleteMutation.mutate(row.original.id)
                }
              }}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  const { data: scheduleList = [] } = useQuery({ queryKey: ['schedules'], queryFn: schedules.list })
  const effectiveTenantId = useEffectiveTenantId()
  const createForm = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
    defaultValues: { binding_type: 'user', ring_strategy: 'simultaneous', tenant_id: effectiveTenantId },
  })

  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      ring_strategy: editItem.ring_strategy,
      outbound_caller_id: editItem.outbound_caller_id || '',
      outbound_cnam: editItem.outbound_cnam || '',
      schedule_id: editItem.schedule_id || '',
      closed_target: editItem.closed_target || '',
      holiday_target: editItem.holiday_target || '',
      shared: editItem.shared ?? false,
      barge_in: editItem.barge_in ?? false,
      max_appearances: editItem.max_appearances ?? 4,
      tenant_id: editItem.tenant_id ?? '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    const validDevices = createDevices.filter(d => d.device_id)
    createMutation.mutate({
      phone_number_id: data.phone_number_id,
      owner_user_id: data.owner_user_id || undefined,
      binding_type: data.binding_type,
      ring_strategy: data.ring_strategy,
      devices: validDevices.length > 0 ? validDevices : undefined,
      flow_id: data.binding_type === 'ivr' && createFlowId ? createFlowId : undefined,
      tenant_id: data.tenant_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    const validDevices = editDevices.filter(d => d.device_id)
    updateMutation.mutate({
      id: editItem.id,
      data: {
        ring_strategy: data.ring_strategy || undefined,
        devices: validDevices,
        outbound_caller_id: data.outbound_caller_id || undefined,
        outbound_cnam: data.outbound_cnam || undefined,
        schedule_id: data.schedule_id ?? '',
        closed_target: data.closed_target ?? '',
        holiday_target: data.holiday_target ?? '',
        flow_id: editItem.binding_type === 'ivr' ? (editFlowId || undefined) : undefined,
        shared: editItem.binding_type === 'user' ? (data.shared ?? false) : undefined,
        max_appearances: editItem.binding_type === 'user' ? data.max_appearances : undefined,
        barge_in: editItem.binding_type === 'user' ? (data.barge_in ?? false) : undefined,
        tenant_id: data.tenant_id || undefined,
      },
    })
  }

  function updateCreateDevice(index: number, field: keyof DeviceBinding, value: string | number | boolean) {
    setCreateDevices(prev => prev.map((d, i) => i === index ? { ...d, [field]: value } : d))
  }

  function updateEditDevice(index: number, field: keyof DeviceBinding, value: string | number | boolean) {
    setEditDevices(prev => prev.map((d, i) => i === index ? { ...d, [field]: value } : d))
  }

  function DeviceRows({ rows, onUpdate, onRemove }: {
    rows: DeviceBinding[]
    onUpdate: (index: number, field: keyof DeviceBinding, value: string | number | boolean) => void
    onRemove: (index: number) => void
  }) {
    return (
      <>
        {rows.map((row, index) => (
          <div key={index} className="grid grid-cols-[1fr_60px_60px_32px] gap-2 items-end">
            <div className="space-y-1">
              {index === 0 && <Label className="text-xs">Device</Label>}
              <Select value={row.device_id} onValueChange={(v) => onUpdate(index, 'device_id', v)}>
                <SelectTrigger><SelectValue placeholder="Select device" /></SelectTrigger>
                <SelectContent>
                  {deviceList.map(dev => (
                    <SelectItem key={dev.id} value={dev.id}>{dev.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              {index === 0 && <Label className="text-xs">Priority</Label>}
              <Input
                type="number"
                min="1"
                max="99"
                value={row.priority}
                onChange={(e) => onUpdate(index, 'priority', parseInt(e.target.value) || 1)}
              />
            </div>
            <div className="space-y-1">
              {index === 0 && <Label className="text-xs">Line</Label>}
              <Input
                type="number"
                min="1"
                max="99"
                value={row.line_appearance}
                onChange={(e) => onUpdate(index, 'line_appearance', parseInt(e.target.value) || 1)}
              />
            </div>
            <div>
              {index === 0 && <Label className="text-xs invisible">X</Label>}
              <Button type="button" variant="ghost" size="icon" className="h-9 w-9" onClick={() => onRemove(index)}>
                <X className="h-4 w-4" />
              </Button>
            </div>
          </div>
        ))}
      </>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Line Bindings</h1>
          <p className="text-muted-foreground">Bind phone numbers to users, devices, and call handling</p>
        </div>
        <Button onClick={() => { createForm.reset({ binding_type: 'user', ring_strategy: 'simultaneous', tenant_id: effectiveTenantId }); setCreateDevices([]); setCreateFlowId(''); setCreateBindingType('user'); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Binding
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="phone_number_id" searchPlaceholder="Search bindings..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Create Line Binding</DialogTitle>
            <DialogDescription>Bind a phone number to a user and devices.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label>Phone Number *</Label>
              <Select onValueChange={(v) => createForm.setValue('phone_number_id', v)}>
                <SelectTrigger><SelectValue placeholder="Select phone number" /></SelectTrigger>
                <SelectContent>
                  {phoneNumberList.map(pn => (
                    <SelectItem key={pn.id} value={pn.id}>
                      {pn.cnam ? `${pn.e164} - ${pn.cnam}` : pn.e164}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {createForm.formState.errors.phone_number_id && <p className="text-sm text-destructive">{createForm.formState.errors.phone_number_id.message}</p>}
            </div>
            <TenantField value={createForm.watch('tenant_id') ?? ''} onChange={(v) => createForm.setValue('tenant_id', v)} />
            <div className="space-y-2">
              <Label>Owner User</Label>
              <Select onValueChange={(v) => createForm.setValue('owner_user_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="None" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {userList.map(u => (
                    <SelectItem key={u.id} value={u.id}>
                      {u.display_name || u.username}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Binding Type *</Label>
                <Select defaultValue="user" onValueChange={(v) => { createForm.setValue('binding_type', v); setCreateBindingType(v) }}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {BINDING_TYPES.map(t => (
                      <SelectItem key={t.value} value={t.value}>{t.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>Ring Strategy</Label>
                <Select defaultValue="simultaneous" onValueChange={(v) => createForm.setValue('ring_strategy', v)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {RING_STRATEGIES.map(s => (
                      <SelectItem key={s.value} value={s.value}>{s.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            {createBindingType === 'ivr' && (
              <div className="space-y-2">
                <Label>IVR Flow *</Label>
                <Select value={createFlowId || '__none__'} onValueChange={(v) => setCreateFlowId(v === '__none__' ? '' : v)}>
                  <SelectTrigger><SelectValue placeholder="Select flow" /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">None</SelectItem>
                    {flowList.map(f => (
                      <SelectItem key={f.id} value={f.id}>{f.name}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            <div className="border-t pt-4 mt-2">
              <div className="flex items-center justify-between mb-3">
                <h4 className="text-sm font-medium">Devices</h4>
                <Button type="button" variant="outline" size="sm" onClick={() => setCreateDevices(prev => [...prev, newDeviceRow()])}>
                  <Plus className="mr-1 h-3 w-3" /> Add Device
                </Button>
              </div>
              {createDevices.length === 0 ? (
                <p className="text-sm text-muted-foreground">No devices added yet.</p>
              ) : (
                <div className="space-y-2">
                  <DeviceRows
                    rows={createDevices}
                    onUpdate={updateCreateDevice}
                    onRemove={(i) => setCreateDevices(prev => prev.filter((_, idx) => idx !== i))}
                  />
                </div>
              )}
            </div>

            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Binding'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); setEditDevices([]); editForm.reset() } }}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Edit Line Binding</DialogTitle>
            <DialogDescription>Update devices, ring strategy, and call handling.</DialogDescription>
          </DialogHeader>

          {editItem && (
            <div className="rounded-md bg-muted p-3 space-y-1 text-sm">
              <div><span className="font-medium">Phone Number:</span> {phoneNumberMap.get(editItem.phone_number_id)?.e164 || editItem.phone_number_id}</div>
              <div><span className="font-medium">Owner:</span> {editItem.owner_user_id ? (userMap.get(editItem.owner_user_id)?.display_name || userMap.get(editItem.owner_user_id)?.username || editItem.owner_user_id) : 'None'}</div>
              <div><span className="font-medium">Type:</span> <Badge variant="outline" className="ml-1">{formatLabel(editItem.binding_type)}</Badge></div>
            </div>
          )}

          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <TenantField value={editForm.watch('tenant_id') ?? ''} onChange={(v) => editForm.setValue('tenant_id', v)} mode="edit" />
            {editItem?.binding_type === 'ivr' && (
              <div className="space-y-2">
                <Label>IVR Flow</Label>
                <Select value={editFlowId || '__none__'} onValueChange={(v) => setEditFlowId(v === '__none__' ? '' : v)}>
                  <SelectTrigger><SelectValue placeholder="Select flow" /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">None</SelectItem>
                    {flowList.map(f => (
                      <SelectItem key={f.id} value={f.id}>{f.name}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Ring Strategy</Label>
                <Select defaultValue={editItem?.ring_strategy} onValueChange={(v) => editForm.setValue('ring_strategy', v)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {RING_STRATEGIES.map(s => (
                      <SelectItem key={s.value} value={s.value}>{s.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="border-t pt-4 mt-2">
              <div className="flex items-center justify-between mb-3">
                <h4 className="text-sm font-medium">Devices</h4>
                <Button type="button" variant="outline" size="sm" onClick={() => setEditDevices(prev => [...prev, newDeviceRow()])}>
                  <Plus className="mr-1 h-3 w-3" /> Add Device
                </Button>
              </div>
              {editDevices.length === 0 ? (
                <p className="text-sm text-muted-foreground">No devices configured.</p>
              ) : (
                <div className="space-y-2">
                  <DeviceRows
                    rows={editDevices}
                    onUpdate={updateEditDevice}
                    onRemove={(i) => setEditDevices(prev => prev.filter((_, idx) => idx !== i))}
                  />
                </div>
              )}
            </div>

            <p className="text-xs text-muted-foreground">
              Forwarding and the ring time are the owner&apos;s calling settings, on the user&apos;s page.
            </p>

            {editItem?.binding_type === 'user' && (
              <div className="border-t pt-4 mt-2">
                <h4 className="text-sm font-medium mb-1">Shared line</h4>
                <p className="text-xs text-muted-foreground mb-3">
                  Every device above registers the number as an account of its own (its second account when
                  provisioned) and they share its appearances: a call rings them all, a held call can be picked up
                  from any. Each device needs credentials of its own unless it is the owner&apos;s.
                </p>
                <div className="grid grid-cols-2 gap-4 items-end">
                  <div className="space-y-2">
                    <label className="flex items-center gap-2 text-sm">
                      <input type="checkbox" {...editForm.register('shared')} />
                      Shared call appearance
                    </label>
                    <label className="flex items-center gap-2 text-sm">
                      <input type="checkbox" disabled={!editForm.watch('shared')} {...editForm.register('barge_in')} />
                      Phones may barge in
                    </label>
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="e-max-appearances">Appearances</Label>
                    <Input
                      id="e-max-appearances"
                      type="number"
                      min="1"
                      max="24"
                      disabled={!editForm.watch('shared')}
                      {...editForm.register('max_appearances')}
                    />
                    {editForm.formState.errors.max_appearances && (
                      <p className="text-sm text-destructive">{editForm.formState.errors.max_appearances.message}</p>
                    )}
                  </div>
                </div>
              </div>
            )}

            <div className="border-t pt-4 mt-2">
              <h4 className="text-sm font-medium mb-1">Schedule</h4>
              <p className="text-xs text-muted-foreground mb-3">
                Outside the schedule&apos;s hours calls go to the closed target; on a holiday to the holiday target, else the
                closed one. Targets: voicemail, +number, extension, flow:&lt;id&gt;, queue:&lt;name&gt; or a SIP URI.
              </p>
              <div className="grid grid-cols-3 gap-4">
                <div className="space-y-2">
                  <Label htmlFor="e-schedule">Schedule</Label>
                  <select
                    id="e-schedule"
                    className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm"
                    {...editForm.register('schedule_id')}
                  >
                    <option value="">Always open</option>
                    {scheduleList.map((sch) => (
                      <option key={sch.id} value={sch.id}>{sch.name}</option>
                    ))}
                  </select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-closed">When closed</Label>
                  <Input id="e-closed" placeholder="voicemail" {...editForm.register('closed_target')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-holiday">On a holiday</Label>
                  <Input id="e-holiday" placeholder="as when closed" {...editForm.register('holiday_target')} />
                </div>
              </div>
            </div>

            <div className="border-t pt-4 mt-2">
              <h4 className="text-sm font-medium mb-3">Outbound Caller ID</h4>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label htmlFor="e-outbound_caller_id">Caller ID Number</Label>
                  <Input id="e-outbound_caller_id" placeholder="+15551234567" {...editForm.register('outbound_caller_id')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-outbound_cnam">CNAM</Label>
                  <Input id="e-outbound_cnam" placeholder="Display name" {...editForm.register('outbound_cnam')} />
                </div>
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setEditItem(null)}>Cancel</Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {updateMutation.isPending ? 'Saving...' : 'Save Changes'}
              </Button>
            </div>
            {updateMutation.isError && (
              <p className="text-sm text-destructive">{(updateMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
