import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Key, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { TokenSecretDialog, rotationNote, tokenStatus } from '@/components/TokenSecretDialog'
import { me } from '@/api/endpoints'

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Request failed'
}

/**
 * Personal API tokens (docs/DEVELOPER_SURFACE.md §3, phase 1): a program
 * acting as you, on your own settings, voicemail and conferences — the
 * /api/v1/me routes and nothing else. Your password and your sign-in are
 * not reachable with one.
 */
export function UserApiTokensPage() {
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [expires, setExpires] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [secret, setSecret] = useState<{ value: string; title: string; note?: string } | null>(null)

  const { data: tokens = [], isLoading } = useQuery({
    queryKey: ['me', 'api-tokens'],
    queryFn: me.listApiTokens,
  })
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['me', 'api-tokens'] })

  const create = useMutation({
    mutationFn: me.createApiToken,
    onSuccess: (result) => {
      invalidate()
      setName('')
      setExpires('')
      setError(null)
      setSecret({ value: result.secret, title: 'Token created' })
    },
    onError: (e) => setError(errorText(e)),
  })
  const rotate = useMutation({
    mutationFn: (id: string) => me.rotateApiToken(id),
    onSuccess: (result) => {
      invalidate()
      setSecret({
        value: result.secret,
        title: 'Token rotated',
        note: rotationNote(result.token.previous_secret_valid_until),
      })
    },
    onError: (e) => setError(errorText(e)),
  })
  const revoke = useMutation({
    mutationFn: me.revokeApiToken,
    onSuccess: invalidate,
    onError: (e) => setError(errorText(e)),
  })

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">API Tokens</h1>
        <p className="text-muted-foreground">
          Let a program act as you on your own settings, voicemail and conferences — nothing else.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>New token</CardTitle>
          <CardDescription>The secret is shown once. Send it as <code>Authorization: Bearer …</code>.</CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault()
              if (!name.trim()) return setError('Name is required')
              create.mutate({ name: name.trim(), expires_at: expires || undefined })
            }}
          >
            <div className="space-y-2">
              <Label htmlFor="pt-name">Name</Label>
              <Input id="pt-name" value={name} maxLength={64} placeholder="e.g., My CRM"
                onChange={(e) => setName(e.target.value)} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="pt-expires">Expires (optional)</Label>
              <Input id="pt-expires" type="date" value={expires} onChange={(e) => setExpires(e.target.value)} />
            </div>
            <Button type="submit" disabled={create.isPending}>
              <Plus className="mr-2 h-4 w-4" /> Create
            </Button>
          </form>
          {error && <p className="mt-3 text-sm text-destructive">{error}</p>}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Your tokens</CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <p className="text-muted-foreground text-sm">Loading…</p>
          ) : tokens.length === 0 ? (
            <p className="text-muted-foreground text-sm">No tokens yet.</p>
          ) : (
            <ul className="divide-y">
              {tokens.map((t) => {
                const status = tokenStatus(t)
                return (
                  <li key={t.id} className="flex items-center justify-between gap-3 py-3">
                    <div className="flex items-center gap-2 min-w-0">
                      <Key className="h-4 w-4 text-muted-foreground shrink-0" />
                      <span className="font-medium truncate">{t.name}</span>
                      <span className="font-mono text-xs bg-muted rounded px-1.5 py-0.5">{t.prefix}…</span>
                      <Badge variant={status === 'active' ? 'outline' : 'secondary'}>
                        {status === 'active'
                          ? (t.last_used_at ? `Used ${new Date(t.last_used_at).toLocaleDateString()}` : 'Never used')
                          : status === 'revoked' ? 'Revoked' : 'Expired'}
                      </Badge>
                    </div>
                    {status === 'active' && (
                      <div className="flex gap-1 shrink-0">
                        <Button variant="ghost" size="icon" title="Rotate"
                          onClick={() => {
                            if (confirm(`Rotate "${t.name}"? The current secret keeps working for 24 hours.`)) {
                              rotate.mutate(t.id)
                            }
                          }}>
                          <RefreshCw className="h-4 w-4" />
                        </Button>
                        <Button variant="ghost" size="icon" title="Revoke"
                          className="text-destructive hover:text-destructive"
                          onClick={() => {
                            if (confirm(`Revoke "${t.name}"? It stops working at once.`)) revoke.mutate(t.id)
                          }}>
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    )}
                  </li>
                )
              })}
            </ul>
          )}
        </CardContent>
      </Card>

      <TokenSecretDialog
        secret={secret?.value ?? null}
        title={secret?.title ?? ''}
        note={secret?.note}
        onClose={() => setSecret(null)}
      />
    </div>
  )
}
