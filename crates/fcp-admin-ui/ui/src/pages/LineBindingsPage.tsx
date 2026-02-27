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
import { lineBindings, users, devices, phoneNumbers } from '@/api/endpoints'
import type { LineBinding, DeviceBinding, UpdateLineBindingRequest } from '@/types'
import { formatDateTime } from '@/lib/utils'

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
  no_answer_timeout_secs: z.coerce.number().min(5).max(300).default(30),
})

const editSchema = z.object({
  ring_strategy: z.string().optional(),
  no_answer_timeout_secs: z.coerce.number().min(5).max(300).optional(),
  forward_on_busy: z.string().optional(),
  forward_on_no_answer: z.string().optional(),
  forward_unconditional: z.string().optional(),
  outbound_caller_id: z.string().optional(),
  outbound_cnam: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function LineBindingsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<LineBinding | null>(null)
  const [createDevices, setCreateDevices] = useState<DeviceBinding[]>([])
  const [editDevices, setEditDevices] = useState<DeviceBinding[]>([])
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['line-bindings'],
    queryFn: lineBindings.list,
  })

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
      cell: ({ row }) => <Badge variant="outline">{formatLabel(row.original.binding_type)}</Badge>,
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
      accessorKey: 'no_answer_timeout_secs',
      header: 'Timeout',
      cell: ({ row }) => `${row.original.no_answer_timeout_secs}s`,
    },
    {
      accessorKey: 'created_at',
      header: 'Created',
      cell: ({ row }) => formatDateTime(row.original.created_at),
    },
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

  const createForm = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
    defaultValues: { binding_type: 'user', ring_strategy: 'simultaneous', no_answer_timeout_secs: 30 },
  })

  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      ring_strategy: editItem.ring_strategy,
      no_answer_timeout_secs: editItem.no_answer_timeout_secs,
      forward_on_busy: editItem.forward_on_busy || '',
      forward_on_no_answer: editItem.forward_on_no_answer || '',
      forward_unconditional: editItem.forward_unconditional || '',
      outbound_caller_id: editItem.outbound_caller_id || '',
      outbound_cnam: editItem.outbound_cnam || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    const validDevices = createDevices.filter(d => d.device_id)
    createMutation.mutate({
      phone_number_id: data.phone_number_id,
      owner_user_id: data.owner_user_id || undefined,
      binding_type: data.binding_type,
      ring_strategy: data.ring_strategy,
      no_answer_timeout_secs: data.no_answer_timeout_secs,
      devices: validDevices.length > 0 ? validDevices : undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    const validDevices = editDevices.filter(d => d.device_id)
    updateMutation.mutate({
      id: editItem.id,
      data: {
        ring_strategy: data.ring_strategy || undefined,
        no_answer_timeout_secs: data.no_answer_timeout_secs,
        devices: validDevices,
        forward_on_busy: data.forward_on_busy || undefined,
        forward_on_no_answer: data.forward_on_no_answer || undefined,
        forward_unconditional: data.forward_unconditional || undefined,
        outbound_caller_id: data.outbound_caller_id || undefined,
        outbound_cnam: data.outbound_cnam || undefined,
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
        <Button onClick={() => { createForm.reset({ binding_type: 'user', ring_strategy: 'simultaneous', no_answer_timeout_secs: 30 }); setCreateDevices([]); setCreateOpen(true) }}>
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
                <Select defaultValue="user" onValueChange={(v) => createForm.setValue('binding_type', v)}>
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
            <div className="space-y-2">
              <Label htmlFor="c-timeout">No Answer Timeout (seconds)</Label>
              <Input id="c-timeout" type="number" min="5" max="300" {...createForm.register('no_answer_timeout_secs')} />
            </div>

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
              <div className="space-y-2">
                <Label htmlFor="e-timeout">Timeout (seconds)</Label>
                <Input id="e-timeout" type="number" min="5" max="300" {...editForm.register('no_answer_timeout_secs')} />
                {editForm.formState.errors.no_answer_timeout_secs && <p className="text-sm text-destructive">{editForm.formState.errors.no_answer_timeout_secs.message}</p>}
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

            <div className="border-t pt-4 mt-2">
              <h4 className="text-sm font-medium mb-3">Call Forwarding</h4>
              <div className="space-y-3">
                <div className="space-y-2">
                  <Label htmlFor="e-forward_unconditional">Forward Unconditional</Label>
                  <Input id="e-forward_unconditional" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_unconditional')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-forward_on_no_answer">Forward on No Answer</Label>
                  <Input id="e-forward_on_no_answer" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_on_no_answer')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-forward_on_busy">Forward on Busy</Label>
                  <Input id="e-forward_on_busy" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_on_busy')} />
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
