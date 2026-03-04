import { useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import { me } from '@/api/endpoints'
import type { MeForwarding } from '@/types'

interface ForwardingState {
  forward_unconditional: string
  forward_on_busy: string
  forward_on_no_answer: string
  no_answer_timeout_secs: number
  fwd_all_enabled: boolean
  fwd_busy_enabled: boolean
  fwd_noanswer_enabled: boolean
}

function stateFromBinding(b: MeForwarding): ForwardingState {
  return {
    forward_unconditional: b.forward_unconditional || '',
    forward_on_busy: b.forward_on_busy || '',
    forward_on_no_answer: b.forward_on_no_answer || '',
    no_answer_timeout_secs: b.no_answer_timeout_secs,
    fwd_all_enabled: !!b.forward_unconditional,
    fwd_busy_enabled: !!b.forward_on_busy,
    fwd_noanswer_enabled: !!b.forward_on_no_answer,
  }
}

export function UserForwardingPage() {
  const [bindings, setBindings] = useState<MeForwarding[]>([])
  const [forms, setForms] = useState<Record<string, ForwardingState>>({})
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState<Record<string, boolean>>({})
  const [messages, setMessages] = useState<Record<string, { type: 'success' | 'error'; text: string }>>({})

  useEffect(() => {
    async function load() {
      try {
        const data = await me.getForwarding()
        setBindings(data)
        const formState: Record<string, ForwardingState> = {}
        for (const b of data) {
          formState[b.id] = stateFromBinding(b)
        }
        setForms(formState)
      } catch {
        // no forwarding
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const updateForm = (id: string, patch: Partial<ForwardingState>) => {
    setForms((prev) => ({ ...prev, [id]: { ...prev[id], ...patch } }))
  }

  const handleSave = async (id: string) => {
    const form = forms[id]
    if (!form) return

    setSaving((prev) => ({ ...prev, [id]: true }))
    setMessages((prev) => {
      const next = { ...prev }
      delete next[id]
      return next
    })

    try {
      const updated = await me.updateForwarding(id, {
        forward_unconditional: form.fwd_all_enabled ? form.forward_unconditional : '',
        forward_on_busy: form.fwd_busy_enabled ? form.forward_on_busy : '',
        forward_on_no_answer: form.fwd_noanswer_enabled ? form.forward_on_no_answer : '',
        no_answer_timeout_secs: form.no_answer_timeout_secs,
      })
      setBindings((prev) => prev.map((b) => (b.id === id ? updated : b)))
      setForms((prev) => ({ ...prev, [id]: stateFromBinding(updated) }))
      setMessages((prev) => ({ ...prev, [id]: { type: 'success', text: 'Forwarding updated.' } }))
    } catch (err) {
      setMessages((prev) => ({
        ...prev,
        [id]: { type: 'error', text: err instanceof Error ? err.message : 'Failed to update' },
      }))
    } finally {
      setSaving((prev) => ({ ...prev, [id]: false }))
    }
  }

  if (loading) {
    return (
      <div className="flex h-32 items-center justify-center text-muted-foreground">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Call Forwarding</h1>
        <p className="text-muted-foreground">Configure where your calls are forwarded.</p>
      </div>

      {bindings.length === 0 ? (
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            No phone lines are assigned to your account.
          </CardContent>
        </Card>
      ) : (
        bindings.map((binding) => {
          const form = forms[binding.id]
          if (!form) return null
          const msg = messages[binding.id]

          return (
            <Card key={binding.id}>
              <CardHeader>
                <CardTitle className="text-base">Line {binding.phone_number || binding.phone_number_id}</CardTitle>
                <CardDescription>Configure call forwarding for this line.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-6">
                {msg && (
                  <div className={cn(
                    'rounded-md p-3 text-sm',
                    msg.type === 'success' ? 'bg-green-500/15 text-green-700 dark:text-green-400' : 'bg-destructive/15 text-destructive',
                  )}>
                    {msg.text}
                  </div>
                )}

                {/* Forward All Calls */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <Label className="text-sm font-medium">Forward All Calls</Label>
                      <p className="text-xs text-muted-foreground">Redirect all incoming calls unconditionally.</p>
                    </div>
                    <Switch
                      checked={form.fwd_all_enabled}
                      onCheckedChange={(checked) => updateForm(binding.id, { fwd_all_enabled: checked })}
                    />
                  </div>
                  {form.fwd_all_enabled && (
                    <Input
                      placeholder="Destination (e.g., +15551234567)"
                      value={form.forward_unconditional}
                      onChange={(e) => updateForm(binding.id, { forward_unconditional: e.target.value })}
                    />
                  )}
                </div>

                {/* Forward on Busy */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <Label className="text-sm font-medium">Forward on Busy</Label>
                      <p className="text-xs text-muted-foreground">Redirect calls when you&apos;re on another call.</p>
                    </div>
                    <Switch
                      checked={form.fwd_busy_enabled}
                      onCheckedChange={(checked) => updateForm(binding.id, { fwd_busy_enabled: checked })}
                    />
                  </div>
                  {form.fwd_busy_enabled && (
                    <Input
                      placeholder="Destination (e.g., +15551234567)"
                      value={form.forward_on_busy}
                      onChange={(e) => updateForm(binding.id, { forward_on_busy: e.target.value })}
                    />
                  )}
                </div>

                {/* Forward on No Answer */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <Label className="text-sm font-medium">Forward on No Answer</Label>
                      <p className="text-xs text-muted-foreground">Redirect calls after the ring timeout.</p>
                    </div>
                    <Switch
                      checked={form.fwd_noanswer_enabled}
                      onCheckedChange={(checked) => updateForm(binding.id, { fwd_noanswer_enabled: checked })}
                    />
                  </div>
                  {form.fwd_noanswer_enabled && (
                    <div className="space-y-3">
                      <Input
                        placeholder="Destination (e.g., +15551234567)"
                        value={form.forward_on_no_answer}
                        onChange={(e) => updateForm(binding.id, { forward_on_no_answer: e.target.value })}
                      />
                      <div className="space-y-1">
                        <Label className="text-xs">No-answer timeout (seconds)</Label>
                        <Input
                          type="number"
                          min={5}
                          max={120}
                          value={form.no_answer_timeout_secs}
                          onChange={(e) => updateForm(binding.id, { no_answer_timeout_secs: parseInt(e.target.value) || 30 })}
                          className="w-24"
                        />
                      </div>
                    </div>
                  )}
                </div>

                <Button onClick={() => handleSave(binding.id)} disabled={saving[binding.id]}>
                  {saving[binding.id] ? 'Saving...' : 'Save Changes'}
                </Button>
              </CardContent>
            </Card>
          )
        })
      )}
    </div>
  )
}
