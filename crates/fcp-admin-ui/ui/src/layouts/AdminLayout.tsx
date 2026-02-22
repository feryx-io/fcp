import { useState, useEffect } from 'react'
import { Link, Outlet, useLocation } from 'react-router-dom'
import {
  LayoutDashboard, Users, Phone, Boxes, MonitorSmartphone,
  Link2, Building2, Settings, LogOut, Moon, Sun, Monitor, Menu,
  ChevronLeft, PhoneCall, FileText, Headphones, Voicemail,
  Radio, Server, Shield, Workflow, Globe, Key,
  ScrollText, Search, X,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { TooltipProvider, Tooltip, TooltipTrigger, TooltipContent } from '@/components/ui/tooltip'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'
import { useWebSocketStore } from '@/stores/websocket'
import { ToastContainer } from '@/components/ToastContainer'
import { CommandPalette } from '@/components/CommandPalette'

interface NavItem {
  label: string
  href: string
  icon: React.ElementType
  section?: string
}

const navItems: NavItem[] = [
  { label: 'Dashboard', href: '/admin', icon: LayoutDashboard },
  // Management
  { label: 'Users', href: '/admin/users', icon: Users, section: 'Management' },
  { label: 'Extensions', href: '/admin/extensions', icon: Phone, section: 'Management' },
  { label: 'Devices', href: '/admin/devices', icon: MonitorSmartphone, section: 'Management' },
  { label: 'Phone Numbers', href: '/admin/phone-numbers', icon: Phone, section: 'Management' },
  { label: 'Line Bindings', href: '/admin/line-bindings', icon: Link2, section: 'Management' },
  { label: 'Tenants', href: '/admin/tenants', icon: Building2, section: 'Management' },
  // Operations
  { label: 'Active Calls', href: '/admin/calls', icon: PhoneCall, section: 'Operations' },
  { label: 'Call History', href: '/admin/cdrs', icon: FileText, section: 'Operations' },
  { label: 'Queues', href: '/admin/queues', icon: Headphones, section: 'Operations' },
  { label: 'Voicemail', href: '/admin/voicemail', icon: Voicemail, section: 'Operations' },
  { label: 'Presence', href: '/admin/presence', icon: Radio, section: 'Operations' },
  // Advanced
  { label: 'SIP Trunks', href: '/admin/trunks', icon: Server, section: 'Advanced' },
  { label: 'Routing Rules', href: '/admin/routing', icon: Shield, section: 'Advanced' },
  { label: 'IVR Flows', href: '/admin/flows', icon: Workflow, section: 'Advanced' },
  { label: 'Cluster', href: '/admin/cluster', icon: Globe, section: 'Advanced' },
  { label: 'Number Porting', href: '/admin/porting', icon: Phone, section: 'Advanced' },
  { label: 'Audit Log', href: '/admin/audit', icon: ScrollText, section: 'Advanced' },
  { label: 'API Tokens', href: '/admin/tokens', icon: Key, section: 'Advanced' },
]

const themeIcon = (theme: string) => {
  switch (theme) {
    case 'dark': return Sun
    case 'light': return Moon
    default: return Monitor
  }
}

const nextTheme = (current: string): 'light' | 'dark' | 'system' => {
  switch (current) {
    case 'light': return 'dark'
    case 'dark': return 'system'
    default: return 'light'
  }
}

const themeLabel = (theme: string) => {
  switch (theme) {
    case 'dark': return 'Dark'
    case 'light': return 'Light'
    default: return 'System'
  }
}

export function AdminLayout() {
  const [collapsed, setCollapsed] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)
  const location = useLocation()
  const { user, logout } = useAuthStore()
  const { theme, setTheme } = useThemeStore()
  const { connected, connect, disconnect } = useWebSocketStore()

  // Connect WebSocket on mount
  useEffect(() => {
    connect()
    return () => disconnect()
  }, [connect, disconnect])

  // Close mobile drawer on navigation
  useEffect(() => {
    setMobileOpen(false)
  }, [location.pathname])

  const isActive = (href: string) => {
    if (href === '/admin') return location.pathname === '/admin'
    return location.pathname.startsWith(href)
  }

  // Group nav items by section
  const sections: { label: string | null; items: NavItem[] }[] = []
  let currentSection: string | null | undefined = undefined
  navItems.forEach((item) => {
    const section = item.section ?? null
    if (section !== currentSection) {
      sections.push({ label: section, items: [] })
      currentSection = section
    }
    sections[sections.length - 1].items.push(item)
  })

  const ThemeIcon = themeIcon(theme)

  const sidebarContent = (
    <>
      {/* Navigation */}
      <nav className="flex-1 overflow-y-auto p-2" aria-label="Main navigation">
        {sections.map((section, sectionIdx) => (
          <div key={sectionIdx}>
            {section.label && !collapsed && (
              <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wider px-3 pt-4 pb-1">
                {section.label}
              </p>
            )}
            {section.label && collapsed && sectionIdx > 0 && (
              <div className="my-2 border-t" />
            )}
            {section.items.map((item) => {
              const Icon = item.icon
              const active = isActive(item.href)

              if (collapsed && !mobileOpen) {
                return (
                  <Tooltip key={item.href}>
                    <TooltipTrigger asChild>
                      <Link
                        to={item.href}
                        aria-current={active ? 'page' : undefined}
                        className={cn(
                          'flex items-center justify-center rounded-md p-2 my-1 transition-colors',
                          active
                            ? 'bg-primary text-primary-foreground'
                            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                        )}
                      >
                        <Icon className="h-5 w-5" />
                      </Link>
                    </TooltipTrigger>
                    <TooltipContent side="right">{item.label}</TooltipContent>
                  </Tooltip>
                )
              }

              return (
                <Link
                  key={item.href}
                  to={item.href}
                  aria-current={active ? 'page' : undefined}
                  className={cn(
                    'flex items-center gap-3 rounded-md px-3 py-2 my-1 text-sm font-medium transition-colors',
                    active
                      ? 'bg-primary text-primary-foreground'
                      : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                  )}
                >
                  <Icon className="h-5 w-5" />
                  {item.label}
                </Link>
              )
            })}
          </div>
        ))}
      </nav>

      {/* Footer */}
      <div className="border-t p-2">
        {collapsed && !mobileOpen ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="w-full"
                aria-label={`Switch theme (current: ${themeLabel(theme)})`}
                onClick={() => setTheme(nextTheme(theme))}
              >
                <ThemeIcon className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">Theme: {themeLabel(theme)}</TooltipContent>
          </Tooltip>
        ) : (
          <Button
            variant="ghost"
            className="w-full justify-start gap-2"
            aria-label={`Switch theme (current: ${themeLabel(theme)})`}
            onClick={() => setTheme(nextTheme(theme))}
          >
            <ThemeIcon className="h-4 w-4" />
            {themeLabel(theme)} Mode
          </Button>
        )}
      </div>
    </>
  )

  return (
    <TooltipProvider>
      {/* Skip to content link */}
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-[70] focus:rounded-md focus:bg-primary focus:px-4 focus:py-2 focus:text-primary-foreground"
      >
        Skip to content
      </a>

      <div className="flex h-screen overflow-hidden">
        {/* Mobile overlay */}
        {mobileOpen && (
          <div
            className="fixed inset-0 z-40 bg-black/50 md:hidden"
            onClick={() => setMobileOpen(false)}
            aria-hidden="true"
          />
        )}

        {/* Sidebar — desktop */}
        <aside
          className={cn(
            'hidden md:flex flex-col border-r bg-card transition-all duration-200',
            collapsed ? 'w-16' : 'w-64',
          )}
        >
          {/* Logo */}
          <div className="flex h-14 items-center border-b px-4">
            {!collapsed && (
              <Link to="/admin" className="flex items-center gap-2">
                <Boxes className="h-6 w-6 text-primary" />
                <span className="text-lg font-bold">FCP Admin</span>
              </Link>
            )}
            <Button
              variant="ghost"
              size="icon"
              className={cn('ml-auto', collapsed && 'mx-auto')}
              aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
              onClick={() => setCollapsed(!collapsed)}
            >
              {collapsed ? <Menu className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
            </Button>
          </div>
          {sidebarContent}
        </aside>

        {/* Sidebar — mobile drawer */}
        <aside
          className={cn(
            'fixed inset-y-0 left-0 z-50 flex w-64 flex-col border-r bg-card transition-transform duration-200 md:hidden',
            mobileOpen ? 'translate-x-0' : '-translate-x-full',
          )}
        >
          <div className="flex h-14 items-center justify-between border-b px-4">
            <Link to="/admin" className="flex items-center gap-2">
              <Boxes className="h-6 w-6 text-primary" />
              <span className="text-lg font-bold">FCP Admin</span>
            </Link>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Close menu"
              onClick={() => setMobileOpen(false)}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
          {sidebarContent}
        </aside>

        {/* Main content */}
        <div className="flex flex-1 flex-col overflow-hidden">
          {/* Header */}
          <header className="flex h-14 items-center justify-between border-b bg-card px-4 md:px-6">
            <div className="flex items-center gap-3">
              {/* Mobile menu button */}
              <Button
                variant="ghost"
                size="icon"
                className="md:hidden"
                aria-label="Open menu"
                onClick={() => setMobileOpen(true)}
              >
                <Menu className="h-5 w-5" />
              </Button>
              <Breadcrumbs />
            </div>
            <div className="flex items-center gap-2 md:gap-4">
              {/* Search hint */}
              <button
                className="hidden md:flex items-center gap-2 text-xs text-muted-foreground bg-muted rounded-md px-3 py-1.5 hover:bg-accent transition-colors"
                aria-label="Open search (Ctrl+K)"
                onClick={() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))}
              >
                <Search className="h-3 w-3" />
                Search...
                <kbd className="bg-background rounded px-1 py-0.5 text-[10px]">⌘K</kbd>
              </button>
              {/* WebSocket indicator */}
              <span
                className={cn(
                  'h-2 w-2 rounded-full',
                  connected ? 'bg-green-500' : 'bg-muted-foreground',
                )}
                role="status"
                aria-label={connected ? 'Live connection active' : 'Disconnected'}
              />
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" className="gap-2" aria-label="User menu">
                    <div className="flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground text-xs font-bold">
                      {(user?.display_name || user?.username || 'U').charAt(0).toUpperCase()}
                    </div>
                    <span className="hidden md:inline">{user?.username}</span>
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuLabel>
                    {user?.display_name || user?.username}
                    <p className="text-xs font-normal text-muted-foreground">{user?.role}</p>
                  </DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem asChild>
                    <Link to="/admin/settings">
                      <Settings className="mr-2 h-4 w-4" />
                      Settings
                    </Link>
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onClick={logout}>
                    <LogOut className="mr-2 h-4 w-4" />
                    Log out
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </header>

          {/* Page content */}
          <main id="main-content" className="flex-1 overflow-y-auto p-4 md:p-6">
            <Outlet />
          </main>
        </div>

        {/* Toast notifications */}
        <ToastContainer />

        {/* Command palette (Cmd+K) */}
        <CommandPalette />
      </div>
    </TooltipProvider>
  )
}

function Breadcrumbs() {
  const location = useLocation()
  const segments = location.pathname.split('/').filter(Boolean)

  const breadcrumbs = segments.map((segment, index) => {
    const href = '/' + segments.slice(0, index + 1).join('/')
    const label = segment
      .replace(/-/g, ' ')
      .replace(/\b\w/g, (c) => c.toUpperCase())
    return { href, label }
  })

  return (
    <nav aria-label="Breadcrumb" className="flex items-center gap-1 text-sm text-muted-foreground">
      {breadcrumbs.map((crumb, i) => (
        <span key={crumb.href} className="flex items-center gap-1">
          {i > 0 && <span aria-hidden="true">/</span>}
          {i === breadcrumbs.length - 1 ? (
            <span className="font-medium text-foreground" aria-current="page">{crumb.label}</span>
          ) : (
            <Link to={crumb.href} className="hover:text-foreground transition-colors">
              {crumb.label}
            </Link>
          )}
        </span>
      ))}
    </nav>
  )
}
