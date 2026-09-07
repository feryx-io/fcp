import { useEffect, useState } from 'react'
import { Copy, ExternalLink, Link as LinkIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { JoinTokenResponse, MintJoinTokenRequest } from '@/api/endpoints'
import type { MeetingResponse } from '@/types'

/**
 * "Join in browser": mints a join token for a meeting and shows the link
 * that opens the WebRTC gateway's join page with it, or the bare token when
 * the admin API has no gateway URL configured.
 */
export function JoinLinkDialog({ meeting, onOpenChange, mint }: {
  meeting: MeetingResponse | null
  onOpenChange: (open: boolean) => void
  mint: (id: string, body: MintJoinTokenRequest) => Promise<JoinTokenResponse>
}) {
  const [name, setName] = useState('')
  const [role, setRole] = useState<'guest' | 'host'>('guest')
  const [ttl, setTtl] = useState('15')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<JoinTokenResponse | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (meeting) { setName(''); setRole('guest'); setTtl('15'); setError(null); setResult(null); setCopied(false) }
  }, [meeting])

  const ttlMinutes = Number.parseInt(ttl, 10)
  const ttlValid = Number.isFinite(ttlMinutes) && ttlMinutes >= 1 && ttlMinutes <= 24 * 60
  const link = result?.join_url ?? null
  const shareable = link ?? result?.token ?? ''

  async function create() {
    if (!meeting) return
    setPending(true)
    setError(null)
    try {
      const body: MintJoinTokenRequest = { role, ttl_secs: ttlMinutes * 60 }
      if (name.trim()) body.display_name = name.trim()
      setResult(await mint(meeting.id, body))
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setPending(false)
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(shareable)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <Dialog open={!!meeting} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Join in browser — Room {meeting?.room_number}</DialogTitle>
          <DialogDescription>
            Create a link that admits someone to this meeting from a browser, with no PIN. The link
            carries a signed token that expires; share it with the person it is for.
          </DialogDescription>
        </DialogHeader>
        {!result ? (
          <div className="space-y-4">
            <div>
              <Label>Participant name</Label>
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder={`${meeting?.name ?? 'Meeting'} ${role}`} disabled={pending} maxLength={64} />
              <p className="text-xs text-muted-foreground mt-1">Shown to the room. The joiner can change it on the page.</p>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label>Role</Label>
                <Select value={role} onValueChange={(v) => setRole(v as 'guest' | 'host')} disabled={pending}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="guest">Guest</SelectItem>
                    <SelectItem value="host">Host (moderator)</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label>Valid for (minutes)</Label>
                <Input type="number" min={1} max={1440} value={ttl} onChange={(e) => setTtl(e.target.value)} disabled={pending} />
              </div>
            </div>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>Cancel</Button>
              <Button onClick={create} disabled={pending || !ttlValid}>
                <LinkIcon className="mr-2 h-4 w-4" />
                {pending ? 'Creating…' : 'Create link'}
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-4">
            <div>
              <Label>{link ? 'Join link' : 'Join token'}</Label>
              <div className="flex gap-2">
                <Input readOnly value={shareable} onFocus={(e) => e.currentTarget.select()} className="font-mono text-xs" />
                <Button variant="outline" size="icon" onClick={copy} title="Copy"><Copy className="h-4 w-4" /></Button>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                {copied ? 'Copied.' : `${result.display_name} as ${result.role}, valid until ${new Date(result.expires_at * 1000).toLocaleString()}.`}
                {!link && ' Set [conference] gateway_url in the admin config to get a link instead of a bare token.'}
              </p>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setResult(null)}>Another</Button>
              {link && (
                <Button onClick={() => window.open(link, '_blank', 'noopener')}>
                  <ExternalLink className="mr-2 h-4 w-4" /> Open
                </Button>
              )}
              <Button variant="outline" onClick={() => onOpenChange(false)}>Done</Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
