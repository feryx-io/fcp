import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { devices } from '@/api/endpoints'
import type { Device } from '@/types'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  device_type: z.string().min(1, 'Device type is required'),
  sip_uri: z.string().optional(),
  mac_address: z.string().optional(),
  model: z.string().optional(),
  location: z.string().optional(),
})

const editSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  sip_uri: z.string().optional(),
  mac_address: z.string().optional(),
  model: z.string().optional(),
  firmware_version: z.string().optional(),
  location: z.string().optional(),
  enabled: z.boolean().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const regBadge = (state: string) => {
  switch (state) {
    case 'registered': return 'success' as const
    case 'unregistered': return 'secondary' as const
    case 'expired': return 'warning' as const
    default: return 'outline' as const
  }
}

export function DevicesPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<Device | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })

  const createMutation = useMutation({
    mutationFn: devices.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<EditForm> }) =>
      devices.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: devices.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['devices'] }),
  })

  const columns: ColumnDef<Device>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'device_type',
      header: 'Type',
      cell: ({ row }) => (
        <Badge variant="secondary">{row.original.device_type.replace(/_/g, ' ')}</Badge>
      ),
    },
    {
      accessorKey: 'mac_address',
      header: 'MAC Address',
      cell: ({ row }) => row.original.mac_address || '-',
    },
    {
      accessorKey: 'model',
      header: 'Model',
      cell: ({ row }) => row.original.model || '-',
    },
    {
      accessorKey: 'sip_uri',
      header: 'SIP URI',
      cell: ({ row }) => row.original.sip_uri ? (
        <code className="text-xs">{row.original.sip_uri}</code>
      ) : <span className="text-muted-foreground">-</span>,
    },
    {
      accessorKey: 'registration_state',
      header: 'Registration',
      cell: ({ row }) => (
        <Badge variant={regBadge(row.original.registration_state)}>
          {row.original.registration_state}
        </Badge>
      ),
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'success' : 'destructive'}>
          {row.original.enabled ? 'Enabled' : 'Disabled'}
        </Badge>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete device "${row.original.name}"?`)) {
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

  const createForm = useForm<CreateForm>({ resolver: zodResolver(createSchema) })
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      name: editItem.name,
      sip_uri: editItem.sip_uri || '',
      mac_address: editItem.mac_address || '',
      model: editItem.model || '',
      firmware_version: editItem.firmware_version || '',
      location: editItem.location || '',
      enabled: editItem.enabled,
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      name: data.name,
      device_type: data.device_type,
      sip_uri: data.sip_uri || undefined,
      mac_address: data.mac_address || undefined,
      model: data.model || undefined,
      location: data.location || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        name: data.name,
        sip_uri: data.sip_uri || undefined,
        mac_address: data.mac_address || undefined,
        model: data.model || undefined,
        firmware_version: data.firmware_version || undefined,
        location: data.location || undefined,
        enabled: data.enabled,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Devices</h1>
          <p className="text-muted-foreground">Manage SIP phones and devices</p>
        </div>
        <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Device
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search devices..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Device</DialogTitle>
            <DialogDescription>Register a new SIP device.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-name">Name *</Label>
              <Input id="c-name" placeholder="e.g., Lobby Phone" {...createForm.register('name')} />
              {createForm.formState.errors.name && <p className="text-sm text-destructive">{createForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Device Type *</Label>
              <Select onValueChange={(v) => createForm.setValue('device_type', v)}>
                <SelectTrigger><SelectValue placeholder="Select type" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="desk_phone">Desk Phone</SelectItem>
                  <SelectItem value="softphone">Softphone</SelectItem>
                  <SelectItem value="webrtc">WebRTC</SelectItem>
                  <SelectItem value="conference_room">Conference Room</SelectItem>
                  <SelectItem value="ata">ATA</SelectItem>
                  <SelectItem value="gateway">Gateway</SelectItem>
                  <SelectItem value="mobile">Mobile</SelectItem>
                  <SelectItem value="other">Other</SelectItem>
                </SelectContent>
              </Select>
              {createForm.formState.errors.device_type && <p className="text-sm text-destructive">{createForm.formState.errors.device_type.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-sip_uri">SIP URI</Label>
              <Input id="c-sip_uri" placeholder="sip:username@domain" {...createForm.register('sip_uri')} />
              <p className="text-xs text-muted-foreground">Must match the AOR the device registers with</p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-mac_address">MAC Address</Label>
                <Input id="c-mac_address" placeholder="AA:BB:CC:DD:EE:FF" {...createForm.register('mac_address')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-model">Model</Label>
                <Input id="c-model" {...createForm.register('model')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-location">Location</Label>
              <Input id="c-location" placeholder="e.g., Floor 2, Room 201" {...createForm.register('location')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Adding...' : 'Add Device'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); editForm.reset() } }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit Device: {editItem?.name}</DialogTitle>
            <DialogDescription>Update device details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-name">Name *</Label>
              <Input id="e-name" {...editForm.register('name')} />
              {editForm.formState.errors.name && <p className="text-sm text-destructive">{editForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-sip_uri">SIP URI</Label>
              <Input id="e-sip_uri" placeholder="sip:username@domain" {...editForm.register('sip_uri')} />
              <p className="text-xs text-muted-foreground">Must match the AOR the device registers with</p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-mac_address">MAC Address</Label>
                <Input id="e-mac_address" {...editForm.register('mac_address')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-model">Model</Label>
                <Input id="e-model" {...editForm.register('model')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-firmware_version">Firmware Version</Label>
              <Input id="e-firmware_version" {...editForm.register('firmware_version')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-location">Location</Label>
              <Input id="e-location" {...editForm.register('location')} />
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
