// The voices the TTS service speaks for a language (docs/TENANT_SPEECH.md
// S2b), for every voice picker: the Speech page, Prompts and the flow
// builder. Empty where there is no TTS service: the default is used.
import { useQuery } from '@tanstack/react-query'
import { speech } from '@/api/endpoints'

export interface VoiceOption {
  value: string
  label: string
}

export function useVoices(language: string | undefined, provider?: string): VoiceOption[] {
  const lang = language || 'en-US'
  const { data } = useQuery({
    queryKey: ['speech', 'voices', provider ?? '', lang],
    queryFn: () => speech.voices(lang, provider),
    staleTime: 5 * 60_000,
    retry: false,
  })
  return (data ?? []).map((v) => ({
    value: v.name,
    label: v.display_name && v.display_name !== v.name ? `${v.display_name} (${v.name})` : v.name,
  }))
}
