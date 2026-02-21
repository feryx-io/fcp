import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, MoreHorizontal, ArrowUpDown, Shield, ShieldOff } from 'lucide-react'
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
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { routing } from '@/api/endpoints'
import type { PolicyRule } from '@/types'

const createRuleSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  priority: z.coerce.number().int().min(0).default(100),
  action: z.enum(['allow', 'deny']),
  destination_pattern: z.string().optional(),
  reason: z.string().optional(),
})

type CreateRuleForm = z.infer<typeof createRuleSchema>

export function RoutingPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: rules = [], isLoading } = useQuery({
    queryKey: ['routing-rules'],
    queryFn: routing.list,
  })

  const createMutation = useMutation({
    mutationFn: routing.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['routing-rules'] })
      setDialogOpen(false)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: routing.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['routing-rules'] }),
  })

  const columns: ColumnDef<PolicyRule>[] = [
    {
      accessorKey: 'priority',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Priority <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <span className="font-mono text-sm">{row.original.priority}</span>
      ),
    },
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'action',
      header: 'Action',
      cell: ({ row }) => (
        <Badge variant={row.original.action === 'allow' ? 'success' : 'destructive'}>
          <span className="flex items-center gap-1">
            {row.original.action === 'allow'
              ? <Shield className="h-3 w-3" />
              : <ShieldOff className="h-3 w-3" />
            }
            {row.original.action}
          </span>
        </Badge>
      ),
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'default' : 'secondary'}>
          {row.original.enabled ? 'Active' : 'Disabled'}
        </Badge>
      ),
    },
    {
      accessorKey: 'destination_match',
      header: 'Destination Pattern',
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {row.original.destination_match?.pattern ?? 'Any'}
        </span>
      ),
    },
    {
      accessorKey: 'applies_to',
      header: 'Applies To',
      cell: ({ row }) => {
        const a = row.original.applies_to
        if (a.all_users) return <span className="text-sm">All Users</span>
        const parts: string[] = []
        if (a.roles?.length) parts.push(`Roles: ${a.roles.join(', ')}`)
        if (a.users?.length) parts.push(`${a.users.length} user(s)`)
        if (a.groups?.length) parts.push(`${a.groups.length} group(s)`)
        return <span className="text-xs">{parts.join('; ') || 'Unspecified'}</span>
      },
    },
    {
      accessorKey: 'time_windows',
      header: 'Schedule',
      cell: ({ row }) => {
        const tw = row.original.time_windows
        if (!tw || tw.length === 0) return <span className="text-muted-foreground text-xs">Always</span>
        return <span className="text-xs">{tw.length} window(s)</span>
      },
    },
    {
      accessorKey: 'reason',
      header: 'Reason',
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground max-w-[150px] truncate block">
          {row.original.reason ?? '-'}
        </span>
      ),
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete rule "${row.original.name}"?`)) {
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
    register, handleSubmit, reset, setValue,
    formState: { errors },
  } = useForm<CreateRuleForm>({
    resolver: zodResolver(createRuleSchema),
    defaultValues: { priority: 100, action: 'allow' },
  })

  const onSubmit = (data: CreateRuleForm) => {
    createMutation.mutate({
      name: data.name,
      priority: data.priority,
      action: data.action,
      destination_match: data.destination_pattern
        ? { pattern: data.destination_pattern, tags: null }
        : undefined,
      reason: data.reason || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Routing Rules</h1>
          <p className="text-muted-foreground">Configure dial plan and policy rules</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" />
          Add Rule
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={rules} searchKey="name" searchPlaceholder="Search rules..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Routing Rule</DialogTitle>
            <DialogDescription>Add a new dial plan or policy rule.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">Rule Name *</Label>
              <Input id="name" {...register('name')} />
              {errors.name && <p className="text-sm text-destructive">{errors.name.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="priority">Priority</Label>
                <Input id="priority" type="number" {...register('priority')} />
              </div>
              <div className="space-y-2">
                <Label>Action</Label>
                <Select defaultValue="allow" onValueChange={(v) => setValue('action', v as 'allow' | 'deny')}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="allow">Allow</SelectItem>
                    <SelectItem value="deny">Deny</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="destination_pattern">Destination Pattern (regex)</Label>
              <Input id="destination_pattern" placeholder="e.g., ^\\+1\\d{10}$" {...register('destination_pattern')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="reason">Reason</Label>
              <Input id="reason" placeholder="Why this rule exists" {...register('reason')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Rule'}
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
