import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Plus, Trash2, Pencil, ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { classesOfService } from '@/api/endpoints'
import type { ClassOfServiceRequest, ClassifyResponse, DestinationClass } from '@/types'

// A tenant's classes of service: what its users may dial, how many calls
// they may be on, how their caller ID leaves (docs/ENTERPRISE_CALLING.md §3.7).

export const DESTINATION_LABELS: Record<DestinationClass, string> = {
  internal: 'Internal',
  emergency: 'Emergency',
  toll_free: 'Toll-free',
  national: 'National',
  international: 'International',
  premium: 'Premium',
  unknown: 'Unknown',
}

const CHOOSABLE: DestinationClass[] = ['internal', 'toll_free', 'national', 'international', 'premium', 'unknown']

const emptyRequest = (): ClassOfServiceRequest => ({
  name: '',
  allowed: ['internal', 'emergency', 'toll_free', 'national'],
  max_calls: 2,
  caller_id: 'line',
  is_default: false,
})

export function ClassesOfServicePage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<{ id: string | null; req: ClassOfServiceRequest } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [dialed, setDialed] = useState('')
  const [classified, setClassified] = useState<ClassifyResponse | null>(null)
  const { data: list = [], isLoading } = useQuery({
    queryKey: ['classes-of-service'],
    queryFn: classesOfService.list,
  })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['classes-of-service'] })

  const save = useMutation({
    mutationFn: (e: { id: string | null; req: ClassOfServiceRequest }) =>
      e.id ? classesOfService.update(e.id, e.req) : classesOfService.create(e.req),
    onSuccess: () => {
      setEditing(null)
      setError(null)
      refresh()
    },
    onError: (err) => setError(err instanceof Error ? err.message : 'Failed to save'),
  })
  const remove = useMutation({
    mutationFn: classesOfService.delete,
    onSuccess: refresh,
    onError: (err) => window.alert(err instanceof Error ? err.message : 'Failed to delete'),
  })
  const classify = useMutation({
    mutationFn: classesOfService.classify,
    onSuccess: setClassified,
  })

  const req = editing?.req
  const update = (patch: Partial<ClassOfServiceRequest>) =>
    setEditing((prev) => (prev ? { ...prev, req: { ...prev.req, ...patch } } : prev))
  const toggle = (c: DestinationClass, on: boolean) =>
    update({ allowed: on ? [...req!.allowed.filter((x) => x !== c), c] : req!.allowed.filter((x) => x !== c) })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Classes of Service</h1>
          <p className="text-muted-foreground">
            What users may dial, how many calls they may be on, and whether their caller ID is withheld. A user
            without a class has the default one; with no default class, anything goes. Emergency calls are always
            allowed.
          </p>
        </div>
        <Button onClick={() => { setError(null); setEditing({ id: null, req: emptyRequest() }) }}>
          <Plus className="mr-2 h-4 w-4" /> New Class
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>May dial</TableHead>
                <TableHead>Calls at once</TableHead>
                <TableHead>Caller ID</TableHead>
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
                    <ShieldCheck className="mx-auto mb-2 h-6 w-6" />
                    No classes yet: every user may dial anything.
                  </TableCell>
                </TableRow>
              )}
              {list.map((c) => (
                <TableRow key={c.id}>
                  <TableCell className="font-medium">
                    {c.name} {c.is_default && <Badge variant="secondary" className="ml-2">Default</Badge>}
                  </TableCell>
                  <TableCell className="text-sm">
                    {c.allowed.filter((a) => a !== 'emergency').map((a) => DESTINATION_LABELS[a]).join(', ') || 'Emergency only'}
                  </TableCell>
                  <TableCell className="text-sm">{c.max_calls ?? 'No limit'}</TableCell>
                  <TableCell className="text-sm">{c.caller_id === 'withheld' ? 'Withheld' : 'Line'}</TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setError(null)
                        setEditing({
                          id: c.id,
                          req: {
                            name: c.name,
                            allowed: c.allowed,
                            max_calls: c.max_calls,
                            caller_id: c.caller_id,
                            is_default: c.is_default,
                          },
                        })
                      }}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => remove.mutate(c.id)}>
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">What is a number?</CardTitle>
          <CardDescription>
            Classified by the tenant&rsquo;s country and its overrides, set on the tenant.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-2">
          <Input
            className="w-64"
            placeholder="e.g. 18005550100"
            value={dialed}
            onChange={(e) => setDialed(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && dialed.trim()) classify.mutate(dialed.trim()) }}
          />
          <Button variant="outline" disabled={!dialed.trim()} onClick={() => classify.mutate(dialed.trim())}>
            Classify
          </Button>
          {classified && (
            <span className="text-sm">
              <Badge>{DESTINATION_LABELS[classified.class]}</Badge>
              {classified.e164 && <span className="ml-2 font-mono text-muted-foreground">{classified.e164}</span>}
            </span>
          )}
        </CardContent>
      </Card>

      <Dialog open={!!editing} onOpenChange={(v) => { if (!v) setEditing(null) }}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing?.id ? 'Edit Class of Service' : 'New Class of Service'}</DialogTitle>
            <DialogDescription>Emergency calls are always allowed.</DialogDescription>
          </DialogHeader>
          {req && (
            <div className="space-y-4">
              <div className="space-y-1">
                <Label>Name</Label>
                <Input value={req.name} onChange={(e) => update({ name: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label>May dial</Label>
                <div className="grid grid-cols-2 gap-2">
                  {CHOOSABLE.map((c) => (
                    <label key={c} className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={req.allowed.includes(c)}
                        onChange={(e) => toggle(c, e.target.checked)}
                      />
                      {DESTINATION_LABELS[c]}
                    </label>
                  ))}
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1">
                  <Label>Calls at once</Label>
                  <Input
                    type="number"
                    min="1"
                    placeholder="No limit"
                    value={req.max_calls ?? ''}
                    onChange={(e) => update({ max_calls: e.target.value ? Number(e.target.value) : null })}
                  />
                </div>
                <div className="space-y-1">
                  <Label>Caller ID</Label>
                  <select
                    className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={req.caller_id}
                    onChange={(e) => update({ caller_id: e.target.value as 'line' | 'withheld' })}
                  >
                    <option value="line">The user&rsquo;s line</option>
                    <option value="withheld">Withheld</option>
                  </select>
                </div>
              </div>
              <div className="flex items-center justify-between rounded-md border p-3">
                <div className="pr-4">
                  <Label>Default class</Label>
                  <p className="text-xs text-muted-foreground">The class of every user who has none of their own.</p>
                </div>
                <Switch checked={req.is_default} onCheckedChange={(v) => update({ is_default: v })} />
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button onClick={() => editing && save.mutate(editing)} disabled={save.isPending || !req?.name.trim()}>
              {save.isPending ? 'Saving…' : 'Save'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
