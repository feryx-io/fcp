import { useState, useRef, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, Upload, Download, CheckCircle2, XCircle, Loader2, FileUp, AlertCircle, RefreshCw, Copy, KeyRound, EyeOff, Eye, ShieldCheck } from 'lucide-react'
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
import { Switch } from '@/components/ui/switch'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table'
import { devices, emergency, locations, tenants, users } from '@/api/endpoints'
import type { AddressValidation, Device, CreateDeviceRequest, UpdateDeviceRequest } from '@/types'
import { TenantField, useEffectiveTenantId, useTenantListFilter, useShowTenantColumn } from '@/components/TenantSelector'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  device_type: z.string().min(1, 'Device type is required'),
  sip_uri: z.string().optional(),
  mac_address: z.string().optional(),
  model: z.string().optional(),
  location: z.string().optional(),
  location_id: z.string().optional(),
  tenant_id: z.string().optional(),
})

const editSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  sip_uri: z.string().optional(),
  mac_address: z.string().optional(),
  model: z.string().optional(),
  firmware_version: z.string().optional(),
  location: z.string().optional(),
  location_id: z.string().optional(),
  enabled: z.boolean().optional(),
  tenant_id: z.string().optional(),
  // The address for this phone alone (docs/E911.md): left empty, the
  // phone is dispatched to its site.
  e911_address1: z.string().optional(),
  e911_address2: z.string().optional(),
  e911_city: z.string().optional(),
  e911_state: z.string().optional(),
  e911_postal_code: z.string().optional(),
  e911_country: z.string().optional(),
  // 'inherit' leaves it to the tenant's trusted device types.
  e911_trust_location: z.enum(['inherit', 'yes', 'no']).optional(),
  // Calling (docs/ENTERPRISE_CALLING.md §3.8): '' is no owner.
  owner_user_id: z.string().optional(),
  hoteling_host: z.boolean().optional(),
  sip_username: z.string().optional(),
  sip_password: z.string().optional(),
  remove_credentials: z.boolean().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const regBadge = (state: string) => {
  switch (state) {
    case 'registered': return 'success' as const
    case 'unregistered': return 'secondary' as const
    case 'expired': return 'warning' as const
    default: return 'outline' as const
  }
}

/// Generate a 192-bit provisioning token rendered as 48 hex chars.
/// Well above the 32-char minimum enforced by the backend.
function generateProvisioningToken(): string {
  const bytes = new Uint8Array(24)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

interface RevealedToken {
  token: string
  deviceName: string
  context: 'created' | 'rotated'
}

/** The credentials part of a device edit: removed, set, re-passworded, or left alone. */
function credentialChange(
  device: Device,
  data: EditForm,
): Pick<UpdateDeviceRequest, 'sip_username' | 'sip_password'> {
  if (data.remove_credentials) return { sip_username: null }
  const username = data.sip_username?.trim() ?? ''
  const password = data.sip_password ?? ''
  if (!username) return {}
  if (username !== (device.sip_username ?? '') || password) {
    return { sip_username: username, sip_password: password || undefined }
  }
  return {}
}

export function DevicesPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [editItem, setEditItem] = useState<Device | null>(null)
  const [revealedToken, setRevealedToken] = useState<RevealedToken | null>(null)
  const [showToken, setShowToken] = useState(false)
  const queryClient = useQueryClient()

  const { data: rawItems = [], isLoading, refetch, isFetching } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })
  const items = useTenantListFilter(rawItems)

  const { data: locationList = [] } = useQuery({
    queryKey: ['locations'],
    queryFn: locations.list,
  })

  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const locationName = (id: string | null) => {
    if (!id) return '-'
    const loc = locationList.find((l) => l.id === id)
    return loc ? loc.name : id
  }

  const showTenantColumn = useShowTenantColumn()
  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: showTenantColumn,
  })
  const tenantName = (id: string | null | undefined) => {
    if (!id) return '-'
    return tenantList.find((t) => t.id === id)?.name ?? id.slice(0, 8)
  }

  const createMutation = useMutation({
    mutationFn: devices.create,
    onSuccess: (created) => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setCreateOpen(false)
      if (created.provisioning_token) {
        setShowToken(false)
        setRevealedToken({
          token: created.provisioning_token,
          deviceName: created.name,
          context: 'created',
        })
      }
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateDeviceRequest }) =>
      devices.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setEditItem(null)
    },
  })

  const rotateTokenMutation = useMutation({
    mutationFn: ({ device, token }: { device: Device; token: string }) =>
      devices.update(device.id, { provisioning_token: token }),
    onSuccess: (_updated, { device, token }) => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      setShowToken(false)
      setRevealedToken({
        token,
        deviceName: device.name,
        context: 'rotated',
      })
    },
  })

  const deleteMutation = useMutation({
    mutationFn: devices.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['devices'] }),
  })

  // Validating a device's own address (docs/E911.md): the normalised
  // address is written back, and editing it clears the mark again.
  const [validation, setValidation] = useState<{
    name: string
    result: AddressValidation | null
    error: string | null
  } | null>(null)
  const validateMutation = useMutation({
    mutationFn: ({ id }: { id: string; name: string }) => emergency.validateDevice(id),
    onSuccess: (res, { name }) => {
      queryClient.invalidateQueries({ queryKey: ['devices'] })
      queryClient.invalidateQueries({ queryKey: ['emergency-readiness'] })
      setValidation({ name, result: res.validation, error: null })
    },
    onError: (e: Error, { name }) => setValidation({ name, result: null, error: e.message }),
  })

  const columns: ColumnDef<Device>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <span className="flex items-center gap-2">
          {row.original.name}
          {row.original.hoteling_host && <Badge variant="outline">Host</Badge>}
        </span>
      ),
    },
    {
      accessorKey: 'device_type',
      header: 'Type',
      cell: ({ row }) => (
        <Badge variant="secondary">{row.original.device_type.replace(/_/g, ' ')}</Badge>
      ),
    },
    {
      accessorKey: 'mac_address',
      header: 'MAC Address',
      cell: ({ row }) => row.original.mac_address || '-',
    },
    {
      accessorKey: 'model',
      header: 'Model',
      cell: ({ row }) => row.original.model || '-',
    },
    {
      accessorKey: 'sip_uri',
      header: 'SIP URI',
      cell: ({ row }) => row.original.sip_uri ? (
        <code className="text-xs">{row.original.sip_uri}</code>
      ) : <span className="text-muted-foreground">-</span>,
    },
    {
      id: 'e911',
      header: 'E911',
      cell: ({ row }) => {
        const override = row.original.e911_override
        if (!override) {
          return <span className="text-xs text-muted-foreground">its site</span>
        }
        return (
          <div className="space-y-1">
            <span className="text-xs">{override.address1}</span>
            <div>
              {override.validated ? (
                <Badge variant="success">validated</Badge>
              ) : (
                <Badge variant="outline">not validated</Badge>
              )}
            </div>
          </div>
        )
      },
    },
    {
      accessorKey: 'registration_state',
      header: 'Registration',
      cell: ({ row }) => (
        <Badge variant={regBadge(row.original.registration_state)}>
          {row.original.registration_state}
        </Badge>
      ),
    },
    {
      accessorKey: 'last_ip_address',
      header: 'IP Address',
      cell: ({ row }) => (
        <span className="text-sm font-mono text-muted-foreground">
          {row.original.last_ip_address || '—'}
        </span>
      ),
    },
    {
      accessorKey: 'location_id',
      header: 'Location',
      cell: ({ row }) => locationName(row.original.location_id),
    },
    {
      id: 'has_provisioning_token',
      header: 'Prov. Token',
      cell: ({ row }) => (
        <Badge variant={row.original.has_provisioning_token ? 'success' : 'secondary'}>
          {row.original.has_provisioning_token ? 'Set' : 'Not set'}
        </Badge>
      ),
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
    ...(showTenantColumn
      ? ([
          {
            id: 'tenant',
            header: 'Tenant',
            cell: ({ row }) => (
              <Badge variant="outline">{tenantName(row.original.tenant_id)}</Badge>
            ),
          },
        ] as ColumnDef<Device>[])
      : []),
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
            {row.original.e911_override && (
              <DropdownMenuItem
                onClick={() =>
                  validateMutation.mutate({ id: row.original.id, name: row.original.name })
                }
              >
                <ShieldCheck className="mr-2 h-4 w-4" />
                Validate E911 address
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              disabled={!row.original.mac_address || rotateTokenMutation.isPending}
              onClick={() => {
                const verb = row.original.has_provisioning_token ? 'Rotate' : 'Generate'
                if (!confirm(
                  `${verb} provisioning token for "${row.original.name}"? Any phone configured with the previous token will stop auto-provisioning until re-deployed.`,
                )) return
                rotateTokenMutation.mutate({
                  device: row.original,
                  token: generateProvisioningToken(),
                })
              }}
            >
              <KeyRound className="mr-2 h-4 w-4" />
              {row.original.has_provisioning_token ? 'Rotate Provisioning Token' : 'Generate Provisioning Token'}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete device "${row.original.name}"?`)) {
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

  const effectiveTenantId = useEffectiveTenantId()
  const createForm = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
    defaultValues: { tenant_id: effectiveTenantId },
  })
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      name: editItem.name,
      sip_uri: editItem.sip_uri || '',
      mac_address: editItem.mac_address || '',
      model: editItem.model || '',
      firmware_version: editItem.firmware_version || '',
      location: editItem.location || '',
      location_id: editItem.location_id || '',
      enabled: editItem.enabled,
      tenant_id: editItem.tenant_id ?? '',
      e911_address1: editItem.e911_override?.address1 ?? '',
      e911_address2: editItem.e911_override?.address2 ?? '',
      e911_city: editItem.e911_override?.city ?? '',
      e911_state: editItem.e911_override?.state ?? '',
      e911_postal_code: editItem.e911_override?.postal_code ?? '',
      e911_country: editItem.e911_override?.country ?? '',
      e911_trust_location:
        editItem.e911_trust_location == null
          ? 'inherit'
          : editItem.e911_trust_location
            ? 'yes'
            : 'no',
      owner_user_id: editItem.owner_user_id ?? '',
      hoteling_host: editItem.hoteling_host,
      sip_username: editItem.sip_username ?? '',
      sip_password: '',
      remove_credentials: false,
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      name: data.name,
      device_type: data.device_type,
      sip_uri: data.sip_uri || undefined,
      mac_address: data.mac_address || undefined,
      model: data.model || undefined,
      location: data.location || undefined,
      location_id: data.location_id || undefined,
      tenant_id: data.tenant_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        name: data.name,
        sip_uri: data.sip_uri || undefined,
        mac_address: data.mac_address || undefined,
        model: data.model || undefined,
        firmware_version: data.firmware_version || undefined,
        location: data.location || undefined,
        location_id: data.location_id || undefined,
        enabled: data.enabled,
        tenant_id: data.tenant_id || undefined,
        // An address needs its street, city, state and postal code to be
        // dispatchable; emptied, the override is removed.
        e911_override: data.e911_address1?.trim()
          ? {
              address1: data.e911_address1.trim(),
              address2: data.e911_address2?.trim() || undefined,
              city: data.e911_city?.trim() ?? '',
              state: data.e911_state?.trim() ?? '',
              postal_code: data.e911_postal_code?.trim() ?? '',
              country: data.e911_country?.trim() || 'US',
            }
          : null,
        e911_trust_location:
          data.e911_trust_location === 'inherit'
            ? null
            : data.e911_trust_location === 'yes',
        owner_user_id: data.owner_user_id || null,
        hoteling_host: data.hoteling_host ?? false,
        ...credentialChange(editItem, data),
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Devices</h1>
          <p className="text-muted-foreground">Manage SIP phones and devices</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} />
            Refresh
          </Button>
          <Button variant="outline" onClick={() => exportDevices(items)}>
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload className="mr-2 h-4 w-4" />
            Bulk Import
          </Button>
          <Button onClick={() => { createForm.reset({ tenant_id: effectiveTenantId }); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Device
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search devices..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Device</DialogTitle>
            <DialogDescription>Register a new SIP device.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-name">Name *</Label>
              <Input id="c-name" placeholder="e.g., Lobby Phone" {...createForm.register('name')} />
              {createForm.formState.errors.name && <p className="text-sm text-destructive">{createForm.formState.errors.name.message}</p>}
            </div>
            <TenantField value={createForm.watch('tenant_id') ?? ''} onChange={(v) => createForm.setValue('tenant_id', v)} />
            <div className="space-y-2">
              <Label>Device Type *</Label>
              <Select onValueChange={(v) => createForm.setValue('device_type', v)}>
                <SelectTrigger><SelectValue placeholder="Select type" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="desk_phone">Desk Phone</SelectItem>
                  <SelectItem value="softphone">Softphone</SelectItem>
                  <SelectItem value="webrtc">WebRTC</SelectItem>
                  <SelectItem value="conference_room">Conference Room</SelectItem>
                  <SelectItem value="ata">ATA</SelectItem>
                  <SelectItem value="gateway">Gateway</SelectItem>
                  <SelectItem value="mobile">Mobile</SelectItem>
                  <SelectItem value="other">Other</SelectItem>
                </SelectContent>
              </Select>
              {createForm.formState.errors.device_type && <p className="text-sm text-destructive">{createForm.formState.errors.device_type.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-sip_uri">SIP URI</Label>
              <Input id="c-sip_uri" placeholder="sip:username@domain" {...createForm.register('sip_uri')} />
              <p className="text-xs text-muted-foreground">Must match the AOR the device registers with</p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-mac_address">MAC Address</Label>
                <Input id="c-mac_address" placeholder="AA:BB:CC:DD:EE:FF" {...createForm.register('mac_address')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-model">Model</Label>
                <Input id="c-model" {...createForm.register('model')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-location">Location</Label>
              <Input id="c-location" placeholder="e.g., Floor 2, Room 201" {...createForm.register('location')} />
            </div>
            <div className="space-y-2">
              <Label>Site Location</Label>
              <Select onValueChange={(v) => createForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc) => (
                    <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Adding...' : 'Add Device'}
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
            <DialogTitle>Edit Device: {editItem?.name}</DialogTitle>
            <DialogDescription>Update device details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-name">Name *</Label>
              <Input id="e-name" {...editForm.register('name')} />
              {editForm.formState.errors.name && <p className="text-sm text-destructive">{editForm.formState.errors.name.message}</p>}
            </div>
            <TenantField value={editForm.watch('tenant_id') ?? ''} onChange={(v) => editForm.setValue('tenant_id', v)} mode="edit" />
            <div className="space-y-2">
              <Label htmlFor="e-sip_uri">SIP URI</Label>
              <Input id="e-sip_uri" placeholder="sip:username@domain" {...editForm.register('sip_uri')} />
              <p className="text-xs text-muted-foreground">Must match the AOR the device registers with</p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-mac_address">MAC Address</Label>
                <Input id="e-mac_address" {...editForm.register('mac_address')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-model">Model</Label>
                <Input id="e-model" {...editForm.register('model')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-firmware_version">Firmware Version</Label>
              <Input id="e-firmware_version" {...editForm.register('firmware_version')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-location">Location</Label>
              <Input id="e-location" {...editForm.register('location')} />
            </div>
            <div className="space-y-2">
              <Label>Site Location</Label>
              <Select defaultValue={editItem?.location_id || '__none__'} onValueChange={(v) => editForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc) => (
                    <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {/* Calling (docs/ENTERPRISE_CALLING.md §3.8): whose phone it is,
                whether users may log in on it, and its own credentials. */}
            <p className="pt-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Calling
            </p>
            <div className="space-y-2">
              <Label>Owner</Label>
              <Select
                value={editForm.watch('owner_user_id') || '__none__'}
                onValueChange={(v) => editForm.setValue('owner_user_id', v === '__none__' ? '' : v)}
              >
                <SelectTrigger><SelectValue placeholder="No owner" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">No owner</SelectItem>
                  {userList
                    .filter((u) => !editItem || u.tenant_id === editItem.tenant_id)
                    .map((u) => (
                      <SelectItem key={u.id} value={u.id}>{u.username}</SelectItem>
                    ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-center justify-between rounded-md border p-3">
              <div className="space-y-0.5 pr-4">
                <Label htmlFor="e-hoteling_host">Hoteling host</Label>
                <p className="text-xs text-muted-foreground">
                  A shared phone: a user dials *88 on it and logs in with their extension and PIN,
                  and it rings for them until they dial *89 or the login runs out.
                </p>
              </div>
              <Switch
                id="e-hoteling_host"
                checked={editForm.watch('hoteling_host') ?? false}
                onCheckedChange={(v) => editForm.setValue('hoteling_host', v)}
              />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-sip_username">Device username</Label>
                <Input
                  id="e-sip_username"
                  placeholder="lobby-phone"
                  disabled={editForm.watch('remove_credentials')}
                  {...editForm.register('sip_username')}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-sip_password">Device password</Label>
                <Input
                  id="e-sip_password"
                  type="password"
                  autoComplete="new-password"
                  placeholder={editItem?.has_sip_credentials ? 'unchanged' : ''}
                  disabled={editForm.watch('remove_credentials')}
                  {...editForm.register('sip_password')}
                />
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Credentials of its own let a shared phone register as itself rather than as its owner;
              without them it uses its owner&rsquo;s. A new username needs a password.
            </p>
            {editItem?.has_sip_credentials && (
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" {...editForm.register('remove_credentials')} />
                Remove the device&rsquo;s own credentials
              </label>
            )}

            {/* Emergency (docs/E911.md): where this phone is dispatched to
                when it is not where its site is. */}
            <p className="pt-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Emergency (E911)
            </p>
            <div className="space-y-2">
              <Label htmlFor="e-e911_address1">Address override</Label>
              <Input
                id="e-e911_address1"
                placeholder="1 State St — leave empty to use the site's address"
                {...editForm.register('e911_address1')}
              />
              <Input placeholder="Suite 300" {...editForm.register('e911_address2')} />
              <div className="grid grid-cols-2 gap-4">
                <Input placeholder="City" {...editForm.register('e911_city')} />
                <Input placeholder="State" {...editForm.register('e911_state')} />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <Input placeholder="Postal code" {...editForm.register('e911_postal_code')} />
                <Input placeholder="Country (US)" {...editForm.register('e911_country')} />
              </div>
              <p className="text-xs text-muted-foreground">
                An address saved here arrives unvalidated; validate it from the row's menu.
                {editItem?.e911_override?.validated
                  ? ' This one is validated.'
                  : editItem?.e911_override
                    ? ' This one has not been validated.'
                    : ''}
              </p>
            </div>
            <div className="space-y-2">
              <Label>Location this device sends</Label>
              <Select
                value={editForm.watch('e911_trust_location') ?? 'inherit'}
                onValueChange={(v) =>
                  editForm.setValue('e911_trust_location', v as 'inherit' | 'yes' | 'no')
                }
              >
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="inherit">As the tenant's device types say</SelectItem>
                  <SelectItem value="yes">Trusted</SelectItem>
                  <SelectItem value="no">Never used</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                An NG911 phone can send its own location with the call; it is used only from a
                device that is trusted to.
              </p>
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

      {/* What the validator said (docs/E911.md, "Address validation") */}
      <Dialog open={!!validation} onOpenChange={() => setValidation(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{validation?.name}</DialogTitle>
            <DialogDescription>
              {validation?.result?.valid
                ? 'The address was accepted and stored in its normalised form.'
                : 'The address was not accepted; nothing was changed.'}
            </DialogDescription>
          </DialogHeader>
          {validation?.error && <p className="text-sm text-destructive">{validation.error}</p>}
          {validation?.result && (
            <div className="space-y-3 text-sm">
              {validation.result.valid ? (
                <Badge variant="success">validated by {validation.result.validator}</Badge>
              ) : (
                <Badge variant="destructive">refused by {validation.result.validator}</Badge>
              )}
              {validation.result.issues.length > 0 && (
                <ul className="list-disc pl-5">
                  {validation.result.issues.map((issue) => (
                    <li key={issue} className="text-destructive">{issue}</li>
                  ))}
                </ul>
              )}
              {validation.result.notes.length > 0 && (
                <ul className="list-disc pl-5 text-muted-foreground">
                  {validation.result.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
              )}
              <p>
                {validation.result.normalized.address1}, {validation.result.normalized.city},{' '}
                {validation.result.normalized.state} {validation.result.normalized.postal_code}
              </p>
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* Bulk Import Dialog */}
      <BulkImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onComplete={() => queryClient.invalidateQueries({ queryKey: ['devices'] })}
        existingItems={items}
      />

      {/* Provisioning Token Reveal Dialog — the token is only shown once. */}
      <Dialog
        open={!!revealedToken}
        onOpenChange={(open) => {
          if (!open) {
            setRevealedToken(null)
            setShowToken(false)
          }
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              Provisioning token {revealedToken?.context === 'created' ? 'created' : 'rotated'}
            </DialogTitle>
            <DialogDescription>
              Copy this token now for <span className="font-medium">{revealedToken?.deviceName}</span>. It cannot be retrieved later. Configure the phone to send it in the <code className="text-xs">X-FCP-Device-Provisioning-Token</code> header.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="flex items-stretch gap-2">
              <Input
                readOnly
                type={showToken ? 'text' : 'password'}
                value={revealedToken?.token ?? ''}
                className="font-mono text-xs"
                onFocus={(e) => e.currentTarget.select()}
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => setShowToken((v) => !v)}
                title={showToken ? 'Hide' : 'Show'}
              >
                {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => {
                  if (revealedToken) navigator.clipboard.writeText(revealedToken.token)
                }}
                title="Copy to clipboard"
              >
                <Copy className="h-4 w-4" />
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              Store this with your phone deployment secrets. Rotating here will lock out any phone still using the previous token.
            </p>
          </div>
          <div className="flex justify-end">
            <Button
              onClick={() => {
                setRevealedToken(null)
                setShowToken(false)
              }}
            >
              Done
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

// --- CSV Export / Import Helpers ---

const EXPORT_COLUMNS = ['name', 'device_type', 'mac_address', 'model', 'sip_uri', 'location', 'location_id', 'enabled'] as const

function escapeCSVField(value: string): string {
  // Neutralize spreadsheet formula injection (CWE-1236):
  // check first non-whitespace/control char to catch " =CMD", "\t@SUM", etc.
  const first = value.match(/[^\s\x00-\x1f]/)?.[0]
  if (first && '=+-@'.includes(first)) {
    value = "'" + value
  }
  if (value.includes(',') || value.includes('"') || value.includes('\n')) {
    return `"${value.replace(/"/g, '""')}"`
  }
  return value
}

function exportDevices(deviceList: Device[]) {
  const header = EXPORT_COLUMNS.join(',')
  const rows = deviceList.map((d) =>
    EXPORT_COLUMNS.map((col) => escapeCSVField(String(d[col] ?? ''))).join(',')
  )
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `devices_export_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}

const IMPORT_COLUMNS = ['name', 'device_type', 'mac_address', 'model', 'sip_uri', 'location', 'location_id'] as const
const KNOWN_DEVICE_TYPES = ['desk_phone', 'softphone', 'webrtc', 'conference_room', 'ata', 'gateway', 'mobile', 'other']

interface ParsedRow {
  [key: string]: string
}

function parseCSV(text: string): { headers: string[]; rows: ParsedRow[] } | { error: string } {
  const lines = text.split(/\r?\n/).filter((line) => line.trim() !== '')
  if (lines.length === 0) return { error: 'File is empty' }

  const parseLine = (line: string): string[] => {
    const fields: string[] = []
    let current = ''
    let inQuotes = false
    for (let i = 0; i < line.length; i++) {
      const ch = line[i]
      if (inQuotes) {
        if (ch === '"') {
          if (i + 1 < line.length && line[i + 1] === '"') {
            current += '"'
            i++
          } else {
            inQuotes = false
          }
        } else {
          current += ch
        }
      } else {
        if (ch === '"') {
          inQuotes = true
        } else if (ch === ',') {
          fields.push(current.trim())
          current = ''
        } else {
          current += ch
        }
      }
    }
    fields.push(current.trim())
    return fields
  }

  const headers = parseLine(lines[0]).map((h) => h.toLowerCase().replace(/\s+/g, '_'))

  if (!headers.includes('name') || !headers.includes('device_type')) {
    return { error: 'CSV must contain at least "name" and "device_type" columns' }
  }

  const rows: ParsedRow[] = []
  for (let i = 1; i < lines.length; i++) {
    const values = parseLine(lines[i])
    const row: ParsedRow = {}
    headers.forEach((h, idx) => {
      row[h] = values[idx] || ''
    })
    rows.push(row)
  }

  return { headers, rows }
}

function validateDeviceImportRow(row: ParsedRow): string[] {
  const errors: string[] = []
  if (!row.name?.trim()) errors.push('Name is required')
  if (!row.device_type?.trim()) errors.push('Device type is required')
  else if (!KNOWN_DEVICE_TYPES.includes(row.device_type.trim())) {
    errors.push(`Unknown device type "${row.device_type.trim()}"`)
  }
  return errors
}

function downloadDeviceTemplate() {
  const header = IMPORT_COLUMNS.join(',')
  const sample = 'Lobby Phone,desk_phone,AA:BB:CC:DD:EE:FF,Yealink T54W,sip:lobby@example.com,Floor 1'
  const csv = `${header}\n${sample}\n`
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'devices_import_template.csv'
  a.click()
  URL.revokeObjectURL(url)
}

type ImportStep = 'upload' | 'preview' | 'import'
type RowStatus = 'pending' | 'importing' | 'success' | 'error'

interface ImportRowState {
  data: ParsedRow
  validationErrors: string[]
  status: RowStatus
  apiError?: string
}

interface ImportedToken {
  deviceName: string
  mac: string
  token: string
}

function BulkImportDialog({
  open,
  onOpenChange,
  onComplete,
  existingItems,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: () => void
  existingItems: Device[]
}) {
  const [step, setStep] = useState<ImportStep>('upload')
  const [rows, setRows] = useState<ImportRowState[]>([])
  const [importedCount, setImportedCount] = useState(0)
  const [failedCount, setFailedCount] = useState(0)
  const [updatedCount, setUpdatedCount] = useState(0)
  const [isImporting, setIsImporting] = useState(false)
  const [fileError, setFileError] = useState<string | null>(null)
  const [isDragOver, setIsDragOver] = useState(false)
  const [overrideExisting, setOverrideExisting] = useState(false)
  const [generatedTokens, setGeneratedTokens] = useState<ImportedToken[]>([])
  const [tokensAcknowledged, setTokensAcknowledged] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const abortRef = useRef(false)

  const reset = useCallback(() => {
    setStep('upload')
    setRows([])
    setImportedCount(0)
    setFailedCount(0)
    setUpdatedCount(0)
    setIsImporting(false)
    setFileError(null)
    setIsDragOver(false)
    setOverrideExisting(false)
    setGeneratedTokens([])
    setTokensAcknowledged(false)
    abortRef.current = false
  }, [])

  const handleClose = (v: boolean) => {
    if (!v) {
      if (isImporting) {
        abortRef.current = true
        return
      }
      // Don't discard freshly-minted provisioning tokens on an accidental close.
      // The user must explicitly acknowledge them first — they cannot be
      // recovered from the backend once this dialog is gone.
      if (step === 'import' && generatedTokens.length > 0 && !tokensAcknowledged) {
        return
      }
      if (step === 'import' && importedCount > 0) {
        onComplete()
      }
      reset()
    }
    onOpenChange(v)
  }

  const processFile = (file: File) => {
    setFileError(null)
    if (!file.name.endsWith('.csv')) {
      setFileError('Please select a .csv file')
      return
    }
    const reader = new FileReader()
    reader.onload = (e) => {
      const text = e.target?.result as string
      const result = parseCSV(text)
      if ('error' in result) {
        setFileError(result.error)
        return
      }
      if (result.rows.length === 0) {
        setFileError('CSV contains no data rows')
        return
      }
      const validated: ImportRowState[] = result.rows.map((data) => ({
        data,
        validationErrors: validateDeviceImportRow(data),
        status: 'pending' as const,
      }))
      setRows(validated)
      setStep('preview')
    }
    reader.readAsText(file)
  }

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) processFile(file)
  }

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragOver(false)
    const file = e.dataTransfer.files[0]
    if (file) processFile(file)
  }

  const validRows = rows.filter((r) => r.validationErrors.length === 0)
  const invalidRows = rows.filter((r) => r.validationErrors.length > 0)

  const startImport = async () => {
    setStep('import')
    setIsImporting(true)
    setImportedCount(0)
    setFailedCount(0)
    setUpdatedCount(0)
    abortRef.current = false

    const updatedRows = rows.map((r) =>
      r.validationErrors.length === 0 ? { ...r, status: 'pending' as const } : { ...r, status: 'error' as const, apiError: 'Skipped (validation errors)' }
    )
    setRows(updatedRows)

    let created = 0
    let updated = 0
    let failed = invalidRows.length

    for (let i = 0; i < updatedRows.length; i++) {
      if (abortRef.current) break
      const row = updatedRows[i]
      if (row.validationErrors.length > 0) continue

      setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'importing' } : r))

      const name = row.data.name.trim()
      const device_type = row.data.device_type.trim()
      const createData: CreateDeviceRequest = { name, device_type }
      if (row.data.mac_address?.trim()) createData.mac_address = row.data.mac_address.trim()
      if (row.data.model?.trim()) createData.model = row.data.model.trim()
      if (row.data.sip_uri?.trim()) createData.sip_uri = row.data.sip_uri.trim()
      if (row.data.location?.trim()) createData.location = row.data.location.trim()
      if (row.data.location_id?.trim()) createData.location_id = row.data.location_id.trim()

      try {
        const createdDevice = await devices.create(createData)
        created++
        setImportedCount(created)
        if (createdDevice.provisioning_token) {
          setGeneratedTokens((prev) => [
            ...prev,
            {
              deviceName: createdDevice.name,
              mac: createdDevice.mac_address ?? '',
              token: createdDevice.provisioning_token as string,
            },
          ])
        }
        setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success' } : r))
      } catch (createErr: unknown) {
        const createMsg = createErr instanceof Error ? createErr.message : 'Unknown error'
        const isConflict = (createErr as { status?: number }).status === 409

        if (overrideExisting && isConflict) {
          const existingDevice = existingItems.find((d) => d.name === name)
          if (existingDevice) {
            try {
              const updateData: Partial<CreateDeviceRequest> = { name, device_type }
              if (row.data.mac_address?.trim()) updateData.mac_address = row.data.mac_address.trim()
              if (row.data.model?.trim()) updateData.model = row.data.model.trim()
              if (row.data.sip_uri?.trim()) updateData.sip_uri = row.data.sip_uri.trim()
              if (row.data.location?.trim()) updateData.location = row.data.location.trim()
              await devices.update(existingDevice.id, updateData)
              updated++
              setUpdatedCount(updated)
              setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success', apiError: 'Updated' } : r))
            } catch (updateErr) {
              failed++
              setFailedCount(failed - invalidRows.length)
              const message = updateErr instanceof Error ? updateErr.message : 'Unknown error'
              setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: `Update failed: ${message}` } : r))
            }
          } else {
            failed++
            setFailedCount(failed - invalidRows.length)
            setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: 'Conflict: device not found in current list for override' } : r))
          }
        } else {
          failed++
          setFailedCount(failed - invalidRows.length)
          setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: createMsg || 'Unknown error' } : r))
        }
      }
    }

    setIsImporting(false)
  }

  const handleDone = () => {
    if (importedCount > 0) onComplete()
    reset()
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {step === 'upload' && 'Bulk Import Devices'}
            {step === 'preview' && 'Preview Import'}
            {step === 'import' && 'Importing Devices'}
          </DialogTitle>
          <DialogDescription>
            {step === 'upload' && 'Upload a CSV file to create multiple devices at once.'}
            {step === 'preview' && `Review ${rows.length} rows before importing.`}
            {step === 'import' && 'Creating devices...'}
          </DialogDescription>
        </DialogHeader>

        {/* Step 1: Upload */}
        {step === 'upload' && (
          <div className="space-y-4">
            <div
              className={`border-2 border-dashed rounded-lg p-8 text-center cursor-pointer transition-colors ${
                isDragOver ? 'border-primary bg-primary/5' : 'border-muted-foreground/25 hover:border-primary/50'
              }`}
              onClick={() => fileInputRef.current?.click()}
              onDragOver={(e) => { e.preventDefault(); setIsDragOver(true) }}
              onDragLeave={() => setIsDragOver(false)}
              onDrop={handleDrop}
            >
              <FileUp className="mx-auto h-10 w-10 text-muted-foreground mb-3" />
              <p className="text-sm font-medium">Drop a CSV file here or click to browse</p>
              <p className="text-xs text-muted-foreground mt-1">Accepts .csv files</p>
              <input
                ref={fileInputRef}
                type="file"
                accept=".csv"
                className="hidden"
                onChange={handleFileChange}
              />
            </div>
            {fileError && (
              <div className="flex items-center gap-2 text-sm text-destructive">
                <AlertCircle className="h-4 w-4 shrink-0" />
                {fileError}
              </div>
            )}
            <div className="flex items-center justify-between">
              <Button variant="link" size="sm" className="px-0" onClick={downloadDeviceTemplate}>
                <Download className="mr-1 h-3 w-3" />
                Download Template
              </Button>
              <p className="text-xs text-muted-foreground">
                Required columns: name, device_type
              </p>
            </div>
          </div>
        )}

        {/* Step 2: Preview */}
        {step === 'preview' && (
          <div className="space-y-4 min-h-0 flex flex-col">
            <div className="flex items-center gap-3 text-sm">
              <Badge variant="default">{validRows.length} valid</Badge>
              {invalidRows.length > 0 && (
                <Badge variant="destructive">{invalidRows.length} with errors</Badge>
              )}
            </div>

            <div className="border rounded-md overflow-auto max-h-[400px] min-h-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-8">#</TableHead>
                    <TableHead>Name</TableHead>
                    <TableHead>Type</TableHead>
                    <TableHead>MAC</TableHead>
                    <TableHead>Model</TableHead>
                    <TableHead>SIP URI</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row, idx) => (
                    <TableRow
                      key={idx}
                      className={row.validationErrors.length > 0 ? 'bg-destructive/5' : ''}
                    >
                      <TableCell className="text-muted-foreground">{idx + 1}</TableCell>
                      <TableCell className="font-medium">{row.data.name || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.device_type || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.mac_address || '-'}</TableCell>
                      <TableCell>{row.data.model || '-'}</TableCell>
                      <TableCell>{row.data.sip_uri || '-'}</TableCell>
                      <TableCell>
                        {row.validationErrors.length > 0 ? (
                          <span className="text-xs text-destructive">{row.validationErrors.join('; ')}</span>
                        ) : (
                          <span className="text-xs text-green-600">OK</span>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            <div className="flex items-center justify-between">
              <Button variant="outline" onClick={() => { setRows([]); setStep('upload') }}>
                Back
              </Button>
              <div className="flex items-center gap-4">
                <div className="flex items-center gap-2">
                  <Switch
                    id="override-existing"
                    checked={overrideExisting}
                    onCheckedChange={setOverrideExisting}
                  />
                  <Label htmlFor="override-existing" className="text-sm cursor-pointer">
                    Override existing devices
                  </Label>
                </div>
                <Button onClick={startImport} disabled={validRows.length === 0}>
                  Import {validRows.length} Device{validRows.length !== 1 ? 's' : ''}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Step 3: Import */}
        {step === 'import' && (
          <div className="space-y-4 min-h-0 flex flex-col">
            {/* Progress bar */}
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>
                  {isImporting ? 'Importing...' : 'Import complete'}
                </span>
                <span className="text-muted-foreground">
                  {importedCount + updatedCount + failedCount} / {validRows.length}
                </span>
              </div>
              <div className="h-2 rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full bg-primary rounded-full transition-all duration-300"
                  style={{ width: `${validRows.length > 0 ? ((importedCount + updatedCount + failedCount) / validRows.length) * 100 : 0}%` }}
                />
              </div>
              {!isImporting && (
                <div className="flex gap-4 text-sm">
                  <span className="text-green-600">{importedCount} created</span>
                  {updatedCount > 0 && <span className="text-blue-600">{updatedCount} updated</span>}
                  {failedCount > 0 && <span className="text-destructive">{failedCount} failed</span>}
                  {invalidRows.length > 0 && <span className="text-muted-foreground">{invalidRows.length} skipped</span>}
                </div>
              )}
            </div>

            <div className="border rounded-md overflow-auto max-h-[350px] min-h-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-8"></TableHead>
                    <TableHead>Name</TableHead>
                    <TableHead>Result</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row, idx) => (
                    <TableRow key={idx}>
                      <TableCell>
                        {row.status === 'importing' && <Loader2 className="h-4 w-4 animate-spin text-primary" />}
                        {row.status === 'success' && <CheckCircle2 className="h-4 w-4 text-green-600" />}
                        {row.status === 'error' && <XCircle className="h-4 w-4 text-destructive" />}
                        {row.status === 'pending' && <span className="block h-4 w-4 rounded-full border-2 border-muted-foreground/25" />}
                      </TableCell>
                      <TableCell className="font-medium">{row.data.name || '(empty)'}</TableCell>
                      <TableCell className="text-sm">
                        {row.status === 'success' && row.apiError === 'Updated' && <span className="text-blue-600">Updated</span>}
                        {row.status === 'success' && row.apiError !== 'Updated' && <span className="text-green-600">Created</span>}
                        {row.status === 'error' && (
                          <span className="text-destructive">{row.apiError || row.validationErrors.join('; ')}</span>
                        )}
                        {row.status === 'importing' && <span className="text-muted-foreground">Creating...</span>}
                        {row.status === 'pending' && <span className="text-muted-foreground">Waiting</span>}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            {!isImporting && generatedTokens.length > 0 && (
              <div className="border rounded-md p-3 space-y-3 bg-amber-50 dark:bg-amber-950/30 border-amber-300 dark:border-amber-800">
                <div className="flex items-start gap-2">
                  <AlertCircle className="h-4 w-4 shrink-0 text-amber-700 dark:text-amber-400 mt-0.5" />
                  <div className="space-y-1">
                    <p className="text-sm font-medium">
                      {generatedTokens.length} provisioning token{generatedTokens.length !== 1 ? 's' : ''} generated
                    </p>
                    <p className="text-xs text-muted-foreground">
                      Tokens are shown once. Copy or download now — they cannot be retrieved from the backend.
                    </p>
                  </div>
                </div>
                <div className="border rounded-md overflow-auto max-h-[160px] bg-background">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Device</TableHead>
                        <TableHead>MAC</TableHead>
                        <TableHead>Token</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {generatedTokens.map((t, idx) => (
                        <TableRow key={idx}>
                          <TableCell className="text-xs font-medium">{t.deviceName}</TableCell>
                          <TableCell className="text-xs font-mono text-muted-foreground">{t.mac || '-'}</TableCell>
                          <TableCell className="text-xs font-mono break-all">{t.token}</TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      const text = generatedTokens
                        .map((t) => `${t.deviceName}\t${t.mac}\t${t.token}`)
                        .join('\n')
                      navigator.clipboard.writeText(text)
                    }}
                  >
                    <Copy className="mr-1 h-3 w-3" />
                    Copy All
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => downloadProvisioningTokensCSV(generatedTokens)}
                  >
                    <Download className="mr-1 h-3 w-3" />
                    Download CSV
                  </Button>
                  <div className="flex-1" />
                  <label className="flex items-center gap-2 text-xs cursor-pointer">
                    <input
                      type="checkbox"
                      checked={tokensAcknowledged}
                      onChange={(e) => setTokensAcknowledged(e.target.checked)}
                      className="rounded border-input"
                    />
                    <span>I&apos;ve saved the tokens</span>
                  </label>
                </div>
              </div>
            )}

            <div className="flex justify-end">
              {isImporting ? (
                <Button variant="outline" onClick={() => { abortRef.current = true }}>
                  Cancel
                </Button>
              ) : (
                <Button
                  onClick={handleDone}
                  disabled={generatedTokens.length > 0 && !tokensAcknowledged}
                  title={
                    generatedTokens.length > 0 && !tokensAcknowledged
                      ? 'Confirm you saved the provisioning tokens before closing'
                      : undefined
                  }
                >
                  Done
                </Button>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function downloadProvisioningTokensCSV(tokens: ImportedToken[]) {
  const header = 'device_name,mac_address,provisioning_token'
  const rows = tokens.map((t) =>
    [t.deviceName, t.mac, t.token].map(escapeCSVField).join(','),
  )
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `provisioning_tokens_${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.csv`
  a.click()
  URL.revokeObjectURL(url)
}
