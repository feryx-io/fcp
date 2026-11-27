// The tenant's speech (docs/TENANT_SPEECH.md, S2b): the provider, voice and
// language the TTS service speaks the tenant's words with when a prompt or
// a flow names none, a voice per language, and a preview of any choice
// before it is saved.
import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Play, Plus, RotateCcw, Save, Trash2 } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { speech } from '@/api/endpoints'
import { useVoices } from '@/lib/voices'
import { PlatformOffer, SpeechAccounts } from './SpeechAccounts'
import { useIsPlatformAdmin } from '@/components/TenantSelector'

const DEFAULT = '__default__'

/** A voice picker for `language` with `provider`'s voices. */
function VoiceSelect({ id, language, provider, value, onChange }: {
  id: string
  language: string
  provider?: string
  value: string
  onChange: (v: string) => void
}) {
  const voices = useVoices(language, provider)
  return (
    <Select value={value || DEFAULT} onValueChange={(v) => onChange(v === DEFAULT ? '' : v)}>
      <SelectTrigger id={id} aria-label={id}><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT}>The provider's default</SelectItem>
        {voices.map((v) => <SelectItem key={v.value} value={v.value}>{v.label}</SelectItem>)}
        {value && !voices.some((v) => v.value === value) && <SelectItem value={value}>{value}</SelectItem>}
      </SelectContent>
    </Select>
  )
}

export function SpeechPage() {
  const queryClient = useQueryClient()
  const platformAdmin = useIsPlatformAdmin()
  const settings = useQuery({ queryKey: ['speech', 'settings'], queryFn: speech.settings })
  const providers = useQuery({ queryKey: ['speech', 'providers'], queryFn: speech.providers, retry: false })
  const [provider, setProvider] = useState('')
  const [language, setLanguage] = useState('')
  const [voice, setVoice] = useState('')
  const [extra, setExtra] = useState<{ language: string; voice: string }[]>([])
  const [text, setText] = useState('Thank you for calling. How can we help you today?')
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const audio = useRef<HTMLAudioElement | null>(null)

  useEffect(() => {
    const s = settings.data
    if (!s) return
    setProvider(s.provider ?? '')
    setLanguage(s.language ?? '')
    setVoice(s.voice ?? '')
    setExtra(Object.entries(s.voices ?? {}).map(([l, v]) => ({ language: l, voice: v })))
  }, [settings.data])

  const defaultProvider = useMemo(
    () => providers.data?.find((p) => p.is_default)?.name,
    [providers.data],
  )
  const spokenLanguage = language || 'en-US'

  const save = useMutation({
    mutationFn: () => speech.save({
      provider: provider || null,
      voice: voice || null,
      language: language || null,
      voices: Object.fromEntries(extra.filter((e) => e.language.trim()).map((e) => [e.language.trim(), e.voice])),
    }),
    onSuccess: () => {
      setError(null)
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
      queryClient.invalidateQueries({ queryKey: ['speech', 'settings'] })
    },
    onError: (e) => setError((e as Error).message),
  })
  const reset = useMutation({
    mutationFn: speech.reset,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['speech', 'settings'] }),
    onError: (e) => setError((e as Error).message),
  })

  const play = async (lang: string, v: string) => {
    try {
      setError(null)
      audio.current?.pause()
      const url = await speech.preview({
        text,
        provider: provider || undefined,
        voice: v || undefined,
        language: lang || undefined,
      })
      const a = new Audio(url)
      a.onended = () => URL.revokeObjectURL(url)
      audio.current = a
      await a.play()
    } catch (e) {
      setError(`The preview could not be spoken: ${(e as Error).message}`)
    }
  }

  if (settings.isLoading) return null

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Speech</h1>
          <p className="text-muted-foreground">
            What your prompts and flows sound like when they name no voice. Unset, the platform's defaults.
          </p>
        </div>
        <div className="flex gap-2">
          {settings.data?.configured && (
            <Button variant="outline" onClick={() => { if (confirm("Go back to the platform's defaults?")) reset.mutate() }}>
              <RotateCcw className="mr-2 h-4 w-4" /> Defaults
            </Button>
          )}
          <Button onClick={() => save.mutate()} disabled={save.isPending}>
            <Save className="mr-2 h-4 w-4" /> {saved ? 'Saved' : 'Save'}
          </Button>
        </div>
      </div>
      {error && <p className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{error}</p>}
      {(providers.error || providers.data?.length === 0) && (
        <p className="rounded border p-3 text-sm text-muted-foreground">
          No TTS service answers this admin, so there are no voices to list or preview. Settings can still be saved.
        </p>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Voice</CardTitle>
          <CardDescription>The provider, language and voice a prompt or flow without its own is spoken with.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-3">
          <div className="space-y-1">
            <Label htmlFor="speech-provider">Provider</Label>
            <Select value={provider || DEFAULT} onValueChange={(v) => { setProvider(v === DEFAULT ? '' : v); setVoice('') }}>
              <SelectTrigger id="speech-provider" aria-label="Provider"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={DEFAULT}>The platform's{defaultProvider ? ` (${defaultProvider})` : ''}</SelectItem>
                {(providers.data ?? []).map((p) => <SelectItem key={p.name} value={p.name}>{p.account ? `${p.label} (your account)` : p.label}</SelectItem>)}
                {provider && !(providers.data ?? []).some((p) => p.name === provider) && (
                  <SelectItem value={provider}>{provider}</SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label htmlFor="speech-language">Language</Label>
            <Input id="speech-language" placeholder="en-US" value={language} onChange={(e) => setLanguage(e.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="speech-voice">Voice</Label>
            <VoiceSelect id="speech-voice" language={spokenLanguage} provider={provider || undefined}
              value={voice} onChange={setVoice} />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0">
          <div>
            <CardTitle>Voices by language</CardTitle>
            <CardDescription>A language spoken in another voice: `es` for every Spanish, or `es-MX` alone.</CardDescription>
          </div>
          <Button variant="outline" size="sm" onClick={() => setExtra((x) => [...x, { language: '', voice: '' }])}>
            <Plus className="mr-2 h-4 w-4" /> Add
          </Button>
        </CardHeader>
        <CardContent className="space-y-3">
          {extra.length === 0 && <p className="text-sm text-muted-foreground">Every language in the voice above.</p>}
          {extra.map((e, i) => (
            <div key={i} className="grid items-end gap-2 sm:grid-cols-[10rem_1fr_auto_auto]">
              <div className="space-y-1">
                <Label htmlFor={`speech-lang-${i}`}>Language</Label>
                <Input id={`speech-lang-${i}`} placeholder="es-ES" value={e.language}
                  onChange={(ev) => setExtra((x) => x.map((y, j) => j === i ? { ...y, language: ev.target.value } : y))} />
              </div>
              <div className="space-y-1">
                <Label htmlFor={`speech-voice-${i}`}>Voice</Label>
                <VoiceSelect id={`speech-voice-${i}`} language={e.language || spokenLanguage} provider={provider || undefined}
                  value={e.voice} onChange={(v) => setExtra((x) => x.map((y, j) => j === i ? { ...y, voice: v } : y))} />
              </div>
              <Button variant="outline" size="icon" aria-label={`Play ${e.language}`} onClick={() => play(e.language, e.voice)}>
                <Play className="h-4 w-4" />
              </Button>
              <Button variant="ghost" size="icon" aria-label={`Remove ${e.language}`}
                onClick={() => setExtra((x) => x.filter((_, j) => j !== i))}>
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>

      <SpeechAccounts />
      {platformAdmin && <PlatformOffer />}

      <Card>
        <CardHeader>
          <CardTitle>Preview</CardTitle>
          <CardDescription>Spoken now with the choice above, saved or not.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-2 sm:flex-row">
          <Input aria-label="Preview text" value={text} maxLength={500} onChange={(e) => setText(e.target.value)} />
          <Button onClick={() => play(language, voice)} disabled={!text.trim()}>
            <Play className="mr-2 h-4 w-4" /> Play
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
