import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown } from 'lucide-react'
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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
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

type CreateForm = z.infer<typeof createSchema>

export function DevicesPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })

  const createMutation = useMutation({
    mutationFn: devices.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setDialogOpen(false)
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete device "${row.original.name || row.original.mac_address}"?`)) {
                  deleteMutation.mutate(row.original.id)
                }
              }}
            >
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  const { register, handleSubmit, reset, setValue, formState: { errors } } = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
  })

  const onSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      name: data.name || undefined,
      firmware_version: data.firmware_version || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Devices</h1>
          <p className="text-muted-foreground">Manage SIP phones and devices</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Device
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search devices..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Device</DialogTitle>
            <DialogDescription>Register a new SIP device.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="mac_address">MAC Address *</Label>
              <Input id="mac_address" placeholder="AA:BB:CC:DD:EE:FF" {...register('mac_address')} />
              {errors.mac_address && <p className="text-sm text-destructive">{errors.mac_address.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Vendor *</Label>
                <Select onValueChange={(v) => setValue('vendor', v)}>
                  <SelectTrigger><SelectValue placeholder="Select vendor" /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="polycom">Polycom</SelectItem>
                    <SelectItem value="yealink">Yealink</SelectItem>
                    <SelectItem value="cisco">Cisco</SelectItem>
                    <SelectItem value="grandstream">Grandstream</SelectItem>
                    <SelectItem value="other">Other</SelectItem>
                  </SelectContent>
                </Select>
                {errors.vendor && <p className="text-sm text-destructive">{errors.vendor.message}</p>}
              </div>
              <div className="space-y-2">
                <Label htmlFor="model">Model *</Label>
                <Input id="model" {...register('model')} />
                {errors.model && <p className="text-sm text-destructive">{errors.model.message}</p>}
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input id="name" placeholder="e.g., Lobby Phone" {...register('name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="firmware_version">Firmware Version</Label>
              <Input id="firmware_version" {...register('firmware_version')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
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
    </div>
  )
}
