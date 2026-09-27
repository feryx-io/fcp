import { useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { CallingSettingsForm } from '@/components/CallingSettingsForm'
import { me } from '@/api/endpoints'
import type { CallingSettings, UpdateCallingRequest } from '@/types'

/** The user's own calling settings (docs/ENTERPRISE_CALLING.md §3.1). */
export function UserCallingPage() {
  const [settings, setSettings] = useState<CallingSettings | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)

  useEffect(() => {
    async function load() {
      try {
        setSettings(await me.getCalling())
      } catch (err) {
        setMessage({ type: 'error', text: err instanceof Error ? err.message : 'Failed to load' })
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const handleSave = async (req: UpdateCallingRequest) => {
    setSaving(true)
    setMessage(null)
    try {
      setSettings(await me.updateCalling(req))
      setMessage({ type: 'success', text: 'Calling settings updated.' })
    } catch (err) {
      setMessage({ type: 'error', text: err instanceof Error ? err.message : 'Failed to update' })
    } finally {
      setSaving(false)
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
        <h1 className="text-2xl font-bold">Calling</h1>
        <p className="text-muted-foreground">How your calls ring, and where they go when you do not answer.</p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Your calls</CardTitle>
          <CardDescription>
            These apply to every call for you, whether it dials your number, your extension or comes from a menu.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {settings ? (
            <CallingSettingsForm settings={settings} onSave={handleSave} saving={saving} message={message} />
          ) : (
            <p className="text-sm text-destructive">{message?.text ?? 'Settings unavailable.'}</p>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
