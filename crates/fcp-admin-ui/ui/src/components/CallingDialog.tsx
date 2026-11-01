import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { CallingSettingsForm } from '@/components/CallingSettingsForm'
import { classesOfService, prompts, schedules, users } from '@/api/endpoints'
import type { UpdateCallingRequest, User } from '@/types'

/** An admin's view of a user's calling settings (docs/ENTERPRISE_CALLING.md §3.1). */
export function CallingDialog({ user, onOpenChange }: { user: User | null; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient()
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const username = user?.username ?? ''
  const query = useQuery({
    queryKey: ['users', username, 'calling'],
    queryFn: () => users.getCalling(username),
    enabled: !!user,
  })
  const music = useQuery({
    queryKey: ['prompts', 'hold-music'],
    queryFn: () => prompts.list(),
    enabled: !!user,
  })
  const scheduleList = useQuery({
    queryKey: ['schedules'],
    queryFn: schedules.list,
    enabled: !!user,
  })
  const classList = useQuery({
    queryKey: ['classes-of-service'],
    queryFn: classesOfService.list,
    enabled: !!user,
  })
  const people = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
    enabled: !!user,
  })
  const presence = useQuery({
    queryKey: ['users', username, 'presence'],
    queryFn: () => users.getPresence(username),
    enabled: !!user,
  })
  const mutation = useMutation({
    mutationFn: (data: UpdateCallingRequest) => users.updateCalling(username, data),
    onSuccess: (updated) => {
      queryClient.setQueryData(['users', username, 'calling'], updated)
      setMessage({ type: 'success', text: 'Calling settings updated.' })
    },
    onError: (err) => setMessage({ type: 'error', text: err instanceof Error ? err.message : 'Failed to update' }),
  })

  return (
    <Dialog open={!!user} onOpenChange={(v) => { if (!v) setMessage(null); onOpenChange(v) }}>
      <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Calling Settings</DialogTitle>
          <DialogDescription>How calls to {user?.display_name || user?.username} are placed.</DialogDescription>
        </DialogHeader>
        {query.isLoading && (
          <div className="flex h-24 items-center justify-center text-muted-foreground">
            <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
          </div>
        )}
        {presence.data && <PresenceLine presence={presence.data} />}
        {query.isError && (
          <p className="text-sm text-destructive">{(query.error as Error).message}</p>
        )}
        {query.data && (
          <CallingSettingsForm
            settings={query.data}
            onSave={(req) => { setMessage(null); mutation.mutate(req) }}
            saving={mutation.isPending}
            message={message}
            schedules={(scheduleList.data ?? []).map((s) => ({ id: s.id, name: s.name }))}
            classes={(classList.data ?? []).map((c) => ({ id: c.id, name: c.name, is_default: c.is_default }))}
            people={(people.data ?? []).map((u) => ({ id: u.id, name: u.display_name || u.username }))}
            holdMusicPrompts={(music.data ?? []).map((p) => ({ id: p.id, name: `${p.name} (${p.category})` }))}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

const shownAs: Record<string, string> = {
  available: 'Available',
  away: 'Away',
  busy: 'Busy',
  dnd: 'Do not disturb',
  ringing: 'Ringing',
  on_call: 'On a call',
  offline: 'Offline',
}

/** What others see of the user, and the status they chose: theirs to change. */
function PresenceLine({ presence }: { presence: NonNullable<Awaited<ReturnType<typeof users.getPresence>>> }) {
  const p = presence.presence
  const parts = [
    p.idle ? 'Away (idle)' : (shownAs[p.state] ?? p.state),
    p.label,
    p.note,
  ].filter(Boolean)
  const chosen = presence.status_label ?? shownAs[presence.status_state] ?? presence.status
  return (
    <p className="text-sm text-muted-foreground" data-testid="user-presence">
      Presence: <span className="text-foreground">{parts.join(' · ')}</span>
      {' '}(chose {chosen}
      {presence.until ? `, until ${new Date(presence.until).toLocaleString()}` : ''})
    </p>
  )
}
