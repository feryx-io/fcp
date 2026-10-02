import { useEffect, useMemo, useState } from 'react'
import qrcode from 'qrcode-generator'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { me } from '@/api/endpoints'
import { ApiRequestError } from '@/api/client'
import type { TotpEnrollment, TotpStatus } from '@/types'

/**
 * Setting up a second factor (docs/IDENTITY_AND_DEVICES.md phase 2b): the
 * secret as a QR code an authenticator app scans, and as text to type, then
 * a code from the app to prove it holds it.
 */
export function TotpSetup({
  enrollment,
  onConfirm,
  busy,
}: {
  enrollment: TotpEnrollment
  onConfirm: (code: string) => void
  busy?: boolean
}) {
  const [code, setCode] = useState('')
  const qr = useMemo(() => {
    const q = qrcode(0, 'M')
    q.addData(enrollment.otpauth_uri)
    q.make()
    return q.createDataURL(4, 8)
  }, [enrollment.otpauth_uri])
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Scan this with an authenticator app (Google Authenticator, 1Password,
        Authy, …), or type the key into it.
      </p>
      <div className="flex justify-center">
        <img src={qr} alt="QR code for your authenticator app" className="rounded border" />
      </div>
      <p className="break-all text-center font-mono text-xs">{enrollment.secret}</p>
      <form
        className="space-y-2"
        onSubmit={(e) => {
          e.preventDefault()
          onConfirm(code.trim())
        }}
      >
        <Label htmlFor="totp-confirm">Code from the app</Label>
        <Input
          id="totp-confirm"
          inputMode="numeric"
          autoComplete="one-time-code"
          value={code}
          onChange={(e) => setCode(e.target.value)}
          placeholder="123456"
          required
        />
        <Button type="submit" className="w-full" disabled={busy}>
          Turn on
        </Button>
      </form>
    </div>
  )
}

/** Recovery codes, shown once: each signs in once without the phone. */
export function RecoveryCodesList({ codes }: { codes: string[] }) {
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">
        Keep these somewhere safe. Each signs you in once if you lose your
        phone; they will not be shown again.
      </p>
      <ul className="grid grid-cols-2 gap-1 rounded border p-3 font-mono text-sm">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
    </div>
  )
}

/**
 * The signed-in user's second factor: on with an app, off or new recovery
 * codes with a current code — off only where the tenant does not require it.
 */
export function TwoFactorCard() {
  const [status, setStatus] = useState<TotpStatus | null>(null)
  const [enrollment, setEnrollment] = useState<TotpEnrollment | null>(null)
  const [codes, setCodes] = useState<string[] | null>(null)
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const reload = () => me.totpStatus().then(setStatus).catch(() => setStatus(null))
  useEffect(() => {
    void reload()
  }, [])

  const run = async (action: () => Promise<void>) => {
    setError(null)
    setBusy(true)
    try {
      await action()
      await reload()
    } catch (err) {
      setError(err instanceof ApiRequestError ? err.body.details || err.body.error : 'Something went wrong')
    } finally {
      setBusy(false)
    }
  }

  if (!status) return null
  return (
    <Card>
      <CardHeader>
        <CardTitle>Two-factor sign-in</CardTitle>
        <CardDescription>
          {status.enabled
            ? `On: signing in asks for a code from your app. ${status.recovery_codes_left} recovery codes left.`
            : 'Off: add a code from an authenticator app to your password.'}
          {status.required && ' Your organisation requires it.'}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {error && <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">{error}</div>}
        {codes && <RecoveryCodesList codes={codes} />}
        {!status.enabled && !enrollment && (
          <Button disabled={busy} onClick={() => void run(async () => setEnrollment(await me.enrollTotp()))}>
            Set up
          </Button>
        )}
        {!status.enabled && enrollment && (
          <div className="max-w-sm">
            <TotpSetup
              enrollment={enrollment}
              busy={busy}
              onConfirm={(c) =>
                void run(async () => {
                  setCodes((await me.confirmTotp(c)).recovery_codes)
                  setEnrollment(null)
                })
              }
            />
          </div>
        )}
        {status.enabled && (
          <div className="flex max-w-md items-end gap-2">
            <div className="flex-1 space-y-2">
              <Label htmlFor="totp-current">Current code</Label>
              <Input
                id="totp-current"
                inputMode="numeric"
                autoComplete="one-time-code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="123456"
              />
            </div>
            <Button
              variant="outline"
              disabled={busy || !code}
              onClick={() =>
                void run(async () => {
                  setCodes((await me.renewRecoveryCodes(code.trim())).recovery_codes)
                  setCode('')
                })
              }
            >
              New recovery codes
            </Button>
            {!status.required && (
              <Button
                variant="destructive"
                disabled={busy || !code}
                onClick={() =>
                  void run(async () => {
                    await me.disableTotp(code.trim())
                    setCode('')
                    setCodes(null)
                  })
                }
              >
                Turn off
              </Button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
