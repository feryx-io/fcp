import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import {
  AlertTriangle, CheckCircle2, Plus, Trash2,
} from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { AddContactButton, EmergencyContacts, contactsToSave, newContact } from '@/components/EmergencyContacts'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { emergency, locations } from '@/api/endpoints'
import type {
  ContactReadiness, EmergencyCall, EmergencyConfig, EmergencyNetwork,
  EmergencyNotification, UserReadiness,
} from '@/types'

type Tab = 'config' | 'readiness' | 'networks' | 'calls'

const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'config', label: 'Configuration' },
  { id: 'readiness', label: 'Readiness' },
  { id: 'networks', label: 'Networks' },
  { id: 'calls', label: 'Calls' },
]

/** Device types a tenant can trust to send their own location. */
const DEVICE_TYPES = ['desk_phone', 'softphone', 'mobile', 'webrtc', 'ata', 'conference_phone']

const when = (at: string | null | undefined) =>
  at ? new Date(at).toLocaleString() : '-'

const ago = (at: string | null | undefined) => {
  if (!at) return 'never'
  const days = Math.floor((Date.now() - Date.parse(at)) / 86_400_000)
  if (days > 1) return `${days} days ago`
  if (days === 1) return 'yesterday'
  return new Date(at).toLocaleTimeString()
}

const outcomeBadge = (outcome: string) => {
  if (outcome === 'answered') return 'success' as const
  if (outcome === 'placed') return 'secondary' as const
  return 'destructive' as const
}

const statusBadge = (status: string) => {
  if (status === 'delivered') return 'success' as const
  if (status === 'retrying') return 'secondary' as const
  return 'destructive' as const
}

/** A list of things that are wrong, as the readiness report words them. */
function Issues({ issues }: { issues: string[] }) {
  if (issues.length === 0) {
    return (
      <span className="inline-flex items-center gap-1 text-sm text-muted-foreground">
        <CheckCircle2 className="h-4 w-4 text-green-600" /> Ready
      </span>
    )
  }
  return (
    <ul className="space-y-1">
      {issues.map((issue) => (
        <li key={issue} className="flex items-start gap-2 text-sm">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-yellow-600" />
          <span>{issue}</span>
        </li>
      ))}
    </ul>
  )
}

function Tile({ label, value, tone }: { label: string; value: number; tone?: 'good' | 'bad' }) {
  const color =
    tone === 'bad' && value > 0
      ? 'text-destructive'
      : tone === 'good'
        ? 'text-green-600'
        : ''
  return (
    <div className="rounded-lg border bg-card p-4">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={`text-2xl font-bold ${color}`}>{value}</p>
    </div>
  )
}

/** The tenant's emergency numbers, what it trusts, and who it tells. */
function ConfigurationTab() {
  const queryClient = useQueryClient()
  const { data: config, isLoading } = useQuery({
    queryKey: ['emergency-config'],
    queryFn: emergency.config,
  })
  const [draft, setDraft] = useState<EmergencyConfig | null>(null)
  const [error, setError] = useState<string | null>(null)
  const current = draft ?? config ?? null

  const save = useMutation({
    mutationFn: emergency.updateConfig,
    onSuccess: (saved) => {
      queryClient.setQueryData(['emergency-config'], saved)
      queryClient.invalidateQueries({ queryKey: ['emergency-readiness'] })
      setDraft(null)
      setError(null)
    },
    onError: (e: Error) => setError(e.message),
  })

  if (isLoading || !current) {
    return <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
  }

  const edit = (change: Partial<EmergencyConfig>) => setDraft({ ...current, ...change })
  const list = (value: string) =>
    value.split(',').map((v) => v.trim()).filter((v) => v.length > 0)

  return (
    <div className="space-y-6">
      {!current.configured && (
        <div className="rounded-lg border border-yellow-500/50 bg-yellow-50 p-4 text-sm dark:bg-yellow-950/30">
          This tenant has never saved an emergency configuration; the defaults below are in force.
        </div>
      )}

      <div className="rounded-lg border bg-card p-4 space-y-4">
        <h2 className="font-semibold">Numbers</h2>
        <div className="grid gap-4 md:grid-cols-3">
          <div className="space-y-1">
            <Label className="text-xs">Emergency numbers</Label>
            <Input
              value={current.emergency_numbers.join(', ')}
              onChange={(e) => edit({ emergency_numbers: list(e.target.value) })}
            />
            <p className="text-xs text-muted-foreground">Dialled strings that reach emergency services.</p>
          </div>
          <div className="space-y-1">
            <Label className="text-xs">Test numbers</Label>
            <Input
              value={current.test_numbers.join(', ')}
              onChange={(e) => edit({ test_numbers: list(e.target.value) })}
            />
            <p className="text-xs text-muted-foreground">
              A call to one of these proves the chain without reaching a PSAP.
            </p>
          </div>
          <div className="space-y-1">
            <Label className="text-xs">Dial prefixes</Label>
            <Input
              value={current.dial_prefixes.join(', ')}
              onChange={(e) => edit({ dial_prefixes: list(e.target.value) })}
            />
            <p className="text-xs text-muted-foreground">Stripped before matching, so 9911 is 911.</p>
          </div>
        </div>
        <div className="space-y-1 md:w-1/2">
          <Label htmlFor="test-call-mode" className="text-xs">A test call is answered by</Label>
          <Select
            value={current.test_call_mode}
            onValueChange={(v) => edit({ test_call_mode: v })}
          >
            <SelectTrigger id="test-call-mode" aria-label="A test call is answered by">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="local">This platform: read back what a real call would carry</SelectItem>
              <SelectItem value="carrier">The carrier: its test service reads back what it holds</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Read back here, a test reaches no carrier: the caller hears their location and where it
            came from, their callback number and the trunks a real call would take. Sent to the
            carrier, it proves the carrier's records, on carriers that offer a test service.
          </p>
        </div>
      </div>

      <div className="rounded-lg border bg-card p-4 space-y-4">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="font-semibold">Who is told</h2>
            <p className="text-sm text-muted-foreground">
              Kari's Law requires on-site notification. Each contact is told when an emergency call
              is placed, and every attempt is recorded against the call. A site's own contacts
              (Locations) are told first; a call contact may be allowed to listen in.
            </p>
          </div>
          <AddContactButton onAdd={() => edit({ contacts: [...current.contacts, newContact()] })} />
        </div>

        <EmergencyContacts
          idPrefix="tenant"
          contacts={current.contacts}
          onChange={(contacts) => edit({ contacts })}
          empty={
            <p className="text-sm text-destructive">
              Nobody would be told about an emergency call from this tenant, unless its sites name
              their own contacts.
            </p>
          }
        />
      </div>

      <div className="rounded-lg border bg-card p-4 space-y-4">
        <h2 className="font-semibold">Location</h2>
        <div className="flex items-center justify-between gap-4">
          <div>
            <Label htmlFor="require-dispatchable-location">Flag calls with no dispatchable location</Label>
            <p className="text-sm text-muted-foreground">
              The call is always placed; with this on, the record and the event say the location was
              missing.
            </p>
          </div>
          <Switch
            id="require-dispatchable-location"
            checked={current.require_dispatchable_location}
            onCheckedChange={(v) => edit({ require_dispatchable_location: v })}
          />
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-1">
            <Label className="text-xs">Device types trusted to send their own location</Label>
            <div className="flex flex-wrap gap-2">
              {DEVICE_TYPES.map((type) => {
                const on = current.trust_device_locations.includes(type)
                return (
                  <Button
                    key={type}
                    type="button"
                    size="sm"
                    variant={on ? 'default' : 'outline'}
                    onClick={() =>
                      edit({
                        trust_device_locations: on
                          ? current.trust_device_locations.filter((t) => t !== type)
                          : [...current.trust_device_locations, type],
                      })
                    }
                  >
                    {type.replace(/_/g, ' ')}
                  </Button>
                )
              })}
            </div>
          </div>
          <div className="space-y-1">
            <Label className="text-xs">A declared location stands for (hours)</Label>
            <Input
              type="number"
              min={1}
              max={720}
              value={Math.round(current.declared_location_ttl_secs / 3600)}
              onChange={(e) =>
                edit({ declared_location_ttl_secs: Math.max(1, Number(e.target.value)) * 3600 })
              }
            />
            <p className="text-xs text-muted-foreground">
              What a user says at /v1/me/location is used for this long.
            </p>
          </div>
        </div>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="flex items-center gap-2">
        <Button
          disabled={!draft || save.isPending}
          onClick={() =>
            save.mutate({
              emergency_numbers: current.emergency_numbers,
              test_numbers: current.test_numbers,
              test_call_mode: current.test_call_mode,
              dial_prefixes: current.dial_prefixes,
              contacts: contactsToSave(current.contacts),
              require_dispatchable_location: current.require_dispatchable_location,
              trust_device_locations: current.trust_device_locations,
              declared_location_ttl_secs: current.declared_location_ttl_secs,
            })
          }
        >
          {save.isPending ? 'Saving...' : 'Save'}
        </Button>
        {draft && (
          <Button variant="outline" onClick={() => { setDraft(null); setError(null) }}>
            Discard
          </Button>
        )}
      </div>
    </div>
  )
}

/** Who has a dispatchable location today, and whether anyone would be told. */
function ReadinessTab() {
  const { data, isLoading } = useQuery({
    queryKey: ['emergency-readiness'],
    queryFn: emergency.readiness,
  })

  const userColumns: ColumnDef<UserReadiness>[] = [
    {
      accessorKey: 'username',
      header: 'User',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.username}</span>
          {row.original.nomadic && (
            <p className="text-xs text-muted-foreground">devices that move</p>
          )}
        </div>
      ),
    },
    {
      id: 'location',
      header: 'Location that would be sent',
      cell: ({ row }) => {
        const location = row.original.location
        if (!location) return <span className="text-sm text-destructive">none</span>
        return (
          <div>
            <span className="text-sm">{location.summary}</span>
            <p className="text-xs text-muted-foreground">
              {location.source.replace(/_/g, ' ')}
              {location.address.validated ? ' · validated' : ' · not validated'}
            </p>
          </div>
        )
      },
    },
    {
      id: 'callback',
      header: 'Callback',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.location?.callback_number ?? '-'}</span>
      ),
    },
    {
      id: 'issues',
      header: 'Issues',
      cell: ({ row }) => <Issues issues={row.original.issues} />,
    },
  ]

  const contactColumns: ColumnDef<ContactReadiness>[] = [
    {
      accessorKey: 'target',
      header: 'Contact',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Badge variant="outline">{row.original.kind}</Badge>
          <span className="text-sm">{row.original.name ?? row.original.target}</span>
          {row.original.listen && <Badge variant="secondary">may listen</Badge>}
        </div>
      ),
    },
    {
      id: 'site',
      header: 'For',
      cell: ({ row }) => (
        <span className={`text-sm ${row.original.site ? '' : 'text-muted-foreground'}`}>
          {row.original.site ?? 'the tenant'}
        </span>
      ),
    },
    {
      id: 'last_delivery',
      header: 'Last delivery',
      cell: ({ row }) => <span className="text-sm">{ago(row.original.last_delivery_at)}</span>,
    },
    {
      id: 'last_test',
      header: 'Last test call',
      cell: ({ row }) => (
        <span className={`text-sm ${row.original.last_test_at ? '' : 'text-muted-foreground'}`}>
          {ago(row.original.last_test_at)}
        </span>
      ),
    },
    {
      id: 'issues',
      header: 'Issues',
      cell: ({ row }) => <Issues issues={row.original.issues} />,
    },
  ]

  if (isLoading || !data) {
    return <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-4 md:grid-cols-6">
        <Tile label="Users" value={data.summary.users} />
        <Tile label="Ready" value={data.summary.ready} tone="good" />
        <Tile label="No location" value={data.summary.missing_location} tone="bad" />
        <Tile label="No callback" value={data.summary.missing_callback} tone="bad" />
        <Tile label="Not validated" value={data.summary.unvalidated_address} tone="bad" />
        <Tile label="Unplaced nomadic" value={data.summary.unplaced_nomadic} tone="bad" />
      </div>

      <div className="space-y-2">
        <div className="flex items-baseline justify-between">
          <h2 className="font-semibold">On-site notification</h2>
          <span className="text-sm text-muted-foreground">
            {data.notifications.deliverable} contact
            {data.notifications.deliverable === 1 ? '' : 's'} can be told
          </span>
        </div>
        {data.notifications.issues.length > 0 && (
          <div className="rounded-lg border border-yellow-500/50 bg-yellow-50 p-4 dark:bg-yellow-950/30">
            <Issues issues={data.notifications.issues} />
          </div>
        )}
        <DataTable columns={contactColumns} data={data.notifications.contacts} />
      </div>

      <div className="space-y-2">
        <h2 className="font-semibold">Users</h2>
        <DataTable columns={userColumns} data={data.users} />
      </div>
    </div>
  )
}

/** The networks that place a device where it is. */
function NetworksTab() {
  const queryClient = useQueryClient()
  const [createOpen, setCreateOpen] = useState(false)
  const [form, setForm] = useState({ cidr: '', location_id: '', name: '', priority: '0', applies_to: 'nomadic' })
  const [error, setError] = useState<string | null>(null)

  const { data: networks = [], isLoading } = useQuery({
    queryKey: ['emergency-networks'],
    queryFn: emergency.networks,
  })
  const { data: sites = [] } = useQuery({ queryKey: ['locations'], queryFn: locations.list })
  const siteName = (id: string) => sites.find((s) => s.id === id)?.name ?? id.slice(0, 8)

  const create = useMutation({
    mutationFn: emergency.createNetwork,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['emergency-networks'] })
      setCreateOpen(false)
      setForm({ cidr: '', location_id: '', name: '', priority: '0', applies_to: 'nomadic' })
      setError(null)
    },
    onError: (e: Error) => setError(e.message),
  })
  const remove = useMutation({
    mutationFn: emergency.deleteNetwork,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['emergency-networks'] }),
  })
  const update = useMutation({
    mutationFn: ({ id, applies_to }: { id: string; applies_to: string }) =>
      emergency.updateNetwork(id, { applies_to }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['emergency-networks'] }),
  })

  const columns: ColumnDef<EmergencyNetwork>[] = [
    {
      accessorKey: 'cidr',
      header: 'Range',
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.cidr}</span>
          {row.original.name && (
            <p className="text-xs text-muted-foreground">{row.original.name}</p>
          )}
        </div>
      ),
    },
    {
      id: 'site',
      header: 'Site',
      cell: ({ row }) => <span className="text-sm">{siteName(row.original.location_id)}</span>,
    },
    {
      accessorKey: 'applies_to',
      header: 'Places',
      cell: ({ row }) => (
        <Select
          value={row.original.applies_to}
          onValueChange={(v) => update.mutate({ id: row.original.id, applies_to: v })}
        >
          <SelectTrigger className="w-40"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="nomadic">devices that move</SelectItem>
            <SelectItem value="all">every device</SelectItem>
          </SelectContent>
        </Select>
      ),
    },
    {
      accessorKey: 'priority',
      header: 'Priority',
      cell: ({ row }) => <span className="text-sm">{row.original.priority}</span>,
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <Button
          variant="ghost"
          size="icon"
          aria-label="Delete network"
          onClick={() => {
            if (confirm(`Delete the network ${row.original.cidr}?`)) remove.mutate(row.original.id)
          }}
        >
          <Trash2 className="h-4 w-4 text-destructive" />
        </Button>
      ),
    },
  ]

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <p className="text-sm text-muted-foreground">
          A device seen on one of these ranges is placed at its site, so a phone that moves is
          dispatched to where it is. The most specific range wins, then the lowest priority.
        </p>
        <Button size="sm" onClick={() => setCreateOpen(true)}>
          <Plus className="mr-2 h-4 w-4" /> Add network
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={networks} />
      )}

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add network</DialogTitle>
            <DialogDescription>A range, and the site an address on it is at.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-1">
              <Label className="text-xs">Range</Label>
              <Input
                placeholder="10.1.2.0/24"
                value={form.cidr}
                onChange={(e) => setForm({ ...form, cidr: e.target.value })}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">Site</Label>
              <Select
                value={form.location_id}
                onValueChange={(v) => setForm({ ...form, location_id: v })}
              >
                <SelectTrigger><SelectValue placeholder="Choose a site" /></SelectTrigger>
                <SelectContent>
                  {sites.map((s) => (
                    <SelectItem key={s.id} value={s.id}>{s.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1">
                <Label className="text-xs">Name (optional)</Label>
                <Input
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                />
              </div>
              <div className="space-y-1">
                <Label className="text-xs">Priority</Label>
                <Input
                  type="number"
                  value={form.priority}
                  onChange={(e) => setForm({ ...form, priority: e.target.value })}
                />
              </div>
            </div>
            <div className="space-y-1">
              <Label className="text-xs">Places</Label>
              <Select
                value={form.applies_to}
                onValueChange={(v) => setForm({ ...form, applies_to: v })}
              >
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="nomadic">devices that move</SelectItem>
                  <SelectItem value="all">every device</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button
                disabled={!form.cidr || !form.location_id || create.isPending}
                onClick={() =>
                  create.mutate({
                    cidr: form.cidr.trim(),
                    location_id: form.location_id,
                    name: form.name.trim() || undefined,
                    priority: Number(form.priority) || 0,
                    applies_to: form.applies_to,
                  })
                }
              >
                Add
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

/** Every emergency call this tenant placed, and who was told about it. */
function CallsTab() {
  const [selected, setSelected] = useState<EmergencyCall | null>(null)
  const { data: calls = [], isLoading } = useQuery({
    queryKey: ['emergency-calls'],
    queryFn: () => emergency.calls(200),
  })
  const { data: notifications = [] } = useQuery({
    queryKey: ['emergency-notifications', selected?.call_id],
    queryFn: () => emergency.notifications(selected!.call_id),
    enabled: !!selected,
  })

  const columns: ColumnDef<EmergencyCall>[] = [
    {
      accessorKey: 'placed_at',
      header: 'Placed',
      cell: ({ row }) => when(row.original.placed_at),
    },
    {
      accessorKey: 'dialed_number',
      header: 'Dialled',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <span className="font-mono text-sm">{row.original.dialed_number}</span>
          {row.original.kind === 'test' && <Badge variant="secondary">test</Badge>}
        </div>
      ),
    },
    {
      accessorKey: 'caller_username',
      header: 'Caller',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.caller_username ?? '-'}</span>
          <p className="text-xs text-muted-foreground font-mono">{row.original.caller_uri}</p>
        </div>
      ),
    },
    {
      id: 'location',
      header: 'Location sent',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.location?.summary ?? 'none'}</span>
          <p className="text-xs text-muted-foreground">
            {row.original.delivery ?? 'not sent'}
            {row.original.location_structured === false ? ' · unstructured' : ''}
          </p>
        </div>
      ),
    },
    {
      accessorKey: 'outcome',
      header: 'Outcome',
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Badge variant={outcomeBadge(row.original.outcome)}>{row.original.outcome}</Badge>
          {row.original.duration_secs != null && (
            <span className="text-xs text-muted-foreground">{row.original.duration_secs}s</span>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'notified',
      header: 'Notified',
      cell: ({ row }) =>
        row.original.notified ? (
          <Badge variant="success">yes</Badge>
        ) : (
          <Badge variant="destructive">no</Badge>
        ),
    },
    {
      id: 'detail',
      cell: ({ row }) => (
        <Button variant="ghost" size="sm" onClick={() => setSelected(row.original)}>
          View
        </Button>
      ),
    },
  ]

  const notificationRow = (n: EmergencyNotification) => (
    <div key={n.id} className="flex items-start justify-between gap-4 border-b py-2 last:border-0">
      <div>
        <div className="flex items-center gap-2">
          <Badge variant="outline">{n.kind}</Badge>
          <span className="text-sm">{n.name ?? n.target}</span>
          <span className="text-xs text-muted-foreground">attempt {n.attempt}</span>
        </div>
        {n.error && <p className="text-xs text-destructive">{n.error}</p>}
      </div>
      <div className="text-right">
        <Badge variant={statusBadge(n.status)}>{n.status}</Badge>
        <p className="text-xs text-muted-foreground">
          {n.delivered_at ? when(n.delivered_at) : when(n.created_at)}
        </p>
      </div>
    </div>
  )

  return (
    <div className="space-y-4">
      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={calls} />
      )}

      <Dialog open={!!selected} onOpenChange={() => setSelected(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {selected?.dialed_number} from {selected?.caller_username ?? selected?.caller_uri}
            </DialogTitle>
            <DialogDescription>
              What was sent with the call, and who was told about it.
            </DialogDescription>
          </DialogHeader>
          {selected && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div>
                  <p className="text-xs text-muted-foreground">Callback number</p>
                  <p className="font-mono">{selected.callback_number ?? 'none'}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Trunk</p>
                  <p>{selected.trunk_name ?? '-'}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Location source</p>
                  <p>{selected.location?.source.replace(/_/g, ' ') ?? 'none'}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Seen at</p>
                  <p className="font-mono">{selected.location?.client_address ?? '-'}</p>
                </div>
                <div className="col-span-2">
                  <p className="text-xs text-muted-foreground">Address</p>
                  <p>{selected.location?.summary ?? 'no dispatchable location'}</p>
                </div>
                {selected.geolocation_error && (
                  <div className="col-span-2">
                    <p className="text-xs text-muted-foreground">The trunk answered the location with</p>
                    <p className="text-destructive">{selected.geolocation_error}</p>
                  </div>
                )}
              </div>

              <div>
                <h3 className="mb-2 font-semibold">Notifications</h3>
                {notifications.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    No attempt was recorded for this call.
                  </p>
                ) : (
                  <div className="max-h-64 overflow-auto">{notifications.map(notificationRow)}</div>
                )}
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}

export function EmergencyPage() {
  const [tab, setTab] = useState<Tab>('config')

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Emergency Calling</h1>
        <p className="text-muted-foreground">
          Where each caller would be dispatched to, who is told when a call is placed, and what
          happened on the calls that were
        </p>
      </div>

      <div className="flex gap-2">
        {TABS.map(({ id, label }) => (
          <Button
            key={id}
            variant={tab === id ? 'default' : 'outline'}
            size="sm"
            onClick={() => setTab(id)}
          >
            {label}
          </Button>
        ))}
      </div>

      {tab === 'config' && <ConfigurationTab />}
      {tab === 'readiness' && <ReadinessTab />}
      {tab === 'networks' && <NetworksTab />}
      {tab === 'calls' && <CallsTab />}
    </div>
  )
}
