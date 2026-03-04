import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { Voicemail, PhoneForwarded, Settings, ArrowRight } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth'
import { me } from '@/api/endpoints'
import type { MeMailboxStats, MeForwarding } from '@/types'

export function UserDashboardPage() {
  const { user } = useAuthStore()
  const [stats, setStats] = useState<MeMailboxStats | null>(null)
  const [forwarding, setForwarding] = useState<MeForwarding[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function load() {
      try {
        const [s, f] = await Promise.allSettled([
          me.getMailboxStats(),
          me.getForwarding(),
        ])
        if (s.status === 'fulfilled') setStats(s.value)
        if (f.status === 'fulfilled') setForwarding(f.value)
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const hasForwarding = forwarding.some(
    (f) => f.forward_unconditional || f.forward_on_busy || f.forward_on_no_answer,
  )

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">
          Welcome{user?.display_name ? `, ${user.display_name}` : ''}
        </h1>
        <p className="text-muted-foreground">Manage your phone settings and voicemail.</p>
      </div>

      {loading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      ) : (
        <>
          {/* Summary cards */}
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">New Voicemails</CardTitle>
                <Voicemail className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{stats?.new_messages ?? 0}</div>
                <p className="text-xs text-muted-foreground">
                  {stats ? `${stats.total_messages} total messages` : 'No mailbox configured'}
                </p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Call Forwarding</CardTitle>
                <PhoneForwarded className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{hasForwarding ? 'Active' : 'Off'}</div>
                <p className="text-xs text-muted-foreground">
                  {forwarding.length} line{forwarding.length !== 1 ? 's' : ''} configured
                </p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Extension</CardTitle>
                <Settings className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{user?.display_name || user?.username}</div>
                <p className="text-xs text-muted-foreground">{user?.role}</p>
              </CardContent>
            </Card>
          </div>

          {/* Quick actions */}
          <div className="grid gap-4 sm:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Voicemail</CardTitle>
                <CardDescription>Listen to messages and manage greetings</CardDescription>
              </CardHeader>
              <CardContent>
                <Button variant="outline" asChild>
                  <Link to="/user/voicemail" className="gap-2">
                    Go to Voicemail <ArrowRight className="h-4 w-4" />
                  </Link>
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Account Settings</CardTitle>
                <CardDescription>Update your profile and change password</CardDescription>
              </CardHeader>
              <CardContent>
                <Button variant="outline" asChild>
                  <Link to="/user/settings" className="gap-2">
                    Go to Settings <ArrowRight className="h-4 w-4" />
                  </Link>
                </Button>
              </CardContent>
            </Card>
          </div>
        </>
      )}
    </div>
  )
}
