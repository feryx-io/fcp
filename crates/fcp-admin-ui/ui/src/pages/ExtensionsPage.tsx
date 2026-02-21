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
import { extensions } from '@/api/endpoints'
import type { Extension } from '@/types'

const createSchema = z.object({
  number: z.string().min(1, 'Extension number is required').regex(/^\d+$/, 'Must be numeric'),
  display_name: z.string().optional(),
  user_id: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>

export function ExtensionsPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['extensions'],
    queryFn: extensions.list,
  })

  const createMutation = useMutation({
    mutationFn: extensions.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setDialogOpen(false)
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete extension "${row.original.number}"?`)) {
                  deleteMutation.mutate(row.original.number)
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
      display_name: data.display_name || undefined,
      user_id: data.user_id || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Extensions</h1>
          <p className="text-muted-foreground">Manage telephone extensions</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Extension
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="number" searchPlaceholder="Search extensions..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Extension</DialogTitle>
            <DialogDescription>Add a new telephone extension.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="number">Extension Number *</Label>
              <Input id="number" placeholder="1001" {...register('number')} />
              {errors.number && <p className="text-sm text-destructive">{errors.number.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="display_name">Display Name</Label>
              <Input id="display_name" {...register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="user_id">User ID</Label>
              <Input id="user_id" placeholder="Optional user association" {...register('user_id')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
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
    </div>
  )
}
