// Which greeting a mailbox's callers hear (#343): the standard greeting,
// one spoken with the owner's name, or the owner's own recording. Used by
// the administrator's Voicemail page and the user's own voicemail page.
import { Label } from '@/components/ui/label'

export const GREETING_CHOICES = [
  { value: '', label: 'Default', description: 'With the name when the owner has a first and last name, standard otherwise.' },
  { value: 'standard', label: 'Standard', description: 'The system greeting.' },
  { value: 'personalized', label: 'With the name', description: '"Sorry, <name> is not available. Please leave a message after the tone."' },
  { value: 'own', label: 'Own recording', description: 'The busy, temporary and unavailable greetings recorded for the mailbox.' },
] as const

export const GREETING_NAMES = [
  { value: 'full_name', label: 'First and last name' },
  { value: 'display_name', label: 'Display name' },
  { value: 'recorded', label: 'Recorded name greeting' },
] as const

export const HEARD_LABEL: Record<string, string> = {
  standard: 'The standard greeting',
  personalized: 'The greeting with the name',
  own: 'The own recording',
}

interface Props {
  id: string
  choice: string
  name: string
  /** What callers hear now, as the mailbox last said. */
  heard?: string
  onChange: (choice: string, name: string) => void
}

export function GreetingChoiceFields({ id, choice, name, heard, onChange }: Props) {
  const picked = GREETING_CHOICES.find(c => c.value === choice) ?? GREETING_CHOICES[0]
  return (
    <div className="space-y-2">
      <Label htmlFor={`${id}-choice`}>Greeting callers hear</Label>
      <select
        id={`${id}-choice`}
        aria-label="Greeting callers hear"
        className="w-full border rounded px-3 py-2 text-sm bg-background"
        value={choice}
        onChange={e => onChange(e.target.value, name)}
      >
        {GREETING_CHOICES.map(c => <option key={c.value} value={c.value}>{c.label}</option>)}
      </select>
      <p className="text-xs text-muted-foreground">{picked.description}</p>
      {choice !== 'standard' && choice !== 'own' && (
        <>
          <Label htmlFor={`${id}-name`}>The name it says</Label>
          <select
            id={`${id}-name`}
            aria-label="The name it says"
            className="w-full border rounded px-3 py-2 text-sm bg-background"
            value={name}
            onChange={e => onChange(choice, e.target.value)}
          >
            {GREETING_NAMES.map(n => <option key={n.value} value={n.value}>{n.label}</option>)}
          </select>
        </>
      )}
      {heard && (
        <p className="text-xs" data-testid="greeting-heard">
          Callers hear now: <span className="font-medium">{HEARD_LABEL[heard] ?? heard}</span>
        </p>
      )}
    </div>
  )
}
