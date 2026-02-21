import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  Plus, MoreHorizontal, ArrowUpDown, Send, Ban,
} from 'lucide-react'
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
import { porting } from '@/api/endpoints'
import type { PortRequest, PortStatus } from '@/types'

const statusBadge = (status: PortStatus) => {
  switch (status) {
    case 'draft': return 'secondary' as const
    case 'submitted': case 'pending': return 'default' as const
    case 'approved': case 'scheduled': return 'success' as const
    case 'in_progress': return 'warning' as const
    case 'completed': return 'success' as const
    case 'rejected': case 'failed': return 'destructive' as const
    case 'cancelled': return 'outline' as const
    default: return 'outline' as const
  }
}

const createPortSchema = z.object({
  phone_numbers: z.string().min(1, 'At least one number required'),
  customer_name: z.string().min(1, 'Customer name required'),
  account_number: z.string().min(1, 'Account number required'),
  pin: z.string().optional(),
  losing_carrier: z.string().min(1, 'Losing carrier required'),
  authorized_contact_name: z.string().min(1, 'Contact name required'),
  authorized_contact_phone: z.string().min(1, 'Contact phone required'),
  authorized_contact_email: z.string().email('Valid email required'),
  street: z.string().min(1, 'Street required'),
  city: z.string().min(1, 'City required'),
  state: z.string().min(1, 'State required'),
  postal_code: z.string().min(1, 'Postal code required'),
  country: z.string().default('US'),
})

type CreatePortForm = z.infer<typeof createPortSchema>

export function PortingPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [selectedPort, setSelectedPort] = useState<PortRequest | null>(null)
  const queryClient = useQueryClient()

  const { data: portList = [], isLoading } = useQuery({
    queryKey: ['port-requests'],
    queryFn: porting.list,
  })

  const createMutation = useMutation({
    mutationFn: porting.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['port-requests'] })
      setDialogOpen(false)
    },
  })

  const submitMutation = useMutation({
    mutationFn: porting.submit,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['port-requests'] }),
  })

  const cancelMutation = useMutation({
    mutationFn: porting.cancel,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['port-requests'] }),
  })

  const deleteMutation = useMutation({
    mutationFn: porting.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['port-requests'] }),
  })

  const columns: ColumnDef<PortRequest>[] = [
    {
      accessorKey: 'id',
      header: 'ID',
      cell: ({ row }) => (
        <button
          className="text-primary hover:underline font-mono text-xs"
          onClick={() => setSelectedPort(row.original)}
        >
          {row.original.id.slice(0, 16)}...
        </button>
      ),
    },
    {
      accessorKey: 'phone_numbers',
      header: 'Numbers',
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.phone_numbers[0]}</span>
          {row.original.phone_numbers.length > 1 && (
            <span className="text-xs text-muted-foreground ml-1">
              +{row.original.phone_numbers.length - 1} more
            </span>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'status',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Status <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <Badge variant={statusBadge(row.original.status)}>
          {row.original.status.replace(/_/g, ' ')}
        </Badge>
      ),
    },
    {
      accessorKey: 'customer_name',
      header: 'Customer',
    },
    {
      accessorKey: 'losing_carrier',
      header: 'From Carrier',
    },
    {
      accessorKey: 'created_at',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Created <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => new Date(row.original.created_at).toLocaleDateString(),
    },
    {
      id: 'actions',
      cell: ({ row }) => {
        const port = row.original
        return (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon">
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {port.status === 'draft' && (
                <DropdownMenuItem onClick={() => submitMutation.mutate(port.id)}>
                  <Send className="mr-2 h-4 w-4" />
                  Submit
                </DropdownMenuItem>
              )}
              {['draft', 'submitted', 'pending'].includes(port.status) && (
                <DropdownMenuItem onClick={() => cancelMutation.mutate(port.id)}>
                  <Ban className="mr-2 h-4 w-4" />
                  Cancel
                </DropdownMenuItem>
              )}
              {port.status === 'draft' && (
                <DropdownMenuItem
                  className="text-destructive"
                  onClick={() => {
                    if (confirm('Delete this port request?')) {
                      deleteMutation.mutate(port.id)
                    }
                  }}
                >
                  Delete
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )
      },
    },
  ]

  const {
    register, handleSubmit, reset,
    formState: { errors },
  } = useForm<CreatePortForm>({
    resolver: zodResolver(createPortSchema),
    defaultValues: { country: 'US' },
  })

  const onSubmit = (data: CreatePortForm) => {
    createMutation.mutate({
      phone_numbers: data.phone_numbers.split(/[,\n]+/).map((n) => n.trim()).filter(Boolean),
      customer_name: data.customer_name,
      account_number: data.account_number,
      pin: data.pin || undefined,
      losing_carrier: data.losing_carrier,
      authorized_contact_name: data.authorized_contact_name,
      authorized_contact_phone: data.authorized_contact_phone,
      authorized_contact_email: data.authorized_contact_email,
      service_address: {
        street: data.street,
        city: data.city,
        state: data.state,
        postal_code: data.postal_code,
        country: data.country,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Number Porting</h1>
          <p className="text-muted-foreground">Manage phone number port requests</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" />
          New Port Request
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={portList} searchKey="customer_name" searchPlaceholder="Search by customer..." />
      )}

      {/* Create Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>New Port Request</DialogTitle>
            <DialogDescription>Submit a request to port phone numbers from another carrier.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label>Phone Numbers * (comma-separated)</Label>
              <Input placeholder="+15551234567, +15551234568" {...register('phone_numbers')} />
              {errors.phone_numbers && <p className="text-sm text-destructive">{errors.phone_numbers.message}</p>}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Customer Name *</Label>
                <Input {...register('customer_name')} />
                {errors.customer_name && <p className="text-sm text-destructive">{errors.customer_name.message}</p>}
              </div>
              <div className="space-y-2">
                <Label>Account Number *</Label>
                <Input {...register('account_number')} />
                {errors.account_number && <p className="text-sm text-destructive">{errors.account_number.message}</p>}
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Losing Carrier *</Label>
                <Input {...register('losing_carrier')} />
                {errors.losing_carrier && <p className="text-sm text-destructive">{errors.losing_carrier.message}</p>}
              </div>
              <div className="space-y-2">
                <Label>PIN</Label>
                <Input {...register('pin')} />
              </div>
            </div>
            <p className="text-sm font-medium pt-2">Authorized Contact</p>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label>Name *</Label>
                <Input {...register('authorized_contact_name')} />
              </div>
              <div className="space-y-2">
                <Label>Phone *</Label>
                <Input {...register('authorized_contact_phone')} />
              </div>
              <div className="space-y-2">
                <Label>Email *</Label>
                <Input {...register('authorized_contact_email')} />
              </div>
            </div>
            <p className="text-sm font-medium pt-2">Service Address</p>
            <div className="space-y-2">
              <Input placeholder="Street" {...register('street')} />
            </div>
            <div className="grid grid-cols-3 gap-4">
              <Input placeholder="City" {...register('city')} />
              <Input placeholder="State" {...register('state')} />
              <Input placeholder="ZIP" {...register('postal_code')} />
            </div>
            <div className="flex justify-end gap-2 pt-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Request'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      {/* Detail Dialog */}
      <Dialog open={!!selectedPort} onOpenChange={() => setSelectedPort(null)}>
        <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Port Request Details</DialogTitle>
            <DialogDescription>Full port request information.</DialogDescription>
          </DialogHeader>
          {selectedPort && (
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div>
                <p className="text-xs text-muted-foreground">ID</p>
                <p className="font-mono text-xs break-all">{selectedPort.id}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Status</p>
                <Badge variant={statusBadge(selectedPort.status)}>
                  {selectedPort.status.replace(/_/g, ' ')}
                </Badge>
              </div>
              <div className="col-span-2">
                <p className="text-xs text-muted-foreground">Numbers</p>
                <p className="font-mono text-sm">{selectedPort.phone_numbers.join(', ')}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Customer</p>
                <p>{selectedPort.customer_name}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Account</p>
                <p className="font-mono">{selectedPort.account_number}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">From Carrier</p>
                <p>{selectedPort.losing_carrier}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">To Carrier</p>
                <p>{selectedPort.winning_carrier}</p>
              </div>
              <div className="col-span-2">
                <p className="text-xs text-muted-foreground">Service Address</p>
                <p className="text-sm">
                  {selectedPort.service_address.street}, {selectedPort.service_address.city},{' '}
                  {selectedPort.service_address.state} {selectedPort.service_address.postal_code}
                </p>
              </div>
              {selectedPort.requested_foc_date && (
                <div>
                  <p className="text-xs text-muted-foreground">Requested FOC Date</p>
                  <p>{new Date(selectedPort.requested_foc_date).toLocaleDateString()}</p>
                </div>
              )}
              {selectedPort.rejection_reason && (
                <div className="col-span-2">
                  <p className="text-xs text-muted-foreground">Rejection Reason</p>
                  <p className="text-destructive">{selectedPort.rejection_reason}</p>
                </div>
              )}
              {selectedPort.notes && (
                <div className="col-span-2">
                  <p className="text-xs text-muted-foreground">Notes</p>
                  <p>{selectedPort.notes}</p>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
