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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { phoneNumbers } from '@/api/endpoints'
import type { PhoneNumber } from '@/types'

const createSchema = z.object({
  number: z.string().min(1, 'Number is required').regex(/^\+?[0-9]+$/, 'Invalid phone number format'),
  type: z.string().optional(),
  display_name: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>

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
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const createMutation = useMutation({
    mutationFn: phoneNumbers.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })
      setDialogOpen(false)
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete number "${row.original.number}"?`)) {
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

  const { register, handleSubmit, reset, setValue, formState: { errors } } = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
  })

  const onSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      type: data.type || undefined,
      display_name: data.display_name || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Phone Numbers</h1>
          <p className="text-muted-foreground">Manage DID numbers and E911 addresses</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Number
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="number" searchPlaceholder="Search numbers..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Phone Number</DialogTitle>
            <DialogDescription>Add a new DID number to the inventory.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="number">Phone Number *</Label>
              <Input id="number" placeholder="+15551234567" {...register('number')} />
              {errors.number && <p className="text-sm text-destructive">{errors.number.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Type</Label>
              <Select onValueChange={(v) => setValue('type', v)}>
                <SelectTrigger><SelectValue placeholder="Select type" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="toll_free">Toll Free</SelectItem>
                  <SelectItem value="international">International</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="display_name">Display Name</Label>
              <Input id="display_name" {...register('display_name')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
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
    </div>
  )
}
