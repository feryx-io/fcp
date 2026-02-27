import { useState, useEffect } from 'react'
import { useThemeStore } from '@/stores/theme'
import { useAuthStore } from '@/stores/auth'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { Badge } from '@/components/ui/badge'
import { config } from '@/api/endpoints'

export function SettingsPage() {
  const { theme, setTheme } = useThemeStore()
  const { user } = useAuthStore()

  const [vmUri, setVmUri] = useState('')
  const [vmUriSaved, setVmUriSaved] = useState('')
  const [vmLoading, setVmLoading] = useState(true)
  const [vmSaving, setVmSaving] = useState(false)
  const [vmError, setVmError] = useState<string | null>(null)
  const [vmSuccess, setVmSuccess] = useState(false)

  useEffect(() => {
    config.get('call-manager', 'b2bua')
      .then(entry => {
        const uri = (entry.value?.voicemail_server_uri as string) || ''
        setVmUri(uri)
        setVmUriSaved(uri)
      })
      .catch(() => {
        // Config entry may not exist yet — that's fine
      })
      .finally(() => setVmLoading(false))
  }, [])

  const saveVmUri = async () => {
    setVmSaving(true)
    setVmError(null)
    setVmSuccess(false)
    try {
      // Read current config to preserve other fields
      let currentValue: Record<string, unknown> = {}
      try {
        const entry = await config.get('call-manager', 'b2bua')
        currentValue = entry.value || {}
      } catch {
        // No existing config — start fresh
      }
      const updatedValue = { ...currentValue, voicemail_server_uri: vmUri }
      await config.update('call-manager', 'b2bua', updatedValue)
      setVmUriSaved(vmUri)
      setVmSuccess(true)
      setTimeout(() => setVmSuccess(false), 3000)
    } catch (err) {
      setVmError((err as Error).message || 'Failed to save')
    } finally {
      setVmSaving(false)
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Settings</h1>
        <p className="text-muted-foreground">Manage your preferences</p>
      </div>

      <div className="grid gap-6 md:grid-cols-2">
        {/* Appearance */}
        <Card>
          <CardHeader>
            <CardTitle>Appearance</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label>Theme</Label>
              <Select value={theme} onValueChange={(v) => setTheme(v as 'light' | 'dark' | 'system')}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="light">Light</SelectItem>
                  <SelectItem value="dark">Dark</SelectItem>
                  <SelectItem value="system">System</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                System follows your OS preference.
              </p>
            </div>
          </CardContent>
        </Card>

        {/* Account Info */}
        <Card>
          <CardHeader>
            <CardTitle>Account</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Username</span>
              <span className="font-medium">{user?.username ?? '-'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Display Name</span>
              <span className="font-medium">{user?.display_name ?? '-'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Role</span>
              <Badge variant="outline">{user?.role ?? '-'}</Badge>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Tenant</span>
              <span className="font-mono text-xs">{user?.tenant_id ?? '-'}</span>
            </div>
          </CardContent>
        </Card>

        {/* Telephony */}
        <Card>
          <CardHeader>
            <CardTitle>Telephony</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="vm-uri">Voicemail Server URI</Label>
              {vmLoading ? (
                <p className="text-sm text-muted-foreground">Loading...</p>
              ) : (
                <>
                  <div className="flex gap-2">
                    <Input
                      id="vm-uri"
                      placeholder="sip:vm@host:port"
                      value={vmUri}
                      onChange={e => { setVmUri(e.target.value); setVmSuccess(false); setVmError(null) }}
                    />
                    <Button
                      onClick={saveVmUri}
                      disabled={vmSaving || vmUri === vmUriSaved}
                    >
                      {vmSaving ? 'Saving...' : 'Save'}
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    SIP URI of the voicemail server for call-forward-on-no-answer routing.
                  </p>
                  {vmError && <p className="text-sm text-destructive">{vmError}</p>}
                  {vmSuccess && <p className="text-sm text-green-600">Saved successfully.</p>}
                </>
              )}
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
