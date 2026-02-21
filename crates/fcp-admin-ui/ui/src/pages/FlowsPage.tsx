import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, MoreHorizontal, ArrowUpDown, Workflow, Pencil } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { flows } from '@/api/endpoints'
import type { FlowResponse } from '@/types'

const createFlowSchema = z.object({
  id: z.string().min(1, 'ID is required').regex(/^[a-z0-9-]+$/, 'Lowercase letters, numbers, and hyphens only'),
  name: z.string().min(1, 'Name is required'),
  description: z.string().optional(),
})

type CreateFlowForm = z.infer<typeof createFlowSchema>

export function FlowsPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const { data, isLoading } = useQuery({
    queryKey: ['flows'],
    queryFn: () => flows.list(),
  })

  const flowList = data?.flows ?? []

  const createMutation = useMutation({
    mutationFn: flows.create,
    onSuccess: (flow) => {
      queryClient.invalidateQueries({ queryKey: ['flows'] })
      setDialogOpen(false)
      navigate(`/admin/flows/${flow.id}`)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: flows.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['flows'] }),
  })

  const columns: ColumnDef<FlowResponse>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <button
            className="font-medium text-primary hover:underline"
            onClick={() => navigate(`/admin/flows/${row.original.id}`)}
          >
            {row.original.name}
          </button>
          {row.original.description && (
            <p className="text-xs text-muted-foreground">{row.original.description}</p>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'id',
      header: 'ID',
      cell: ({ row }) => <span className="font-mono text-xs">{row.original.id}</span>,
    },
    {
      accessorKey: 'version',
      header: 'Version',
      cell: ({ row }) => <Badge variant="outline">v{row.original.version}</Badge>,
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'success' : 'secondary'}>
          {row.original.enabled ? 'Active' : 'Disabled'}
        </Badge>
      ),
    },
    {
      accessorKey: 'nodes',
      header: 'Nodes',
      cell: ({ row }) => {
        const count = Object.keys(row.original.nodes || {}).length
        return <span className="text-sm">{count}</span>
      },
    },
    {
      accessorKey: 'updated_at',
      header: 'Last Updated',
      cell: ({ row }) => row.original.updated_at
        ? new Date(row.original.updated_at).toLocaleDateString()
        : '-',
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon">
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => navigate(`/admin/flows/${row.original.id}`)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit Flow
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete flow "${row.original.name}"?`)) {
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

  const {
    register, handleSubmit, reset,
    formState: { errors },
  } = useForm<CreateFlowForm>({ resolver: zodResolver(createFlowSchema) })

  const onSubmit = (data: CreateFlowForm) => {
    createMutation.mutate({
      id: data.id,
      name: data.name,
      description: data.description,
      initial_node: 'start',
      nodes: {
        start: { type: 'say', label: 'Start', text: 'Welcome', next: null },
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">IVR Flows</h1>
          <p className="text-muted-foreground">Build and manage interactive voice response flows</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" />
          New Flow
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={flowList} searchKey="name" searchPlaceholder="Search flows..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Workflow className="h-5 w-5" />
              Create New Flow
            </DialogTitle>
            <DialogDescription>Create a new IVR flow. You can edit it visually after creation.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="flow-id">Flow ID *</Label>
              <Input id="flow-id" placeholder="e.g., main-menu" {...register('id')} />
              {errors.id && <p className="text-sm text-destructive">{errors.id.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="flow-name">Display Name *</Label>
              <Input id="flow-name" placeholder="e.g., Main Menu" {...register('name')} />
              {errors.name && <p className="text-sm text-destructive">{errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="flow-desc">Description</Label>
              <Input id="flow-desc" placeholder="Optional description" {...register('description')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create & Edit'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
