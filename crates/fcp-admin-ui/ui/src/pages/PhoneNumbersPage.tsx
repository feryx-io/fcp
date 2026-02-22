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
import { phoneNumbers } from '@/api/endpoints'
import type { PhoneNumber } from '@/types'

const createSchema = z.object({
  number: z.string().min(1, 'Number is required').regex(/^\+?[0-9]+$/, 'Invalid phone number format'),
  type: z.string().optional(),
  display_name: z.string().optional(),
})

const editSchema = z.object({
  display_name: z.string().optional(),
  type: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const statusBadge = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'available': return 'secondary' as const
    case 'reserved': return 'warning' as const
    case 'porting': return 'default' as const
    default: return 'outline' as const
  }
}

export function PhoneNumbersPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<PhoneNumber | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const createMutation = useMutation({
    mutationFn: phoneNumbers.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<EditForm> }) =>
      phoneNumbers.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: phoneNumbers.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['phone-numbers'] }),
  })

  const columns: ColumnDef<PhoneNumber>[] = [
    {
      accessorKey: 'number',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Number <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'display_name',
      header: 'Display Name',
      cell: ({ row }) => row.original.display_name || '-',
    },
    {
      accessorKey: 'type',
      header: 'Type',
      cell: ({ row }) => <Badge variant="outline">{row.original.type}</Badge>,
    },
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => <Badge variant={statusBadge(row.original.status)}>{row.original.status}</Badge>,
    },
    {
      accessorKey: 'tenant_id',
      header: 'Tenant',
      cell: ({ row }) => row.original.tenant_id || 'Unassigned',
    },
    {
      accessorKey: 'e911_address',
      header: 'E911',
      cell: ({ row }) => row.original.e911_address ? (
        <Badge variant="success">Configured</Badge>
      ) : (
        <Badge variant="warning">Not set</Badge>
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
                if (confirm(`Delete number "${row.original.number}"?`)) {
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
      display_name: editItem.display_name || '',
      type: editItem.type || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      type: data.type || undefined,
      display_name: data.display_name || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        display_name: data.display_name || undefined,
        type: data.type || undefined,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Phone Numbers</h1>
          <p className="text-muted-foreground">Manage DID numbers and E911 addresses</p>
        </div>
        <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Number
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="number" searchPlaceholder="Search numbers..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Phone Number</DialogTitle>
            <DialogDescription>Add a new DID number to the inventory.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-number">Phone Number *</Label>
              <Input id="c-number" placeholder="+15551234567" {...createForm.register('number')} />
              {createForm.formState.errors.number && <p className="text-sm text-destructive">{createForm.formState.errors.number.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Type</Label>
              <Select onValueChange={(v) => createForm.setValue('type', v)}>
                <SelectTrigger><SelectValue placeholder="Select type" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="toll_free">Toll Free</SelectItem>
                  <SelectItem value="international">International</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-display_name">Display Name</Label>
              <Input id="c-display_name" {...createForm.register('display_name')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Adding...' : 'Add Number'}
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
            <DialogTitle>Edit Number: {editItem?.number}</DialogTitle>
            <DialogDescription>Update phone number details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-display_name">Display Name</Label>
              <Input id="e-display_name" {...editForm.register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label>Type</Label>
              <Select defaultValue={editItem?.type} onValueChange={(v) => editForm.setValue('type', v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="toll_free">Toll Free</SelectItem>
                  <SelectItem value="international">International</SelectItem>
                </SelectContent>
              </Select>
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
