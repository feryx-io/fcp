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
import { RecoveryCodesList, TotpSetup } from '@/components/TotpSetup'
import type { LoginResponse, TotpEnrollment } from '@/types'

/**
 * Signing in: a password, then — when the user has a second factor, or
 * their tenant requires one they have not set up — a code, or setting one
 * up (docs/IDENTITY_AND_DEVICES.md phase 2b).
 */
type Step =
  | { kind: 'password' }
  | { kind: 'code'; ticket: string; recovery: boolean }
  | { kind: 'enroll'; ticket: string; enrollment: TotpEnrollment | null }
  | { kind: 'recovery-codes'; codes: string[]; response: LoginResponse }

export function LoginPage() {
  const [username, setUsername] = useState('')
  const [organisation, setOrganisation] = useState<string | null>(null)
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [step, setStep] = useState<Step>({ kind: 'password' })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const navigate = useNavigate()
  const { login } = useAuthStore()

  // An email finds its tenant by its domain (docs/IDENTITY_AND_DEVICES.md
  // phase 2): say which organisation the sign-in goes to.
  const discover = () => {
    if (!username.includes('@')) return
    auth
      .discover(username)
      .then((found) => setOrganisation(found.tenant_name ?? null))
      .catch(() => setOrganisation(null))
  }

  const enter = (response: LoginResponse) => {
    if (!response.token) return
    login(response.token, response.user, response.expires_in)
    navigate(response.user.role === 'end_user' ? '/user' : '/admin')
  }

  /** Where a response leads: in, a second step, or a new password. */
  const follow = async (response: LoginResponse) => {
    if (response.password_expired) {
      // Store credentials in ephemeral memory (not in browser history state)
      setExpiredPasswordState({ username, currentPassword: password })
      navigate('/change-password', { replace: true })
      return
    }
    if (response.mfa_required === 'totp' && response.mfa_token) {
      setCode('')
      setStep({ kind: 'code', ticket: response.mfa_token, recovery: false })
      return
    }
    if (response.mfa_required === 'totp_enrollment' && response.mfa_token) {
      const ticket = response.mfa_token
      setStep({ kind: 'enroll', ticket, enrollment: null })
      setStep({ kind: 'enroll', ticket, enrollment: await auth.enrollTotp(ticket) })
      return
    }
    if (response.recovery_codes?.length) {
      setStep({ kind: 'recovery-codes', codes: response.recovery_codes, response })
      return
    }
    enter(response)
  }

  const attempt = async (action: () => Promise<LoginResponse>) => {
    setError(null)
    setLoading(true)
    try {
      await follow(await action())
    } catch (err) {
      if (err instanceof ApiRequestError) {
        const message = err.body.details || err.body.error
        setError(message)
        // A ticket that ran out: from the password again.
        if (message === 'Sign in again') setStep({ kind: 'password' })
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
          <CardDescription>
            {step.kind === 'password' && 'Sign in to your account'}
            {step.kind === 'code' &&
              (step.recovery ? 'Enter a recovery code' : 'Enter the code from your authenticator app')}
            {step.kind === 'enroll' && 'Your organisation requires a second factor: set one up'}
            {step.kind === 'recovery-codes' && 'Your recovery codes'}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {error && (
            <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">{error}</div>
          )}

          {step.kind === 'password' && (
            <form
              onSubmit={(e) => {
                e.preventDefault()
                void attempt(() => auth.login({ username, password }))
              }}
              className="space-y-4"
            >
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
          )}

          {step.kind === 'code' && (
            <form
              onSubmit={(e) => {
                e.preventDefault()
                const answer = step.recovery ? { recovery_code: code.trim() } : { code: code.trim() }
                void attempt(() => auth.loginTotp(step.ticket, answer))
              }}
              className="space-y-4"
            >
              <Input
                id="code"
                inputMode={step.recovery ? 'text' : 'numeric'}
                autoComplete="one-time-code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder={step.recovery ? 'xxxxx-xxxxx' : '123456'}
                required
                autoFocus
              />
              <Button type="submit" className="w-full" disabled={loading}>
                {loading ? 'Checking...' : 'Continue'}
              </Button>
              <button
                type="button"
                className="w-full text-xs text-muted-foreground underline"
                onClick={() => { setCode(''); setStep({ ...step, recovery: !step.recovery }) }}
              >
                {step.recovery ? 'Use the app instead' : 'Lost your phone? Use a recovery code'}
              </button>
            </form>
          )}

          {step.kind === 'enroll' &&
            (step.enrollment ? (
              <TotpSetup
                enrollment={step.enrollment}
                busy={loading}
                onConfirm={(c) => void attempt(() => auth.confirmTotp(step.ticket, c))}
              />
            ) : (
              <p className="text-sm text-muted-foreground">Preparing…</p>
            ))}

          {step.kind === 'recovery-codes' && (
            <div className="space-y-4">
              <RecoveryCodesList codes={step.codes} />
              <Button className="w-full" onClick={() => enter(step.response)}>
                I have saved them
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
