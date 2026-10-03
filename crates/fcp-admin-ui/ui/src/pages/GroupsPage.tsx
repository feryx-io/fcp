import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Plus, Trash2, Pencil, UsersRound, UserMinus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { groups } from '@/api/endpoints'
import type { Group, GroupRequest } from '@/types'

// A tenant's groups (docs/USERS.md §5): a role their members hold besides
// their own, the identity provider's group they follow, and who is in them.

/** The roles a group may give: not a platform admin's, nor a department admin's. */
const ROLES: { value: string; label: string }[] = [
  { value: '', label: 'No role' },
  { value: 'tenant_admin', label: 'Tenant admin' },
  { value: 'telecom_admin', label: 'Telecom admin' },
  { value: 'user_admin', label: 'User admin' },
  { value: 'read_only', label: 'Read only' },
  { value: 'end_user', label: 'End user' },
]

const roleLabel = (role?: string | null) =>
  ROLES.find((r) => r.value === (role ?? ''))?.label ?? role ?? ''

const emptyRequest = (): GroupRequest => ({ name: '', description: null, role: null, idp_group: null })

export function GroupsPage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<{ id: string | null; req: GroupRequest } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [membersOf, setMembersOf] = useState<Group | null>(null)
  const { data: list = [], isLoading } = useQuery({ queryKey: ['groups'], queryFn: groups.list })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['groups'] })

  const save = useMutation({
    mutationFn: (e: { id: string | null; req: GroupRequest }) =>
      e.id ? groups.update(e.id, e.req) : groups.create(e.req),
    onSuccess: () => {
      setEditing(null)
      setError(null)
      refresh()
    },
    onError: (err) => setError(err instanceof Error ? err.message : 'Failed to save'),
  })
  const remove = useMutation({
    mutationFn: groups.delete,
    onSuccess: refresh,
    onError: (err) => window.alert(err instanceof Error ? err.message : 'Failed to delete'),
  })

  const req = editing?.req
  const update = (patch: Partial<GroupRequest>) =>
    setEditing((prev) => (prev ? { ...prev, req: { ...prev.req, ...patch } } : prev))

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Groups</h1>
          <p className="text-muted-foreground">
            A group's role is its members' too, besides their own: someone in two groups may do what either
            allows. A group that follows an identity provider's group is filled at every sign-in through it. A
            queue may take its agents from a group.
          </p>
        </div>
        <Button onClick={() => { setError(null); setEditing({ id: null, req: emptyRequest() }) }}>
          <Plus className="mr-2 h-4 w-4" /> New Group
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Role</TableHead>
                <TableHead>Follows</TableHead>
                <TableHead>Members</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading && (
                <TableRow><TableCell colSpan={5} className="text-muted-foreground">Loading…</TableCell></TableRow>
              )}
              {!isLoading && list.length === 0 && (
                <TableRow>
                  <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                    <UsersRound className="mx-auto mb-2 h-6 w-6" />
                    No groups yet.
                  </TableCell>
                </TableRow>
              )}
              {list.map((g) => (
                <TableRow key={g.id}>
                  <TableCell className="font-medium">
                    {g.name}
                    {g.description && <div className="text-xs text-muted-foreground">{g.description}</div>}
                  </TableCell>
                  <TableCell>{g.role ? <Badge variant="secondary">{roleLabel(g.role)}</Badge> : '-'}</TableCell>
                  <TableCell className="text-sm">{g.idp_group ?? '-'}</TableCell>
                  <TableCell>
                    <Button variant="link" className="p-0" onClick={() => setMembersOf(g)}>
                      {g.member_count}
                    </Button>
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setError(null)
                        setEditing({
                          id: g.id,
                          req: { name: g.name, description: g.description, role: g.role, idp_group: g.idp_group },
                        })
                      }}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        if (window.confirm(`Delete the group "${g.name}"? Its members lose its role.`)) remove.mutate(g.id)
                      }}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Dialog open={!!editing} onOpenChange={(open) => !open && setEditing(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editing?.id ? 'Edit Group' : 'New Group'}</DialogTitle>
            <DialogDescription>Only a tenant administrator makes groups and fills them.</DialogDescription>
          </DialogHeader>
          {req && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="g-name">Name</Label>
                <Input id="g-name" value={req.name} onChange={(e) => update({ name: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="g-description">Description</Label>
                <Input
                  id="g-description"
                  value={req.description ?? ''}
                  onChange={(e) => update({ description: e.target.value || null })}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="g-role">Role its members hold</Label>
                <select
                  id="g-role"
                  className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={req.role ?? ''}
                  onChange={(e) => update({ role: e.target.value || null })}
                >
                  {ROLES.map((r) => <option key={r.value} value={r.value}>{r.label}</option>)}
                </select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="g-idp">Follows the identity provider's group</Label>
                <Input
                  id="g-idp"
                  placeholder="e.g. helpdesk (as the provider names it)"
                  value={req.idp_group ?? ''}
                  onChange={(e) => update({ idp_group: e.target.value || null })}
                />
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button disabled={save.isPending || !req?.name.trim()} onClick={() => editing && save.mutate(editing)}>
              {save.isPending ? 'Saving…' : 'Save'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {membersOf && <MembersDialog group={membersOf} onClose={() => { setMembersOf(null); refresh() }} />}
    </div>
  )
}

function MembersDialog({ group, onClose }: { group: Group; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [username, setUsername] = useState('')
  const [error, setError] = useState<string | null>(null)
  const key = ['groups', group.id, 'members']
  const { data: members = [], isLoading } = useQuery({ queryKey: key, queryFn: () => groups.members(group.id) })
  const changed = () => queryClient.invalidateQueries({ queryKey: key })
  const add = useMutation({
    mutationFn: (name: string) => groups.addMember(group.id, name),
    onSuccess: () => { setUsername(''); setError(null); changed() },
    onError: (err) => setError(err instanceof Error ? err.message : 'Failed to add'),
  })
  const drop = useMutation({
    mutationFn: (name: string) => groups.removeMember(group.id, name),
    onSuccess: changed,
  })

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{group.name}: members</DialogTitle>
          <DialogDescription>
            {group.idp_group
              ? `Set at every sign-in through the identity provider from its "${group.idp_group}" group.`
              : 'Added and removed here.'}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <form
            className="flex gap-2"
            onSubmit={(e) => { e.preventDefault(); if (username.trim()) add.mutate(username.trim()) }}
          >
            <Input placeholder="username" value={username} onChange={(e) => setUsername(e.target.value)} />
            <Button type="submit" disabled={add.isPending || !username.trim()}>Add</Button>
          </form>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Table>
            <TableBody>
              {isLoading && <TableRow><TableCell className="text-muted-foreground">Loading…</TableCell></TableRow>}
              {!isLoading && members.length === 0 && (
                <TableRow><TableCell className="text-muted-foreground">Nobody yet.</TableCell></TableRow>
              )}
              {members.map((m) => (
                <TableRow key={m.user_id}>
                  <TableCell>
                    {m.username}
                    {m.display_name && <span className="ml-2 text-muted-foreground">{m.display_name}</span>}
                    {m.account_status !== 'active' && <Badge variant="outline" className="ml-2">{m.account_status}</Badge>}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="sm" onClick={() => drop.mutate(m.username)}>
                      <UserMinus className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
