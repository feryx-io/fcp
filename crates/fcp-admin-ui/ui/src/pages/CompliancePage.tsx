import { useEffect, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { AlertTriangle, Download, Eye, Play, Plus, Trash2, Unlock } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
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
import { compliance, prompts } from '@/api/endpoints'
import type {
  CompliancePolicy, DataClass, LegalHold, RetentionRun, SubjectIdentifier, SubjectRequest,
  UpdateCompliancePolicy,
} from '@/types'

type Tab = 'policy' | 'retention' | 'holds' | 'requests'

const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'policy', label: 'Policy' },
  { id: 'retention', label: 'Retention runs' },
  { id: 'holds', label: 'Legal holds' },
  { id: 'requests', label: 'Subject requests' },
]

const CLASSES: Array<{ value: DataClass; label: string }> = [
  { value: 'cdr', label: 'Call records' },
  { value: 'call_recording', label: 'Call recordings' },
  { value: 'conference_recording', label: 'Conference recordings' },
  { value: 'voicemail', label: 'Voicemail' },
  { value: 'sip_trace', label: 'SIP traces' },
  { value: 'flow_trace', label: 'Flow traces' },
  { value: 'audit_event', label: 'Audit log' },
  { value: 'device_capture', label: 'App captures' },
  { value: 'conference_chat', label: 'Conference chat' },
  { value: 'webinar_question', label: 'Webinar questions' },
  { value: 'webinar_registrant', label: 'Webinar registrations' },
]

const classLabel = (c: string) => CLASSES.find((k) => k.value === c)?.label ?? c

const SUBJECT_KINDS: Array<{ value: SubjectIdentifier['type']; label: string; hint: string }> = [
  { value: 'user', label: 'User', hint: 'alice' },
  { value: 'number', label: 'Number', hint: '+15185550143' },
  { value: 'aor', label: 'SIP address', hint: 'alice@pbx.example.com' },
  { value: 'email', label: 'Email', hint: 'alice@example.com' },
]

function subjectText(s: SubjectIdentifier) {
  switch (s.type) {
    case 'user': return s.username
    case 'number': return s.number
    case 'aor': return s.aor
    case 'email': return s.address
  }
}

function makeSubject(type: SubjectIdentifier['type'], value: string): SubjectIdentifier {
  switch (type) {
    case 'user': return { type, username: value }
    case 'number': return { type, number: value }
    case 'aor': return { type, aor: value }
    case 'email': return { type, address: value }
  }
}

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Request failed'
}

const when = (at: string | null | undefined) => (at ? new Date(at).toLocaleString() : '-')
const fromUnix = (secs: number | null | undefined) =>
  secs ? new Date(secs * 1000).toLocaleDateString() : null
const toUnix = (date: string) => (date ? Math.floor(Date.parse(date) / 1000) : undefined)

/** A query that failed, most often for want of the permission. */
function Failed({ error }: { error: unknown }) {
  return (
    <div className="flex items-start gap-2 rounded-md border border-destructive/50 p-4 text-sm">
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
      <span>{errorText(error)}</span>
    </div>
  )
}

function Loading() {
  return <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
}

/** Subjects added one at a time: a kind and a value. */
function SubjectsInput({
  subjects, onChange,
}: { subjects: SubjectIdentifier[]; onChange: (s: SubjectIdentifier[]) => void }) {
  const [kind, setKind] = useState<SubjectIdentifier['type']>('user')
  const [value, setValue] = useState('')
  const add = () => {
    const v = value.trim()
    if (!v) return
    onChange([...subjects, makeSubject(kind, v)])
    setValue('')
  }
  return (
    <div className="space-y-2">
      <div className="flex gap-2">
        <Select value={kind} onValueChange={(v) => setKind(v as SubjectIdentifier['type'])}>
          <SelectTrigger className="w-40"><SelectValue /></SelectTrigger>
          <SelectContent>
            {SUBJECT_KINDS.map((k) => <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>)}
          </SelectContent>
        </Select>
        <Input placeholder={SUBJECT_KINDS.find((k) => k.value === kind)?.hint} value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add() } }} />
        <Button type="button" variant="outline" onClick={add}>Add</Button>
      </div>
      {subjects.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {subjects.map((s, i) => (
            <Badge key={i} variant="secondary" className="gap-1">
              {SUBJECT_KINDS.find((k) => k.value === s.type)?.label}: {subjectText(s)}
              <button type="button" className="ml-1" title="Remove"
                onClick={() => onChange(subjects.filter((_, j) => j !== i))}>
                <Trash2 className="h-3 w-3" />
              </button>
            </Badge>
          ))}
        </div>
      )}
    </div>
  )
}

// ─── Policy ────────────────────────────────────────────────────────────────

const RETENTION: Array<{ key: keyof CompliancePolicy & keyof UpdateCompliancePolicy; label: string }> = [
  { key: 'cdr_retention_days', label: 'Call records' },
  { key: 'recording_retention_days', label: 'Call recordings' },
  { key: 'voicemail_retention_days', label: 'Voicemail' },
  { key: 'sip_trace_retention_days', label: 'SIP traces' },
  { key: 'flow_trace_retention_days', label: 'Flow traces' },
  { key: 'audit_retention_days', label: 'Audit log' },
  { key: 'conference_chat_retention_days', label: 'Conference chat' },
  { key: 'webinar_question_retention_days', label: 'Webinar questions' },
  { key: 'webinar_registrant_retention_days', label: 'Webinar registrations' },
]

/** Blank is "keep forever" (null); otherwise whole days. */
const days = (text: string) => (text.trim() === '' ? null : Number(text))

function PolicyTab() {
  const queryClient = useQueryClient()
  const policy = useQuery({ queryKey: ['compliance', 'policy'], queryFn: compliance.policy })
  const { data: promptList = [] } = useQuery({ queryKey: ['prompts'], queryFn: () => prompts.list() })
  const [form, setForm] = useState<Record<string, string>>({})
  const [consent, setConsent] = useState('none')
  const [promptId, setPromptId] = useState('')
  const [onFailure, setOnFailure] = useState('record')
  const [cdrErasure, setCdrErasure] = useState('pseudonymise')
  const [archiveRequired, setArchiveRequired] = useState(false)
  const [saved, setSaved] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const p = policy.data
    if (!p) return
    const text = (n: number | null | undefined) => (n == null ? '' : String(n))
    setForm({
      ...Object.fromEntries(RETENTION.map(({ key }) => [key, text(p[key] as number | null)])),
      recording_notice_interval_secs: text(p.recording_notice_interval_secs),
      subject_request_days: String(p.subject_request_days),
    })
    setConsent(p.recording_consent)
    setPromptId(p.recording_consent_prompt_id ?? '')
    setOnFailure(p.recording_consent_on_failure)
    setCdrErasure(p.cdr_erasure)
    setArchiveRequired(p.cdr_archive_required)
  }, [policy.data])

  const save = useMutation({
    mutationFn: () => {
      const body: UpdateCompliancePolicy = {
        recording_consent: consent,
        recording_consent_prompt_id: promptId || null,
        recording_consent_on_failure: onFailure,
        recording_notice_interval_secs: days(form.recording_notice_interval_secs ?? ''),
        subject_request_days: Number(form.subject_request_days),
        cdr_erasure: cdrErasure,
        cdr_archive_required: archiveRequired,
      }
      for (const { key } of RETENTION) {
        (body as Record<string, unknown>)[key] = days(form[key] ?? '')
      }
      return compliance.updatePolicy(body)
    },
    onSuccess: (p) => {
      queryClient.setQueryData(['compliance', 'policy'], p)
      setError(null)
      setSaved(`Saved ${new Date(p.updated_at).toLocaleTimeString()}`)
    },
    onError: (e) => { setSaved(null); setError(errorText(e)) },
  })

  if (policy.isLoading) return <Loading />
  if (policy.error) return <Failed error={policy.error} />
  const p = policy.data!
  const field = (key: string) => ({
    value: form[key] ?? '',
    onChange: (e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, [key]: e.target.value }),
  })

  return (
    <form className="space-y-6 max-w-3xl" onSubmit={(e) => { e.preventDefault(); save.mutate() }}>
      {!p.configured && (
        <p className="text-sm text-muted-foreground">
          No policy has been saved for this tenant: these are the defaults.
        </p>
      )}
      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Retention</h2>
        <p className="text-sm text-muted-foreground">
          Days each kind of data is kept before the daily sweep deletes it. Blank keeps it forever.
          Anything under a legal hold is kept regardless.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
          {RETENTION.map(({ key, label }) => (
            <div key={key} className="space-y-1">
              <Label htmlFor={key}>{label}</Label>
              <Input id={key} type="number" min={1} max={36500} placeholder="Forever" {...field(key)} />
            </div>
          ))}
        </div>
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={archiveRequired} onCheckedChange={setArchiveRequired} />
          Delete call records only once the archive has confirmed them
        </label>
        <p className="text-xs text-muted-foreground">
          Archive confirmed through: {p.cdr_archived_before ? fromUnix(p.cdr_archived_before) : 'nothing yet'}
        </p>
      </section>

      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Recording consent</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div className="space-y-1">
            <Label>Before a recording starts</Label>
            <Select value={consent} onValueChange={setConsent}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="none">Nothing is played</SelectItem>
                <SelectItem value="announce">An announcement is played</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {consent === 'announce' && (
            <>
              <div className="space-y-1">
                <Label>Announcement</Label>
                <Select value={promptId || '_default'} onValueChange={(v) => setPromptId(v === '_default' ? '' : v)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="_default">The built-in announcement</SelectItem>
                    {promptList.map((pr) => <SelectItem key={pr.id} value={pr.id}>{pr.name}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <Label>When it cannot be played</Label>
                <Select value={onFailure} onValueChange={setOnFailure}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="record">Record anyway</SelectItem>
                    <SelectItem value="skip">Do not record</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </>
          )}
          <div className="space-y-1">
            <Label htmlFor="notice">Notice tone every (seconds)</Label>
            <Input id="notice" type="number" min={1} placeholder="None" {...field('recording_notice_interval_secs')} />
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-lg font-semibold">Subject requests</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div className="space-y-1">
            <Label htmlFor="due">Days to answer</Label>
            <Input id="due" type="number" min={1} {...field('subject_request_days')} />
          </div>
          <div className="space-y-1">
            <Label>An erasure's call records are</Label>
            <Select value={cdrErasure} onValueChange={setCdrErasure}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="pseudonymise">Pseudonymised (kept for billing)</SelectItem>
                <SelectItem value="delete">Deleted</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
      </section>

      {error && <p className="text-sm text-destructive">{error}</p>}
      <div className="flex items-center gap-3">
        <Button type="submit" disabled={save.isPending}>{save.isPending ? 'Saving...' : 'Save policy'}</Button>
        {saved && <span className="text-sm text-muted-foreground">{saved}</span>}
      </div>
    </form>
  )
}

// ─── Retention runs ──────────────────────────────────────────────────────────

function RetentionTab() {
  const queryClient = useQueryClient()
  const runs = useQuery({ queryKey: ['compliance', 'runs'], queryFn: compliance.retentionRuns })
  const run = useMutation({
    mutationFn: compliance.runRetention,
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ['compliance', 'runs'] })
      alert(`Deleted ${r.total_deleted} item(s)${r.errors.length ? `, with ${r.errors.length} error(s)` : ''}.`)
    },
    onError: (e) => alert(errorText(e)),
  })
  const columns: ColumnDef<RetentionRun>[] = [
    { accessorKey: 'ran_at', header: 'Ran', cell: ({ row }) => when(row.original.ran_at) },
    { accessorKey: 'cdrs_deleted', header: 'Call records' },
    {
      id: 'recordings', header: 'Recordings',
      cell: ({ row }) => row.original.recordings_deleted + row.original.conference_recordings_deleted,
    },
    { accessorKey: 'voicemail_messages_deleted', header: 'Voicemail' },
    { accessorKey: 'conference_chat_deleted', header: 'Chat' },
    {
      id: 'webinars', header: 'Webinars',
      cell: ({ row }) => row.original.webinar_questions_deleted + row.original.webinar_registrants_deleted,
    },
    {
      id: 'traces', header: 'Traces',
      cell: ({ row }) => row.original.sip_traces_deleted + row.original.flow_traces_deleted,
    },
    { accessorKey: 'audit_events_deleted', header: 'Audit' },
    { accessorKey: 'total_deleted', header: 'Total' },
    {
      id: 'errors', header: 'Errors',
      cell: ({ row }) => row.original.errors.length === 0
        ? <span className="text-muted-foreground">-</span>
        : <Badge variant="destructive" title={row.original.errors.join('\n')}>{row.original.errors.length}</Badge>,
    },
  ]
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          The sweep runs daily under the policy; a run now deletes what is already past its days.
        </p>
        <Button disabled={run.isPending} onClick={() => {
          if (confirm('Delete everything past its retention now? Held items are kept.')) run.mutate()
        }}>
          <Play className="mr-2 h-4 w-4" />{run.isPending ? 'Running...' : 'Run now'}
        </Button>
      </div>
      {runs.isLoading ? <Loading /> : runs.error ? <Failed error={runs.error} /> : (
        <DataTable columns={columns} data={runs.data ?? []} />
      )}
    </div>
  )
}

// ─── Legal holds ───────────────────────────────────────────────────────────

function HoldsTab() {
  const queryClient = useQueryClient()
  const [all, setAll] = useState(false)
  const [placing, setPlacing] = useState(false)
  const [viewing, setViewing] = useState<string | null>(null)
  const [releasing, setReleasing] = useState<LegalHold | null>(null)
  const [releaseReason, setReleaseReason] = useState('')
  const [matter, setMatter] = useState('')
  const [reason, setReason] = useState('')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [subjects, setSubjects] = useState<SubjectIdentifier[]>([])
  const [callIds, setCallIds] = useState('')
  const [classes, setClasses] = useState<Set<DataClass>>(new Set())
  const [formError, setFormError] = useState<string | null>(null)

  const holds = useQuery({ queryKey: ['compliance', 'holds', all], queryFn: () => compliance.holds(all) })
  const detail = useQuery({
    queryKey: ['compliance', 'hold', viewing],
    queryFn: () => compliance.hold(viewing!),
    enabled: !!viewing,
  })
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['compliance', 'holds'] })

  const place = useMutation({
    mutationFn: () => compliance.placeHold({
      matter: matter.trim(),
      reason: reason.trim() || null,
      from: toUnix(from),
      to: toUnix(to),
      subjects,
      call_ids: callIds.split(/[\s,]+/).filter(Boolean),
      classes: [...classes],
    }),
    onSuccess: () => { invalidate(); setPlacing(false) },
    onError: (e) => setFormError(errorText(e)),
  })
  const release = useMutation({
    mutationFn: () => compliance.releaseHold(releasing!.id, releaseReason.trim()),
    onSuccess: () => { invalidate(); setReleasing(null) },
    onError: (e) => alert(errorText(e)),
  })

  const openPlace = () => {
    setMatter(''); setReason(''); setFrom(''); setTo(''); setSubjects([]); setCallIds('')
    setClasses(new Set()); setFormError(null); setPlacing(true)
  }
  const toggleClass = (c: DataClass) => {
    const next = new Set(classes)
    if (next.has(c)) next.delete(c)
    else next.add(c)
    setClasses(next)
  }

  const scope = (h: LegalHold) => {
    const parts: string[] = []
    if (h.subjects.length) parts.push(h.subjects.map(subjectText).join(', '))
    if (h.call_ids.length) parts.push(`${h.call_ids.length} call(s)`)
    if (h.from || h.to) parts.push(`${fromUnix(h.from) ?? 'the start'} to ${fromUnix(h.to) ?? 'now'}`)
    parts.push(h.classes.length ? h.classes.map(classLabel).join(', ') : 'every kind of data')
    return parts.join(' · ')
  }

  const columns: ColumnDef<LegalHold>[] = [
    {
      accessorKey: 'matter', header: 'Matter',
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.matter}</div>
          {row.original.reason && <div className="text-xs text-muted-foreground">{row.original.reason}</div>}
        </div>
      ),
    },
    { id: 'scope', header: 'Holds', cell: ({ row }) => <span className="text-sm">{scope(row.original)}</span> },
    {
      id: 'placed', header: 'Placed',
      cell: ({ row }) => (
        <div className="text-sm">{when(row.original.created_at)}
          <div className="text-xs text-muted-foreground">{row.original.created_by}</div>
        </div>
      ),
    },
    {
      id: 'status', header: 'Status',
      cell: ({ row }) => row.original.active
        ? <Badge variant="success">Active</Badge>
        : <Badge variant="secondary" title={row.original.release_reason ?? ''}>
            Released {when(row.original.released_at)}
          </Badge>,
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <div className="flex justify-end gap-1">
          <Button variant="ghost" size="icon" title="What it holds now" onClick={() => setViewing(row.original.id)}>
            <Eye className="h-4 w-4" />
          </Button>
          {row.original.active && (
            <Button variant="ghost" size="icon" title="Release"
              onClick={() => { setReleaseReason(''); setReleasing(row.original) }}>
              <Unlock className="h-4 w-4" />
            </Button>
          )}
        </div>
      ),
    },
  ]

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={all} onCheckedChange={setAll} /> Show released holds
        </label>
        <Button onClick={openPlace}><Plus className="mr-2 h-4 w-4" />Place hold</Button>
      </div>
      {holds.isLoading ? <Loading /> : holds.error ? <Failed error={holds.error} /> : (
        <DataTable columns={columns} data={holds.data ?? []} searchKey="matter" searchPlaceholder="Search matters..." />
      )}

      <Dialog open={placing} onOpenChange={(o) => !o && setPlacing(false)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Place a legal hold</DialogTitle>
            <DialogDescription>
              Nothing the hold covers is deleted, by retention, by a person or by an erasure, until it is released.
              Leave a part blank to hold everything along it.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={(e) => {
            e.preventDefault()
            if (!matter.trim()) return setFormError('A matter is required')
            place.mutate()
          }}>
            <div className="space-y-1">
              <Label htmlFor="matter">Matter *</Label>
              <Input id="matter" placeholder="e.g., Smith v. Acme, 2026-117" value={matter}
                onChange={(e) => setMatter(e.target.value)} />
            </div>
            <div className="space-y-1">
              <Label htmlFor="hold-reason">Reason</Label>
              <Input id="hold-reason" value={reason} onChange={(e) => setReason(e.target.value)} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1">
                <Label htmlFor="from">From</Label>
                <Input id="from" type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="to">Before</Label>
                <Input id="to" type="date" value={to} onChange={(e) => setTo(e.target.value)} />
              </div>
            </div>
            <div className="space-y-1">
              <Label>People</Label>
              <SubjectsInput subjects={subjects} onChange={setSubjects} />
            </div>
            <div className="space-y-1">
              <Label htmlFor="calls">Calls (ids)</Label>
              <Input id="calls" placeholder="call ids, separated by commas" value={callIds}
                onChange={(e) => setCallIds(e.target.value)} />
            </div>
            <div className="space-y-1">
              <Label>Kinds of data ({classes.size === 0 ? 'all' : classes.size})</Label>
              <div className="grid grid-cols-2 gap-1 rounded-md border p-3">
                {CLASSES.map((c) => (
                  <label key={c.value} className="flex items-center gap-2 text-sm">
                    <input type="checkbox" className="rounded border-input"
                      checked={classes.has(c.value)} onChange={() => toggleClass(c.value)} />
                    {c.label}
                  </label>
                ))}
              </div>
            </div>
            {formError && <p className="text-sm text-destructive">{formError}</p>}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setPlacing(false)}>Cancel</Button>
              <Button type="submit" disabled={place.isPending}>{place.isPending ? 'Placing...' : 'Place hold'}</Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!viewing} onOpenChange={(o) => !o && setViewing(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{detail.data?.matter ?? 'Hold'}</DialogTitle>
            <DialogDescription>{detail.data && scope(detail.data)}</DialogDescription>
          </DialogHeader>
          {detail.isLoading ? <Loading /> : detail.error ? <Failed error={detail.error} /> : (
            <div className="space-y-3 text-sm">
              <table className="w-full">
                <tbody>
                  {Object.entries(detail.data?.held ?? {}).map(([c, n]) => (
                    <tr key={c} className="border-t">
                      <td className="py-1">{classLabel(c)}</td>
                      <td className="text-right">{n == null ? 'too many to count' : n}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {(detail.data?.subject_keys.length ?? 0) > 0 && (
                <p className="text-xs text-muted-foreground">
                  Matched as: {detail.data!.subject_keys.join(', ')}
                </p>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>

      <Dialog open={!!releasing} onOpenChange={(o) => !o && setReleasing(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Release {releasing?.matter}</DialogTitle>
            <DialogDescription>
              What it held becomes subject to retention again; anything already past its days goes at the next sweep.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); release.mutate() }}>
            <div className="space-y-1">
              <Label htmlFor="release-reason">Reason *</Label>
              <Input id="release-reason" value={releaseReason} onChange={(e) => setReleaseReason(e.target.value)} />
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setReleasing(null)}>Cancel</Button>
              <Button type="submit" variant="destructive" disabled={!releaseReason.trim() || release.isPending}>
                Release
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}

// ─── Subject requests ─────────────────────────────────────────────────────────

const statusVariant = (r: SubjectRequest) => {
  if (r.status === 'failed') return 'destructive' as const
  if (r.status === 'completed') return 'success' as const
  if (r.status === 'preview') return 'default' as const
  return 'secondary' as const
}

const statusText = (r: SubjectRequest) =>
  r.status === 'preview' ? 'Awaiting confirmation' : r.status[0].toUpperCase() + r.status.slice(1)

/** What an erasure did, or would do, per kind of data. */
function ErasureTable({ report }: { report: NonNullable<SubjectRequest['preview']> }) {
  return (
    <div className="space-y-2">
      <table className="w-full text-sm">
        <thead>
          <tr className="text-left text-muted-foreground">
            <th className="py-1">Data</th><th>Done</th><th className="text-right">Items</th><th className="text-right">Held</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(report.classes).map(([c, r]) => (
            <tr key={c} className="border-t">
              <td className="py-1">{classLabel(c)}</td>
              <td>{r.treatment.replace('_', ' ')}</td>
              <td className="text-right">{r.items}</td>
              <td className="text-right">{r.held ?? 0}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {Object.entries(report.kept ?? {}).map(([k, why]) => (
        <p key={k} className="text-xs text-muted-foreground">Kept, {k}: {why}</p>
      ))}
      {Object.entries(report.held_by ?? {}).map(([id, m]) => (
        <p key={id} className="text-xs text-muted-foreground">Held by {m}</p>
      ))}
      {Object.entries(report.not_reached ?? {}).map(([k, why]) => (
        <p key={k} className="text-xs text-muted-foreground">Not reached, {k}: {why}</p>
      ))}
    </div>
  )
}

function RequestDetail({ id, onClose }: { id: string; onClose: () => void }) {
  const queryClient = useQueryClient()
  const request = useQuery({
    queryKey: ['compliance', 'request', id],
    queryFn: () => compliance.subjectRequest(id),
    // Received and running requests move on by themselves.
    refetchInterval: (q) => (['received', 'running', 'confirmed'].includes(q.state.data?.status ?? '') ? 3000 : false),
  })
  const confirm_ = useMutation({
    mutationFn: () => compliance.confirmErasure(id),
    onSuccess: (r) => {
      queryClient.setQueryData(['compliance', 'request', id], r)
      queryClient.invalidateQueries({ queryKey: ['compliance', 'requests'] })
    },
    onError: (e) => alert(errorText(e)),
  })
  const download = useMutation({
    mutationFn: () => compliance.bundleUrl(id),
    onSuccess: (url) => {
      const a = document.createElement('a')
      a.href = url
      a.download = `subject-export-${id}.tar.gz`
      a.click()
      setTimeout(() => URL.revokeObjectURL(url), 10_000)
    },
    onError: (e) => alert(errorText(e)),
  })
  const r = request.data
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{r ? `${r.kind === 'export' ? 'Export' : 'Erasure'} for ${r.subjects.map(subjectText).join(', ')}` : 'Request'}</DialogTitle>
          <DialogDescription>{r?.reference}</DialogDescription>
        </DialogHeader>
        {request.isLoading ? <Loading /> : request.error ? <Failed error={request.error} /> : r && (
          <div className="space-y-4 text-sm">
            <div className="flex flex-wrap gap-2 items-center">
              <Badge variant={statusVariant(r)}>{statusText(r)}</Badge>
              {r.overdue && <Badge variant="destructive">Overdue</Badge>}
              <span className="text-muted-foreground">
                Received {when(r.received_at)} by {r.requested_by} · due {when(r.due_at)}
              </span>
            </div>
            {r.error && <Failed error={new Error(r.error)} />}
            {r.expansion && (
              <p className="text-xs text-muted-foreground">Found as: {r.expansion.identifiers.join(', ')}</p>
            )}

            {r.kind === 'export' && r.report && (
              <div className="space-y-2">
                <table className="w-full">
                  <thead>
                    <tr className="text-left text-muted-foreground">
                      <th className="py-1">Data</th><th className="text-right">Items</th><th className="text-right">Files</th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(r.report.classes).map(([c, k]) => (
                      <tr key={c} className="border-t">
                        <td className="py-1">{classLabel(c)}</td>
                        <td className="text-right">{k.items}</td>
                        <td className="text-right">{k.files ?? 0}{(k.missing_files?.length ?? 0) > 0 && ` (${k.missing_files!.length} missing)`}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {(r.report.review?.length ?? 0) > 0 && (
                  <div className="rounded-md border border-yellow-500/50 p-3">
                    <p className="font-medium flex items-center gap-1">
                      <AlertTriangle className="h-4 w-4 text-yellow-600" />
                      Review before handing over: other people are heard in
                    </p>
                    <ul className="mt-1 font-mono text-xs">{r.report.review!.map((f) => <li key={f}>{f}</li>)}</ul>
                  </div>
                )}
                {Object.entries(r.report.not_exported).map(([k, why]) => (
                  <p key={k} className="text-xs text-muted-foreground">Not exported, {k}: {why}</p>
                ))}
              </div>
            )}
            {r.kind === 'export' && r.bundle && (
              <div className="flex items-center justify-between rounded-md border p-3">
                <div>
                  <div>{(r.bundle.size_bytes / 1_048_576).toFixed(1)} MB · expires {when(r.bundle.expires_at)}</div>
                  <div className="font-mono text-xs text-muted-foreground break-all">sha256 {r.bundle.sha256}</div>
                </div>
                <Button disabled={r.bundle.removed || download.isPending} onClick={() => download.mutate()}
                  title="Each download is in the audit log">
                  <Download className="mr-2 h-4 w-4" />{r.bundle.removed ? 'Expired' : 'Download'}
                </Button>
              </div>
            )}

            {r.kind === 'erasure' && r.status === 'preview' && r.preview && (
              <div className="space-y-3">
                <p>Nothing has been erased yet. Confirm to do this:</p>
                <ErasureTable report={r.preview} />
                <div className="flex justify-end">
                  <Button variant="destructive" disabled={confirm_.isPending} onClick={() => {
                    if (confirm('Erase this person\'s data as shown? It cannot be undone.')) confirm_.mutate()
                  }}>
                    <Trash2 className="mr-2 h-4 w-4" />Confirm erasure
                  </Button>
                </div>
              </div>
            )}
            {r.erasure && (
              <div className="space-y-2">
                <p className="font-medium">Erased {when(r.completed_at)}{r.confirmed_by && `, confirmed by ${r.confirmed_by}`}</p>
                <ErasureTable report={r.erasure} />
                {Object.entries(r.erasure.tables ?? {}).length > 0 && (
                  <p className="text-xs text-muted-foreground">
                    Rows removed: {Object.entries(r.erasure.tables!).map(([t, n]) => `${t} ${n}`).join(', ')}
                  </p>
                )}
              </div>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function RequestsTab() {
  const queryClient = useQueryClient()
  const [filing, setFiling] = useState(false)
  const [viewing, setViewing] = useState<string | null>(null)
  const [kind, setKind] = useState<'export' | 'erasure'>('export')
  const [subjects, setSubjects] = useState<SubjectIdentifier[]>([])
  const [reference, setReference] = useState('')
  const [formError, setFormError] = useState<string | null>(null)

  const requests = useQuery({
    queryKey: ['compliance', 'requests'],
    queryFn: compliance.subjectRequests,
    refetchInterval: 10_000,
  })
  const file = useMutation({
    mutationFn: () => compliance.fileSubjectRequest({ kind, subjects, reference: reference.trim() || null }),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ['compliance', 'requests'] })
      setFiling(false)
      setViewing(r.id)
    },
    onError: (e) => setFormError(errorText(e)),
  })

  const columns: ColumnDef<SubjectRequest>[] = [
    {
      id: 'subjects', header: 'For',
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.subjects.map(subjectText).join(', ')}</div>
          {row.original.reference && <div className="text-xs text-muted-foreground">{row.original.reference}</div>}
        </div>
      ),
    },
    {
      accessorKey: 'kind', header: 'Kind',
      cell: ({ row }) => <Badge variant="outline">{row.original.kind === 'export' ? 'Export' : 'Erasure'}</Badge>,
    },
    {
      accessorKey: 'status', header: 'Status',
      cell: ({ row }) => (
        <div className="flex gap-1">
          <Badge variant={statusVariant(row.original)}>{statusText(row.original)}</Badge>
          {row.original.overdue && <Badge variant="destructive">Overdue</Badge>}
        </div>
      ),
    },
    { accessorKey: 'received_at', header: 'Received', cell: ({ row }) => when(row.original.received_at) },
    { accessorKey: 'due_at', header: 'Due', cell: ({ row }) => when(row.original.due_at) },
    {
      id: 'actions',
      cell: ({ row }) => (
        <div className="flex justify-end">
          <Button variant="ghost" size="icon" title="Open" onClick={() => setViewing(row.original.id)}>
            <Eye className="h-4 w-4" />
          </Button>
        </div>
      ),
    },
  ]

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          A person's request for their data, or for it to be erased. An erasure shows what it would do and waits
          for a confirmation.
        </p>
        <Button onClick={() => {
          setKind('export'); setSubjects([]); setReference(''); setFormError(null); setFiling(true)
        }}>
          <Plus className="mr-2 h-4 w-4" />File request
        </Button>
      </div>
      {requests.isLoading ? <Loading /> : requests.error ? <Failed error={requests.error} /> : (
        <DataTable columns={columns} data={requests.data ?? []} />
      )}

      <Dialog open={filing} onOpenChange={(o) => !o && setFiling(false)}>
        <DialogContent className="max-w-xl">
          <DialogHeader>
            <DialogTitle>File a subject request</DialogTitle>
            <DialogDescription>
              Name the person every way they are known here: their user, numbers, addresses.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={(e) => {
            e.preventDefault()
            if (subjects.length === 0) return setFormError('Add the person the request is about')
            file.mutate()
          }}>
            <div className="space-y-1">
              <Label>Kind</Label>
              <Select value={kind} onValueChange={(v) => setKind(v as 'export' | 'erasure')}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="export">Export: a copy of their data</SelectItem>
                  <SelectItem value="erasure">Erasure: their data removed</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1">
              <Label>The person *</Label>
              <SubjectsInput subjects={subjects} onChange={setSubjects} />
            </div>
            <div className="space-y-1">
              <Label htmlFor="reference">Your reference</Label>
              <Input id="reference" placeholder="a ticket or a letter" value={reference}
                onChange={(e) => setReference(e.target.value)} />
            </div>
            {formError && <p className="text-sm text-destructive">{formError}</p>}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setFiling(false)}>Cancel</Button>
              <Button type="submit" disabled={file.isPending}>{file.isPending ? 'Filing...' : 'File'}</Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      {viewing && <RequestDetail id={viewing} onClose={() => setViewing(null)} />}
    </div>
  )
}

/** Retention and consent, legal holds, and people's requests for their data. */
export function CompliancePage() {
  const [tab, setTab] = useState<Tab>('policy')
  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Compliance</h1>
        <p className="text-muted-foreground">
          How long data is kept, what callers are told about recording, what is held for a matter, and people's
          requests for their own data. Deletions are in the audit log.
        </p>
      </div>
      <div className="flex gap-2 flex-wrap">
        {TABS.map(({ id, label }) => (
          <Button key={id} variant={tab === id ? 'default' : 'outline'} size="sm" onClick={() => setTab(id)}>
            {label}
          </Button>
        ))}
      </div>
      {tab === 'policy' && <PolicyTab />}
      {tab === 'retention' && <RetentionTab />}
      {tab === 'holds' && <HoldsTab />}
      {tab === 'requests' && <RequestsTab />}
    </div>
  )
}
