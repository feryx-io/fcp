import { useMemo, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Plus, Trash2, RefreshCw, Send, History, RotateCcw, Pencil } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { TokenSecretDialog, rotationNote } from '@/components/TokenSecretDialog'
import { webhooks } from '@/api/endpoints'
import type { Webhook, WebhookDeliveryOutcome, WebhookEventType } from '@/types'

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Request failed'
}

function outcomeText(o: WebhookDeliveryOutcome) {
  if (o.delivered) return `Delivered (HTTP ${o.status_code})`
  return `Not delivered: ${o.error ?? 'no answer'}`
}

/**
 * The tenant's webhooks (docs/DEVELOPER_SURFACE.md §3, phase 5). The server
 * makes each secret and shows it once; every delivery carries
 * `X-FCP-Signature: t=<unix>,v1=<hex>`, verified with the SDKs'
 * `verifyWebhook`. A URL must be public HTTPS. Rotating keeps the old secret
 * signing beside the new one for a day.
 */
export function WebhooksPage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<Webhook | 'new' | null>(null)
  const [secret, setSecret] = useState<{ value: string; title: string; note?: string } | null>(null)
  const [history, setHistory] = useState<Webhook | null>(null)
  const [url, setUrl] = useState('')
  const [description, setDescription] = useState('')
  const [enabled, setEnabled] = useState(true)
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [formError, setFormError] = useState<string | null>(null)

  const { data: list = [], isLoading } = useQuery({ queryKey: ['webhooks'], queryFn: webhooks.list })
  const { data: eventTypes = [] } = useQuery({
    queryKey: ['webhooks', 'event-types'],
    queryFn: webhooks.eventTypes,
  })
  const categories = useMemo(() => {
    const by = new Map<string, WebhookEventType[]>()
    for (const t of eventTypes) by.set(t.category, [...(by.get(t.category) ?? []), t])
    return [...by.entries()]
  }, [eventTypes])
  const deliveries = useQuery({
    queryKey: ['webhooks', history?.id, 'deliveries'],
    queryFn: () => webhooks.deliveries(history!.id),
    enabled: !!history,
  })

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['webhooks'] })
  const saveMutation = useMutation({
    mutationFn: async () => {
      const events = [...chosen]
      if (editing === 'new') {
        const made = await webhooks.create({ url: url.trim(), events, enabled, description })
        setSecret({ value: made.secret, title: 'Webhook created' })
      } else if (editing) {
        await webhooks.update(editing.id, { url: url.trim(), events, enabled, description })
      }
    },
    onSuccess: () => {
      invalidate()
      setEditing(null)
    },
    onError: (e) => setFormError(errorText(e)),
  })
  const rotateMutation = useMutation({
    mutationFn: (id: string) => webhooks.rotate(id),
    onSuccess: (result) => {
      invalidate()
      setSecret({
        value: result.secret,
        title: 'Secret rotated',
        note: `Deliveries are signed with both secrets meanwhile. ${rotationNote(result.webhook.previous_secret_valid_until)}`,
      })
    },
    onError: (e) => alert(errorText(e)),
  })
  const testMutation = useMutation({
    mutationFn: (id: string) => webhooks.test(id),
    onSuccess: (o) => {
      invalidate()
      alert(outcomeText(o))
    },
    onError: (e) => alert(errorText(e)),
  })
  const replayMutation = useMutation({
    mutationFn: ({ id, delivery }: { id: string; delivery: string }) => webhooks.replay(id, delivery),
    onSuccess: (o) => {
      deliveries.refetch()
      alert(outcomeText(o))
    },
    onError: (e) => alert(errorText(e)),
  })
  const deleteMutation = useMutation({ mutationFn: webhooks.delete, onSuccess: invalidate })

  const open = (w: Webhook | 'new') => {
    setUrl(w === 'new' ? '' : w.url)
    setDescription(w === 'new' ? '' : w.description ?? '')
    setEnabled(w === 'new' ? true : w.enabled)
    setChosen(new Set(w === 'new' ? [] : w.events))
    setFormError(null)
    setEditing(w)
  }
  const toggle = (name: string) => {
    const next = new Set(chosen)
    if (next.has(name)) next.delete(name)
    else next.add(name)
    setChosen(next)
  }
  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!url.trim()) return setFormError('A URL is required')
    saveMutation.mutate()
  }

  const columns: ColumnDef<Webhook>[] = [
    {
      accessorKey: 'url',
      header: 'URL',
      cell: ({ row }) => (
        <div>
          <div className="font-mono text-sm">{row.original.url}</div>
          {row.original.description && (
            <div className="text-xs text-muted-foreground">{row.original.description}</div>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'events',
      header: 'Events',
      cell: ({ row }) => {
        const events = row.original.events
        if (events.length === 0) return <Badge variant="secondary">Every event</Badge>
        return (
          <div className="flex gap-1 flex-wrap" title={events.join(', ')}>
            {events.slice(0, 3).map((e) => <Badge key={e} variant="outline" className="text-xs">{e}</Badge>)}
            {events.length > 3 && <Badge variant="outline" className="text-xs">+{events.length - 3}</Badge>}
          </div>
        )
      },
    },
    {
      id: 'status',
      header: 'Status',
      cell: ({ row }) => (
        <div className="flex gap-1 flex-wrap">
          {row.original.enabled
            ? <Badge variant="outline">Enabled</Badge>
            : <Badge variant="secondary">Disabled</Badge>}
          <span className="font-mono text-xs bg-muted rounded px-1.5 py-0.5">{row.original.secret_prefix}…</span>
          {row.original.previous_secret_valid_until && (
            <Badge variant="outline" title="The secret a rotation replaced also signs until then">
              Old secret until {new Date(row.original.previous_secret_valid_until).toLocaleString()}
            </Badge>
          )}
        </div>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => {
        const w = row.original
        return (
          <div className="flex justify-end gap-1">
            <Button variant="ghost" size="icon" title="Send a test delivery"
              disabled={!w.enabled || testMutation.isPending} onClick={() => testMutation.mutate(w.id)}>
              <Send className="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="icon" title="Deliveries" onClick={() => setHistory(w)}>
              <History className="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="icon" title="Edit" onClick={() => open(w)}>
              <Pencil className="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="icon" title="Rotate the secret: the old one signs beside it for a day"
              onClick={() => {
                if (confirm('Rotate the secret? Deliveries carry both signatures for 24 hours.')) {
                  rotateMutation.mutate(w.id)
                }
              }}>
              <RefreshCw className="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="icon" className="text-destructive hover:text-destructive" title="Delete"
              onClick={() => {
                if (confirm(`Delete the webhook to ${w.url}, with its deliveries?`)) deleteMutation.mutate(w.id)
              }}>
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
          <h1 className="text-3xl font-bold tracking-tight">Webhooks</h1>
          <p className="text-muted-foreground">
            Your events, POSTed to your HTTPS endpoint and signed with its secret: verify{' '}
            <code className="text-xs">X-FCP-Signature</code> with the SDK's <code className="text-xs">verifyWebhook</code>.
          </p>
        </div>
        <Button onClick={() => open('new')}>
          <Plus className="mr-2 h-4 w-4" />
          Add Webhook
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={list} searchKey="url" searchPlaceholder="Search webhooks..." />
      )}

      <Dialog open={!!editing} onOpenChange={(o) => !o && setEditing(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{editing === 'new' ? 'Add Webhook' : 'Edit Webhook'}</DialogTitle>
            <DialogDescription>
              A public HTTPS URL. Choose events, or none for every event.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={submit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="webhook-url">URL *</Label>
              <Input id="webhook-url" placeholder="https://example.com/fcp/events" value={url}
                onChange={(e) => setUrl(e.target.value)} />
            </div>
            <div className="space-y-2">
              <Label htmlFor="webhook-description">Description</Label>
              <Input id="webhook-description" placeholder="e.g., CRM" value={description} maxLength={256}
                onChange={(e) => setDescription(e.target.value)} />
            </div>
            <label className="flex items-center gap-2 text-sm">
              <Switch checked={enabled} onCheckedChange={setEnabled} /> Enabled
            </label>
            <div className="space-y-2">
              <Label>Events ({chosen.size === 0 ? 'every event' : chosen.size})</Label>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 rounded-md border p-3 max-h-80 overflow-y-auto">
                {categories.map(([category, types]) => (
                  <div key={category} className="space-y-1">
                    <label className="flex items-center gap-2 text-sm font-medium">
                      <input type="checkbox" className="rounded border-input"
                        checked={chosen.has(`${category}.*`)} onChange={() => toggle(`${category}.*`)} />
                      {category}.* <span className="text-muted-foreground font-normal">(all {types.length})</span>
                    </label>
                    {!chosen.has(`${category}.*`) && types.map((t) => (
                      <label key={t.name} className="flex items-center gap-2 pl-5 text-sm">
                        <input type="checkbox" className="rounded border-input"
                          checked={chosen.has(t.name)} onChange={() => toggle(t.name)} />
                        <span className="font-mono text-xs">{t.name}</span>
                      </label>
                    ))}
                  </div>
                ))}
              </div>
            </div>
            {formError && <p className="text-sm text-destructive">{formError}</p>}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
              <Button type="submit" disabled={saveMutation.isPending}>
                {saveMutation.isPending ? 'Saving...' : 'Save'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!history} onOpenChange={(o) => !o && setHistory(null)}>
        <DialogContent className="max-w-3xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Deliveries</DialogTitle>
            <DialogDescription className="font-mono text-xs">{history?.url}</DialogDescription>
          </DialogHeader>
          {deliveries.isLoading ? (
            <div className="text-muted-foreground text-sm">Loading...</div>
          ) : (deliveries.data?.data.length ?? 0) === 0 ? (
            <div className="text-muted-foreground text-sm">No deliveries yet.</div>
          ) : (
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-muted-foreground">
                  <th className="py-1">When</th><th>Event</th><th>Result</th><th>Attempts</th><th />
                </tr>
              </thead>
              <tbody>
                {deliveries.data!.data.map((d) => (
                  <tr key={d.id} className="border-t">
                    <td className="py-1">{new Date(d.created_at).toLocaleString()}</td>
                    <td className="font-mono text-xs">{d.event_type}</td>
                    <td>
                      {d.delivered
                        ? <Badge variant="outline">HTTP {d.status_code}</Badge>
                        : <Badge variant="destructive" title={d.error ?? ''}>
                            {d.status_code ? `HTTP ${d.status_code}` : 'Failed'}
                          </Badge>}
                      {d.next_retry_at && (
                        <span className="ml-1 text-xs text-muted-foreground">
                          retry {new Date(d.next_retry_at).toLocaleTimeString()}
                        </span>
                      )}
                    </td>
                    <td>{d.attempts}</td>
                    <td className="text-right">
                      <Button variant="ghost" size="icon" title="Send again now, signed now"
                        disabled={replayMutation.isPending}
                        onClick={() => replayMutation.mutate({ id: history!.id, delivery: d.id })}>
                        <RotateCcw className="h-4 w-4" />
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </DialogContent>
      </Dialog>

      <TokenSecretDialog
        secret={secret?.value ?? null}
        title={secret?.title ?? ''}
        note={secret?.note ?? 'Verify each delivery\'s X-FCP-Signature with it.'}
        onClose={() => setSecret(null)}
      />
    </div>
  )
}
