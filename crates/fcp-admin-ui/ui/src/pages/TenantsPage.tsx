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
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { tenants } from '@/api/endpoints'
import type { Tenant } from '@/types'
import { formatDateTime } from '@/lib/utils'
import { useIsPlatformAdmin } from '@/components/TenantSelector'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required').max(255),
  description: z.string().optional(),
  contact_email: z.string().email().optional().or(z.literal('')),
  contact_phone: z.string().optional(),
  max_users: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_phone_numbers: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_concurrent_calls: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
})

const editSchema = z.object({
  name: z.string().min(1, 'Name is required').max(255),
  description: z.string().optional(),
  contact_email: z.string().email().optional().or(z.literal('')),
  contact_phone: z.string().optional(),
  max_users: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_phone_numbers: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_concurrent_calls: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const statusBadge = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'suspended': return 'destructive' as const
    case 'trial': return 'warning' as const
    default: return 'secondary' as const
  }
}

export function TenantsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<Tenant | null>(null)
  const queryClient = useQueryClient()
  const isPlatformAdmin = useIsPlatformAdmin()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
  })

  const createMutation = useMutation({
    mutationFn: tenants.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<EditForm> }) =>
      tenants.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] })
      setEditItem(null)
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
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            {isPlatformAdmin && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive"
                  onClick={() => {
                    if (confirm(`Delete tenant "${row.original.name}"? This action cannot be undone.`)) {
                      deleteMutation.mutate(row.original.id)
                    }
                  }}
                >
                  <Trash2 className="mr-2 h-4 w-4" />
                  Delete
                </DropdownMenuItem>
              </>
            )}
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
      description: editItem.description || '',
      contact_email: editItem.contact_email || '',
      contact_phone: editItem.contact_phone || '',
      max_users: editItem.max_users ?? undefined,
      max_phone_numbers: editItem.max_phone_numbers ?? undefined,
      max_concurrent_calls: editItem.max_concurrent_calls ?? undefined,
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      description: data.description || undefined,
      contact_email: data.contact_email || undefined,
      contact_phone: data.contact_phone || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        ...data,
        description: data.description || undefined,
        contact_email: data.contact_email || undefined,
        contact_phone: data.contact_phone || undefined,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Tenants</h1>
          <p className="text-muted-foreground">
            {isPlatformAdmin
              ? 'Manage tenant organizations'
              : 'View and update your tenant settings'}
          </p>
        </div>
        {isPlatformAdmin && (
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Tenant
          </Button>
        )}
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search tenants..." />
      )}

      {/* Create Dialog — platform admins only */}
      {isPlatformAdmin && (
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Tenant</DialogTitle>
            <DialogDescription>Add a new tenant organization.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-name">Name *</Label>
              <Input id="c-name" {...createForm.register('name')} />
              {createForm.formState.errors.name && <p className="text-sm text-destructive">{createForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-description">Description</Label>
              <Input id="c-description" {...createForm.register('description')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-contact_email">Contact Email</Label>
                <Input id="c-contact_email" type="email" {...createForm.register('contact_email')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-contact_phone">Contact Phone</Label>
                <Input id="c-contact_phone" {...createForm.register('contact_phone')} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-max_users">Max Users</Label>
                <Input id="c-max_users" type="number" min="0" {...createForm.register('max_users')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-max_phone_numbers">Max Numbers</Label>
                <Input id="c-max_phone_numbers" type="number" min="0" {...createForm.register('max_phone_numbers')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-max_concurrent_calls">Max Calls</Label>
                <Input id="c-max_concurrent_calls" type="number" min="0" {...createForm.register('max_concurrent_calls')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
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
      )}

      {/* Edit Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); editForm.reset() } }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit Tenant: {editItem?.name}</DialogTitle>
            <DialogDescription>Update tenant details and quotas.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-name">Name *</Label>
              <Input id="e-name" {...editForm.register('name')} />
              {editForm.formState.errors.name && <p className="text-sm text-destructive">{editForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-description">Description</Label>
              <Input id="e-description" {...editForm.register('description')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-contact_email">Contact Email</Label>
                <Input id="e-contact_email" type="email" {...editForm.register('contact_email')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-contact_phone">Contact Phone</Label>
                <Input id="e-contact_phone" {...editForm.register('contact_phone')} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-max_users">Max Users</Label>
                <Input id="e-max_users" type="number" min="0" {...editForm.register('max_users')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-max_phone_numbers">Max Numbers</Label>
                <Input id="e-max_phone_numbers" type="number" min="0" {...editForm.register('max_phone_numbers')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-max_concurrent_calls">Max Calls</Label>
                <Input id="e-max_concurrent_calls" type="number" min="0" {...editForm.register('max_concurrent_calls')} />
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
