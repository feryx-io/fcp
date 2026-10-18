import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { CalendarClock, Plus, Trash2, Pencil, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { schedules } from '@/api/endpoints'
import type { Holiday, Schedule, ScheduleRequest, Weekday, WeeklyWindow } from '@/types'

// A tenant's schedules — business hours and holidays in a timezone — that
// numbers, users, queues and flows name (docs/ENTERPRISE_CALLING.md §3.5).

const DAYS: Weekday[] = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

const emptyRequest = (): ScheduleRequest => ({
  name: '',
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
  weekly: [{ days: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri'], start: '09:00', end: '17:00' }],
  holidays: [],
})

const stateBadge = (state: Schedule['state']) => {
  switch (state) {
    case 'open':
      return <Badge className="bg-green-600">Open</Badge>
    case 'holiday':
      return <Badge className="bg-amber-600">Holiday</Badge>
    default:
      return <Badge variant="secondary">Closed</Badge>
  }
}

const describeWindow = (w: WeeklyWindow) =>
  `${w.days.join(', ')} ${w.start === w.end ? 'all day' : `${w.start}–${w.end}`}`

export function SchedulesPage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<{ id: string | null; req: ScheduleRequest } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const { data: list = [], isLoading } = useQuery({
    queryKey: ['schedules'],
    queryFn: schedules.list,
    refetchInterval: 60000,
  })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['schedules'] })

  const save = useMutation({
    mutationFn: (e: { id: string | null; req: ScheduleRequest }) =>
      e.id ? schedules.update(e.id, e.req) : schedules.create(e.req),
    onSuccess: () => {
      setEditing(null)
      setError(null)
      refresh()
    },
    onError: (err) => setError(err instanceof Error ? err.message : 'Failed to save'),
  })
  const remove = useMutation({ mutationFn: schedules.delete, onSuccess: refresh })

  const req = editing?.req
  const update = (patch: Partial<ScheduleRequest>) =>
    setEditing((prev) => (prev ? { ...prev, req: { ...prev.req, ...patch } } : prev))
  const setWindow = (i: number, patch: Partial<WeeklyWindow>) =>
    update({ weekly: (req!.weekly ?? []).map((w, j) => (j === i ? { ...w, ...patch } : w)) })
  const setHoliday = (i: number, patch: Partial<Holiday>) =>
    update({ holidays: (req!.holidays ?? []).map((h, j) => (j === i ? { ...h, ...patch } : h)) })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Schedules</h1>
          <p className="text-muted-foreground">
            Business hours and holidays. Numbers, users, queues and flows name a schedule and say where calls go
            when it is closed.
          </p>
        </div>
        <Button onClick={() => { setError(null); setEditing({ id: null, req: emptyRequest() }) }}>
          <Plus className="mr-2 h-4 w-4" /> New Schedule
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Now</TableHead>
                <TableHead>Hours</TableHead>
                <TableHead>Holidays</TableHead>
                <TableHead>Timezone</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading && (
                <TableRow><TableCell colSpan={6} className="text-muted-foreground">Loading…</TableCell></TableRow>
              )}
              {!isLoading && list.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    <CalendarClock className="mx-auto mb-2 h-6 w-6" />
                    No schedules yet.
                  </TableCell>
                </TableRow>
              )}
              {list.map((s) => (
                <TableRow key={s.id}>
                  <TableCell className="font-medium">{s.name}</TableCell>
                  <TableCell>{stateBadge(s.state)}</TableCell>
                  <TableCell className="text-sm">
                    {s.weekly.length === 0 ? 'Never open' : s.weekly.map(describeWindow).join('; ')}
                  </TableCell>
                  <TableCell className="text-sm">{s.holidays.length}</TableCell>
                  <TableCell className="text-sm">{s.timezone}</TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      title="Edit"
                      onClick={() => {
                        setError(null)
                        setEditing({
                          id: s.id,
                          req: { name: s.name, timezone: s.timezone, weekly: s.weekly, holidays: s.holidays },
                        })
                      }}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      title="Delete"
                      onClick={() => {
                        if (confirm(`Delete the schedule "${s.name}"?`)) remove.mutate(s.id)
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

      <Dialog open={!!editing} onOpenChange={(v) => { if (!v) setEditing(null) }}>
        <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{editing?.id ? 'Edit Schedule' : 'New Schedule'}</DialogTitle>
            <DialogDescription>
              Open inside any window, closed outside them, and on a holiday whatever the hours say.
            </DialogDescription>
          </DialogHeader>
          {req && (
            <div className="space-y-5">
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1">
                  <Label>Name</Label>
                  <Input value={req.name} onChange={(e) => update({ name: e.target.value })} />
                </div>
                <div className="space-y-1">
                  <Label>Timezone</Label>
                  <Input
                    value={req.timezone ?? ''}
                    placeholder="America/New_York"
                    onChange={(e) => update({ timezone: e.target.value })}
                  />
                </div>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label>Hours</Label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => update({ weekly: [...(req.weekly ?? []), { days: ['Mon'], start: '09:00', end: '17:00' }] })}
                  >
                    <Plus className="mr-1 h-3 w-3" /> Add hours
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  A start equal to its end is the whole day; an end before its start runs past midnight.
                </p>
                {(req.weekly ?? []).map((w, i) => (
                  <div key={i} className="flex flex-wrap items-center gap-2 rounded-md border p-2">
                    {DAYS.map((d) => (
                      <label key={d} className="flex items-center gap-1 text-xs">
                        <input
                          type="checkbox"
                          checked={w.days.includes(d)}
                          onChange={(e) =>
                            setWindow(i, {
                              days: e.target.checked ? DAYS.filter((x) => x === d || w.days.includes(x)) : w.days.filter((x) => x !== d),
                            })
                          }
                        />
                        {d}
                      </label>
                    ))}
                    <Input type="time" className="w-28" value={w.start} onChange={(e) => setWindow(i, { start: e.target.value })} />
                    <span className="text-xs">to</span>
                    <Input type="time" className="w-28" value={w.end} onChange={(e) => setWindow(i, { end: e.target.value })} />
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      title="Remove these hours"
                      onClick={() => update({ weekly: (req.weekly ?? []).filter((_, j) => j !== i) })}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label>Holidays</Label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      update({ holidays: [...(req.holidays ?? []), { name: '', from: new Date().toISOString().slice(0, 10), to: null }] })
                    }
                  >
                    <Plus className="mr-1 h-3 w-3" /> Add holiday
                  </Button>
                </div>
                {(req.holidays ?? []).map((h, i) => (
                  <div key={i} className="flex flex-wrap items-center gap-2 rounded-md border p-2">
                    <Input className="w-48" placeholder="Name" value={h.name} onChange={(e) => setHoliday(i, { name: e.target.value })} />
                    <Input type="date" className="w-40" value={h.from} onChange={(e) => setHoliday(i, { from: e.target.value })} />
                    <span className="text-xs">to</span>
                    <Input
                      type="date"
                      className="w-40"
                      value={h.to ?? ''}
                      onChange={(e) => setHoliday(i, { to: e.target.value || null })}
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      title="Remove this holiday"
                      onClick={() => update({ holidays: (req.holidays ?? []).filter((_, j) => j !== i) })}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>

              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button onClick={() => editing && save.mutate(editing)} disabled={save.isPending}>
              {save.isPending ? 'Saving…' : 'Save'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
