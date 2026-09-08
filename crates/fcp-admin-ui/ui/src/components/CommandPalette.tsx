import { useState, useEffect, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  LayoutDashboard, Users, Phone, MonitorSmartphone, Link2,
  Building2, PhoneCall, FileText, Headphones, Voicemail,
  Radio, Server, Shield, Workflow, Globe, Key,
  ScrollText, Search, Disc,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  Dialog, DialogContent,
} from '@/components/ui/dialog'

interface CommandItem {
  label: string
  href: string
  icon: React.ElementType
  keywords: string[]
}

const commands: CommandItem[] = [
  { label: 'Dashboard', href: '/admin', icon: LayoutDashboard, keywords: ['home', 'overview', 'stats'] },
  { label: 'Users', href: '/admin/users', icon: Users, keywords: ['accounts', 'people', 'credentials'] },
  { label: 'Extensions', href: '/admin/extensions', icon: Phone, keywords: ['ext', 'numbers', 'dial'] },
  { label: 'Devices', href: '/admin/devices', icon: MonitorSmartphone, keywords: ['phones', 'hardware', 'mac'] },
  { label: 'Phone Numbers', href: '/admin/phone-numbers', icon: Phone, keywords: ['did', 'numbers', 'e911'] },
  { label: 'Line Bindings', href: '/admin/line-bindings', icon: Link2, keywords: ['binding', 'assign'] },
  { label: 'Tenants', href: '/admin/tenants', icon: Building2, keywords: ['organization', 'company'] },
  { label: 'Active Calls', href: '/admin/calls', icon: PhoneCall, keywords: ['live', 'monitor', 'hangup'] },
  { label: 'Call History (CDRs)', href: '/admin/cdrs', icon: FileText, keywords: ['records', 'history', 'logs'] },
  { label: 'Queues', href: '/admin/queues', icon: Headphones, keywords: ['acd', 'agents', 'waiting'] },
  { label: 'Voicemail', href: '/admin/voicemail', icon: Voicemail, keywords: ['mailbox', 'messages'] },
  { label: 'Presence', href: '/admin/presence', icon: Radio, keywords: ['status', 'available', 'busy'] },
  { label: 'SIP Trunks', href: '/admin/trunks', icon: Server, keywords: ['sbc', 'carrier', 'pstn'] },
  { label: 'Routing Rules', href: '/admin/routing', icon: Shield, keywords: ['policy', 'dialplan', 'rules'] },
  { label: 'IVR Flows', href: '/admin/flows', icon: Workflow, keywords: ['ivr', 'menu', 'builder'] },
  { label: 'Cluster Monitor', href: '/admin/cluster', icon: Globe, keywords: ['nodes', 'health', 'system'] },
  { label: 'Number Porting', href: '/admin/porting', icon: Phone, keywords: ['port', 'transfer', 'carrier'] },
  { label: 'Recordings', href: '/admin/recordings', icon: Disc, keywords: ['recording', 'conference', 'webm', 'wav', 'playback'] },
  { label: 'Audit Log', href: '/admin/audit', icon: ScrollText, keywords: ['audit', 'who', 'changes'] },
  { label: 'API Tokens', href: '/admin/tokens', icon: Key, keywords: ['api', 'key', 'secret'] },
]

export function CommandPalette() {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [selectedIndex, setSelectedIndex] = useState(0)
  const navigate = useNavigate()

  // Listen for Cmd+K / Ctrl+K
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault()
        setOpen((prev) => !prev)
        setSearch('')
        setSelectedIndex(0)
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [])

  const filtered = search
    ? commands.filter((cmd) => {
        const q = search.toLowerCase()
        return (
          cmd.label.toLowerCase().includes(q) ||
          cmd.keywords.some((kw) => kw.includes(q))
        )
      })
    : commands

  const go = useCallback((href: string) => {
    navigate(href)
    setOpen(false)
    setSearch('')
  }, [navigate])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setSelectedIndex((i) => Math.max(i - 1, 0))
    } else if (e.key === 'Enter' && filtered[selectedIndex]) {
      go(filtered[selectedIndex].href)
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-md p-0 gap-0">
        <div className="flex items-center border-b px-3">
          <Search className="h-4 w-4 text-muted-foreground mr-2" />
          <input
            className="flex-1 py-3 text-sm bg-transparent outline-none placeholder:text-muted-foreground"
            placeholder="Search pages..."
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIndex(0) }}
            onKeyDown={handleKeyDown}
            autoFocus
          />
          <kbd className="text-xs text-muted-foreground bg-muted rounded px-1.5 py-0.5">esc</kbd>
        </div>
        <div className="max-h-72 overflow-y-auto p-1">
          {filtered.length === 0 ? (
            <p className="text-sm text-muted-foreground text-center py-6">No results found.</p>
          ) : (
            filtered.map((cmd, i) => {
              const Icon = cmd.icon
              return (
                <button
                  key={cmd.href}
                  className={cn(
                    'flex items-center gap-3 w-full rounded-md px-3 py-2 text-sm transition-colors',
                    i === selectedIndex
                      ? 'bg-accent text-accent-foreground'
                      : 'text-foreground hover:bg-accent/50',
                  )}
                  onClick={() => go(cmd.href)}
                  onMouseEnter={() => setSelectedIndex(i)}
                >
                  <Icon className="h-4 w-4 text-muted-foreground" />
                  {cmd.label}
                </button>
              )
            })
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
