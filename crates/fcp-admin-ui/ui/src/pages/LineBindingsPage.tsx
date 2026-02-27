import { useState, useMemo } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { lineBindings, extensions, devices, phoneNumbers } from '@/api/endpoints'
import type { LineBinding } from '@/types'
import { formatDateTime } from '@/lib/utils'

const createSchema = z.object({
  extension_id: z.string().min(1, 'Extension is required'),
  device_id: z.string().optional(),
  phone_number_id: z.string().optional(),
  line_index: z.coerce.number().min(1).optional(),
  label: z.string().optional(),
})

const editSchema = z.object({
  device_id: z.string().optional(),
  phone_number_id: z.string().optional(),
  line_index: z.coerce.number().min(1).optional(),
  label: z.string().optional(),
  no_answer_timeout_secs: z.coerce.number().min(5).max(300).optional(),
  forward_on_no_answer: z.string().optional(),
  forward_on_busy: z.string().optional(),
  forward_unconditional: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function LineBindingsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<LineBinding | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['line-bindings'],
    queryFn: lineBindings.list,
  })

  const { data: extensionList = [] } = useQuery({
    queryKey: ['extensions'],
    queryFn: extensions.list,
  })
  const { data: deviceList = [] } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })
  const { data: phoneNumberList = [] } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const extensionMap = useMemo(() => new Map(extensionList.map(e => [e.id, e])), [extensionList])
  const deviceMap = useMemo(() => new Map(deviceList.map(d => [d.id, d])), [deviceList])
  const phoneNumberMap = useMemo(() => new Map(phoneNumberList.map(p => [p.id, p])), [phoneNumberList])

  const createMutation = useMutation({
    mutationFn: lineBindings.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['line-bindings'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<EditForm> }) =>
      lineBindings.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['line-bindings'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: lineBindings.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['line-bindings'] }),
  })

  const columns: ColumnDef<LineBinding>[] = [
    {
      accessorKey: 'extension_id',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Extension <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => {
        const ext = extensionMap.get(row.original.extension_id)
        return ext ? (ext.display_name ? `${ext.number} - ${ext.display_name}` : ext.number) : row.original.extension_id
      },
    },
    {
      accessorKey: 'device_id',
      header: 'Device',
      cell: ({ row }) => {
        if (!row.original.device_id) return '-'
        const dev = deviceMap.get(row.original.device_id)
        return dev ? dev.name : row.original.device_id
      },
    },
    {
      accessorKey: 'phone_number_id',
      header: 'Phone Number',
      cell: ({ row }) => {
        if (!row.original.phone_number_id) return '-'
        const pn = phoneNumberMap.get(row.original.phone_number_id)
        return pn ? (pn.cnam ? `${pn.e164} - ${pn.cnam}` : pn.e164) : row.original.phone_number_id
      },
    },
    {
      accessorKey: 'line_index',
      header: 'Line',
    },
    {
      accessorKey: 'label',
      header: 'Label',
      cell: ({ row }) => row.original.label || '-',
    },
    {
      accessorKey: 'no_answer_timeout_secs',
      header: 'Ring Timeout',
      cell: ({ row }) => `${row.original.no_answer_timeout_secs}s`,
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
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm('Delete this line binding?')) {
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
      device_id: editItem.device_id || '',
      phone_number_id: editItem.phone_number_id || '',
      line_index: editItem.line_index,
      label: editItem.label || '',
      no_answer_timeout_secs: editItem.no_answer_timeout_secs,
      forward_on_no_answer: editItem.forward_on_no_answer || '',
      forward_on_busy: editItem.forward_on_busy || '',
      forward_unconditional: editItem.forward_unconditional || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      device_id: data.device_id || undefined,
      phone_number_id: data.phone_number_id || undefined,
      label: data.label || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        device_id: data.device_id || undefined,
        phone_number_id: data.phone_number_id || undefined,
        line_index: data.line_index,
        label: data.label || undefined,
        no_answer_timeout_secs: data.no_answer_timeout_secs,
        forward_on_no_answer: data.forward_on_no_answer || undefined,
        forward_on_busy: data.forward_on_busy || undefined,
        forward_unconditional: data.forward_unconditional || undefined,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Line Bindings</h1>
          <p className="text-muted-foreground">Manage extension-device-number relationships</p>
        </div>
        <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Binding
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="extension_id" searchPlaceholder="Search bindings..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Line Binding</DialogTitle>
            <DialogDescription>Bind an extension to a device and phone number.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label>Extension *</Label>
              <Select onValueChange={(v) => createForm.setValue('extension_id', v)}>
                <SelectTrigger><SelectValue placeholder="Select extension" /></SelectTrigger>
                <SelectContent>
                  {extensionList.map(ext => (
                    <SelectItem key={ext.id} value={ext.id}>
                      {ext.display_name ? `${ext.number} - ${ext.display_name}` : ext.number}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {createForm.formState.errors.extension_id && <p className="text-sm text-destructive">{createForm.formState.errors.extension_id.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Device</Label>
              <Select onValueChange={(v) => createForm.setValue('device_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="None" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {deviceList.map(dev => (
                    <SelectItem key={dev.id} value={dev.id}>{dev.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Phone Number</Label>
              <Select onValueChange={(v) => createForm.setValue('phone_number_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="None" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {phoneNumberList.map(pn => (
                    <SelectItem key={pn.id} value={pn.id}>
                      {pn.cnam ? `${pn.e164} - ${pn.cnam}` : pn.e164}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-line_index">Line Index</Label>
                <Input id="c-line_index" type="number" min="1" defaultValue="1" {...createForm.register('line_index')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-label">Label</Label>
                <Input id="c-label" {...createForm.register('label')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Binding'}
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
            <DialogTitle>Edit Line Binding</DialogTitle>
            <DialogDescription>Update binding for extension {editItem ? (extensionMap.get(editItem.extension_id)?.number || editItem.extension_id) : ''}.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label>Device</Label>
              <Select defaultValue={editItem?.device_id || '__none__'} onValueChange={(v) => editForm.setValue('device_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {deviceList.map(dev => (
                    <SelectItem key={dev.id} value={dev.id}>{dev.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Phone Number</Label>
              <Select defaultValue={editItem?.phone_number_id || '__none__'} onValueChange={(v) => editForm.setValue('phone_number_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {phoneNumberList.map(pn => (
                    <SelectItem key={pn.id} value={pn.id}>
                      {pn.cnam ? `${pn.e164} - ${pn.cnam}` : pn.e164}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-line_index">Line Index</Label>
                <Input id="e-line_index" type="number" min="1" {...editForm.register('line_index')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-label">Label</Label>
                <Input id="e-label" {...editForm.register('label')} />
              </div>
            </div>
            <div className="border-t pt-4 mt-2">
              <h4 className="text-sm font-medium mb-3">Call Handling</h4>
              <div className="space-y-3">
                <div className="space-y-2">
                  <Label htmlFor="e-no_answer_timeout_secs">Ring Timeout (seconds)</Label>
                  <Input id="e-no_answer_timeout_secs" type="number" min="5" max="300" {...editForm.register('no_answer_timeout_secs')} />
                  {editForm.formState.errors.no_answer_timeout_secs && <p className="text-sm text-destructive">{editForm.formState.errors.no_answer_timeout_secs.message}</p>}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-forward_unconditional">Forward Unconditional</Label>
                  <Input id="e-forward_unconditional" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_unconditional')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-forward_on_no_answer">Forward on No Answer</Label>
                  <Input id="e-forward_on_no_answer" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_on_no_answer')} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="e-forward_on_busy">Forward on Busy</Label>
                  <Input id="e-forward_on_busy" placeholder="voicemail, extension, or SIP URI" {...editForm.register('forward_on_busy')} />
                </div>
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
