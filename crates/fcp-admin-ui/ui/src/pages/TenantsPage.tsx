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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { tenants } from '@/api/endpoints'
import type { Tenant } from '@/types'
import { formatDateTime } from '@/lib/utils'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required').max(255),
  description: z.string().optional(),
  contact_email: z.string().email().optional().or(z.literal('')),
  contact_phone: z.string().optional(),
  max_users: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_phone_numbers: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_concurrent_calls: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
})

type CreateForm = z.infer<typeof createSchema>

const statusBadge = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'suspended': return 'destructive' as const
    case 'trial': return 'warning' as const
    default: return 'secondary' as const
  }
}

export function TenantsPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
  })

  const createMutation = useMutation({
    mutationFn: tenants.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] })
      setDialogOpen(false)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: tenants.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tenants'] }),
  })

  const columns: ColumnDef<Tenant>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'description',
      header: 'Description',
      cell: ({ row }) => row.original.description || '-',
    },
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => <Badge variant={statusBadge(row.original.status)}>{row.original.status}</Badge>,
    },
    {
      accessorKey: 'contact_email',
      header: 'Contact',
      cell: ({ row }) => row.original.contact_email || '-',
    },
    {
      accessorKey: 'max_users',
      header: 'Max Users',
      cell: ({ row }) => row.original.max_users ?? 'Unlimited',
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
                if (confirm(`Delete tenant "${row.original.name}"? This action cannot be undone.`)) {
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
      description: data.description || undefined,
      contact_email: data.contact_email || undefined,
      contact_phone: data.contact_phone || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Tenants</h1>
          <p className="text-muted-foreground">Manage tenant organizations</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Tenant
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search tenants..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Tenant</DialogTitle>
            <DialogDescription>Add a new tenant organization.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">Name *</Label>
              <Input id="name" {...register('name')} />
              {errors.name && <p className="text-sm text-destructive">{errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Input id="description" {...register('description')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="contact_email">Contact Email</Label>
                <Input id="contact_email" type="email" {...register('contact_email')} />
                {errors.contact_email && <p className="text-sm text-destructive">{errors.contact_email.message}</p>}
              </div>
              <div className="space-y-2">
                <Label htmlFor="contact_phone">Contact Phone</Label>
                <Input id="contact_phone" {...register('contact_phone')} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="max_users">Max Users</Label>
                <Input id="max_users" type="number" min="0" {...register('max_users')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="max_phone_numbers">Max Numbers</Label>
                <Input id="max_phone_numbers" type="number" min="0" {...register('max_phone_numbers')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="max_concurrent_calls">Max Calls</Label>
                <Input id="max_concurrent_calls" type="number" min="0" {...register('max_concurrent_calls')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Tenant'}
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
