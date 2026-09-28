import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { LogOut, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { hoteling } from '@/api/endpoints'
import type { HotelingSession } from '@/types'

// Who is logged in on which host phone (docs/ENTERPRISE_CALLING.md §3.8).
// A login happens at the phone (*88); here an admin sees and ends them.

const ENDED_LABELS: Record<string, string> = {
  logout: 'Logged out',
  admin: 'Ended by an admin',
  expired: 'Expired',
  replaced: 'Replaced by another login',
}

const when = (t: string | null) => (t ? new Date(t).toLocaleString() : '-')

export function HotelingPage() {
  const queryClient = useQueryClient()
  const [all, setAll] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { data: sessions = [], isLoading, isFetching, refetch } = useQuery({
    queryKey: ['hoteling-sessions', all],
    queryFn: () => hoteling.list(all),
  })
  const end = useMutation({
    mutationFn: (s: HotelingSession) => hoteling.end(s.id),
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['hoteling-sessions'] })
    },
    onError: (err) => setError(err instanceof Error ? err.message : 'Failed to end the session'),
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Hoteling</h1>
          <p className="text-muted-foreground">Users logged in on shared host phones</p>
        </div>
        <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
          <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} />
          Refresh
        </Button>
      </div>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Sessions</CardTitle>
              <CardDescription>
                A user dials *88 on a device marked as a hoteling host and logs in with their extension
                and PIN; the phone rings for them and calls as them until they dial *89, an admin ends
                the session, or it runs out (the tenant&rsquo;s hoteling login length, 12 hours unless
                set). Mark hosts on the Devices page.
              </CardDescription>
            </div>
            <div className="flex items-center gap-2">
              <Switch id="h-all" checked={all} onCheckedChange={setAll} />
              <Label htmlFor="h-all">Show ended</Label>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {error && <p className="mb-2 text-sm text-destructive">{error}</p>}
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading...</p>
          ) : sessions.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {all ? 'No sessions yet.' : 'Nobody is logged in on a host phone.'}
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Host phone</TableHead>
                  <TableHead>User</TableHead>
                  <TableHead>Started</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead>State</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {sessions.map((s) => (
                  <TableRow key={s.id}>
                    <TableCell>{s.device_name ?? s.device_id}</TableCell>
                    <TableCell>{s.username ?? s.user_id}</TableCell>
                    <TableCell>{when(s.started_at)}</TableCell>
                    <TableCell>{when(s.expires_at)}</TableCell>
                    <TableCell>
                      {s.active ? (
                        <Badge variant="success">Logged in</Badge>
                      ) : (
                        <Badge variant="secondary">
                          {ENDED_LABELS[s.ended_by ?? ''] ?? 'Ended'} {s.ended_at ? when(s.ended_at) : ''}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      {s.active && (
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={end.isPending}
                          onClick={() => end.mutate(s)}
                        >
                          <LogOut className="mr-2 h-4 w-4" /> End
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
