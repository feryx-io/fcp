import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, KeyRound, Trash2 } from 'lucide-react'
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
import { users } from '@/api/endpoints'
import type { User } from '@/types'

const createUserSchema = z.object({
  username: z.string().min(1, 'Username is required').max(64),
  password: z.string().min(8, 'Password must be at least 8 characters').max(128),
  display_name: z.string().optional(),
  email: z.string().email().optional().or(z.literal('')),
  role: z.string().optional(),
  extension: z.string().optional(),
  department: z.string().optional(),
})

const editUserSchema = z.object({
  display_name: z.string().optional(),
  email: z.string().email().optional().or(z.literal('')),
  role: z.string().optional(),
  extension: z.string().optional(),
  department: z.string().optional(),
  account_status: z.string().optional(),
})

const changePasswordSchema = z.object({
  password: z.string().min(8, 'Password must be at least 8 characters').max(128),
  confirm_password: z.string(),
}).refine((data) => data.password === data.confirm_password, {
  message: 'Passwords do not match',
  path: ['confirm_password'],
})

type CreateUserForm = z.infer<typeof createUserSchema>
type EditUserForm = z.infer<typeof editUserSchema>
type ChangePasswordForm = z.infer<typeof changePasswordSchema>

const roleBadgeVariant = (role: string) => {
  switch (role) {
    case 'super_admin': case 'superadmin': return 'destructive' as const
    case 'telecom_admin': case 'telecomadmin': return 'default' as const
    case 'tenant_admin': case 'tenantadmin': return 'default' as const
    case 'operator': return 'secondary' as const
    default: return 'outline' as const
  }
}

const statusBadgeVariant = (status: string) => {
  switch (status) {
    case 'active': return 'success' as const
    case 'suspended': return 'warning' as const
    case 'disabled': return 'destructive' as const
    default: return 'secondary' as const
  }
}

export function UsersPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editUser, setEditUser] = useState<User | null>(null)
  const [passwordUser, setPasswordUser] = useState<User | null>(null)
  const queryClient = useQueryClient()

  const { data: userList = [], isLoading } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

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

  const deleteMutation = useMutation({
    mutationFn: users.delete,
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
        <Badge variant={statusBadgeVariant(row.original.account_status)}>
          {row.original.account_status}
        </Badge>
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
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="mr-2 h-4 w-4" />
          Add User
        </Button>
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
      />

      {/* Edit User Dialog */}
      <EditUserDialog
        user={editUser}
        onOpenChange={(open) => { if (!open) setEditUser(null) }}
        mutation={updateMutation}
      />

      {/* Change Password Dialog */}
      <ChangePasswordDialog
        user={passwordUser}
        onOpenChange={(open) => { if (!open) setPasswordUser(null) }}
        mutation={passwordMutation}
      />
    </div>
  )
}

function CreateUserDialog({
  open,
  onOpenChange,
  mutation,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<User, Error, Parameters<typeof users.create>[0]>>
}) {
  const {
    register,
    handleSubmit,
    reset,
    setValue,
    formState: { errors },
  } = useForm<CreateUserForm>({
    resolver: zodResolver(createUserSchema),
    defaultValues: { role: 'user' },
  })

  const onSubmit = (data: CreateUserForm) => {
    mutation.mutate({
      ...data,
      email: data.email || undefined,
      display_name: data.display_name || undefined,
      extension: data.extension || undefined,
      department: data.department || undefined,
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
            <Select defaultValue="user" onValueChange={(v) => setValue('role', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="enduser">User</SelectItem>
                <SelectItem value="operator">Operator</SelectItem>
                <SelectItem value="tenantadmin">Tenant Admin</SelectItem>
                <SelectItem value="telecomadmin">Telecom Admin</SelectItem>
                <SelectItem value="superadmin">Super Admin</SelectItem>
              </SelectContent>
            </Select>
          </div>
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
}: {
  user: User | null
  onOpenChange: (open: boolean) => void
  mutation: ReturnType<typeof useMutation<unknown, Error, { username: string; data: EditUserForm }>>
}) {
  const {
    register,
    handleSubmit,
    setValue,
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
      account_status: user.account_status || 'active',
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
                  <SelectItem value="enduser">User</SelectItem>
                  <SelectItem value="operator">Operator</SelectItem>
                  <SelectItem value="tenantadmin">Tenant Admin</SelectItem>
                  <SelectItem value="telecomadmin">Telecom Admin</SelectItem>
                  <SelectItem value="superadmin">Super Admin</SelectItem>
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
