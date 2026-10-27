import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Pencil, Plus, Search, Trash2, X } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { me } from '@/api/endpoints'
import type { PersonalContact, PersonalContactRequest } from '@/types'

/** A contact being made (no id) or changed. */
interface Editing {
  id?: string
  req: PersonalContactRequest
}

const blank = (): PersonalContactRequest => ({
  name: '',
  company: null,
  numbers: [{ label: '', number: '' }],
  email: null,
  notes: null,
})

const asRequest = (c: PersonalContact): PersonalContactRequest => ({
  name: c.name,
  company: c.company ?? null,
  numbers: c.numbers.length ? c.numbers.map((n) => ({ ...n })) : [{ label: '', number: '' }],
  email: c.email ?? null,
  notes: c.notes ?? null,
})

const matches = (c: PersonalContact, q: string) =>
  !q ||
  [c.name, c.company ?? '', c.email ?? '', ...c.numbers.map((n) => n.number)].some((v) =>
    v.toLowerCase().includes(q),
  )

/**
 * Your contacts (docs/RECORD_AND_CONTACTS.md §3): the people you keep, on
 * your account, so the app has them wherever you sign in and your site's
 * phones do too. The app names a caller from them.
 */
export function UserContactsPage() {
  const queryClient = useQueryClient()
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<Editing | null>(null)
  const [error, setError] = useState<string | null>(null)
  const { data: contacts = [], isLoading } = useQuery({
    queryKey: ['me', 'contacts'],
    queryFn: me.listContacts,
  })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['me', 'contacts'] })
  const fail = (e: unknown) => setError(e instanceof Error ? e.message : 'Request failed')

  const save = useMutation({
    mutationFn: ({ id, req }: Editing) => {
      const body: PersonalContactRequest = {
        ...req,
        numbers: (req.numbers ?? []).filter((n) => n.number.trim()),
      }
      return id ? me.updateContact(id, body) : me.createContact(body)
    },
    onSuccess: () => {
      setEditing(null)
      setError(null)
      refresh()
    },
    onError: fail,
  })
  const remove = useMutation({ mutationFn: me.deleteContact, onSuccess: refresh, onError: fail })

  const req = editing?.req
  const update = (change: Partial<PersonalContactRequest>) =>
    setEditing((e) => (e ? { ...e, req: { ...e.req, ...change } } : e))
  const numbers = req?.numbers ?? []
  const setNumber = (i: number, change: { label?: string; number?: string }) =>
    update({ numbers: numbers.map((n, j) => (j === i ? { ...n, ...change } : n)) })

  const confirmRemove = (c: PersonalContact) => {
    if (window.confirm(`Delete ${c.name}? They go from every app you sign in to.`)) remove.mutate(c.id)
  }

  const q = query.trim().toLowerCase()
  const shown = contacts.filter((c) => matches(c, q))

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Contacts</h1>
          <p className="text-muted-foreground">
            The people you keep. The app has them wherever you sign in, and names them when they call.
          </p>
        </div>
        <Button onClick={() => { setError(null); setEditing({ req: blank() }) }}>
          <Plus className="mr-2 h-4 w-4" /> New contact
        </Button>
      </div>
      {error && !editing && <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">{error}</div>}
      <Card>
        <CardHeader>
          <CardTitle>Your contacts</CardTitle>
          <CardDescription>Contacts kept only on one device stay on that device and are not shown here.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="relative">
            <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
            <Input
              aria-label="Search contacts"
              placeholder="Search by name, company, email or number"
              className="pl-9"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
          {!isLoading && contacts.length === 0 && (
            <p className="text-sm text-muted-foreground">No contacts yet.</p>
          )}
          {!isLoading && contacts.length > 0 && shown.length === 0 && (
            <p className="text-sm text-muted-foreground">No contact matches.</p>
          )}
          {shown.map((c) => (
            <div key={c.id} className="flex items-center justify-between rounded-md border p-3">
              <div className="min-w-0">
                <div className="font-medium">{c.name}</div>
                <div className="text-sm text-muted-foreground">
                  {[
                    c.company,
                    ...c.numbers.map((n) => (n.label ? `${n.number} (${n.label})` : n.number)),
                    c.email,
                  ].filter(Boolean).join(' · ')}
                </div>
                {c.notes && <div className="truncate text-xs text-muted-foreground">{c.notes}</div>}
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Edit ${c.name}`}
                  onClick={() => { setError(null); setEditing({ id: c.id, req: asRequest(c) }) }}
                >
                  <Pencil className="h-4 w-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Delete ${c.name}`}
                  disabled={remove.isPending}
                  onClick={() => confirmRemove(c)}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <Dialog open={!!editing} onOpenChange={(open) => !open && setEditing(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editing?.id ? 'Edit contact' : 'New contact'}</DialogTitle>
            <DialogDescription>Kept on your account: every app you sign in to has it.</DialogDescription>
          </DialogHeader>
          {req && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="c-name">Name</Label>
                <Input id="c-name" value={req.name} onChange={(e) => update({ name: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-company">Company</Label>
                <Input
                  id="c-company"
                  value={req.company ?? ''}
                  onChange={(e) => update({ company: e.target.value || null })}
                />
              </div>
              <div className="space-y-2">
                <Label>Numbers</Label>
                {numbers.map((n, i) => (
                  <div key={i} className="flex gap-2">
                    <Input
                      aria-label={`Number ${i + 1}`}
                      placeholder="Number"
                      className="flex-[3]"
                      value={n.number}
                      onChange={(e) => setNumber(i, { number: e.target.value })}
                    />
                    <Input
                      aria-label={`Label ${i + 1}`}
                      placeholder="mobile, work…"
                      className="flex-[2]"
                      value={n.label ?? ''}
                      onChange={(e) => setNumber(i, { label: e.target.value })}
                    />
                    {numbers.length > 1 && (
                      <Button
                        variant="ghost"
                        size="icon"
                        type="button"
                        aria-label={`Remove number ${i + 1}`}
                        onClick={() => update({ numbers: numbers.filter((_, j) => j !== i) })}
                      >
                        <X className="h-4 w-4" />
                      </Button>
                    )}
                  </div>
                ))}
                <Button
                  variant="outline"
                  size="sm"
                  type="button"
                  onClick={() => update({ numbers: [...numbers, { label: '', number: '' }] })}
                >
                  <Plus className="mr-2 h-4 w-4" /> Add a number
                </Button>
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-email">Email</Label>
                <Input
                  id="c-email"
                  type="email"
                  value={req.email ?? ''}
                  onChange={(e) => update({ email: e.target.value || null })}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-notes">Notes</Label>
                <Input
                  id="c-notes"
                  value={req.notes ?? ''}
                  onChange={(e) => update({ notes: e.target.value || null })}
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
    </div>
  )
}
