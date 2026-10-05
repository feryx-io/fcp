import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Laptop, Phone, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { me } from '@/api/endpoints'
import type { UserDevice } from '@/types'

function when(at?: string | null) {
  return at ? new Date(at).toLocaleString() : '—'
}

/**
 * Your devices (docs/SOFTPHONE.md §2.1): desk phones, and every computer or
 * phone the app is signed in on. Removing one signs the app out there and
 * stops it ringing.
 */
export function UserDevicesPage() {
  const queryClient = useQueryClient()
  const [error, setError] = useState<string | null>(null)
  const { data: devices = [], isLoading } = useQuery({
    queryKey: ['me', 'devices'],
    queryFn: me.listDevices,
  })
  const remove = useMutation({
    mutationFn: me.removeDevice,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['me', 'devices'] }),
    onError: (e) => setError(e instanceof Error ? e.message : 'Request failed'),
  })

  const confirmRemove = (d: UserDevice) => {
    const what = d.app ? `Sign the app out on ${d.name} and remove it?` : `Remove ${d.name}?`
    if (window.confirm(what)) remove.mutate(d.id)
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Devices</h1>
        <p className="text-muted-foreground">
          Your phones, and everywhere the app is signed in. Remove one you no longer use or have lost.
        </p>
      </div>
      {error && <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">{error}</div>}
      <Card>
        <CardHeader>
          <CardTitle>Your devices</CardTitle>
          <CardDescription>Each rings when you are called.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
          {!isLoading && devices.length === 0 && (
            <p className="text-sm text-muted-foreground">No devices yet.</p>
          )}
          {devices.map((d) => (
            <div key={d.id} className="flex items-center justify-between rounded-md border p-3">
              <div className="flex items-center gap-3">
                {d.app ? <Laptop className="h-5 w-5" /> : <Phone className="h-5 w-5" />}
                <div>
                  <div className="font-medium">{d.name}</div>
                  <div className="text-sm text-muted-foreground">
                    {d.app
                      ? [d.app.platform, d.app.version && `app ${d.app.version}`].filter(Boolean).join(' · ') ||
                        'The app'
                      : d.model || d.device_type}
                    {' · '}
                    {d.registration_state === 'registered'
                      ? `registered ${when(d.last_registration)}`
                      : d.registration_state}
                  </div>
                  {d.app && (
                    <div className="text-xs text-muted-foreground">
                      {d.app.signed_in
                        ? `Signed in ${when(d.app.signed_in_at)}, last seen ${when(d.app.last_seen)}`
                        : 'Signed out'}
                    </div>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-2">
                {d.app && <Badge variant={d.app.signed_in ? 'default' : 'secondary'}>{d.app.signed_in ? 'Signed in' : 'Signed out'}</Badge>}
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove ${d.name}`}
                  disabled={remove.isPending}
                  onClick={() => confirmRemove(d)}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}
