import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { useForm } from 'react-hook-form'
import { Plus, MoreHorizontal, ArrowUpDown, Pencil, Trash2, RefreshCw } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { lcrRoutes, trunks } from '@/api/endpoints'
import type { LcrRoute, CreateLcrRouteRequest, SipTrunk } from '@/types'

interface RouteFormData {
  prefix: string
  trunk_id: string
  cost_per_minute: number
  priority: number
  enabled: boolean
  description: string
}

const defaultValues: RouteFormData = {
  prefix: '',
  trunk_id: '',
  cost_per_minute: 0,
  priority: 100,
  enabled: true,
  description: '',
}

function routeToForm(r: LcrRoute): RouteFormData {
  return {
    prefix: r.prefix,
    trunk_id: r.trunk_id,
    cost_per_minute: r.cost_per_minute,
    priority: r.priority,
    enabled: r.enabled,
    description: r.description ?? '',
  }
}

function formToRequest(f: RouteFormData): CreateLcrRouteRequest {
  return {
    prefix: f.prefix.trim(),
    trunk_id: f.trunk_id,
    cost_per_minute: f.cost_per_minute,
    priority: f.priority,
    enabled: f.enabled,
    // An empty description clears it on an edit.
    description: f.description,
  }
}

/**
 * A tenant's least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c):
 * what a prefix costs on each of its own trunks, which orders those trunks
 * for the numbers the prefix matches.
 */
export function LcrRoutesPage() {
  const queryClient = useQueryClient()
  const [createOpen, setCreateOpen] = useState(false)
  const [editRoute, setEditRoute] = useState<LcrRoute | null>(null)
  const [deleteRoute, setDeleteRoute] = useState<LcrRoute | null>(null)
  const [error, setError] = useState<string | null>(null)

  const createForm = useForm<RouteFormData>({ defaultValues })
  const editForm = useForm<RouteFormData>({ defaultValues })

  const { data: routes = [], isLoading, isFetching, refetch } = useQuery({
    queryKey: ['lcr-routes'],
    queryFn: lcrRoutes.list,
  })
  const { data: trunkList = [] } = useQuery({ queryKey: ['trunks'], queryFn: trunks.list })
  const trunkName = (id: string) => trunkList.find((t) => t.id === id)?.name ?? id

  const done = () => queryClient.invalidateQueries({ queryKey: ['lcr-routes'] })
  const createMutation = useMutation({
    mutationFn: (data: CreateLcrRouteRequest) => lcrRoutes.create(data),
    onSuccess: () => { done(); setCreateOpen(false); createForm.reset() },
    onError: (e: Error) => setError(e.message),
  })
  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateLcrRouteRequest> }) => lcrRoutes.update(id, data),
    onSuccess: () => { done(); setEditRoute(null) },
    onError: (e: Error) => setError(e.message),
  })
  const deleteMutation = useMutation({
    mutationFn: (id: string) => lcrRoutes.delete(id),
    onSuccess: () => { done(); setDeleteRoute(null) },
    onError: (e: Error) => setError(e.message),
  })

  const columns: ColumnDef<LcrRoute>[] = [
    {
      accessorKey: 'prefix',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Prefix <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => <span className="font-mono">{row.original.prefix}</span>,
    },
    {
      accessorKey: 'trunk_id',
      header: 'Trunk',
      cell: ({ row }) => <span className="font-medium">{trunkName(row.original.trunk_id)}</span>,
    },
    {
      accessorKey: 'cost_per_minute',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Cost / min <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => <span className="font-mono text-sm">{row.original.cost_per_minute}</span>,
    },
    {
      accessorKey: 'priority',
      header: 'Priority',
      cell: ({ row }) => <span className="font-mono text-sm">{row.original.priority}</span>,
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
      accessorKey: 'description',
      header: 'Description',
      cell: ({ row }) => <span className="text-sm text-muted-foreground">{row.original.description}</span>,
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => { setError(null); editForm.reset(routeToForm(row.original)); setEditRoute(row.original) }}>
              <Pencil className="h-3.5 w-3.5 mr-2" /> Edit
            </DropdownMenuItem>
            <DropdownMenuItem className="text-destructive" onClick={() => { setError(null); setDeleteRoute(row.original) }}>
              <Trash2 className="h-3.5 w-3.5 mr-2" /> Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Least-Cost Routes</h1>
          <p className="text-muted-foreground">What each destination costs on your trunks, which decides the order they are tried in</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} /> Refresh
          </Button>
          <Button onClick={() => { setError(null); createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Route
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">How routes apply</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-1">
          <p>
            When a number you dial matches a prefix here, the trunks priced for it are tried first — longest prefix,
            then cheapest, then lowest priority — one at a time, and then the platform's trunks unless you refuse them.
            Only the trunks priced for that number are offered it; your other trunks are not.
          </p>
          <p>
            A number no prefix matches takes your ordinary choice by trunk priority and weight. The cost orders trunks
            and nothing else: it is what the carrier charges, not what you are charged. Emergency calls do not use
            these routes. Use <strong>Test routing</strong> on the Routing Rules page to see what a number would do.
          </p>
        </CardContent>
      </Card>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={routes} searchKey="prefix" searchPlaceholder="Search prefixes..." />
      )}

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Add Least-Cost Route</DialogTitle>
            <DialogDescription>Price a prefix on one of your trunks.</DialogDescription>
          </DialogHeader>
          <RouteForm form={createForm} trunkList={trunkList} error={error}
            onSubmit={(d) => createMutation.mutate(formToRequest(d))} isSubmitting={createMutation.isPending} />
        </DialogContent>
      </Dialog>

      <Dialog open={!!editRoute} onOpenChange={() => setEditRoute(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Edit Route: {editRoute?.prefix}</DialogTitle>
            <DialogDescription>On {editRoute ? trunkName(editRoute.trunk_id) : ''}.</DialogDescription>
          </DialogHeader>
          <RouteForm form={editForm} trunkList={trunkList} error={error} submitLabel="Save Changes"
            onSubmit={(d) => editRoute && updateMutation.mutate({ id: editRoute.id, data: formToRequest(d) })}
            isSubmitting={updateMutation.isPending} />
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteRoute} onOpenChange={() => setDeleteRoute(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Route</DialogTitle>
            <DialogDescription>
              Delete the route for <strong>{deleteRoute?.prefix}</strong> on {deleteRoute ? trunkName(deleteRoute.trunk_id) : ''}?
            </DialogDescription>
          </DialogHeader>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRoute(null)}>Cancel</Button>
            <Button variant="destructive" onClick={() => deleteRoute && deleteMutation.mutate(deleteRoute.id)}>Delete</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function RouteForm({ form, trunkList, error, onSubmit, isSubmitting, submitLabel = 'Add Route' }: {
  form: ReturnType<typeof useForm<RouteFormData>>
  trunkList: SipTrunk[]
  error: string | null
  onSubmit: (data: RouteFormData) => void
  isSubmitting: boolean
  submitLabel?: string
}) {
  const { register, handleSubmit, watch, setValue } = form
  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
      <div className="grid grid-cols-2 gap-4">
        <div>
          <Label htmlFor="prefix">Prefix</Label>
          <Input id="prefix" className="font-mono" placeholder="+44" {...register('prefix')} />
        </div>
        <div>
          <Label>Trunk</Label>
          <Select value={watch('trunk_id') || undefined} onValueChange={(v) => setValue('trunk_id', v)}>
            <SelectTrigger><SelectValue placeholder="Select trunk..." /></SelectTrigger>
            <SelectContent>
              {trunkList.map((t) => (
                <SelectItem key={t.id} value={t.id}>{t.name}{t.enabled ? '' : ' (disabled)'}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label htmlFor="cost_per_minute">Cost per minute</Label>
          <Input id="cost_per_minute" type="number" step="0.000001" min="0"
            {...register('cost_per_minute', { valueAsNumber: true })} />
        </div>
        <div>
          <Label htmlFor="priority">Priority (breaks cost ties)</Label>
          <Input id="priority" type="number" min="0" max="1000" {...register('priority', { valueAsNumber: true })} />
        </div>
        <div className="col-span-2">
          <Label htmlFor="description">Description</Label>
          <Input id="description" placeholder="e.g. UK via the cheap carrier" {...register('description')} />
        </div>
        <div className="flex items-center gap-2">
          <Switch id="enabled" checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
          <Label htmlFor="enabled">Enabled</Label>
        </div>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <DialogFooter>
        <Button type="submit" disabled={isSubmitting || !watch('trunk_id')}>
          {isSubmitting ? 'Saving...' : submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}
