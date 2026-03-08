import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, Trash2, Copy, Key, Eye, EyeOff } from 'lucide-react'
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
import { apiTokens } from '@/api/endpoints'
import type { ApiToken } from '@/types'

const AVAILABLE_PERMISSIONS = [
  { value: 'admin', label: 'Admin — full access to all admin endpoints' },
  { value: 'read', label: 'Read — read-only access' },
] as const

const createTokenSchema = z.object({
  name: z.string().min(1, 'Name is required').max(64),
  permissions: z.array(z.string()).default([]),
  expires_at: z.string().optional(),
})

type CreateTokenForm = z.infer<typeof createTokenSchema>

export function ApiTokensPage() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [newSecret, setNewSecret] = useState<string | null>(null)
  const [showSecret, setShowSecret] = useState(false)
  const queryClient = useQueryClient()

  const { data: tokenList = [], isLoading } = useQuery({
    queryKey: ['api-tokens'],
    queryFn: apiTokens.list,
  })

  const createMutation = useMutation({
    mutationFn: apiTokens.create,
    onSuccess: (result) => {
      queryClient.invalidateQueries({ queryKey: ['api-tokens'] })
      setDialogOpen(false)
      setNewSecret(result.secret)
      setShowSecret(true)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: apiTokens.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['api-tokens'] }),
  })

  const columns: ColumnDef<ApiToken>[] = [
    {
      accessorKey: 'name',
      header: 'Name',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Key className="h-4 w-4 text-muted-foreground" />
          <span className="font-medium">{row.original.name}</span>
        </div>
      ),
    },
    {
      accessorKey: 'prefix',
      header: 'Prefix',
      cell: ({ row }) => (
        <span className="font-mono text-xs bg-muted rounded px-2 py-1">
          {row.original.prefix}...
        </span>
      ),
    },
    {
      accessorKey: 'permissions',
      header: 'Permissions',
      cell: ({ row }) => (
        <div className="flex gap-1 flex-wrap">
          {row.original.permissions.length === 0 ? (
            <Badge variant="secondary">None</Badge>
          ) : (
            row.original.permissions.slice(0, 3).map((p) => (
              <Badge key={p} variant="outline" className="text-xs">{p}</Badge>
            ))
          )}
          {row.original.permissions.length > 3 && (
            <Badge variant="outline" className="text-xs">+{row.original.permissions.length - 3}</Badge>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'expires_at',
      header: 'Expires',
      cell: ({ row }) => {
        if (!row.original.expires_at) return <span className="text-muted-foreground text-sm">Never</span>
        const expires = new Date(row.original.expires_at)
        const isExpired = expires < new Date()
        return (
          <Badge variant={isExpired ? 'destructive' : 'outline'}>
            {isExpired ? 'Expired' : expires.toLocaleDateString()}
          </Badge>
        )
      },
    },
    {
      accessorKey: 'last_used_at',
      header: 'Last Used',
      cell: ({ row }) => row.original.last_used_at
        ? new Date(row.original.last_used_at).toLocaleDateString()
        : <span className="text-muted-foreground text-sm">Never</span>,
    },
    {
      accessorKey: 'created_at',
      header: 'Created',
      cell: ({ row }) => new Date(row.original.created_at).toLocaleDateString(),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <Button
          variant="ghost"
          size="icon"
          className="text-destructive hover:text-destructive"
          onClick={() => {
            if (confirm(`Revoke token "${row.original.name}"? This cannot be undone.`)) {
              deleteMutation.mutate(row.original.id)
            }
          }}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      ),
    },
  ]

  const {
    register, handleSubmit, reset,
    formState: { errors },
  } = useForm<CreateTokenForm>({ resolver: zodResolver(createTokenSchema) })

  const onSubmit = (data: CreateTokenForm) => {
    createMutation.mutate({
      name: data.name,
      permissions: data.permissions,
      expires_at: data.expires_at || undefined,
    })
  }

  const copySecret = () => {
    if (newSecret) {
      navigator.clipboard.writeText(newSecret)
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">API Tokens</h1>
          <p className="text-muted-foreground">Manage API access tokens for programmatic access</p>
        </div>
        <Button onClick={() => { reset(); setDialogOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" />
          Create Token
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={tokenList} searchKey="name" searchPlaceholder="Search tokens..." />
      )}

      {/* Create Token Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create API Token</DialogTitle>
            <DialogDescription>Generate a new token for API access. The secret will only be shown once.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="token-name">Token Name *</Label>
              <Input id="token-name" placeholder="e.g., CI/CD Pipeline" {...register('name')} />
              {errors.name && <p className="text-sm text-destructive">{errors.name.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Permissions</Label>
              <p className="text-xs text-muted-foreground">Tokens with no permissions selected cannot access admin endpoints.</p>
              {AVAILABLE_PERMISSIONS.map((perm) => (
                <label key={perm.value} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    value={perm.value}
                    {...register('permissions')}
                    className="rounded border-input"
                  />
                  <span>{perm.label}</span>
                </label>
              ))}
            </div>
            <div className="space-y-2">
              <Label htmlFor="token-expires">Expires (optional)</Label>
              <Input id="token-expires" type="date" {...register('expires_at')} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Token'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      {/* Secret Display Dialog */}
      <Dialog open={!!newSecret} onOpenChange={() => { setNewSecret(null); setShowSecret(false) }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Token Created</DialogTitle>
            <DialogDescription>
              Copy your token secret now. It will not be shown again.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <div className="flex-1 relative">
                <Input
                  readOnly
                  type={showSecret ? 'text' : 'password'}
                  value={newSecret ?? ''}
                  className="font-mono text-xs pr-16"
                />
                <div className="absolute right-1 top-1 flex gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    onClick={() => setShowSecret(!showSecret)}
                  >
                    {showSecret ? <EyeOff className="h-3 w-3" /> : <Eye className="h-3 w-3" />}
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    onClick={copySecret}
                  >
                    <Copy className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Store this secret securely. You will not be able to see it again.
            </p>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
