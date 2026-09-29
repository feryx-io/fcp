import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, Trash2, Copy, Eye, EyeOff, KeyRound, Power, PowerOff, Cable } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import { sites, cluster } from '@/api/endpoints'
import { formatDuration, epochSecs } from '@/lib/utils'
import { useEffectiveTenantId } from '@/components/TenantSelector'
import type { SiteInfo, SiteWithToken } from '@/types'

// A site is a tenant's own stack on the tenant's network, enrolled to
// this cloud and kept by fcp-link (docs/HYBRID_CONTROL_PLANE.md). Made
// here with a one-time enrollment token, which the site's operator gives
// fcp-link; shown here as its link's heartbeats describe it.

const createSchema = z.object({
  name: z.string().trim().min(1, 'Name is required').max(100),
})

type CreateForm = z.infer<typeof createSchema>

const ago = (at?: string) =>
  at === undefined ? 'never' : formatDuration(Math.max(0, Math.floor(Date.now() / 1000 - epochSecs(at)))) + ' ago'

export function SitesPage() {
  const tenantId = useEffectiveTenantId()
  // A platform admin on "All Tenants" sees every site and manages none:
  // a site is made in a tenant.
  const canManage = tenantId !== ''
  const [createOpen, setCreateOpen] = useState(false)
  const [token, setToken] = useState<SiteWithToken | null>(null)
  const [showToken, setShowToken] = useState(false)
  const [detail, setDetail] = useState<SiteInfo | null>(null)
  const [deleting, setDeleting] = useState<SiteInfo | null>(null)
  const queryClient = useQueryClient()

  const { data: siteList = [], isLoading } = useQuery({
    queryKey: ['sites', tenantId],
    queryFn: async () => {
      if (canManage) return sites.list(tenantId)
      const all = await cluster.sites()
      return all.sites
    },
    refetchInterval: 15000,
  })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['sites'] })

  const createMutation = useMutation({
    mutationFn: (data: CreateForm) => sites.create({ name: data.name }, tenantId),
    onSuccess: (made) => {
      refresh()
      setCreateOpen(false)
      setToken(made)
      setShowToken(true)
    },
  })
  const tokenMutation = useMutation({
    mutationFn: (id: string) => sites.newToken(id, tenantId),
    onSuccess: (made) => {
      refresh()
      setToken(made)
      setShowToken(true)
    },
  })
  const disableMutation = useMutation({
    mutationFn: (id: string) => sites.disable(id, tenantId),
    onSuccess: refresh,
  })
  const enableMutation = useMutation({
    mutationFn: (id: string) => sites.enable(id, tenantId),
    onSuccess: refresh,
  })
  const deleteMutation = useMutation({
    mutationFn: (id: string) => sites.delete(id, tenantId),
    onSuccess: () => {
      refresh()
      setDeleting(null)
    },
  })

  const {
    register, handleSubmit, reset,
    formState: { errors },
  } = useForm<CreateForm>({ resolver: zodResolver(createSchema) })

  const columns: ColumnDef<SiteInfo>[] = [
    {
      accessorKey: 'name',
      header: 'Site',
      cell: ({ row }) => (
        <button className="flex items-center gap-2 text-left" onClick={() => setDetail(row.original)}>
          <Cable className="h-4 w-4 text-muted-foreground" />
          <span className="font-medium">{row.original.name}</span>
        </button>
      ),
    },
    ...(canManage ? [] : [{
      accessorKey: 'tenant_id',
      header: 'Tenant',
      cell: ({ row }: { row: { original: SiteInfo } }) => (
        <span className="font-mono text-xs">{row.original.tenant_id}</span>
      ),
    } as ColumnDef<SiteInfo>]),
    {
      accessorKey: 'status',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.status === 'enrolled' ? 'success' : row.original.status === 'disabled' ? 'destructive' : 'outline'}>
          {row.original.status}
        </Badge>
      ),
    },
    {
      accessorKey: 'link_up',
      header: 'Link',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Badge variant={row.original.link_up ? 'success' : 'warning'}>{row.original.link_up ? 'up' : 'down'}</Badge>
          <span className="text-muted-foreground text-xs">{ago(row.original.last_seen_at)}</span>
        </div>
      ),
    },
    {
      id: 'sync',
      header: 'Sync',
      cell: ({ row }) => {
        const site = row.original
        if (site.sync_cursor === undefined) return <span className="text-muted-foreground">—</span>
        if (site.sync_error) return <Badge variant="destructive" title={site.sync_error}>error</Badge>
        if (site.sync_behind) return <Badge variant="warning">{site.sync_behind} behind</Badge>
        return <Badge variant="success">current</Badge>
      },
    },
    {
      id: 'registrations',
      header: 'Registered',
      cell: ({ row }) => {
        const regs = row.original.registrations
        if (!regs) return <span className="text-muted-foreground">—</span>
        return <span>{regs.total_users} user{regs.total_users === 1 ? '' : 's'}, {regs.total_contacts} contact{regs.total_contacts === 1 ? '' : 's'}</span>
      },
    },
    {
      id: 'nodes',
      header: 'Nodes',
      cell: ({ row }) => (
        <span>
          {row.original.nodes.length}
          {row.original.nodes.some((n) => !n.healthy) && (
            <span className="ml-1 text-yellow-600">({row.original.nodes.filter((n) => !n.healthy).length} unhealthy)</span>
          )}
        </span>
      ),
    },
    {
      id: 'trunk',
      header: 'Link trunk',
      cell: ({ row }) => row.original.trunk_id
        ? <span className="font-mono text-xs">{row.original.trunk_id}</span>
        : <span className="text-muted-foreground text-xs">none</span>,
    },
    {
      id: 'actions',
      cell: ({ row }) => canManage ? (
        <div className="flex items-center gap-1 justify-end">
          <Button
            variant="ghost" size="icon" title="New enrollment token"
            disabled={row.original.status === 'disabled' || tokenMutation.isPending}
            onClick={() => tokenMutation.mutate(row.original.id)}
          >
            <KeyRound className="h-4 w-4" />
          </Button>
          {row.original.status === 'disabled' ? (
            <Button variant="ghost" size="icon" title="Enable" onClick={() => enableMutation.mutate(row.original.id)}>
              <Power className="h-4 w-4" />
            </Button>
          ) : (
            <Button variant="ghost" size="icon" title="Disable: its certificate and secret are refused" onClick={() => disableMutation.mutate(row.original.id)}>
              <PowerOff className="h-4 w-4" />
            </Button>
          )}
          <Button variant="ghost" size="icon" className="text-destructive hover:text-destructive" title="Delete" onClick={() => setDeleting(row.original)}>
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ) : null,
    },
  ]

  const copyToken = () => {
    if (token) navigator.clipboard.writeText(token.enrollment_token)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Sites</h1>
          <p className="text-muted-foreground">
            A tenant's own stack on its network, enrolled to this cloud and kept in step by its link
          </p>
        </div>
        {canManage ? (
          <Button onClick={() => { reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" />
            New Site
          </Button>
        ) : (
          <p className="text-sm text-muted-foreground">Pick a tenant to make or change a site.</p>
        )}
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={siteList} searchKey="name" searchPlaceholder="Search sites..." />
      )}

      {/* Create */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>New Site</DialogTitle>
            <DialogDescription>
              The site gets a one-time enrollment token, shown once: give it to fcp-link at the site.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSubmit((data) => createMutation.mutate(data))} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="site-name">Name *</Label>
              <Input id="site-name" placeholder="e.g. acme-hq" {...register('name')} />
              {errors.name && <p className="text-sm text-destructive">{errors.name.message}</p>}
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Site'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* The token, once */}
      <Dialog open={!!token} onOpenChange={() => { setToken(null); setShowToken(false) }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Enrollment token for {token?.name}</DialogTitle>
            <DialogDescription>
              Copy it now; it is not shown again. It is spent by the first enrollment and expires
              {token ? ' ' + new Date(token.enrollment_token_expires_at).toLocaleString() : ''}.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="relative">
              <Input
                readOnly
                type={showToken ? 'text' : 'password'}
                value={token?.enrollment_token ?? ''}
                className="font-mono text-xs pr-16"
              />
              <div className="absolute right-1 top-1 flex gap-1">
                <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setShowToken(!showToken)}>
                  {showToken ? <EyeOff className="h-3 w-3" /> : <Eye className="h-3 w-3" />}
                </Button>
                <Button variant="ghost" size="icon" className="h-7 w-7" onClick={copyToken}>
                  <Copy className="h-3 w-3" />
                </Button>
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              At the site: <code className="font-mono">FCP_LINK_ENROLLMENT_TOKEN</code> in fcp-link's environment, or
              <code className="font-mono"> [link] enrollment_token</code> in its configuration (docs/SITE_SETUP.md).
            </p>
          </div>
        </DialogContent>
      </Dialog>

      {/* Detail */}
      <Dialog open={!!detail} onOpenChange={() => setDetail(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{detail?.name}</DialogTitle>
            <DialogDescription className="font-mono text-xs">{detail?.id}</DialogDescription>
          </DialogHeader>
          {detail && (
            <div className="space-y-4 text-sm">
              <div className="grid grid-cols-2 gap-x-6 gap-y-2">
                <div><span className="text-muted-foreground">Status</span><div>{detail.status}</div></div>
                <div><span className="text-muted-foreground">Link</span><div>{detail.link_up ? 'up' : 'down'}, last seen {ago(detail.last_seen_at)}</div></div>
                <div><span className="text-muted-foreground">Enrolled</span><div>{detail.enrolled_at ? new Date(detail.enrolled_at).toLocaleString() : 'not yet'}</div></div>
                <div><span className="text-muted-foreground">Certificate ends</span><div>{detail.certificate_expires_at ? new Date(detail.certificate_expires_at).toLocaleDateString() : '—'}</div></div>
                <div><span className="text-muted-foreground">Link release</span><div className="font-mono text-xs">{detail.link_version ?? '—'}</div></div>
                <div><span className="text-muted-foreground">Link trunk</span><div className="font-mono text-xs">{detail.trunk_id ?? 'none'}</div></div>
                <div>
                  <span className="text-muted-foreground">Sync</span>
                  <div>
                    {detail.sync_cursor === undefined ? 'no site database' : (
                      <>
                        cursor {detail.sync_cursor}
                        {detail.sync_behind ? `, ${detail.sync_behind} behind` : ', current'}
                        {detail.sync_applied_at ? `, applied ${ago(detail.sync_applied_at)}` : ''}
                      </>
                    )}
                    {detail.sync_error && <div className="text-destructive">{detail.sync_error}</div>}
                  </div>
                </div>
                <div>
                  <span className="text-muted-foreground">Registered</span>
                  <div>
                    {detail.registrations
                      ? `${detail.registrations.total_users} users, ${detail.registrations.total_contacts} contacts`
                      : '—'}
                  </div>
                </div>
              </div>
              <div>
                <h3 className="font-medium mb-1">Nodes</h3>
                {detail.nodes.length === 0 ? (
                  <p className="text-muted-foreground">None seen yet.</p>
                ) : (
                  <table className="w-full text-xs">
                    <thead><tr className="text-left text-muted-foreground"><th className="py-1">Node</th><th>Type</th><th>Release</th><th>Health</th></tr></thead>
                    <tbody>
                      {detail.nodes.map((n) => (
                        <tr key={n.node_id} className="border-t">
                          <td className="py-1 font-mono">{n.node_id}</td>
                          <td>{n.node_type}</td>
                          <td className="font-mono">{n.version}</td>
                          <td>{n.healthy ? 'healthy' : <span className="text-yellow-600">unhealthy</span>}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>
              {detail.registrations && detail.registrations.users.length > 0 && (
                <div>
                  <h3 className="font-medium mb-1">Registered at the site</h3>
                  <table className="w-full text-xs">
                    <thead><tr className="text-left text-muted-foreground"><th className="py-1">Address of record</th><th>Contacts</th><th>Since</th></tr></thead>
                    <tbody>
                      {detail.registrations.users.slice(0, 50).map((u) => (
                        <tr key={u.aor} className="border-t">
                          <td className="py-1 font-mono">{u.aor}</td>
                          <td>{u.contacts}</td>
                          <td>{ago(u.since)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {detail.registrations.users.length > 50 && (
                    <p className="text-muted-foreground mt-1">and {detail.registrations.users.length - 50} more</p>
                  )}
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* Delete */}
      <Dialog open={!!deleting} onOpenChange={() => setDeleting(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Delete {deleting?.name}?</DialogTitle>
            <DialogDescription>
              The site's link is refused from then on and its link trunk goes with it. The site's own
              stack keeps running on what it has; nothing reaches it from here again.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleting(null)}>Cancel</Button>
            <Button variant="destructive" disabled={deleteMutation.isPending} onClick={() => deleting && deleteMutation.mutate(deleting.id)}>
              {deleteMutation.isPending ? 'Deleting...' : 'Delete Site'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
