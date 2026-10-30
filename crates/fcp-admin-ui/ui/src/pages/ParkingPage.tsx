import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { CircleParking, Pencil, PhoneForwarded, PhoneOff, Plus, Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { locations, parking, prompts } from '@/api/endpoints'
import type { ParkedCall, ParkingLot, ParkingLotRequest } from '@/types'

// A tenant's parking lots — ranges of slots numbered as the tenant likes —
// and the calls waiting in them now, on whichever call manager holds them
// (docs/PARKING.md).

const NONE = '__none__'

const emptyRequest = (): ParkingLotRequest => ({
  name: '',
  first: '',
  last: '',
  park_number: null,
  timeout_secs: 60,
  fallback: null,
  music: null,
  location_id: null,
  enabled: true,
})

const requestOf = (lot: ParkingLot): ParkingLotRequest => ({
  name: lot.name,
  first: lot.first,
  last: lot.last,
  park_number: lot.park_number ?? null,
  timeout_secs: lot.timeout_secs,
  fallback: lot.fallback ?? null,
  music: lot.music ?? null,
  location_id: lot.location_id ?? null,
  enabled: lot.enabled,
})

/** The slots of a lot, zero-padded to its width, as the server numbers them. */
const slotsOf = (lot: ParkingLot): string[] => {
  const width = lot.first.length
  const first = Number(lot.first)
  const last = Number(lot.last)
  if (!Number.isFinite(first) || !Number.isFinite(last) || last < first) return []
  return Array.from({ length: last - first + 1 }, (_, i) => String(first + i).padStart(width, '0'))
}

const waited = (secs: number) => {
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return m > 0 ? `${m}m ${s.toString().padStart(2, '0')}s` : `${s}s`
}

const message = (err: unknown) => (err instanceof Error ? err.message : 'Something went wrong')

export function ParkingPage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<{ id: string | null; req: ParkingLotRequest } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [sending, setSending] = useState<{ slot: string; target: string } | null>(null)
  const [boardError, setBoardError] = useState<string | null>(null)

  const lots = useQuery({ queryKey: ['parking-lots'], queryFn: parking.lots })
  const calls = useQuery({
    queryKey: ['parking-calls'],
    queryFn: parking.calls,
    refetchInterval: 3000,
    retry: false,
  })
  const music = useQuery({ queryKey: ['prompts', 'moh'], queryFn: () => prompts.list({ category: 'moh' }) })
  const sites = useQuery({ queryKey: ['locations'], queryFn: locations.list })

  const refreshLots = () => queryClient.invalidateQueries({ queryKey: ['parking-lots'] })
  const refreshCalls = () => queryClient.invalidateQueries({ queryKey: ['parking-calls'] })

  const save = useMutation({
    mutationFn: (e: { id: string | null; req: ParkingLotRequest }) =>
      e.id ? parking.updateLot(e.id, e.req) : parking.createLot(e.req),
    onSuccess: () => {
      setEditing(null)
      setError(null)
      refreshLots()
    },
    onError: (err) => setError(message(err)),
  })
  const remove = useMutation({ mutationFn: parking.deleteLot, onSuccess: refreshLots })
  const retrieve = useMutation({
    mutationFn: (e: { slot: string; target: string }) => parking.retrieve(e.slot, e.target),
    onSuccess: () => {
      setSending(null)
      setBoardError(null)
      refreshCalls()
    },
    onError: (err) => setBoardError(message(err)),
  })
  const hangUp = useMutation({
    mutationFn: parking.hangUp,
    onSuccess: () => {
      setBoardError(null)
      refreshCalls()
    },
    onError: (err) => setBoardError(message(err)),
  })

  const lotList = lots.data ?? []
  const parked = calls.data ?? []
  // The cloud's own calls by slot; a site's calls, which number the same
  // lots, are shown apart under their site.
  const own = parked.filter((c) => !c.site_id)
  const bySlot = new Map(own.map((c) => [c.slot, c]))
  // Calls in the node's own range when the tenant has no lot, or in a
  // lot since removed: shown apart, so none is out of sight.
  const lotSlots = new Set(lotList.flatMap(slotsOf))
  const elsewhere = own.filter((c) => !lotSlots.has(c.slot))
  const atSites = parked.filter((c) => c.site_id)

  const req = editing?.req
  const update = (patch: Partial<ParkingLotRequest>) =>
    setEditing((prev) => (prev ? { ...prev, req: { ...prev.req, ...patch } } : prev))

  const callCell = (call: ParkedCall) => (
    <div className="space-y-1">
      <div className="text-sm font-medium">
        {call.caller?.name || call.caller?.uri || 'Unknown caller'}
      </div>
      <div className="text-xs text-muted-foreground">
        by {call.parker_extension} · {waited(call.parked_secs)}
        {call.timeout_secs > 0 ? ` of ${waited(call.timeout_secs)}` : ''}
      </div>
      {call.site_name ? (
        // A site's call is taken back or hung up at the site.
        <Badge variant="secondary">At {call.site_name}</Badge>
      ) : (
      <div className="flex gap-1">
        <Button
          variant="outline"
          size="sm"
          title="Retrieve to…"
          onClick={() => setSending({ slot: call.slot, target: '' })}
        >
          <PhoneForwarded className="h-3 w-3" />
        </Button>
        <Button
          variant="outline"
          size="sm"
          title="Hang up"
          onClick={() => {
            if (confirm(`Hang up the call parked in ${call.slot}?`)) hangUp.mutate(call.slot)
          }}
        >
          <PhoneOff className="h-3 w-3" />
        </Button>
      </div>
      )}
    </div>
  )

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Parking</h1>
          <p className="text-muted-foreground">
            Lots of slots numbered as you like. Park with the lot&apos;s park number or *70, or by transferring to a
            slot; dial the slot to take the call back.
          </p>
        </div>
        <Button onClick={() => { setError(null); setEditing({ id: null, req: emptyRequest() }) }}>
          <Plus className="mr-2 h-4 w-4" /> New Lot
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Slots</TableHead>
                <TableHead>Park number</TableHead>
                <TableHead>Waits</TableHead>
                <TableHead>Then</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {lots.isLoading && (
                <TableRow><TableCell colSpan={6} className="text-muted-foreground">Loading…</TableCell></TableRow>
              )}
              {!lots.isLoading && lotList.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    <CircleParking className="mx-auto mb-2 h-6 w-6" />
                    No lots yet: calls park in the call manager&apos;s own range (701–720).
                  </TableCell>
                </TableRow>
              )}
              {lotList.map((lot) => (
                <TableRow key={lot.id}>
                  <TableCell className="font-medium">
                    {lot.name} {!lot.enabled && <Badge variant="secondary">Off</Badge>}
                  </TableCell>
                  <TableCell className="font-mono text-sm">
                    {lot.first}–{lot.last} <span className="text-muted-foreground">({lot.slots})</span>
                  </TableCell>
                  <TableCell className="font-mono text-sm">{lot.park_number ?? '—'}</TableCell>
                  <TableCell className="text-sm">{waited(lot.timeout_secs)}</TableCell>
                  <TableCell className="text-sm">
                    The parker{lot.fallback ? `, then ${lot.fallback}` : ''}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      title="Edit"
                      onClick={() => { setError(null); setEditing({ id: lot.id, req: requestOf(lot) }) }}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      title="Delete"
                      onClick={() => {
                        if (confirm(`Delete the lot "${lot.name}"? Calls in it stay until taken or timed out.`)) {
                          remove.mutate(lot.id)
                        }
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

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold">Parked now</h2>
          <span className="text-xs text-muted-foreground">
            {calls.isError ? 'No call manager answered' : `${parked.length} parked · updates every 3 s`}
          </span>
        </div>
        {boardError && <p className="text-sm text-destructive">{boardError}</p>}
        {lotList.filter((l) => l.enabled).map((lot) => (
          <Card key={lot.id}>
            <CardHeader className="py-3">
              <CardTitle className="text-base">{lot.name}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5" data-testid={`board-${lot.id}`}>
                {slotsOf(lot).map((slot) => {
                  const call = bySlot.get(slot)
                  return (
                    <div
                      key={slot}
                      className={
                        call
                          ? 'rounded-md border border-amber-500 bg-amber-50 p-2 dark:bg-amber-950'
                          : 'rounded-md border p-2 text-muted-foreground'
                      }
                    >
                      <div className="font-mono text-sm font-semibold">{slot}</div>
                      {call ? callCell(call) : <div className="text-xs">Empty</div>}
                    </div>
                  )
                })}
              </div>
            </CardContent>
          </Card>
        ))}
        {atSites.length > 0 && (
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="text-base">At the sites</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
                {atSites.map((call) => (
                  <div key={`${call.site_id}-${call.slot}`} className="rounded-md border border-amber-500 p-2">
                    <div className="font-mono text-sm font-semibold">{call.slot}</div>
                    {callCell(call)}
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        )}
        {elsewhere.length > 0 && (
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="text-base">Other slots</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
                {elsewhere.map((call) => (
                  <div key={call.slot} className="rounded-md border border-amber-500 p-2">
                    <div className="font-mono text-sm font-semibold">{call.slot}</div>
                    {callCell(call)}
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        )}
      </div>

      <Dialog open={!!editing} onOpenChange={(v) => { if (!v) setEditing(null) }}>
        <DialogContent className="max-w-xl">
          <DialogHeader>
            <DialogTitle>{editing?.id ? 'Edit Lot' : 'New Lot'}</DialogTitle>
            <DialogDescription>
              First and last slot have the same number of digits (2 to 8, a leading zero is kept), at most 1,000
              slots, and none may be a number something else already dials.
            </DialogDescription>
          </DialogHeader>
          {req && (
            <div className="space-y-4">
              <div className="space-y-1">
                <Label htmlFor="lot-name">Name</Label>
                <Input id="lot-name" value={req.name} onChange={(e) => update({ name: e.target.value })} />
              </div>
              <div className="grid grid-cols-3 gap-4">
                <div className="space-y-1">
                  <Label htmlFor="lot-first">First slot</Label>
                  <Input id="lot-first" value={req.first} placeholder="701" onChange={(e) => update({ first: e.target.value })} />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="lot-last">Last slot</Label>
                  <Input id="lot-last" value={req.last} placeholder="720" onChange={(e) => update({ last: e.target.value })} />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="lot-park-number">Park number</Label>
                  <Input
                    id="lot-park-number"
                    value={req.park_number ?? ''}
                    placeholder="700"
                    onChange={(e) => update({ park_number: e.target.value || null })}
                  />
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1">
                  <Label htmlFor="lot-timeout">Waits (seconds)</Label>
                  <Input
                    id="lot-timeout"
                    type="number"
                    min={1}
                    max={3600}
                    value={req.timeout_secs ?? 60}
                    onChange={(e) => update({ timeout_secs: Number(e.target.value) })}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="lot-fallback">Then, if the parker does not answer</Label>
                  <Input
                    id="lot-fallback"
                    value={req.fallback ?? ''}
                    placeholder="voicemail, ext:1000, queue:front"
                    onChange={(e) => update({ fallback: e.target.value || null })}
                  />
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1">
                  <Label htmlFor="lot-music">Music</Label>
                  <select
                    id="lot-music"
                    className="h-9 w-full rounded-md border bg-background px-2 text-sm"
                    value={req.music ?? NONE}
                    onChange={(e) => update({ music: e.target.value === NONE ? null : e.target.value })}
                  >
                    <option value={NONE}>The parker&apos;s or the tenant&apos;s</option>
                    {(music.data ?? []).map((p) => (
                      <option key={p.id} value={p.id}>{p.name}</option>
                    ))}
                  </select>
                </div>
                <div className="space-y-1">
                  <Label htmlFor="lot-location">Site</Label>
                  <select
                    id="lot-location"
                    className="h-9 w-full rounded-md border bg-background px-2 text-sm"
                    value={req.location_id ?? NONE}
                    onChange={(e) => update({ location_id: e.target.value === NONE ? null : e.target.value })}
                  >
                    <option value={NONE}>Every site</option>
                    {(sites.data ?? []).map((l) => (
                      <option key={l.id} value={l.id}>{l.name}</option>
                    ))}
                  </select>
                </div>
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={req.enabled ?? true}
                  onChange={(e) => update({ enabled: e.target.checked })}
                />
                Calls park here
              </label>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button disabled={save.isPending} onClick={() => editing && save.mutate(editing)}>
              Save
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!sending} onOpenChange={(v) => { if (!v) setSending(null) }}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Retrieve {sending?.slot} to…</DialogTitle>
            <DialogDescription>
              Their phones ring with the call; answering takes it. Unanswered, their own plan decides.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1">
            <Label htmlFor="retrieve-target">Extension or user</Label>
            <Input
              id="retrieve-target"
              value={sending?.target ?? ''}
              placeholder="1003"
              onChange={(e) => setSending((s) => (s ? { ...s, target: e.target.value } : s))}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setSending(null)}>Cancel</Button>
            <Button
              disabled={!sending?.target.trim() || retrieve.isPending}
              onClick={() => sending && retrieve.mutate({ slot: sending.slot, target: sending.target.trim() })}
            >
              Ring them
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
