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
import { phoneNumbers } from '@/api/endpoints'
import type { PhoneNumber } from '@/types'

const createSchema = z.object({
  e164: z.string().min(1, 'E.164 number is required').regex(/^\+?[0-9]+$/, 'Invalid phone number format'),
  cnam: z.string().optional(),
  carrier: z.string().optional(),
})

const editSchema = z.object({
  cnam: z.string().optional(),
  recording_enabled: z.boolean().optional(),
  block_anonymous: z.boolean().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const stateBadge = (state: string) => {
  switch (state) {
    case 'active': return 'success' as const
    case 'available': case 'inventory': return 'secondary' as const
    case 'reserved': case 'pending': return 'warning' as const
    case 'porting': return 'default' as const
    case 'released': case 'disconnected': return 'destructive' as const
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
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
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
      accessorKey: 'e164',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Number <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'cnam',
      header: 'CNAM',
      cell: ({ row }) => row.original.cnam || '-',
    },
    {
      accessorKey: 'state',
      header: 'State',
      cell: ({ row }) => <Badge variant={stateBadge(row.original.state)}>{row.original.state}</Badge>,
    },
    {
      accessorKey: 'carrier',
      header: 'Carrier',
      cell: ({ row }) => row.original.carrier || '-',
    },
    {
      accessorKey: 'rate_center',
      header: 'Rate Center',
      cell: ({ row }) => row.original.rate_center || '-',
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
      accessorKey: 'tags',
      header: 'Tags',
      cell: ({ row }) => row.original.tags?.length
        ? row.original.tags.map((t) => <Badge key={t} variant="outline" className="mr-1">{t}</Badge>)
        : '-',
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
                if (confirm(`Delete number "${row.original.e164}"?`)) {
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
      cnam: editItem.cnam || '',
      recording_enabled: editItem.recording_enabled,
      block_anonymous: editItem.block_anonymous,
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      e164: data.e164,
      cnam: data.cnam || undefined,
      carrier: data.carrier || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        cnam: data.cnam || undefined,
        recording_enabled: data.recording_enabled,
        block_anonymous: data.block_anonymous,
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
        <DataTable columns={columns} data={items} searchKey="e164" searchPlaceholder="Search numbers..." />
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
              <Label htmlFor="c-e164">E.164 Number *</Label>
              <Input id="c-e164" placeholder="+15551234567" {...createForm.register('e164')} />
              {createForm.formState.errors.e164 && <p className="text-sm text-destructive">{createForm.formState.errors.e164.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-cnam">CNAM</Label>
                <Input id="c-cnam" placeholder="Caller ID name" {...createForm.register('cnam')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-carrier">Carrier</Label>
                <Input id="c-carrier" {...createForm.register('carrier')} />
              </div>
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
            <DialogTitle>Edit Number: {editItem?.e164}</DialogTitle>
            <DialogDescription>Update phone number settings.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-cnam">CNAM</Label>
              <Input id="e-cnam" placeholder="Caller ID name" {...editForm.register('cnam')} />
            </div>
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" {...editForm.register('recording_enabled')} className="rounded" />
                Recording enabled
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" {...editForm.register('block_anonymous')} className="rounded" />
                Block anonymous
              </label>
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
