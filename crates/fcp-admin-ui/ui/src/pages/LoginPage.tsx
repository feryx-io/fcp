import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Boxes } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useAuthStore } from '@/stores/auth'
import { auth } from '@/api/endpoints'
import { ApiRequestError } from '@/api/client'
import { setExpiredPasswordState } from '@/stores/expired-password'

export function LoginPage() {
  const [username, setUsername] = useState('')
  const [organisation, setOrganisation] = useState<string | null>(null)

  // An email finds its tenant by its domain (docs/IDENTITY_AND_DEVICES.md
  // phase 2): say which organisation the sign-in goes to.
  const discover = () => {
    if (!username.includes('@')) return
    auth
      .discover(username)
      .then((found) => setOrganisation(found.tenant_name ?? null))
      .catch(() => setOrganisation(null))
  }
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const navigate = useNavigate()
  const { login } = useAuthStore()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setLoading(true)

    try {
      const response = await auth.login({ username, password })
      if (response.password_expired) {
        // Store credentials in ephemeral memory (not in browser history state)
        setExpiredPasswordState({ username, currentPassword: password })
        navigate('/change-password', { replace: true })
        return
      }
      login(response.token, response.user, response.expires_in)
      if (response.user.role === 'end_user') {
        navigate('/user')
      } else {
        navigate('/admin')
      }
    } catch (err) {
      if (err instanceof ApiRequestError) {
        setError(err.body.details || err.body.error)
      } else {
        setError('Failed to connect to server')
      }
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4">
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="flex justify-center mb-4">
            <Boxes className="h-12 w-12 text-primary" />
          </div>
          <CardTitle className="text-2xl">FCP Portal</CardTitle>
          <CardDescription>Sign in to your account</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            {error && (
              <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">
                {error}
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="username">Username or email</Label>
              <Input
                id="username"
                value={username}
                onChange={(e) => { setUsername(e.target.value); setOrganisation(null) }}
                onBlur={discover}
                placeholder="Username or name@company.com"
                required
                autoComplete="username"
                autoFocus
              />
              {organisation && (
                <p className="text-xs text-muted-foreground">Signing in to {organisation}</p>
              )}
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="Enter your password"
                required
                autoComplete="current-password"
              />
            </div>
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Signing in...' : 'Sign in'}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
