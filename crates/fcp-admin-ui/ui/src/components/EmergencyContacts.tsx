import { Mail, MessageSquare, PhoneCall, Plus, Trash2, Webhook } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import type { ContactKind, NotifyContact } from '@/types'

const CONTACT_KINDS: Array<{ value: ContactKind; label: string; hint: string }> = [
  { value: 'email', label: 'Email', hint: 'security@example.com' },
  { value: 'sms', label: 'SMS', hint: '+15185550143' },
  { value: 'call', label: 'Call', hint: '2001 or sip:frontdesk@pbx.example.com' },
  { value: 'webhook', label: 'Webhook', hint: 'a webhook endpoint id' },
]

const KIND_ICON: Record<ContactKind, React.ElementType> = {
  email: Mail,
  sms: MessageSquare,
  call: PhoneCall,
  webhook: Webhook,
}

/** A new, empty contact row. */
export const newContact = (): NotifyContact => ({ kind: 'email', target: '', listen: false })

/** The contacts as the API takes them: blank rows dropped, listening only for a call. */
export const contactsToSave = (contacts: NotifyContact[]) =>
  contacts
    .filter((c) => c.target.trim().length > 0)
    .map((c) => ({
      kind: c.kind,
      target: c.target.trim(),
      name: c.name?.trim() || undefined,
      listen: c.kind === 'call' && c.listen,
    }))

/** The add button for a contact list, for a section's header. */
export function AddContactButton({ onAdd }: { onAdd: () => void }) {
  return (
    <Button size="sm" variant="outline" type="button" onClick={onAdd}>
      <Plus className="mr-2 h-4 w-4" /> Add contact
    </Button>
  )
}

/**
 * Who is told about an emergency call (`docs/E911.md` "On-site notification"):
 * the tenant's contacts on the Emergency page, a site's on its Location. A
 * call contact may be allowed to listen in to the call once told.
 */
export function EmergencyContacts({
  contacts,
  onChange,
  empty,
  idPrefix,
}: {
  contacts: NotifyContact[]
  onChange: (contacts: NotifyContact[]) => void
  /** What an empty list means, shown in its place. */
  empty: React.ReactNode
  /** Keeps the listen switches' ids apart when two lists share a page. */
  idPrefix: string
}) {
  const set = (index: number, change: Partial<NotifyContact>) =>
    onChange(contacts.map((c, i) => (i === index ? { ...c, ...change } : c)))

  if (contacts.length === 0) return <>{empty}</>

  return (
    <div className="space-y-2">
      {contacts.map((contact, index) => {
        const Icon = KIND_ICON[contact.kind as ContactKind]
        const hint = CONTACT_KINDS.find((k) => k.value === contact.kind)?.hint
        const listenId = `${idPrefix}-listen-${index}`
        return (
          <div key={index} className="flex flex-wrap items-start gap-2">
            <div className="w-32">
              <Select
                value={contact.kind}
                onValueChange={(v) => set(index, { kind: v as ContactKind })}
              >
                <SelectTrigger aria-label="Contact kind">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CONTACT_KINDS.map((k) => (
                    <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-48 flex-1">
              <div className="flex items-center gap-2">
                <Icon className="h-4 w-4 text-muted-foreground" />
                <Input
                  aria-label="Contact target"
                  value={contact.target}
                  placeholder={hint}
                  onChange={(e) => set(index, { target: e.target.value })}
                />
              </div>
              {(contact.issues ?? []).length > 0 && (
                <p className="mt-1 text-xs text-destructive">{(contact.issues ?? []).join('; ')}</p>
              )}
            </div>
            <div className="w-40">
              <Input
                aria-label="Contact name"
                value={contact.name ?? ''}
                placeholder="Name (optional)"
                onChange={(e) => set(index, { name: e.target.value })}
              />
            </div>
            {contact.kind === 'call' && (
              <div className="flex h-9 items-center gap-2" title="Once told, they may press 1 to listen to the call or 2 to listen and speak">
                <Switch
                  id={listenId}
                  checked={contact.listen}
                  onCheckedChange={(checked) => set(index, { listen: checked })}
                />
                <Label htmlFor={listenId} className="text-xs">May listen in</Label>
              </div>
            )}
            <Button
              variant="ghost"
              size="icon"
              type="button"
              aria-label="Remove contact"
              onClick={() => onChange(contacts.filter((_, i) => i !== index))}
            >
              <Trash2 className="h-4 w-4 text-destructive" />
            </Button>
          </div>
        )
      })}
    </div>
  )
}
