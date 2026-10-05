import { useEffect, useRef, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
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
import type { DiscoverResponse, LoginResponse, TotpEnrollment } from '@/types'

/** What a failed sign-in through a provider means to the person. */
const SSO_ERRORS: Record<string, string> = {
  bad_state: 'That sign-in was started elsewhere or has expired. Try again.',
  expired: 'That sign-in took too long. Try again.',
  not_provisioned: 'Your account is not set up here yet. Ask your administrator.',
  disabled: 'Your account is disabled.',
  provider_refused: 'Your identity provider did not sign you in.',
  provider_unavailable: 'Your identity provider could not be reached. Try again shortly.',
  bad_id_token: 'Your identity provider\'s answer could not be verified.',
  username_taken: 'An account with your name already exists. Ask your administrator to link it.',
  no_username: 'Your identity provider gave no name to make an account with.',
  super_admin: 'A platform administrator signs in with a password.',
  claim_mismatch: 'This account is not one your organisation signs in with.',
  bad_assertion: 'Your identity provider\'s answer could not be verified.',
}

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

/** An app's sign-in waiting for this one (docs/SOFTPHONE.md §2.2): kept
 * across an identity provider's round trip, which loses the query. */
const APP_REQUEST = 'fcp_app_request'

function pendingApp(): string | null {
  try {
    return sessionStorage.getItem(APP_REQUEST)
  } catch {
    return null
  }
}

export function LoginPage() {
  const [username, setUsername] = useState('')
  const [organisation, setOrganisation] = useState<string | null>(null)
  const [found, setFound] = useState<DiscoverResponse | null>(null)
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [step, setStep] = useState<Step>({ kind: 'password' })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [appSignedIn, setAppSignedIn] = useState<string | null>(null)
  const navigate = useNavigate()
  const { login } = useAuthStore()

  // An email finds its tenant by its domain (docs/IDENTITY_AND_DEVICES.md
  // phase 2): say which organisation the sign-in goes to.
  const discover = () => {
    if (!username.includes('@')) return
    auth
      .discover(username)
      .then((found) => {
        setOrganisation(found.tenant_name ?? null)
        setFound(found)
      })
      .catch(() => {
        setOrganisation(null)
        setFound(null)
      })
  }
  const providers = found?.providers ?? []
  const passwordOffered = !found || found.methods.includes('password')

  const enter = (response: LoginResponse) => {
    if (!response.token) return
    login(response.token, response.user, response.expires_in)
    const app = pendingApp()
    if (app) {
      try {
        sessionStorage.removeItem(APP_REQUEST)
      } catch {
        /* nothing kept */
      }
      // Signed in for an app: grant its sign-in and send the browser back to it.
      auth
        .appApprove(app)
        .then(({ redirect_url, client_id }) => {
          setAppSignedIn(client_id)
          window.location.href = redirect_url
        })
        .catch((err) => {
          setError(
            err instanceof ApiRequestError
              ? err.body.details || err.body.error
              : 'Signing in the app failed',
          )
        })
      return
    }
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

  // Back from an identity provider (docs/IDENTITY_AND_DEVICES.md phase 3):
  // a one-time code to exchange, once, or why it failed.
  const [params, setParams] = useSearchParams()
  const handled = useRef(false)
  useEffect(() => {
    // Opened by an app to sign it in: remember it until the sign-in ends.
    const app = params.get('app')
    if (app) {
      try {
        sessionStorage.setItem(APP_REQUEST, app)
      } catch {
        /* the grant then follows only a sign-in without a provider */
      }
    }
    if (handled.current) return
    const code = params.get('sso_code')
    const failed = params.get('sso_error')
    if (!code && !failed) return
    handled.current = true
    setParams({}, { replace: true })
    if (failed) setError(SSO_ERRORS[failed] ?? 'Signing in through your identity provider failed.')
    if (code) void attempt(() => auth.ssoExchange(code))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params])

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
          {appSignedIn && (
            <div className="rounded-md bg-primary/10 p-3 text-sm">
              {appSignedIn} is signed in. You can close this tab and return to the app.
            </div>
          )}
          {!appSignedIn && pendingApp() && (
            <div className="rounded-md bg-muted p-3 text-sm text-muted-foreground">
              Signing in an app on this computer.
            </div>
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
                  onChange={(e) => { setUsername(e.target.value); setOrganisation(null); setFound(null) }}
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
              {providers.map((p) => (
                <Button
                  key={p.id}
                  type="button"
                  variant={passwordOffered ? 'outline' : 'default'}
                  className="w-full"
                  onClick={() => { window.location.href = auth.ssoStartUrl(p.id) }}
                >
                  Continue with {p.name}
                </Button>
              ))}
              {passwordOffered && (
                <>
                  {providers.length > 0 && (
                    <p className="text-center text-xs text-muted-foreground">or with your password</p>
                  )}
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
                </>
              )}
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
