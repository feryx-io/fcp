import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { lineBindings } from '@/api/endpoints'
import type { LineBinding } from '@/types'
import { formatDateTime } from '@/lib/utils'

const createSchema = z.object({
  extension_id: z.string().min(1, 'Extension is required'),
  device_id: z.string().optional(),
  phone_number_id: z.string().optional(),
  line_index: z.coerce.number().min(1).optional(),
  label: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>

export function LineBindingsPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['line-bindings'],
    queryFn: lineBindings.list,
  })

  const createMutation = useMutation({
    mutationFn: lineBindings.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['line-bindings'] })
      setDialogOpen(false)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: lineBindings.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['line-bindings'] }),
  })

  const columns: ColumnDef<LineBinding>[] = [
    {
      accessorKey: 'extension_id',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Extension <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'device_id',
      header: 'Device',
      cell: ({ row }) => row.original.device_id || '-',
    },
    {
      accessorKey: 'phone_number_id',
      header: 'Phone Number',
      cell: ({ row }) => row.original.phone_number_id || '-',
    },
    {
      accessorKey: 'line_index',
      header: 'Line',
    },
    {
      accessorKey: 'label',
      header: 'Label',
      cell: ({ row }) => row.original.label || '-',
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm('Delete this line binding?')) {
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

  const { register, handleSubmit, reset, formState: { errors } } = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
  })

  const onSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      device_id: data.device_id || undefined,
      phone_number_id: data.phone_number_id || undefined,
      label: data.label || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Line Bindings</h1>
          <p className="text-muted-foreground">Manage extension-device-number relationships</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Binding
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="extension_id" searchPlaceholder="Search bindings..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Line Binding</DialogTitle>
            <DialogDescription>Bind an extension to a device and phone number.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="extension_id">Extension ID *</Label>
              <Input id="extension_id" {...register('extension_id')} />
              {errors.extension_id && <p className="text-sm text-destructive">{errors.extension_id.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="device_id">Device ID</Label>
              <Input id="device_id" {...register('device_id')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="phone_number_id">Phone Number ID</Label>
              <Input id="phone_number_id" {...register('phone_number_id')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="line_index">Line Index</Label>
                <Input id="line_index" type="number" min="1" defaultValue="1" {...register('line_index')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="label">Label</Label>
                <Input id="label" {...register('label')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
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
    </div>
  )
}
