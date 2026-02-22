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
import { formatDateTime } from '@/lib/utils'

const createSchema = z.object({
  mac_address: z.string().min(1, 'MAC address is required').regex(/^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$/, 'Invalid MAC format (e.g., AA:BB:CC:DD:EE:FF)'),
  vendor: z.string().min(1, 'Vendor is required'),
  model: z.string().min(1, 'Model is required'),
  name: z.string().optional(),
  firmware_version: z.string().optional(),
})

const editSchema = z.object({
  name: z.string().optional(),
  vendor: z.string().min(1, 'Vendor is required'),
  model: z.string().min(1, 'Model is required'),
  firmware_version: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

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
      cell: ({ row }) => row.original.name || row.original.mac_address,
    },
    {
      accessorKey: 'mac_address',
      header: 'MAC Address',
    },
    {
      accessorKey: 'vendor',
      header: 'Vendor',
    },
    {
      accessorKey: 'model',
      header: 'Model',
    },
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.status === 'online' ? 'success' : 'secondary'}>
          {row.original.status}
        </Badge>
      ),
    },
    {
      accessorKey: 'last_seen',
      header: 'Last Seen',
      cell: ({ row }) => row.original.last_seen ? formatDateTime(row.original.last_seen) : 'Never',
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
                if (confirm(`Delete device "${row.original.name || row.original.mac_address}"?`)) {
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
      name: editItem.name || '',
      vendor: editItem.vendor,
      model: editItem.model,
      firmware_version: editItem.firmware_version || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      name: data.name || undefined,
      firmware_version: data.firmware_version || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        name: data.name || undefined,
        vendor: data.vendor,
        model: data.model,
        firmware_version: data.firmware_version || undefined,
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
              <Label htmlFor="c-mac_address">MAC Address *</Label>
              <Input id="c-mac_address" placeholder="AA:BB:CC:DD:EE:FF" {...createForm.register('mac_address')} />
              {createForm.formState.errors.mac_address && <p className="text-sm text-destructive">{createForm.formState.errors.mac_address.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Vendor *</Label>
                <Select onValueChange={(v) => createForm.setValue('vendor', v)}>
                  <SelectTrigger><SelectValue placeholder="Select vendor" /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="polycom">Polycom</SelectItem>
                    <SelectItem value="yealink">Yealink</SelectItem>
                    <SelectItem value="cisco">Cisco</SelectItem>
                    <SelectItem value="grandstream">Grandstream</SelectItem>
                    <SelectItem value="other">Other</SelectItem>
                  </SelectContent>
                </Select>
                {createForm.formState.errors.vendor && <p className="text-sm text-destructive">{createForm.formState.errors.vendor.message}</p>}
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-model">Model *</Label>
                <Input id="c-model" {...createForm.register('model')} />
                {createForm.formState.errors.model && <p className="text-sm text-destructive">{createForm.formState.errors.model.message}</p>}
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-name">Name</Label>
              <Input id="c-name" placeholder="e.g., Lobby Phone" {...createForm.register('name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-firmware_version">Firmware Version</Label>
              <Input id="c-firmware_version" {...createForm.register('firmware_version')} />
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
            <DialogTitle>Edit Device: {editItem?.name || editItem?.mac_address}</DialogTitle>
            <DialogDescription>Update device details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-name">Name</Label>
              <Input id="e-name" {...editForm.register('name')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Vendor *</Label>
                <Select defaultValue={editItem?.vendor} onValueChange={(v) => editForm.setValue('vendor', v)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="polycom">Polycom</SelectItem>
                    <SelectItem value="yealink">Yealink</SelectItem>
                    <SelectItem value="cisco">Cisco</SelectItem>
                    <SelectItem value="grandstream">Grandstream</SelectItem>
                    <SelectItem value="other">Other</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-model">Model *</Label>
                <Input id="e-model" {...editForm.register('model')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-firmware_version">Firmware Version</Label>
              <Input id="e-firmware_version" {...editForm.register('firmware_version')} />
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
