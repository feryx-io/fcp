import { useEffect, useState, useRef, useCallback } from 'react'
import { Play, Pause, Trash2, Upload, CheckCircle, Clock, CalendarClock, MailOpen, Mail, Save, Download, Power, PowerOff, Volume2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { me } from '@/api/endpoints'
import type { VoicemailMessage, MeMailbox, GreetingResponse } from '@/types'

// Kokoro-82M TTS voices
const KOKORO_VOICES: { value: string; label: string; lang: string }[] = [
  { value: 'af_heart', label: 'Heart (American Female)', lang: 'en-US' },
  { value: 'af_alloy', label: 'Alloy (American Female)', lang: 'en-US' },
  { value: 'af_aoede', label: 'Aoede (American Female)', lang: 'en-US' },
  { value: 'af_bella', label: 'Bella (American Female)', lang: 'en-US' },
  { value: 'af_jessica', label: 'Jessica (American Female)', lang: 'en-US' },
  { value: 'af_kore', label: 'Kore (American Female)', lang: 'en-US' },
  { value: 'af_nicole', label: 'Nicole (American Female)', lang: 'en-US' },
  { value: 'af_nova', label: 'Nova (American Female)', lang: 'en-US' },
  { value: 'af_river', label: 'River (American Female)', lang: 'en-US' },
  { value: 'af_sarah', label: 'Sarah (American Female)', lang: 'en-US' },
  { value: 'af_sky', label: 'Sky (American Female)', lang: 'en-US' },
  { value: 'am_adam', label: 'Adam (American Male)', lang: 'en-US' },
  { value: 'am_echo', label: 'Echo (American Male)', lang: 'en-US' },
  { value: 'am_eric', label: 'Eric (American Male)', lang: 'en-US' },
  { value: 'am_fenrir', label: 'Fenrir (American Male)', lang: 'en-US' },
  { value: 'am_liam', label: 'Liam (American Male)', lang: 'en-US' },
  { value: 'am_michael', label: 'Michael (American Male)', lang: 'en-US' },
  { value: 'am_onyx', label: 'Onyx (American Male)', lang: 'en-US' },
  { value: 'am_puck', label: 'Puck (American Male)', lang: 'en-US' },
  { value: 'am_santa', label: 'Santa (American Male)', lang: 'en-US' },
  { value: 'bf_alice', label: 'Alice (British Female)', lang: 'en-GB' },
  { value: 'bf_emma', label: 'Emma (British Female)', lang: 'en-GB' },
  { value: 'bf_isabella', label: 'Isabella (British Female)', lang: 'en-GB' },
  { value: 'bf_lily', label: 'Lily (British Female)', lang: 'en-GB' },
  { value: 'bm_daniel', label: 'Daniel (British Male)', lang: 'en-GB' },
  { value: 'bm_fable', label: 'Fable (British Male)', lang: 'en-GB' },
  { value: 'bm_george', label: 'George (British Male)', lang: 'en-GB' },
  { value: 'bm_lewis', label: 'Lewis (British Male)', lang: 'en-GB' },
  { value: 'ef_dora', label: 'Dora (Spanish Female)', lang: 'es-ES' },
  { value: 'em_alex', label: 'Alex (Spanish Male)', lang: 'es-ES' },
  { value: 'em_santa', label: 'Santa (Spanish Male)', lang: 'es-ES' },
  { value: 'ff_siwis', label: 'Siwis (French Female)', lang: 'fr-FR' },
  { value: 'hf_alpha', label: 'Alpha (Hindi Female)', lang: 'hi-IN' },
  { value: 'hf_beta', label: 'Beta (Hindi Female)', lang: 'hi-IN' },
  { value: 'hm_omega', label: 'Omega (Hindi Male)', lang: 'hi-IN' },
  { value: 'hm_psi', label: 'Psi (Hindi Male)', lang: 'hi-IN' },
  { value: 'if_sara', label: 'Sara (Italian Female)', lang: 'it-IT' },
  { value: 'im_nicola', label: 'Nicola (Italian Male)', lang: 'it-IT' },
  { value: 'jf_alpha', label: 'Alpha (Japanese Female)', lang: 'ja-JP' },
  { value: 'jf_gongitsune', label: 'Gongitsune (Japanese Female)', lang: 'ja-JP' },
  { value: 'jf_nezumi', label: 'Nezumi (Japanese Female)', lang: 'ja-JP' },
  { value: 'jf_tebukuro', label: 'Tebukuro (Japanese Female)', lang: 'ja-JP' },
  { value: 'jm_kumo', label: 'Kumo (Japanese Male)', lang: 'ja-JP' },
  { value: 'zf_xiaobei', label: 'Xiaobei (Chinese Female)', lang: 'zh-CN' },
  { value: 'zf_xiaoni', label: 'Xiaoni (Chinese Female)', lang: 'zh-CN' },
  { value: 'zf_xiaoxiao', label: 'Xiaoxiao (Chinese Female)', lang: 'zh-CN' },
  { value: 'zf_xiaoyi', label: 'Xiaoyi (Chinese Female)', lang: 'zh-CN' },
  { value: 'zm_yunjian', label: 'Yunjian (Chinese Male)', lang: 'zh-CN' },
  { value: 'zm_yunxi', label: 'Yunxi (Chinese Male)', lang: 'zh-CN' },
  { value: 'zm_yunxia', label: 'Yunxia (Chinese Male)', lang: 'zh-CN' },
  { value: 'zm_yunyang', label: 'Yunyang (Chinese Male)', lang: 'zh-CN' },
  { value: 'pf_dora', label: 'Dora (Portuguese Female)', lang: 'pt-BR' },
  { value: 'pm_alex', label: 'Alex (Portuguese Male)', lang: 'pt-BR' },
  { value: 'pm_santa', label: 'Santa (Portuguese Male)', lang: 'pt-BR' },
]

const LANGUAGES: { value: string; label: string }[] = [
  { value: 'en-US', label: 'English (US)' },
  { value: 'en-GB', label: 'English (UK)' },
  { value: 'es-ES', label: 'Spanish' },
  { value: 'fr-FR', label: 'French' },
  { value: 'hi-IN', label: 'Hindi' },
  { value: 'it-IT', label: 'Italian' },
  { value: 'ja-JP', label: 'Japanese' },
  { value: 'zh-CN', label: 'Chinese (Mandarin)' },
  { value: 'pt-BR', label: 'Portuguese (Brazil)' },
]

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

      {tab === 'messages' && <MessagesTab retentionDays={mailbox.retention_days ?? null} />}
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
      const date = new Date(msg.created_at).toISOString().slice(0, 10)
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

  // TTS dialog state
  const [ttsTarget, setTtsTarget] = useState<string | null>(null)
  const [ttsText, setTtsText] = useState('')
  const [ttsLang, setTtsLang] = useState('en-US')
  const [ttsVoice, setTtsVoice] = useState('af_heart')
  const [ttsExpires, setTtsExpires] = useState('')
  const [ttsSaving, setTtsSaving] = useState(false)
  const [ttsError, setTtsError] = useState<string | null>(null)

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

  const openTtsDialog = (type: string) => {
    setTtsTarget(type)
    setTtsText('')
    setTtsLang('en-US')
    setTtsVoice('af_heart')
    setTtsExpires('')
    setTtsError(null)
  }

  const handleTtsSynthesize = async () => {
    if (!ttsTarget || !ttsText.trim()) return
    setTtsSaving(true)
    setTtsError(null)

    try {
      const hours = ttsTarget === 'temporary' && ttsExpires ? parseInt(ttsExpires) : undefined
      const greeting = await me.synthesizeGreeting({
        greeting_type: ttsTarget,
        text: ttsText,
        language: ttsLang,
        voice: ttsVoice,
        expires_in_hours: hours,
      })
      setGreetings((prev) => {
        const filtered = prev.filter((g) => g.greeting_type !== ttsTarget)
        return [...filtered, greeting]
      })
      setTtsTarget(null)
    } catch (err) {
      setTtsError(err instanceof Error ? err.message : 'TTS synthesis failed')
    } finally {
      setTtsSaving(false)
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

  const filteredVoices = KOKORO_VOICES.filter((v) => v.lang === ttsLang)

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
        const isExpired = greeting?.expires_at && Date.parse(greeting.expires_at) < Date.now()

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
                    Expires {new Date(greeting.expires_at).toLocaleString()}
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
                    <Button variant="ghost" size="icon" title="Replace with TTS" onClick={() => openTtsDialog(gt.id)}>
                      <Volume2 className="h-4 w-4" />
                    </Button>
                    <Button variant="ghost" size="icon" title="Replace with file" onClick={() => handleUpload(gt.id)}>
                      <Upload className="h-4 w-4" />
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
                    <Button variant="outline" size="sm" className="gap-1" onClick={() => openTtsDialog(gt.id)}>
                      <Volume2 className="h-3 w-3" />
                      TTS
                    </Button>
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

      {/* TTS Synthesis Dialog */}
      <Dialog open={!!ttsTarget} onOpenChange={(open) => { if (!open) setTtsTarget(null) }}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Generate Greeting with TTS</DialogTitle>
            <DialogDescription>
              Type your greeting message and choose a voice. The audio will be generated automatically.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>Greeting Text</Label>
              <textarea
                value={ttsText}
                onChange={(e) => setTtsText(e.target.value)}
                placeholder="Hi, you've reached my voicemail. Please leave a message after the tone."
                rows={3}
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label>Language</Label>
                <Select value={ttsLang} onValueChange={(v) => { setTtsLang(v); setTtsVoice(KOKORO_VOICES.find((x) => x.lang === v)?.value || 'af_heart') }}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {LANGUAGES.map((l) => (
                      <SelectItem key={l.value} value={l.value}>{l.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label>Voice</Label>
                <Select value={ttsVoice} onValueChange={setTtsVoice}>
                  <SelectTrigger><SelectValue placeholder="Select voice" /></SelectTrigger>
                  <SelectContent>
                    {filteredVoices.map((v) => (
                      <SelectItem key={v.value} value={v.value}>{v.label}</SelectItem>
                    ))}
                    {filteredVoices.length === 0 && (
                      <SelectItem value="af_heart" disabled>No voices for this language</SelectItem>
                    )}
                  </SelectContent>
                </Select>
              </div>
            </div>
            {ttsTarget === 'temporary' && (
              <div>
                <Label>Expires in (hours, optional)</Label>
                <Input
                  type="number"
                  min={1}
                  value={ttsExpires}
                  onChange={(e) => setTtsExpires(e.target.value)}
                  placeholder="e.g. 24"
                  className="w-32"
                />
              </div>
            )}
            {ttsError && (
              <p className="text-sm text-destructive">{ttsError}</p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setTtsTarget(null)}>Cancel</Button>
              <Button onClick={handleTtsSynthesize} disabled={ttsSaving || !ttsText.trim()}>
                {ttsSaving ? 'Generating...' : 'Generate Greeting'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
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

function ExpiryIndicator({ createdAt, retentionDays }: { createdAt: string; retentionDays: number }) {
  const expiresAt = Date.parse(createdAt) + retentionDays * 86_400_000 // ms
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

function formatDate(at: string): string {
  return new Date(at).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })
}
