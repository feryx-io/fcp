import { useMemo, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, Trash2, Key, RefreshCw, AlertTriangle, User } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { TokenSecretDialog, rotationNote, tokenStatus } from '@/components/TokenSecretDialog'
import { apiTokens } from '@/api/endpoints'
import type { ApiToken, GrantablePermission } from '@/types'

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Request failed'
}

/**
 * The tenant's API tokens (docs/DEVELOPER_SURFACE.md §3, phase 1). A token
 * carries permissions its maker holds; it names its maker, who answers for
 * it. Revoking keeps the row; rotating gives a new secret and keeps the old
 * one working for a grace period. Personal tokens appear here too, and can
 * be revoked, but only their maker rotates them.
 */
export function ApiTokensPage() {
  const queryClient = useQueryClient()
  const [dialogOpen, setDialogOpen] = useState(false)
  const [secret, setSecret] = useState<{ value: string; title: string; note?: string } | null>(null)
  const [name, setName] = useState('')
  const [expires, setExpires] = useState('')
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [formError, setFormError] = useState<string | null>(null)

  const { data: tokenList = [], isLoading } = useQuery({
    queryKey: ['api-tokens'],
    queryFn: apiTokens.list,
  })
  const { data: grantable = [] } = useQuery({
    queryKey: ['api-tokens', 'permissions'],
    queryFn: apiTokens.permissions,
  })
  const groups = useMemo(() => {
    const by = new Map<string, GrantablePermission[]>()
    for (const p of grantable) by.set(p.group, [...(by.get(p.group) ?? []), p])
    return [...by.entries()]
  }, [grantable])

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['api-tokens'] })
  const createMutation = useMutation({
    mutationFn: apiTokens.create,
    onSuccess: (result) => {
      invalidate()
      setDialogOpen(false)
      setSecret({ value: result.secret, title: 'Token created' })
    },
    onError: (e) => setFormError(errorText(e)),
  })
  const rotateMutation = useMutation({
    mutationFn: (id: string) => apiTokens.rotate(id),
    onSuccess: (result) => {
      invalidate()
      setSecret({
        value: result.secret,
        title: 'Token rotated',
        note: rotationNote(result.token.previous_secret_valid_until),
      })
    },
    onError: (e) => alert(errorText(e)),
  })
  const revokeMutation = useMutation({
    mutationFn: apiTokens.delete,
    onSuccess: invalidate,
  })

  const openCreate = () => {
    setName('')
    setExpires('')
    setChosen(new Set())
    setFormError(null)
    setDialogOpen(true)
  }
  const toggle = (perm: string) => {
    const next = new Set(chosen)
    if (next.has(perm)) next.delete(perm)
    else next.add(perm)
    setChosen(next)
  }
  const toggleGroup = (perms: GrantablePermission[]) => {
    const all = perms.every((p) => chosen.has(p.name))
    const next = new Set(chosen)
    for (const p of perms) {
      if (all) next.delete(p.name)
      else next.add(p.name)
    }
    setChosen(next)
  }
  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim()) return setFormError('Name is required')
    if (chosen.size === 0) return setFormError('Choose at least one permission')
    createMutation.mutate({
      name: name.trim(),
      permissions: [...chosen],
      expires_at: expires || undefined,
    })
  }

  const columns: ColumnDef<ApiToken>[] = [
    {
      accessorKey: 'name',
      header: 'Name',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          {row.original.scope === 'personal'
            ? <User className="h-4 w-4 text-muted-foreground" />
            : <Key className="h-4 w-4 text-muted-foreground" />}
          <span className="font-medium">{row.original.name}</span>
          <span className="font-mono text-xs bg-muted rounded px-1.5 py-0.5">{row.original.prefix}…</span>
        </div>
      ),
    },
    {
      accessorKey: 'created_by',
      header: 'Made by',
      cell: ({ row }) => row.original.created_by
        || <span className="text-muted-foreground text-sm">platform</span>,
    },
    {
      accessorKey: 'permissions',
      header: 'Reaches',
      cell: ({ row }) => {
        const t = row.original
        if (t.scope === 'personal') return <Badge variant="secondary">Its maker's /me</Badge>
        if (t.legacy_full_access) {
          return (
            <Badge variant="destructive" className="gap-1" title="Made before permissions were checked: review and replace it">
              <AlertTriangle className="h-3 w-3" /> Full access (legacy)
            </Badge>
          )
        }
        return (
          <div className="flex gap-1 flex-wrap" title={t.permissions.join(', ')}>
            {t.permissions.slice(0, 3).map((p) => (
              <Badge key={p} variant="outline" className="text-xs">{p}</Badge>
            ))}
            {t.permissions.length > 3 && (
              <Badge variant="outline" className="text-xs">+{t.permissions.length - 3}</Badge>
            )}
          </div>
        )
      },
    },
    {
      id: 'status',
      header: 'Status',
      cell: ({ row }) => {
        const t = row.original
        const status = tokenStatus(t)
        if (status === 'revoked') {
          return (
            <Badge variant="secondary" title={`Revoked by ${t.revoked_by ?? 'unknown'}`}>
              Revoked {new Date(t.revoked_at!).toLocaleDateString()}
            </Badge>
          )
        }
        if (status === 'expired') return <Badge variant="destructive">Expired</Badge>
        return (
          <div className="flex gap-1 flex-wrap">
            <Badge variant="outline">
              {t.expires_at ? `Until ${new Date(t.expires_at).toLocaleDateString()}` : 'No expiry'}
            </Badge>
            {t.previous_secret_valid_until && (
              <Badge variant="outline" title="The secret a rotation replaced still works until then">
                Old secret until {new Date(t.previous_secret_valid_until).toLocaleString()}
              </Badge>
            )}
          </div>
        )
      },
    },
    {
      accessorKey: 'last_used_at',
      header: 'Last used',
      cell: ({ row }) => row.original.last_used_at
        ? new Date(row.original.last_used_at).toLocaleString()
        : <span className="text-muted-foreground text-sm">Never</span>,
    },
    {
      id: 'actions',
      cell: ({ row }) => {
        const t = row.original
        if (t.revoked_at) return null
        return (
          <div className="flex justify-end gap-1">
            {t.scope === 'admin' && (
              <Button
                variant="ghost"
                size="icon"
                title="Rotate: a new secret, the old one working for a day"
                onClick={() => {
                  if (confirm(`Rotate "${t.name}"? The current secret keeps working for 24 hours.`)) {
                    rotateMutation.mutate(t.id)
                  }
                }}
              >
                <RefreshCw className="h-4 w-4" />
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              className="text-destructive hover:text-destructive"
              title="Revoke"
              onClick={() => {
                if (confirm(`Revoke "${t.name}"? It stops working at once.`)) {
                  revokeMutation.mutate(t.id)
                }
              }}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        )
      },
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">API Tokens</h1>
          <p className="text-muted-foreground">
            Tokens for programs. Each carries permissions you hold, and stops working if its maker is disabled.{' '}
            <a href="/admin/api" target="_blank" rel="noreferrer" className="underline">
              API reference
            </a>
          </p>
        </div>
        <Button onClick={openCreate} disabled={grantable.length === 0}>
          <Plus className="mr-2 h-4 w-4" />
          Create Token
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={tokenList} searchKey="name" searchPlaceholder="Search tokens..." />
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Create API Token</DialogTitle>
            <DialogDescription>
              Choose what the token reaches. You can grant only permissions you hold.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={submit} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="token-name">Name *</Label>
                <Input id="token-name" placeholder="e.g., CRM integration" value={name}
                  maxLength={64} onChange={(e) => setName(e.target.value)} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="token-expires">Expires (optional)</Label>
                <Input id="token-expires" type="date" value={expires}
                  onChange={(e) => setExpires(e.target.value)} />
              </div>
            </div>
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>Permissions ({chosen.size})</Label>
                <Button type="button" variant="link" size="sm" className="h-auto p-0"
                  onClick={() => setChosen(chosen.size === grantable.length
                    ? new Set()
                    : new Set(grantable.map((p) => p.name)))}>
                  {chosen.size === grantable.length ? 'Clear all' : 'Everything I may grant'}
                </Button>
              </div>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 rounded-md border p-3">
                {groups.map(([group, perms]) => (
                  <div key={group} className="space-y-1">
                    <label className="flex items-center gap-2 text-sm font-medium">
                      <input
                        type="checkbox"
                        className="rounded border-input"
                        checked={perms.every((p) => chosen.has(p.name))}
                        onChange={() => toggleGroup(perms)}
                      />
                      {group}
                    </label>
                    {perms.map((p) => (
                      <label key={p.name} className="flex items-center gap-2 pl-5 text-sm">
                        <input
                          type="checkbox"
                          className="rounded border-input"
                          checked={chosen.has(p.name)}
                          onChange={() => toggle(p.name)}
                        />
                        <span>{p.label}</span>
                      </label>
                    ))}
                  </div>
                ))}
              </div>
            </div>
            {formError && <p className="text-sm text-destructive">{formError}</p>}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Token'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      <TokenSecretDialog
        secret={secret?.value ?? null}
        title={secret?.title ?? ''}
        note={secret?.note}
        onClose={() => setSecret(null)}
      />
    </div>
  )
}
