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
import { Switch } from '@/components/ui/switch'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { tenants } from '@/api/endpoints'
import type { Tenant, CreateTenantRequest, DestinationClass, DestinationOverride } from '@/types'
import { formatDateTime } from '@/lib/utils'
import { useIsPlatformAdmin } from '@/components/TenantSelector'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required').max(255),
  description: z.string().optional(),
  contact_email: z.string().email().optional().or(z.literal('')),
  contact_phone: z.string().optional(),
  max_users: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_phone_numbers: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_concurrent_calls: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  sip_domains: z.string().optional(),
  platform_trunk_fallback: z.boolean().optional(),
})

const editSchema = z.object({
  name: z.string().min(1, 'Name is required').max(255),
  description: z.string().optional(),
  contact_email: z.string().email().optional().or(z.literal('')),
  contact_phone: z.string().optional(),
  max_users: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_phone_numbers: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  max_concurrent_calls: z.coerce.number().positive().optional().or(z.literal(0).transform(() => undefined)),
  sip_domains: z.string().optional(),
  login_domains: z.string().optional(),
  require_totp: z.enum(['none', 'admins', 'everyone']).optional(),
  scim: z.boolean().optional(),
  password_login: z.enum(['allowed', 'admins_only', 'disabled']).optional(),
  platform_trunk_fallback: z.boolean().optional(),
  calling_country: z.string().optional(),
  calling_timezone: z.string().optional(),
  calling_overrides: z.string().optional(),
  hoteling_session_hours: z.coerce.number().int().min(0).max(168).optional(),
  grace_days: z.coerce.number().int().min(0).max(3650).optional(),
  require_device_credentials: z.boolean().optional(),
  provisioning_networks: z.string().optional(),
  meetings_owner: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const COUNTRIES = ['US', 'CA', 'GB', 'DE', 'FR', 'AU', 'NZ']

/** `prefix class` per line, e.g. `+1900 premium` or `118 premium`. */
function parseOverrides(input: string | undefined): DestinationOverride[] {
  return (input ?? '')
    .split('\n')
    .map((l) => l.trim().split(/\s+/))
    .filter((parts) => parts.length === 2 && parts[0])
    .map(([prefix, cls]) => ({ prefix, class: cls as DestinationClass }))
}

/** Split a free-form sip_domains textarea into a clean array. */
function parseSipDomains(input: string | undefined): string[] {
  if (!input) return []
  return input
    .split(/[\s,]+/)
    .map((d) => d.trim().toLowerCase())
    .filter((d) => d.length > 0)
}

const statusBadge = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'suspended': return 'destructive' as const
    case 'trial': return 'warning' as const
    default: return 'secondary' as const
  }
}

export function TenantsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<Tenant | null>(null)
  const queryClient = useQueryClient()
  const isPlatformAdmin = useIsPlatformAdmin()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
  })

  const createMutation = useMutation({
    mutationFn: tenants.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateTenantRequest> }) =>
      tenants.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tenants'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: tenants.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tenants'] }),
  })

  const columns: ColumnDef<Tenant>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'description',
      header: 'Description',
      cell: ({ row }) => row.original.description || '-',
    },
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => <Badge variant={statusBadge(row.original.status)}>{row.original.status}</Badge>,
    },
    {
      accessorKey: 'contact_email',
      header: 'Contact',
      cell: ({ row }) => row.original.contact_email || '-',
    },
    {
      accessorKey: 'max_users',
      header: 'Max Users',
      cell: ({ row }) => row.original.max_users ?? 'Unlimited',
    },
    {
      id: 'sip_domains',
      header: 'SIP Domains',
      cell: ({ row }) => {
        const domains = row.original.sip_domains ?? []
        if (domains.length === 0) return <span className="text-muted-foreground">—</span>
        return (
          <div className="flex flex-wrap gap-1">
            {domains.map((d) => (
              <Badge key={d} variant="outline" className="font-mono text-xs">{d}</Badge>
            ))}
          </div>
        )
      },
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
            {isPlatformAdmin && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive"
                  onClick={() => {
                    if (confirm(`Delete tenant "${row.original.name}"? This action cannot be undone.`)) {
                      deleteMutation.mutate(row.original.id)
                    }
                  }}
                >
                  <Trash2 className="mr-2 h-4 w-4" />
                  Delete
                </DropdownMenuItem>
              </>
            )}
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
      name: editItem.name,
      description: editItem.description || '',
      contact_email: editItem.contact_email || '',
      contact_phone: editItem.contact_phone || '',
      max_users: editItem.max_users ?? undefined,
      max_phone_numbers: editItem.max_phone_numbers ?? undefined,
      max_concurrent_calls: editItem.max_concurrent_calls ?? undefined,
      sip_domains: (editItem.sip_domains ?? []).join('\n'),
      login_domains: (editItem.login_domains ?? []).join('\n'),
      require_totp: editItem.sign_in?.require_totp ?? 'none',
      scim: editItem.sign_in?.scim ?? false,
      password_login: editItem.sign_in?.password_login ?? 'allowed',
      platform_trunk_fallback: editItem.platform_trunk_fallback ?? true,
      calling_country: editItem.calling?.country ?? '',
      calling_timezone: editItem.calling?.timezone ?? '',
      calling_overrides: (editItem.calling?.destination_overrides ?? [])
        .map((o) => `${o.prefix} ${o.class}`)
        .join('\n'),
      hoteling_session_hours: editItem.calling?.hoteling_session_hours ?? undefined,
      grace_days: editItem.offboarding?.grace_days ?? 30,
      require_device_credentials: editItem.provisioning?.require_device_credentials ?? false,
      provisioning_networks: (editItem.provisioning?.networks ?? []).join('\n'),
      meetings_owner: editItem.offboarding?.meetings_owner ?? '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    const { sip_domains: rawDomains, ...rest } = data
    createMutation.mutate({
      ...rest,
      description: data.description || undefined,
      contact_email: data.contact_email || undefined,
      contact_phone: data.contact_phone || undefined,
      sip_domains: parseSipDomains(rawDomains),
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    const {
      sip_domains: rawDomains,
      login_domains: rawLoginDomains,
      calling_country,
      calling_timezone,
      calling_overrides,
      hoteling_session_hours,
      require_totp,
      password_login,
      scim,
      grace_days,
      meetings_owner,
      require_device_credentials,
      provisioning_networks,
      ...rest
    } = data
    updateMutation.mutate({
      id: editItem.id,
      data: {
        ...rest,
        calling: {
          country: calling_country || null,
          timezone: calling_timezone || null,
          destination_overrides: parseOverrides(calling_overrides),
          hoteling_session_hours: hoteling_session_hours || null,
        },
        description: data.description || undefined,
        contact_email: data.contact_email || undefined,
        contact_phone: data.contact_phone || undefined,
        sip_domains: parseSipDomains(rawDomains),
        sign_in: {
          password_login: password_login ?? 'allowed',
          require_totp: require_totp ?? 'none',
          scim: scim ?? false,
        },
        provisioning: {
          require_device_credentials: require_device_credentials ?? false,
          networks: (provisioning_networks ?? '').split(/[\s,]+/).map((n) => n.trim()).filter(Boolean),
        },
        offboarding: {
          grace_days: grace_days ?? 30,
          meetings_owner: meetings_owner?.trim() || null,
        },
        // A login domain decides where a person's sign-in goes: only a
        // platform admin sets them (docs/IDENTITY_AND_DEVICES.md phase 2).
        ...(isPlatformAdmin ? { login_domains: parseSipDomains(rawLoginDomains) } : {}),
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Tenants</h1>
          <p className="text-muted-foreground">
            {isPlatformAdmin
              ? 'Manage tenant organizations'
              : 'View and update your tenant settings'}
          </p>
        </div>
        {isPlatformAdmin && (
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Tenant
          </Button>
        )}
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search tenants..." />
      )}

      {/* Create Dialog — platform admins only */}
      {isPlatformAdmin && (
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Tenant</DialogTitle>
            <DialogDescription>Add a new tenant organization.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-name">Name *</Label>
              <Input id="c-name" {...createForm.register('name')} />
              {createForm.formState.errors.name && <p className="text-sm text-destructive">{createForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-description">Description</Label>
              <Input id="c-description" {...createForm.register('description')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-contact_email">Contact Email</Label>
                <Input id="c-contact_email" type="email" {...createForm.register('contact_email')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-contact_phone">Contact Phone</Label>
                <Input id="c-contact_phone" {...createForm.register('contact_phone')} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-max_users">Max Users</Label>
                <Input id="c-max_users" type="number" min="0" {...createForm.register('max_users')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-max_phone_numbers">Max Numbers</Label>
                <Input id="c-max_phone_numbers" type="number" min="0" {...createForm.register('max_phone_numbers')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-max_concurrent_calls">Max Calls</Label>
                <Input id="c-max_concurrent_calls" type="number" min="0" {...createForm.register('max_concurrent_calls')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-sip_domains">SIP Domains</Label>
              <textarea
                id="c-sip_domains"
                rows={3}
                placeholder="acme.fcp.net&#10;sip.acme.com"
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring font-mono"
                {...createForm.register('sip_domains')}
              />
              <p className="text-xs text-muted-foreground">
                One per line. SIP REGISTER / INVITE requests whose From or
                Request-URI host matches one of these will be routed to this
                tenant. IP addresses and <code>localhost</code> are not
                allowed here &mdash; they fall back to the default tenant.
              </p>
            </div>
            <div className="flex items-center justify-between rounded-md border p-3">
              <div className="space-y-0.5 pr-4">
                <Label htmlFor="c-platform_trunk_fallback">Use platform trunks</Label>
                <p className="text-xs text-muted-foreground">
                  When this tenant&rsquo;s own trunks cannot take a call, let it
                  leave on one of the platform&rsquo;s. Turn this off for a
                  tenant whose calls must never leave on a shared carrier.
                </p>
              </div>
              <Switch
                id="c-platform_trunk_fallback"
                checked={createForm.watch('platform_trunk_fallback') ?? true}
                onCheckedChange={(v) => createForm.setValue('platform_trunk_fallback', v)}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Tenant'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>
      )}

      {/* Edit Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); editForm.reset() } }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit Tenant: {editItem?.name}</DialogTitle>
            <DialogDescription>Update tenant details and quotas.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-name">Name *</Label>
              <Input id="e-name" {...editForm.register('name')} />
              {editForm.formState.errors.name && <p className="text-sm text-destructive">{editForm.formState.errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-description">Description</Label>
              <Input id="e-description" {...editForm.register('description')} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-contact_email">Contact Email</Label>
                <Input id="e-contact_email" type="email" {...editForm.register('contact_email')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-contact_phone">Contact Phone</Label>
                <Input id="e-contact_phone" {...editForm.register('contact_phone')} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-max_users">Max Users</Label>
                <Input id="e-max_users" type="number" min="0" {...editForm.register('max_users')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-max_phone_numbers">Max Numbers</Label>
                <Input id="e-max_phone_numbers" type="number" min="0" {...editForm.register('max_phone_numbers')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-max_concurrent_calls">Max Calls</Label>
                <Input id="e-max_concurrent_calls" type="number" min="0" {...editForm.register('max_concurrent_calls')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-sip_domains">SIP Domains</Label>
              <textarea
                id="e-sip_domains"
                rows={3}
                placeholder="acme.fcp.net&#10;sip.acme.com"
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring font-mono"
                {...editForm.register('sip_domains')}
              />
              <p className="text-xs text-muted-foreground">
                One per line. SIP REGISTER / INVITE requests whose From or
                Request-URI host matches one of these will be routed to this
                tenant. Leave blank to use only the default-tenant fallback.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-login_domains">Login Domains</Label>
              <textarea
                id="e-login_domains"
                rows={2}
                placeholder="acme.com"
                disabled={!isPlatformAdmin}
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring font-mono disabled:opacity-60"
                {...editForm.register('login_domains')}
              />
              <p className="text-xs text-muted-foreground">
                One per line. Someone signing in as name@one-of-these is this
                tenant's. Set by a platform admin.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-password_login">Passwords</Label>
              <select
                id="e-password_login"
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                {...editForm.register('password_login')}
              >
                <option value="allowed">Everyone may sign in with a password</option>
                <option value="admins_only">Only administrators may</option>
                <option value="disabled">Nobody may: single sign-on only</option>
              </select>
              <p className="text-xs text-muted-foreground">
                Restricting passwords needs an enabled identity provider (Single Sign-On). A
                platform super admin is never shut out.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-require_totp">Two-factor sign-in</Label>
              <select
                id="e-require_totp"
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                {...editForm.register('require_totp')}
              >
                <option value="none">Optional for everyone</option>
                <option value="admins">Required for administrators</option>
                <option value="everyone">Required for everyone</option>
              </select>
              <p className="text-xs text-muted-foreground">
                Whoever it is required of sets up an authenticator app at their
                next password sign-in.
              </p>
            </div>
            <div className="flex items-center justify-between gap-4">
              <div>
                <Label htmlFor="e-scim">Users come from SCIM</Label>
                <p className="text-xs text-muted-foreground">
                  Single sign-on then makes nobody on first sign-in, and signs in only the users the
                  tenant's provisioner made (an API token holding SCIM provisioning).
                </p>
              </div>
              <Switch
                id="e-scim"
                checked={editForm.watch('scim') ?? false}
                onCheckedChange={(v) => editForm.setValue('scim', v)}
              />
            </div>
            <div className="flex items-center justify-between gap-4">
              <div>
                <Label htmlFor="e-require_device_credentials">Phones give their own credentials</Label>
                <p className="text-xs text-muted-foreground">
                  Besides the provisioning URL, each phone must send HTTP credentials whose password is its
                  device&apos;s provisioning token.
                </p>
              </div>
              <Switch
                id="e-require_device_credentials"
                checked={editForm.watch('require_device_credentials') ?? false}
                onCheckedChange={(v) => editForm.setValue('require_device_credentials', v)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-provisioning_networks">Phones fetch from (networks)</Label>
              <textarea
                id="e-provisioning_networks"
                className="flex min-h-[60px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                placeholder={'10.20.0.0/16\n203.0.113.7'}
                {...editForm.register('provisioning_networks')}
              />
              <p className="text-xs text-muted-foreground">One per line; empty is anywhere.</p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-grace_days">Days before a deleted user is purged</Label>
                <Input id="e-grace_days" type="number" min={0} max={3650} {...editForm.register('grace_days')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-meetings_owner">Their meetings go to</Label>
                <Input id="e-meetings_owner" placeholder="username (or nobody)" {...editForm.register('meetings_owner')} />
              </div>
              <p className="col-span-2 text-xs text-muted-foreground">
                A deleted user is suspended at once and can be restored until the
                purge. 0 purges at once.
              </p>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="e-calling_country">Country</Label>
                <select
                  id="e-calling_country"
                  className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  {...editForm.register('calling_country')}
                >
                  <option value="">North American numbering</option>
                  {COUNTRIES.map((c) => (
                    <option key={c} value={c}>{c}</option>
                  ))}
                </select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="e-calling_timezone">Timezone</Label>
                <Input id="e-calling_timezone" placeholder="America/New_York" {...editForm.register('calling_timezone')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-calling_overrides">Number ranges</Label>
              <textarea
                id="e-calling_overrides"
                rows={3}
                placeholder="+1900 premium&#10;118 premium"
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring font-mono"
                {...editForm.register('calling_overrides')}
              />
              <p className="text-xs text-muted-foreground">
                The country decides how dialled numbers are read for classes of service. One range per line: a
                prefix (E.164 with +, or digits as dialled) and internal, toll_free, national, international,
                premium or unknown. The timezone is the default for new schedules.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-hoteling_session_hours">Hoteling login lasts (hours)</Label>
              <Input
                id="e-hoteling_session_hours"
                type="number"
                min={1}
                max={168}
                placeholder="12"
                {...editForm.register('hoteling_session_hours')}
              />
              <p className="text-xs text-muted-foreground">
                A user who logs in on a host phone with *88 is logged out after this long.
              </p>
            </div>
            <div className="flex items-center justify-between rounded-md border p-3">
              <div className="space-y-0.5 pr-4">
                <Label htmlFor="e-platform_trunk_fallback">Use platform trunks</Label>
                <p className="text-xs text-muted-foreground">
                  When this tenant&rsquo;s own trunks cannot take a call, let it
                  leave on one of the platform&rsquo;s. Turn this off for a
                  tenant whose calls must never leave on a shared carrier.
                </p>
              </div>
              <Switch
                id="e-platform_trunk_fallback"
                checked={editForm.watch('platform_trunk_fallback') ?? true}
                onCheckedChange={(v) => editForm.setValue('platform_trunk_fallback', v)}
              />
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
