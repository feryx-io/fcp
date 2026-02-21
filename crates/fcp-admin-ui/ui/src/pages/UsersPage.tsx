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
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
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

type CreateUserForm = z.infer<typeof createUserSchema>

const roleBadgeVariant = (role: string) => {
  switch (role) {
    case 'super_admin': return 'destructive' as const
    case 'telecom_admin': return 'default' as const
    case 'tenant_admin': return 'default' as const
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
  const [dialogOpen, setDialogOpen] = useState(false)
  const queryClient = useQueryClient()

  const { data: userList = [], isLoading } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const createMutation = useMutation({
    mutationFn: users.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] })
      setDialogOpen(false)
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
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete user "${row.original.username}"?`)) {
                  deleteMutation.mutate(row.original.username)
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
    createMutation.mutate({
      ...data,
      email: data.email || undefined,
      display_name: data.display_name || undefined,
      extension: data.extension || undefined,
      department: data.department || undefined,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Users</h1>
          <p className="text-muted-foreground">Manage user accounts and credentials</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" />
          Add User
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={userList} searchKey="username" searchPlaceholder="Search users..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create User</DialogTitle>
            <DialogDescription>Add a new user account to the system.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="username">Username *</Label>
              <Input id="username" {...register('username')} />
              {errors.username && <p className="text-sm text-destructive">{errors.username.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">Password *</Label>
              <Input id="password" type="password" {...register('password')} />
              {errors.password && <p className="text-sm text-destructive">{errors.password.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="display_name">Display Name</Label>
              <Input id="display_name" {...register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input id="email" type="email" {...register('email')} />
              {errors.email && <p className="text-sm text-destructive">{errors.email.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Role</Label>
              <Select defaultValue="user" onValueChange={(v) => setValue('role', v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="user">User</SelectItem>
                  <SelectItem value="operator">Operator</SelectItem>
                  <SelectItem value="tenant_admin">Tenant Admin</SelectItem>
                  <SelectItem value="telecom_admin">Telecom Admin</SelectItem>
                  <SelectItem value="super_admin">Super Admin</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="extension">Extension</Label>
                <Input id="extension" {...register('extension')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="department">Department</Label>
                <Input id="department" {...register('department')} />
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create User'}
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
