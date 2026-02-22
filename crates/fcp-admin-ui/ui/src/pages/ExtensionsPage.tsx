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
import { extensions } from '@/api/endpoints'
import type { Extension } from '@/types'

const createSchema = z.object({
  number: z.string().min(1, 'Extension number is required').regex(/^\d+$/, 'Must be numeric'),
  display_name: z.string().optional(),
  user_id: z.string().optional(),
})

const editSchema = z.object({
  display_name: z.string().optional(),
  user_id: z.string().optional(),
  type: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function ExtensionsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<Extension | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['extensions'],
    queryFn: extensions.list,
  })

  const createMutation = useMutation({
    mutationFn: extensions.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ number, data }: { number: string; data: Partial<EditForm> }) =>
      extensions.update(number, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: extensions.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['extensions'] }),
  })

  const columns: ColumnDef<Extension>[] = [
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
      cell: ({ row }) => <Badge variant="secondary">{row.original.type}</Badge>,
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
      accessorKey: 'user_id',
      header: 'User',
      cell: ({ row }) => row.original.user_id || '-',
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
                if (confirm(`Delete extension "${row.original.number}"?`)) {
                  deleteMutation.mutate(row.original.number)
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
      user_id: editItem.user_id || '',
      type: editItem.type || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      display_name: data.display_name || undefined,
      user_id: data.user_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      number: editItem.number,
      data: {
        display_name: data.display_name || undefined,
        user_id: data.user_id || undefined,
        type: data.type || undefined,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Extensions</h1>
          <p className="text-muted-foreground">Manage telephone extensions</p>
        </div>
        <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Extension
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="number" searchPlaceholder="Search extensions..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Extension</DialogTitle>
            <DialogDescription>Add a new telephone extension.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-number">Extension Number *</Label>
              <Input id="c-number" placeholder="1001" {...createForm.register('number')} />
              {createForm.formState.errors.number && <p className="text-sm text-destructive">{createForm.formState.errors.number.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-display_name">Display Name</Label>
              <Input id="c-display_name" {...createForm.register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-user_id">User ID</Label>
              <Input id="c-user_id" placeholder="Optional user association" {...createForm.register('user_id')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Extension'}
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
            <DialogTitle>Edit Extension: {editItem?.number}</DialogTitle>
            <DialogDescription>Update extension details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-display_name">Display Name</Label>
              <Input id="e-display_name" {...editForm.register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-user_id">User ID</Label>
              <Input id="e-user_id" {...editForm.register('user_id')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-type">Type</Label>
              <Input id="e-type" {...editForm.register('type')} />
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
