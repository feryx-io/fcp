import { useState, useRef, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, KeyRound, Phone, Trash2, LockOpen, Upload, Download, CheckCircle2, XCircle, Loader2, FileUp, AlertCircle } from 'lucide-react'
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
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Switch } from '@/components/ui/switch'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table'
import { users, locations } from '@/api/endpoints'
import type { User, CreateUserRequest, UpdateUserRequest, Location } from '@/types'
import { TenantField, useEffectiveTenantId } from '@/components/TenantSelector'

const createUserSchema = z.object({
  username: z.string().min(1, 'Username is required').max(64),
  password: z.string().min(8, 'Password must be at least 8 characters').max(128),
  display_name: z.string().optional(),
  email: z.string().email().optional().or(z.literal('')),
  role: z.string().optional(),
  extension: z.string().optional(),
  department: z.string().optional(),
  location_id: z.string().optional(),
  tenant_id: z.string().optional(),
})

const editUserSchema = z.object({
  display_name: z.string().optional(),
  email: z.string().email().optional().or(z.literal('')),
  role: z.string().optional(),
  extension: z.string().optional(),
  department: z.string().optional(),
  location_id: z.string().optional(),
  account_status: z.string().optional(),
  tenant_id: z.string().optional(),
})

const changePasswordSchema = z.object({
  password: z.string().min(8, 'Password must be at least 8 characters').max(128),
  confirm_password: z.string(),
}).refine((data) => data.password === data.confirm_password, {
  message: 'Passwords do not match',
  path: ['confirm_password'],
})

const digestCredentialsSchema = z.object({
  digest_password: z.string().min(1, 'Password is required').max(128),
  confirm_password: z.string(),
}).refine((data) => data.digest_password === data.confirm_password, {
  message: 'Passwords do not match',
  path: ['confirm_password'],
})

type CreateUserForm = z.infer<typeof createUserSchema>
type EditUserForm = z.infer<typeof editUserSchema>
type ChangePasswordForm = z.infer<typeof changePasswordSchema>
type DigestCredentialsForm = z.infer<typeof digestCredentialsSchema>

const roleBadgeVariant = (role: string) => {
  switch (role) {
    case 'super_admin': case 'superadmin': return 'destructive' as const
    case 'tenant_admin': case 'tenantadmin': return 'destructive' as const
    case 'telecom_admin': case 'telecomadmin': return 'default' as const
    case 'user_admin': case 'useradmin': return 'default' as const
    case 'department_admin': case 'departmentadmin': return 'secondary' as const
    case 'read_only': case 'readonly': return 'secondary' as const
    case 'end_user': case 'enduser': return 'outline' as const
    default: return 'outline' as const
  }
}

const statusBadgeVariant = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'suspended': return 'warning' as const
    case 'locked': return 'destructive' as const
    case 'disabled': return 'destructive' as const
    default: return 'secondary' as const
  }
}

export function UsersPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [editUser, setEditUser] = useState<User | null>(null)
  const [passwordUser, setPasswordUser] = useState<User | null>(null)
  const [digestUser, setDigestUser] = useState<User | null>(null)
  const queryClient = useQueryClient()

  const { data: userList = [], isLoading } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const { data: locationList = [] } = useQuery({
    queryKey: ['locations'],
    queryFn: locations.list,
  })

  const locationName = (id: string | null) => {
    if (!id) return '-'
    const loc = locationList.find((l: Location) => l.id === id)
    return loc ? loc.name : id
  }

  const createMutation = useMutation({
    mutationFn: users.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ username, data }: { username: string; data: EditUserForm }) =>
      users.update(username, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] })
      setEditUser(null)
    },
  })

  const passwordMutation = useMutation({
    mutationFn: ({ username, password }: { username: string; password: string }) =>
      users.updatePassword(username, password),
    onSuccess: () => {
      setPasswordUser(null)
    },
  })

  const digestMutation = useMutation({
    mutationFn: ({ username, digest_password }: { username: string; digest_password: string }) =>
      users.updateDigestPassword(username, digest_password),
    onSuccess: () => setDigestUser(null),
  })

  const deleteDigestMutation = useMutation({
    mutationFn: users.deleteDigestPassword,
    onSuccess: () => setDigestUser(null),
  })

  const deleteMutation = useMutation({
    mutationFn: users.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['users'] }),
  })

  const unlockMutation = useMutation({
    mutationFn: users.unlock,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['users'] }),
  })

  const columns: ColumnDef<User>[] = [
    {
      accessorKey: 'username',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Username <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'display_name',
      header: 'Display Name',
      cell: ({ row }) => row.original.display_name || '-',
    },
    {
      accessorKey: 'role',
      header: 'Role',
      cell: ({ row }) => (
        <Badge variant={roleBadgeVariant(row.original.role)}>
          {row.original.role.replace(/_/g, ' ')}
        </Badge>
      ),
    },
    {
      accessorKey: 'account_status',
      header: 'Status',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Badge variant={statusBadgeVariant(row.original.account_status)}>
            {row.original.account_status}
          </Badge>
          {row.original.account_status === 'locked' && row.original.failed_login_attempts > 0 && (
            <span className="text-xs text-muted-foreground">
              ({row.original.failed_login_attempts} attempts)
            </span>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'email',
      header: 'Email',
      cell: ({ row }) => row.original.email || '-',
    },
    {
      accessorKey: 'extension',
      header: 'Extension',
      cell: ({ row }) => row.original.extension || '-',
    },
    {
      accessorKey: 'location_id',
      header: 'Location',
      cell: ({ row }) => locationName(row.original.location_id),
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
            <DropdownMenuItem onClick={() => setEditUser(row.original)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setPasswordUser(row.original)}>
              <KeyRound className="mr-2 h-4 w-4" />
              Change Password
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setDigestUser(row.original)}>
              <Phone className="mr-2 h-4 w-4" />
              Set Digest Credentials
            </DropdownMenuItem>
            {row.original.account_status === 'locked' && (
              <DropdownMenuItem onClick={() => unlockMutation.mutate(row.original.username)}>
                <LockOpen className="mr-2 h-4 w-4" />
                Unlock Account
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete user "${row.original.username}"?`)) {
                  deleteMutation.mutate(row.original.username)
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

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Users</h1>
          <p className="text-muted-foreground">Manage user accounts and credentials</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => exportUsers(userList)}>
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload className="mr-2 h-4 w-4" />
            Bulk Import
          </Button>
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="mr-2 h-4 w-4" />
            Add User
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={userList} searchKey="username" searchPlaceholder="Search users..." />
      )}

      {/* Create User Dialog */}
      <CreateUserDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        mutation={createMutation}
        locationList={locationList}
      />

      {/* Edit User Dialog */}
      <EditUserDialog
        user={editUser}
        onOpenChange={(open) => { if (!open) setEditUser(null) }}
        mutation={updateMutation}
        locationList={locationList}
      />

      {/* Change Password Dialog */}
      <ChangePasswordDialog
        user={passwordUser}
        onOpenChange={(open) => { if (!open) setPasswordUser(null) }}
        mutation={passwordMutation}
      />

      {/* Digest Credentials Dialog */}
      <DigestCredentialsDialog
        user={digestUser}
        onOpenChange={(open) => { if (!open) setDigestUser(null) }}
        mutation={digestMutation}
        deleteMutation={deleteDigestMutation}
      />

      {/* Bulk Import Dialog */}
      <BulkImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onComplete={() => queryClient.invalidateQueries({ queryKey: ['users'] })}
      />
    </div>
  )
}

function CreateUserDialog({
  open,
  onOpenChange,
  mutation,
  locationList,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<User, Error, Parameters<typeof users.create>[0]>>
  locationList: Location[]
}) {
  const effectiveTenantId = useEffectiveTenantId()
  const {
    register,
    handleSubmit,
    reset,
    setValue,
    watch,
    formState: { errors },
  } = useForm<CreateUserForm>({
    resolver: zodResolver(createUserSchema),
    defaultValues: { role: 'end_user', tenant_id: effectiveTenantId },
  })

  const onSubmit = (data: CreateUserForm) => {
    mutation.mutate({
      ...data,
      email: data.email || undefined,
      display_name: data.display_name || undefined,
      extension: data.extension || undefined,
      department: data.department || undefined,
      location_id: data.location_id || undefined,
      tenant_id: data.tenant_id || undefined,
    })
  }

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) reset(); onOpenChange(v) }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create User</DialogTitle>
          <DialogDescription>Add a new user account to the system.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="create-username">Username *</Label>
            <Input id="create-username" {...register('username')} />
            {errors.username && <p className="text-sm text-destructive">{errors.username.message}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="create-password">Password *</Label>
            <Input id="create-password" type="password" {...register('password')} />
            {errors.password && <p className="text-sm text-destructive">{errors.password.message}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="create-display_name">Display Name</Label>
            <Input id="create-display_name" {...register('display_name')} />
          </div>
          <div className="space-y-2">
            <Label htmlFor="create-email">Email</Label>
            <Input id="create-email" type="email" {...register('email')} />
            {errors.email && <p className="text-sm text-destructive">{errors.email.message}</p>}
          </div>
          <div className="space-y-2">
            <Label>Role</Label>
            <Select defaultValue="end_user" onValueChange={(v) => setValue('role', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="end_user">End User</SelectItem>
                <SelectItem value="read_only">Read Only</SelectItem>
                <SelectItem value="department_admin">Department Admin</SelectItem>
                <SelectItem value="user_admin">User Admin</SelectItem>
                <SelectItem value="telecom_admin">Telecom Admin</SelectItem>
                <SelectItem value="tenant_admin">Tenant Admin</SelectItem>
                <SelectItem value="super_admin">Super Admin</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <TenantField value={watch('tenant_id') ?? ''} onChange={(v) => setValue('tenant_id', v)} />
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="create-extension">Extension</Label>
              <Input id="create-extension" {...register('extension')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="create-department">Department</Label>
              <Input id="create-department" {...register('department')} />
            </div>
          </div>
          <div className="space-y-2">
            <Label>Location</Label>
            <Select onValueChange={(v) => setValue('location_id', v === '__none__' ? '' : v)}>
              <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">None</SelectItem>
                {locationList.map((loc: Location) => (
                  <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? 'Creating...' : 'Create User'}
            </Button>
          </div>
          {mutation.isError && (
            <p className="text-sm text-destructive">{(mutation.error as Error).message}</p>
          )}
        </form>
      </DialogContent>
    </Dialog>
  )
}

function EditUserDialog({
  user,
  onOpenChange,
  mutation,
  locationList,
}: {
  user: User | null
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<unknown, Error, { username: string; data: EditUserForm }>>
  locationList: Location[]
}) {
  const {
    register,
    handleSubmit,
    setValue,
    watch,
    reset,
    formState: { errors },
  } = useForm<EditUserForm>({
    resolver: zodResolver(editUserSchema),
  })

  // Reset form when user changes
  const [lastUser, setLastUser] = useState<string | null>(null)
  if (user && user.username !== lastUser) {
    setLastUser(user.username)
    reset({
      display_name: user.display_name || '',
      email: user.email || '',
      role: user.role || '',
      extension: user.extension || '',
      department: user.department || '',
      location_id: user.location_id || '',
      account_status: user.account_status || 'active',
      tenant_id: user.tenant_id ?? '',
    })
  }

  const onSubmit = (data: EditUserForm) => {
    if (!user) return
    mutation.mutate({
      username: user.username,
      data: {
        ...data,
        email: data.email || undefined,
        display_name: data.display_name || undefined,
        extension: data.extension || undefined,
        department: data.department || undefined,
        location_id: data.location_id || undefined,
        tenant_id: data.tenant_id || undefined,
      },
    })
  }

  return (
    <Dialog open={!!user} onOpenChange={(v) => { if (!v) { setLastUser(null); reset() } onOpenChange(v) }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit User: {user?.username}</DialogTitle>
          <DialogDescription>Update user account details.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="edit-display_name">Display Name</Label>
            <Input id="edit-display_name" {...register('display_name')} />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-email">Email</Label>
            <Input id="edit-email" type="email" {...register('email')} />
            {errors.email && <p className="text-sm text-destructive">{errors.email.message}</p>}
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Role</Label>
              <Select value={undefined} defaultValue={user?.role} onValueChange={(v) => setValue('role', v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="end_user">End User</SelectItem>
                  <SelectItem value="read_only">Read Only</SelectItem>
                  <SelectItem value="department_admin">Department Admin</SelectItem>
                  <SelectItem value="user_admin">User Admin</SelectItem>
                  <SelectItem value="telecom_admin">Telecom Admin</SelectItem>
                  <SelectItem value="tenant_admin">Tenant Admin</SelectItem>
                  <SelectItem value="super_admin">Super Admin</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Status</Label>
              <Select defaultValue={user?.account_status || 'active'} onValueChange={(v) => setValue('account_status', v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="active">Active</SelectItem>
                  <SelectItem value="suspended">Suspended</SelectItem>
                  <SelectItem value="disabled">Disabled</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <TenantField value={watch('tenant_id') ?? ''} onChange={(v) => setValue('tenant_id', v)} />
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="edit-extension">Extension</Label>
              <Input id="edit-extension" {...register('extension')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-department">Department</Label>
              <Input id="edit-department" {...register('department')} />
            </div>
          </div>
          <div className="space-y-2">
            <Label>Location</Label>
            <Select defaultValue={user?.location_id || '__none__'} onValueChange={(v) => setValue('location_id', v === '__none__' ? '' : v)}>
              <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">None</SelectItem>
                {locationList.map((loc: Location) => (
                  <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? 'Saving...' : 'Save Changes'}
            </Button>
          </div>
          {mutation.isError && (
            <p className="text-sm text-destructive">{(mutation.error as Error).message}</p>
          )}
        </form>
      </DialogContent>
    </Dialog>
  )
}

function ChangePasswordDialog({
  user,
  onOpenChange,
  mutation,
}: {
  user: User | null
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<unknown, Error, { username: string; password: string }>>
}) {
  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<ChangePasswordForm>({
    resolver: zodResolver(changePasswordSchema),
  })

  const onSubmit = (data: ChangePasswordForm) => {
    if (!user) return
    mutation.mutate({ username: user.username, password: data.password })
  }

  return (
    <Dialog open={!!user} onOpenChange={(v) => { if (!v) reset(); onOpenChange(v) }}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Change Password</DialogTitle>
          <DialogDescription>Set a new password for {user?.username}.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="new-password">New Password *</Label>
            <Input id="new-password" type="password" {...register('password')} />
            {errors.password && <p className="text-sm text-destructive">{errors.password.message}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="confirm-password">Confirm Password *</Label>
            <Input id="confirm-password" type="password" {...register('confirm_password')} />
            {errors.confirm_password && <p className="text-sm text-destructive">{errors.confirm_password.message}</p>}
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? 'Updating...' : 'Update Password'}
            </Button>
          </div>
          {mutation.isError && (
            <p className="text-sm text-destructive">{(mutation.error as Error).message}</p>
          )}
          {mutation.isSuccess && (
            <p className="text-sm text-green-600">Password updated successfully.</p>
          )}
        </form>
      </DialogContent>
    </Dialog>
  )
}

// --- CSV Export / Import Helpers ---

const EXPORT_COLUMNS = ['username', 'display_name', 'email', 'role', 'extension', 'department', 'location_id', 'account_status'] as const

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

function exportUsers(userList: User[]) {
  const header = EXPORT_COLUMNS.join(',')
  const rows = userList.map((u) =>
    EXPORT_COLUMNS.map((col) => escapeCSVField(String(u[col] ?? ''))).join(',')
  )
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `users_export_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}

const IMPORT_COLUMNS = ['username', 'password', 'display_name', 'email', 'role', 'extension', 'department', 'location_id'] as const
const KNOWN_ROLES = ['end_user', 'read_only', 'department_admin', 'user_admin', 'telecom_admin', 'tenant_admin', 'super_admin']

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

  if (!headers.includes('username') || !headers.includes('password')) {
    return { error: 'CSV must contain at least "username" and "password" columns' }
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

function validateImportRow(row: ParsedRow): string[] {
  const errors: string[] = []
  if (!row.username?.trim()) errors.push('Username is required')
  if (!row.password?.trim()) errors.push('Password is required')
  else if (row.password.trim().length < 8) errors.push('Password must be at least 8 characters')
  if (row.email?.trim() && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(row.email.trim())) {
    errors.push('Invalid email format')
  }
  if (row.role?.trim() && !KNOWN_ROLES.includes(row.role.trim())) {
    errors.push(`Unknown role "${row.role.trim()}"`)
  }
  return errors
}

function downloadTemplate() {
  const header = IMPORT_COLUMNS.join(',')
  const sample = 'jdoe,SecurePass123,Jane Doe,jane@example.com,end_user,1001,Engineering'
  const csv = `${header}\n${sample}\n`
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'users_import_template.csv'
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

function BulkImportDialog({
  open,
  onOpenChange,
  onComplete,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: () => void
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
    abortRef.current = false
  }, [])

  const handleClose = (v: boolean) => {
    if (!v) {
      if (isImporting) {
        abortRef.current = true
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
        validationErrors: validateImportRow(data),
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

      const username = row.data.username.trim()
      const password = row.data.password.trim()
      const updateData: UpdateUserRequest = {}
      if (row.data.display_name?.trim()) updateData.display_name = row.data.display_name.trim()
      if (row.data.email?.trim()) updateData.email = row.data.email.trim()
      if (row.data.role?.trim()) updateData.role = row.data.role.trim()
      else updateData.role = 'end_user'
      if (row.data.extension?.trim()) updateData.extension = row.data.extension.trim()
      if (row.data.department?.trim()) updateData.department = row.data.department.trim()
      if (row.data.location_id?.trim()) updateData.location_id = row.data.location_id.trim()

      try {
        const req: CreateUserRequest = { username, password, ...updateData }
        await users.create(req)
        created++
        setImportedCount(created)
        setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success' } : r))
      } catch (createErr: unknown) {
        const createMsg = createErr instanceof Error ? createErr.message : 'Unknown error'
        const isConflict = (createErr as { status?: number }).status === 409

        if (overrideExisting && isConflict) {
          try {
            await users.update(username, updateData)
            await users.updatePassword(username, password)
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
            {step === 'upload' && 'Bulk Import Users'}
            {step === 'preview' && 'Preview Import'}
            {step === 'import' && 'Importing Users'}
          </DialogTitle>
          <DialogDescription>
            {step === 'upload' && 'Upload a CSV file to create multiple user accounts at once.'}
            {step === 'preview' && `Review ${rows.length} rows before importing.`}
            {step === 'import' && 'Creating user accounts...'}
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
              <Button variant="link" size="sm" className="px-0" onClick={downloadTemplate}>
                <Download className="mr-1 h-3 w-3" />
                Download Template
              </Button>
              <p className="text-xs text-muted-foreground">
                Required columns: username, password
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
                    <TableHead>Username</TableHead>
                    <TableHead>Display Name</TableHead>
                    <TableHead>Email</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Ext</TableHead>
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
                      <TableCell className="font-medium">{row.data.username || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.display_name || '-'}</TableCell>
                      <TableCell>{row.data.email || '-'}</TableCell>
                      <TableCell>{row.data.role || 'end_user'}</TableCell>
                      <TableCell>{row.data.extension || '-'}</TableCell>
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
                    Override existing users
                  </Label>
                </div>
                <Button onClick={startImport} disabled={validRows.length === 0}>
                  Import {validRows.length} User{validRows.length !== 1 ? 's' : ''}
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
                    <TableHead>Username</TableHead>
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
                      <TableCell className="font-medium">{row.data.username || '(empty)'}</TableCell>
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

            <div className="flex justify-end">
              {isImporting ? (
                <Button variant="outline" onClick={() => { abortRef.current = true }}>
                  Cancel
                </Button>
              ) : (
                <Button onClick={handleDone}>Done</Button>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function DigestCredentialsDialog({
  user,
  onOpenChange,
  mutation,
  deleteMutation,
}: {
  user: User | null
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<unknown, Error, { username: string; digest_password: string }>>
  deleteMutation: ReturnType<typeof useMutation<unknown, Error, string>>
}) {
  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<DigestCredentialsForm>({
    resolver: zodResolver(digestCredentialsSchema),
  })

  const onSubmit = (data: DigestCredentialsForm) => {
    if (!user) return
    mutation.mutate({ username: user.username, digest_password: data.digest_password })
  }

  return (
    <Dialog open={!!user} onOpenChange={(v) => { if (!v) reset(); onOpenChange(v) }}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Set Digest Credentials</DialogTitle>
          <DialogDescription>
            Set SIP digest credentials for {user?.username}. These are used for phone registration (separate from the web password).
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="digest-password">Digest Password *</Label>
            <Input id="digest-password" type="password" {...register('digest_password')} />
            {errors.digest_password && <p className="text-sm text-destructive">{errors.digest_password.message}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="digest-confirm-password">Confirm Password *</Label>
            <Input id="digest-confirm-password" type="password" {...register('confirm_password')} />
            {errors.confirm_password && <p className="text-sm text-destructive">{errors.confirm_password.message}</p>}
          </div>
          <div className="flex justify-between">
            <Button
              type="button"
              variant="outline"
              className="text-destructive"
              disabled={deleteMutation.isPending}
              onClick={() => {
                if (!user) return
                if (confirm(`Clear digest credentials for "${user.username}"? SIP auth will fall back to the main password.`)) {
                  deleteMutation.mutate(user.username)
                }
              }}
            >
              {deleteMutation.isPending ? 'Clearing...' : 'Clear Credentials'}
            </Button>
            <div className="flex gap-2">
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending ? 'Setting...' : 'Set Credentials'}
              </Button>
            </div>
          </div>
          {mutation.isError && (
            <p className="text-sm text-destructive">{(mutation.error as Error).message}</p>
          )}
          {deleteMutation.isError && (
            <p className="text-sm text-destructive">{(deleteMutation.error as Error).message}</p>
          )}
          {mutation.isSuccess && (
            <p className="text-sm text-green-600">Digest credentials set successfully.</p>
          )}
          {deleteMutation.isSuccess && (
            <p className="text-sm text-green-600">Digest credentials cleared.</p>
          )}
        </form>
      </DialogContent>
    </Dialog>
  )
}
