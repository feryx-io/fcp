import { useState } from 'react'
import { Copy, Eye, EyeOff } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'

/** A token's secret, shown once: after it is made or rotated. */
export function TokenSecretDialog({
  secret,
  title,
  note,
  onClose,
}: {
  secret: string | null
  title: string
  note?: string
  onClose: () => void
}) {
  const [show, setShow] = useState(true)
  return (
    <Dialog open={!!secret} onOpenChange={() => { setShow(true); onClose() }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>Copy the secret now. It will not be shown again.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="relative">
            <Input
              readOnly
              type={show ? 'text' : 'password'}
              value={secret ?? ''}
              className="font-mono text-xs pr-16"
            />
            <div className="absolute right-1 top-1 flex gap-1">
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setShow(!show)}>
                {show ? <EyeOff className="h-3 w-3" /> : <Eye className="h-3 w-3" />}
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => secret && navigator.clipboard.writeText(secret)}
              >
                <Copy className="h-3 w-3" />
              </Button>
            </div>
          </div>
          {note && <p className="text-xs text-muted-foreground">{note}</p>}
        </div>
      </DialogContent>
    </Dialog>
  )
}

/** A token's state, for a badge: revoked, expired, or active. */
export function tokenStatus(t: { revoked_at?: string | null; expires_at?: string | null }) {
  if (t.revoked_at) return 'revoked' as const
  if (t.expires_at && new Date(t.expires_at) < new Date()) return 'expired' as const
  return 'active' as const
}

/** The note a rotation's secret carries. */
export function rotationNote(until?: string | null) {
  return until
    ? `The previous secret keeps working until ${new Date(until).toLocaleString()}.`
    : 'The previous secret no longer works.'
}
