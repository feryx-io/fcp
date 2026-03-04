import { useEffect, useState, useRef, useCallback } from 'react'
import { Play, Pause, Trash2, Upload, CheckCircle, Clock, CalendarClock, MailOpen, Mail, Save, Download, Power, PowerOff } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { me } from '@/api/endpoints'
import type { VoicemailMessage, MeMailbox, GreetingResponse } from '@/types'

type Tab = 'messages' | 'greetings' | 'settings'

export function UserVoicemailPage() {
  const [tab, setTab] = useState<Tab>('messages')
  const [mailbox, setMailbox] = useState<MeMailbox | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    async function load() {
      try {
        const mb = await me.getMailbox()
        setMailbox(mb)
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load voicemail')
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  if (loading) {
    return (
      <div className="flex h-32 items-center justify-center text-muted-foreground">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    )
  }

  if (error || !mailbox) {
    return (
      <div className="space-y-4">
        <h1 className="text-2xl font-bold">Voicemail</h1>
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            {error || 'No voicemail mailbox is configured for your account.'}
          </CardContent>
        </Card>
      </div>
    )
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: 'messages', label: 'Messages' },
    { id: 'greetings', label: 'Greetings' },
    { id: 'settings', label: 'Settings' },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Voicemail</h1>
        <p className="text-muted-foreground">Extension {mailbox.extension}</p>
      </div>

      {/* Tab bar */}
      <div className="flex gap-1 border-b">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={cn(
              'px-4 py-2 text-sm font-medium border-b-2 -mb-px transition-colors',
              tab === t.id
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === 'messages' && <MessagesTab retentionDays={mailbox.retention_days} />}
      {tab === 'greetings' && <GreetingsTab />}
      {tab === 'settings' && <SettingsTab mailbox={mailbox} onUpdate={setMailbox} />}
    </div>
  )
}

// ---- Messages Tab ----

function MessagesTab({ retentionDays }: { retentionDays: number | null }) {
  const [messages, setMessages] = useState<VoicemailMessage[]>([])
  const [loading, setLoading] = useState(true)
  const [playingId, setPlayingId] = useState<string | null>(null)
  const [audioUrl, setAudioUrl] = useState<string | null>(null)
  const audioRef = useRef<HTMLAudioElement>(null)

  useEffect(() => {
    me.listMessages()
      .then(setMessages)
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const handlePlay = useCallback(async (id: string) => {
    if (playingId === id) {
      audioRef.current?.pause()
      setPlayingId(null)
      return
    }

    try {
      if (audioUrl) URL.revokeObjectURL(audioUrl)
      const url = await me.fetchMessageAudio(id)
      setAudioUrl(url)
      setPlayingId(id)
      // Audio will autoplay after state update via effect
    } catch {
      // Failed to load audio
    }
  }, [playingId, audioUrl])

  // Autoplay when audioUrl changes
  useEffect(() => {
    if (audioUrl && playingId && audioRef.current) {
      audioRef.current.src = audioUrl
      audioRef.current.play().catch(() => {})
    }
  }, [audioUrl, playingId])

  const handleAudioEnded = () => {
    setPlayingId(null)
  }

  const handleMarkStatus = async (id: string, status: string) => {
    try {
      const updated = await me.updateMessage(id, { status })
      setMessages((prev) => prev.map((m) => (m.id === id ? updated : m)))
    } catch {
      // Failed
    }
  }

  const handleDelete = async (id: string) => {
    try {
      await me.deleteMessage(id)
      setMessages((prev) => prev.filter((m) => m.id !== id))
    } catch {
      // Failed
    }
  }

  const handleDownload = async (msg: VoicemailMessage) => {
    try {
      const url = await me.fetchMessageAudio(msg.id)
      const caller = (msg.caller_name || msg.caller || 'unknown').replace(/[^a-zA-Z0-9._-]/g, '_')
      const date = new Date(msg.created_at * 1000).toISOString().slice(0, 10)
      const a = document.createElement('a')
      a.href = url
      a.download = `voicemail-${caller}-${date}.wav`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
    } catch {
      // Failed
    }
  }

  // Cleanup blob URLs
  useEffect(() => {
    return () => {
      if (audioUrl) URL.revokeObjectURL(audioUrl)
    }
  }, [audioUrl])

  if (loading) {
    return (
      <div className="flex h-24 items-center justify-center text-muted-foreground">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    )
  }

  if (messages.length === 0) {
    return (
      <Card>
        <CardContent className="py-8 text-center text-muted-foreground">
          No voicemail messages.
        </CardContent>
      </Card>
    )
  }

  return (
    <div className="space-y-2">
      <audio ref={audioRef} onEnded={handleAudioEnded} className="hidden" />
      {messages.map((msg) => (
        <Card key={msg.id} className={cn(msg.status === 'new' && 'border-primary/50')}>
          <CardContent className="flex items-center gap-4 py-3">
            {/* Play button */}
            <Button
              variant="ghost"
              size="icon"
              className="shrink-0"
              onClick={() => handlePlay(msg.id)}
            >
              {playingId === msg.id ? (
                <Pause className="h-4 w-4" />
              ) : (
                <Play className="h-4 w-4" />
              )}
            </Button>

            {/* Info */}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="font-medium truncate">
                  {msg.caller_name || msg.caller}
                </span>
                <StatusBadge status={msg.status} />
                {msg.priority === 'urgent' && (
                  <Badge variant="destructive" className="text-xs">Urgent</Badge>
                )}
              </div>
              <div className="text-xs text-muted-foreground flex gap-3">
                {msg.caller_name && <span>{msg.caller}</span>}
                <span>{formatDuration(msg.duration)}</span>
                <span>{formatDate(msg.created_at)}</span>
                {retentionDays && msg.status !== 'saved' && (
                  <ExpiryIndicator createdAt={msg.created_at} retentionDays={retentionDays} />
                )}
              </div>
            </div>

            {/* Actions */}
            <div className="flex gap-1 shrink-0">
              {msg.status === 'new' && (
                <Button variant="ghost" size="icon" title="Mark as heard" onClick={() => handleMarkStatus(msg.id, 'heard')}>
                  <MailOpen className="h-4 w-4" />
                </Button>
              )}
              {msg.status === 'heard' && (
                <Button variant="ghost" size="icon" title="Mark as unheard" onClick={() => handleMarkStatus(msg.id, 'new')}>
                  <Mail className="h-4 w-4" />
                </Button>
              )}
              {msg.status !== 'saved' && (
                <Button variant="ghost" size="icon" title="Save message" onClick={() => handleMarkStatus(msg.id, 'saved')}>
                  <Save className="h-4 w-4" />
                </Button>
              )}
              <Button variant="ghost" size="icon" title="Download" onClick={() => handleDownload(msg)}>
                <Download className="h-4 w-4" />
              </Button>
              <Button variant="ghost" size="icon" title="Delete" onClick={() => handleDelete(msg.id)}>
                <Trash2 className="h-4 w-4 text-destructive" />
              </Button>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}

function StatusBadge({ status }: { status: string }) {
  switch (status) {
    case 'new':
      return <Badge className="text-xs bg-blue-500/15 text-blue-700 dark:text-blue-400 border-0">New</Badge>
    case 'heard':
      return <Badge variant="secondary" className="text-xs">Heard</Badge>
    case 'saved':
      return <Badge variant="outline" className="text-xs">Saved</Badge>
    default:
      return <Badge variant="secondary" className="text-xs">{status}</Badge>
  }
}

// ---- Greetings Tab ----

const GREETING_TYPES = [
  { id: 'unavailable', label: 'Unavailable', description: 'Played when you cannot take calls.' },
  { id: 'busy', label: 'Busy', description: 'Played when your line is busy.' },
  { id: 'name', label: 'Name', description: 'Your recorded name announcement.' },
  { id: 'temporary', label: 'Temporary', description: 'Overrides all greetings. Can have an expiration.' },
]

function GreetingsTab() {
  const [greetings, setGreetings] = useState<GreetingResponse[]>([])
  const [loading, setLoading] = useState(true)
  const [uploadType, setUploadType] = useState<string | null>(null)
  const [expiresHours, setExpiresHours] = useState('')
  const [playingType, setPlayingType] = useState<string | null>(null)
  const [audioUrl, setAudioUrl] = useState<string | null>(null)
  const audioRef = useRef<HTMLAudioElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    me.listGreetings()
      .then(setGreetings)
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const handleUpload = async (type: string) => {
    setUploadType(type)
    fileInputRef.current?.click()
  }

  const handleFileSelected = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file || !uploadType) return

    try {
      const hours = uploadType === 'temporary' && expiresHours ? parseInt(expiresHours) : undefined
      const greeting = await me.uploadGreeting(uploadType, file, hours)
      setGreetings((prev) => {
        const filtered = prev.filter((g) => g.greeting_type !== uploadType)
        return [...filtered, greeting]
      })
    } catch {
      // Failed
    } finally {
      setUploadType(null)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  const handleDelete = async (type: string) => {
    try {
      await me.deleteGreeting(type)
      setGreetings((prev) => prev.filter((g) => g.greeting_type !== type))
    } catch {
      // Failed
    }
  }

  const handleToggleActive = async (type: string, active: boolean) => {
    try {
      await me.setGreetingActive(type, active)
      setGreetings((prev) =>
        prev.map((g) => (g.greeting_type === type ? { ...g, is_active: active } : g)),
      )
    } catch {
      // Failed
    }
  }

  const handlePlay = async (type: string) => {
    if (playingType === type) {
      audioRef.current?.pause()
      setPlayingType(null)
      return
    }

    try {
      if (audioUrl) URL.revokeObjectURL(audioUrl)
      const url = await me.fetchGreetingAudio(type)
      setAudioUrl(url)
      setPlayingType(type)
    } catch {
      // Failed
    }
  }

  useEffect(() => {
    if (audioUrl && playingType && audioRef.current) {
      audioRef.current.src = audioUrl
      audioRef.current.play().catch(() => {})
    }
  }, [audioUrl, playingType])

  useEffect(() => {
    return () => {
      if (audioUrl) URL.revokeObjectURL(audioUrl)
    }
  }, [audioUrl])

  if (loading) {
    return (
      <div className="flex h-24 items-center justify-center text-muted-foreground">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <audio ref={audioRef} onEnded={() => setPlayingType(null)} className="hidden" />
      <input ref={fileInputRef} type="file" accept="audio/*" className="hidden" onChange={handleFileSelected} />

      {GREETING_TYPES.map((gt) => {
        const greeting = greetings.find((g) => g.greeting_type === gt.id)
        const isExpired = greeting?.expires_at && greeting.expires_at * 1000 < Date.now()

        return (
          <Card key={gt.id}>
            <CardContent className="flex items-center gap-4 py-4">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{gt.label}</span>
                  {greeting && (
                    <>
                      {greeting.is_active ? (
                        <Badge className="text-xs bg-green-500/15 text-green-700 dark:text-green-400 border-0">Active</Badge>
                      ) : (
                        <Badge variant="secondary" className="text-xs">Inactive</Badge>
                      )}
                      {isExpired && (
                        <Badge variant="destructive" className="text-xs">Expired</Badge>
                      )}
                    </>
                  )}
                </div>
                <p className="text-xs text-muted-foreground">{gt.description}</p>
                {greeting?.expires_at && !isExpired && (
                  <p className="text-xs text-muted-foreground flex items-center gap-1 mt-1">
                    <Clock className="h-3 w-3" />
                    Expires {new Date(greeting.expires_at * 1000).toLocaleString()}
                  </p>
                )}
              </div>

              <div className="flex items-center gap-1 shrink-0">
                {greeting ? (
                  <>
                    <Button variant="ghost" size="icon" title="Play" onClick={() => handlePlay(gt.id)}>
                      {playingType === gt.id ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      title={greeting.is_active ? 'Deactivate' : 'Activate'}
                      onClick={() => handleToggleActive(gt.id, !greeting.is_active)}
                    >
                      {greeting.is_active ? <PowerOff className="h-4 w-4" /> : <Power className="h-4 w-4" />}
                    </Button>
                    <Button variant="ghost" size="icon" title="Delete" onClick={() => handleDelete(gt.id)}>
                      <Trash2 className="h-4 w-4 text-destructive" />
                    </Button>
                  </>
                ) : (
                  <div className="flex items-center gap-2">
                    {gt.id === 'temporary' && (
                      <Input
                        type="number"
                        min={1}
                        placeholder="Hours"
                        className="w-20 h-8 text-xs"
                        value={expiresHours}
                        onChange={(e) => setExpiresHours(e.target.value)}
                      />
                    )}
                    <Button variant="outline" size="sm" className="gap-1" onClick={() => handleUpload(gt.id)}>
                      <Upload className="h-3 w-3" />
                      Upload
                    </Button>
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
        )
      })}
    </div>
  )
}

// ---- Settings Tab ----

function SettingsTab({ mailbox, onUpdate }: { mailbox: MeMailbox; onUpdate: (mb: MeMailbox) => void }) {
  const [pin, setPin] = useState('')
  const [emailNotification, setEmailNotification] = useState(mailbox.email_notification)
  const [emailAddress, setEmailAddress] = useState(mailbox.email_address || '')
  const [attachAudio, setAttachAudio] = useState(mailbox.attach_audio)
  const [saving, setSaving] = useState(false)
  const [msg, setMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null)

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault()
    setSaving(true)
    setMsg(null)

    try {
      const updated = await me.updateMailbox({
        pin: pin || undefined,
        email_notification: emailNotification,
        email_address: emailAddress || undefined,
        attach_audio: attachAudio,
      })
      onUpdate(updated)
      setPin('')
      setMsg({ type: 'success', text: 'Voicemail settings updated.' })
    } catch (err) {
      setMsg({ type: 'error', text: err instanceof Error ? err.message : 'Failed to update' })
    } finally {
      setSaving(false)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Voicemail Settings</CardTitle>
        <CardDescription>Update your voicemail preferences.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSave} className="space-y-6">
          {msg && (
            <div className={cn(
              'rounded-md p-3 text-sm',
              msg.type === 'success' ? 'bg-green-500/15 text-green-700 dark:text-green-400' : 'bg-destructive/15 text-destructive',
            )}>
              {msg.text}
            </div>
          )}

          {/* PIN */}
          <div className="space-y-2">
            <Label htmlFor="pin">Voicemail PIN</Label>
            <Input
              id="pin"
              type="password"
              value={pin}
              onChange={(e) => setPin(e.target.value)}
              placeholder="Enter new PIN (4-8 digits)"
              pattern="[0-9]{4,8}"
              className="w-48"
            />
            <p className="text-xs text-muted-foreground">Leave blank to keep current PIN.</p>
          </div>

          {/* Email notifications */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <Label className="text-sm font-medium">Email Notifications</Label>
                <p className="text-xs text-muted-foreground">Get notified when new voicemails arrive.</p>
              </div>
              <Switch checked={emailNotification} onCheckedChange={setEmailNotification} />
            </div>
            {emailNotification && (
              <Input
                type="email"
                value={emailAddress}
                onChange={(e) => setEmailAddress(e.target.value)}
                placeholder="your@email.com"
              />
            )}
          </div>

          {/* Attach audio */}
          <div className="flex items-center justify-between">
            <div>
              <Label className="text-sm font-medium">Attach Audio to Email</Label>
              <p className="text-xs text-muted-foreground">Include the voicemail recording as an email attachment.</p>
            </div>
            <Switch checked={attachAudio} onCheckedChange={setAttachAudio} />
          </div>

          {/* Read-only info */}
          <div className="border-t pt-4">
            <h4 className="text-sm font-medium mb-2">Mailbox Information</h4>
            <dl className="grid gap-2 sm:grid-cols-2 text-sm">
              <div>
                <dt className="text-muted-foreground">Status</dt>
                <dd className="font-medium flex items-center gap-1">
                  {mailbox.enabled ? (
                    <><CheckCircle className="h-3 w-3 text-green-500" /> Enabled</>
                  ) : (
                    <span className="text-muted-foreground">Disabled</span>
                  )}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Max Messages</dt>
                <dd className="font-medium">{mailbox.max_messages}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Max Duration</dt>
                <dd className="font-medium">{mailbox.max_message_duration}s</dd>
              </div>
            </dl>
          </div>

          <Button type="submit" disabled={saving}>
            {saving ? 'Saving...' : 'Save Settings'}
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

// ---- Expiry Indicator ----

function ExpiryIndicator({ createdAt, retentionDays }: { createdAt: number; retentionDays: number }) {
  const expiresAt = (createdAt + retentionDays * 86400) * 1000 // ms
  const now = Date.now()
  const remainingMs = expiresAt - now

  if (remainingMs <= 0) {
    return (
      <span className="text-destructive flex items-center gap-1">
        <CalendarClock className="h-3 w-3" />
        Expired
      </span>
    )
  }

  const remainingDays = Math.ceil(remainingMs / 86400000)
  const label = remainingDays === 1 ? '1 day' : `${remainingDays} days`

  return (
    <span className="text-muted-foreground flex items-center gap-1">
      <CalendarClock className="h-3 w-3" />
      Expires in {label}
    </span>
  )
}

// ---- Helpers ----

function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60)
  const secs = seconds % 60
  return `${mins}:${secs.toString().padStart(2, '0')}`
}

function formatDate(epochSecs: number): string {
  return new Date(epochSecs * 1000).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })
}
