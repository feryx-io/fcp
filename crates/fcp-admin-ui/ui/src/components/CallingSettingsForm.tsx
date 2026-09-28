import { useEffect, useState } from 'react'
import { Voicemail } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'
import type { CallingSettings, FindMe, UpdateCallingRequest } from '@/types'

const VM = 'voicemail'

type ForwardKey = 'forward_all' | 'forward_busy' | 'forward_no_answer' | 'forward_unreachable'

interface ForwardState {
  enabled: boolean
  voicemail: boolean
  target: string
}

interface FormState {
  dnd: boolean
  call_waiting: boolean
  /** '' is the node's default */
  no_answer_secs: string
  /** 'tenant', 'none' or a prompt's id */
  hold_music: string
  after_hours_schedule_id: string
  after_hours_forward: string
  find_me: FindMe
  forwards: Record<ForwardKey, ForwardState>
  /** '' is the tenant's default class */
  class_of_service_id: string
}

const FORWARDS: { key: ForwardKey; label: string; hint: string }[] = [
  { key: 'forward_all', label: 'Forward All Calls', hint: 'Every call, without ringing your phones.' },
  { key: 'forward_busy', label: 'Forward on Busy', hint: 'When your phones refuse the call, or you are in do not disturb.' },
  { key: 'forward_no_answer', label: 'Forward on No Answer', hint: 'When nobody answers within the ring time.' },
  { key: 'forward_unreachable', label: 'Forward when Unreachable', hint: 'When none of your phones is registered.' },
]

/** A target as the API shows it, for the field: `number:+1…` is `+1…`, `ext:2001` is `2001`, a SIP URI stays. */
function fieldOf(target: string): string {
  if (target.startsWith('number:')) return target.slice('number:'.length)
  if (target.startsWith('ext:')) return target.slice('ext:'.length)
  return target
}

function forwardState(target: string | null): ForwardState {
  if (!target) return { enabled: false, voicemail: false, target: '' }
  const voicemail = target.toLowerCase() === VM
  return { enabled: true, voicemail, target: voicemail ? '' : fieldOf(target) }
}

function stateOf(s: CallingSettings): FormState {
  return {
    dnd: s.dnd,
    call_waiting: s.call_waiting,
    no_answer_secs: s.no_answer_secs === null ? '' : String(s.no_answer_secs),
    hold_music: s.hold_music ?? 'tenant',
    after_hours_schedule_id: s.after_hours_schedule_id ?? '',
    after_hours_forward: s.after_hours_forward ? fieldOf(s.after_hours_forward) : '',
    find_me: s.find_me ?? { enabled: false, steps: [] },
    class_of_service_id: s.class_of_service_id ?? '',
    forwards: {
      forward_all: forwardState(s.forward_all),
      forward_busy: forwardState(s.forward_busy),
      forward_no_answer: forwardState(s.forward_no_answer),
      forward_unreachable: forwardState(s.forward_unreachable),
    },
  }
}

/** What to send: a forward off is an empty string, which clears it. */
export function requestOf(form: FormState): UpdateCallingRequest {
  const forward = (f: ForwardState) => (f.enabled ? (f.voicemail ? VM : f.target.trim()) : '')
  return {
    dnd: form.dnd,
    call_waiting: form.call_waiting,
    no_answer_secs: form.no_answer_secs.trim() === '' ? 0 : parseInt(form.no_answer_secs, 10) || 0,
    hold_music: form.hold_music === 'tenant' ? '' : form.hold_music,
    after_hours_schedule_id: form.after_hours_schedule_id,
    after_hours_forward: form.after_hours_forward.trim(),
    find_me: {
      enabled: form.find_me.enabled,
      steps: form.find_me.steps.map((st) => ({
        ...st,
        targets: st.targets.map((t) => t.trim()).filter(Boolean),
      })),
    },
    forward_all: forward(form.forwards.forward_all),
    forward_busy: forward(form.forwards.forward_busy),
    forward_no_answer: forward(form.forwards.forward_no_answer),
    forward_unreachable: forward(form.forwards.forward_unreachable),
  }
}

/**
 * A user's calling settings — do not disturb, the four forwards and the
 * ring time (docs/ENTERPRISE_CALLING.md §3.1) — edited by the user on
 * their Calling page and by an admin on the user's dialog.
 */
export function CallingSettingsForm({
  settings,
  onSave,
  saving,
  message,
  holdMusicPrompts,
  schedules,
  classes,
}: {
  settings: CallingSettings
  onSave: (req: UpdateCallingRequest) => void
  saving: boolean
  message?: { type: 'success' | 'error'; text: string } | null
  /** The tenant's prompts to choose hold music from (an admin sees them). */
  holdMusicPrompts?: { id: string; name: string }[]
  /** The tenant's schedules for after-hours (an admin sees them). */
  schedules?: { id: string; name: string }[]
  /** The tenant's classes of service; given, an admin sets the user's. */
  classes?: { id: string; name: string; is_default: boolean }[]
}) {
  const [form, setForm] = useState<FormState>(() => stateOf(settings))
  useEffect(() => {
    setForm(stateOf(settings))
  }, [settings])

  const updateForward = (key: ForwardKey, patch: Partial<ForwardState>) =>
    setForm((prev) => ({ ...prev, forwards: { ...prev.forwards, [key]: { ...prev.forwards[key], ...patch } } }))

  return (
    <div className="space-y-6">
      {message && (
        <div
          className={cn(
            'rounded-md p-3 text-sm',
            message.type === 'success'
              ? 'bg-green-500/15 text-green-700 dark:text-green-400'
              : 'bg-destructive/15 text-destructive',
          )}
        >
          {message.text}
        </div>
      )}

      <div className="flex items-center justify-between">
        <div>
          <Label className="text-sm font-medium">Do Not Disturb</Label>
          <p className="text-xs text-muted-foreground">
            Calls do not ring; they get the busy forward, voicemail, or a busy signal.
          </p>
        </div>
        <Switch checked={form.dnd} onCheckedChange={(dnd) => setForm((prev) => ({ ...prev, dnd }))} />
      </div>

      <div className="flex items-center justify-between">
        <div>
          <Label className="text-sm font-medium">Call Waiting</Label>
          <p className="text-xs text-muted-foreground">
            A second call rings while you are on one; off, it gets the busy forward, voicemail, or a busy signal.
          </p>
        </div>
        <Switch
          checked={form.call_waiting}
          onCheckedChange={(call_waiting) => setForm((prev) => ({ ...prev, call_waiting }))}
        />
      </div>

      <div className="space-y-1">
        <Label className="text-sm font-medium">Ring time (seconds)</Label>
        <p className="text-xs text-muted-foreground">
          How long your phones ring before the no-answer forward or voicemail. Blank for the system default.
        </p>
        <Input
          type="number"
          min={5}
          max={300}
          placeholder="default"
          value={form.no_answer_secs}
          onChange={(e) => setForm((prev) => ({ ...prev, no_answer_secs: e.target.value }))}
          className="w-28"
        />
      </div>

      {classes && (
        <div className="space-y-1">
          <Label className="text-sm font-medium">Class of service</Label>
          <p className="text-xs text-muted-foreground">
            What the user may dial, how many calls they may be on, and how their caller ID leaves.
          </p>
          <select
            className="flex h-9 w-64 rounded-md border border-input bg-transparent px-3 py-1 text-sm"
            value={form.class_of_service_id}
            onChange={(e) => setForm((prev) => ({ ...prev, class_of_service_id: e.target.value }))}
          >
            <option value="">
              {classes.find((c) => c.is_default)
                ? `The default (${classes.find((c) => c.is_default)!.name})`
                : 'No restriction (no default class)'}
            </option>
            {classes.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </div>
      )}

      {schedules && (
        <div className="space-y-2">
          <Label className="text-sm font-medium">After hours</Label>
          <p className="text-xs text-muted-foreground">
            Outside this schedule&apos;s hours, calls go to the target without ringing.
          </p>
          <div className="flex flex-wrap gap-2">
            <select
              className="flex h-9 w-56 rounded-md border border-input bg-transparent px-3 py-1 text-sm"
              value={form.after_hours_schedule_id}
              onChange={(e) => setForm((prev) => ({ ...prev, after_hours_schedule_id: e.target.value }))}
            >
              <option value="">No after-hours schedule</option>
              {schedules.map((sch) => (
                <option key={sch.id} value={sch.id}>{sch.name}</option>
              ))}
            </select>
            <Input
              className="w-64"
              placeholder="voicemail, an extension, +number"
              value={form.after_hours_forward}
              onChange={(e) => setForm((prev) => ({ ...prev, after_hours_forward: e.target.value }))}
            />
          </div>
        </div>
      )}

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <div>
            <Label className="text-sm font-medium">Find Me</Label>
            <p className="text-xs text-muted-foreground">
              When your phones are not answered, ring these one after another before the no-answer forward. Targets:
              devices (your phones), an extension, a +number. Confirm asks whoever answers to press 1.
            </p>
          </div>
          <Switch
            checked={form.find_me.enabled}
            onCheckedChange={(enabled) => setForm((prev) => ({ ...prev, find_me: { ...prev.find_me, enabled } }))}
          />
        </div>
        {form.find_me.enabled && (
          <div className="space-y-2">
            {form.find_me.steps.map((st, i) => (
              <div key={i} className="flex flex-wrap items-center gap-2 rounded-md border p-2">
                <span className="text-xs text-muted-foreground">Step {i + 1}</span>
                <Input
                  className="w-64"
                  placeholder="devices, 2002, +15551234567"
                  value={st.targets.join(', ')}
                  onChange={(e) =>
                    setForm((prev) => ({
                      ...prev,
                      find_me: {
                        ...prev.find_me,
                        steps: prev.find_me.steps.map((x, j) => (j === i ? { ...x, targets: e.target.value.split(',') } : x)),
                      },
                    }))
                  }
                />
                <Input
                  type="number"
                  min={5}
                  max={300}
                  className="w-20"
                  value={st.ring_secs}
                  onChange={(e) =>
                    setForm((prev) => ({
                      ...prev,
                      find_me: {
                        ...prev.find_me,
                        steps: prev.find_me.steps.map((x, j) => (j === i ? { ...x, ring_secs: parseInt(e.target.value) || 20 } : x)),
                      },
                    }))
                  }
                />
                <span className="text-xs">s</span>
                <label className="flex items-center gap-1 text-xs">
                  <input
                    type="checkbox"
                    checked={st.confirm}
                    onChange={(e) =>
                      setForm((prev) => ({
                        ...prev,
                        find_me: {
                          ...prev.find_me,
                          steps: prev.find_me.steps.map((x, j) => (j === i ? { ...x, confirm: e.target.checked } : x)),
                        },
                      }))
                    }
                  />
                  Confirm
                </label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    setForm((prev) => ({
                      ...prev,
                      find_me: { ...prev.find_me, steps: prev.find_me.steps.filter((_, j) => j !== i) },
                    }))
                  }
                >
                  Remove
                </Button>
              </div>
            ))}
            {form.find_me.steps.length < 5 && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  setForm((prev) => ({
                    ...prev,
                    find_me: {
                      ...prev.find_me,
                      steps: [...prev.find_me.steps, { targets: [''], ring_secs: 20, confirm: false }],
                    },
                  }))
                }
              >
                Add step
              </Button>
            )}
          </div>
        )}
      </div>

      <div className="space-y-1">
        <Label className="text-sm font-medium">Hold music</Label>
        <p className="text-xs text-muted-foreground">What a caller you put on hold hears.</p>
        <Select value={form.hold_music} onValueChange={(hold_music) => setForm((prev) => ({ ...prev, hold_music }))}>
          <SelectTrigger className="w-64">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="tenant">The organisation&apos;s music</SelectItem>
            <SelectItem value="none">Silence</SelectItem>
            {(holdMusicPrompts ?? []).map((p) => (
              <SelectItem key={p.id} value={p.id}>{p.name}</SelectItem>
            ))}
            {form.hold_music !== 'tenant' && form.hold_music !== 'none'
              && !(holdMusicPrompts ?? []).some((p) => p.id === form.hold_music) && (
              <SelectItem value={form.hold_music}>A chosen recording</SelectItem>
            )}
          </SelectContent>
        </Select>
      </div>

      {FORWARDS.map(({ key, label, hint }) => {
        const f = form.forwards[key]
        return (
          <div key={key} className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <Label className="text-sm font-medium">{label}</Label>
                <p className="text-xs text-muted-foreground">{hint}</p>
              </div>
              <Switch checked={f.enabled} onCheckedChange={(enabled) => updateForward(key, { enabled })} />
            </div>
            {f.enabled && (
              <div className="space-y-2">
                <label className="flex cursor-pointer items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={f.voicemail}
                    onChange={(e) => updateForward(key, { voicemail: e.target.checked })}
                    className="rounded border-input"
                  />
                  <Voicemail className="h-4 w-4 text-muted-foreground" />
                  Send to voicemail
                </label>
                <Input
                  placeholder="Extension, +number or SIP URI"
                  value={f.voicemail ? '' : f.target}
                  onChange={(e) => updateForward(key, { target: e.target.value })}
                  disabled={f.voicemail}
                  className={f.voicemail ? 'opacity-50' : ''}
                />
              </div>
            )}
          </div>
        )
      })}

      <Button
        onClick={() =>
          onSave(classes ? { ...requestOf(form), class_of_service_id: form.class_of_service_id } : requestOf(form))
        }
        disabled={saving}
      >
        {saving ? 'Saving...' : 'Save Changes'}
      </Button>
    </div>
  )
}
