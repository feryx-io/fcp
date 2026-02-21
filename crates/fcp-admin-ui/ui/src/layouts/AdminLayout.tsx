import { useState, useEffect } from 'react'
import { Link, Outlet, useLocation } from 'react-router-dom'
import {
  LayoutDashboard, Users, Phone, Boxes, MonitorSmartphone,
  Link2, Building2, Settings, LogOut, Moon, Sun, Menu,
  ChevronLeft, PhoneCall, FileText, Headphones, Voicemail,
  Radio,
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
]

export function AdminLayout() {
  const [collapsed, setCollapsed] = useState(false)
  const location = useLocation()
  const { user, logout } = useAuthStore()
  const { theme, setTheme } = useThemeStore()
  const { connected, connect, disconnect } = useWebSocketStore()

  // Connect WebSocket on mount
  useEffect(() => {
    connect()
    return () => disconnect()
  }, [connect, disconnect])

  const isActive = (href: string) => {
    if (href === '/admin') return location.pathname === '/admin'
    return location.pathname.startsWith(href)
  }

  // Group nav items by section
  const sections: { label: string | null; items: NavItem[] }[] = []
  let currentSection: string | null = null
  navItems.forEach((item) => {
    const section = item.section ?? null
    if (section !== currentSection) {
      sections.push({ label: section, items: [] })
      currentSection = section
    }
    sections[sections.length - 1].items.push(item)
  })

  return (
    <TooltipProvider>
      <div className="flex h-screen overflow-hidden">
        {/* Sidebar */}
        <aside
          className={cn(
            'flex flex-col border-r bg-card transition-all duration-200',
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
              onClick={() => setCollapsed(!collapsed)}
            >
              {collapsed ? <Menu className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
            </Button>
          </div>

          {/* Navigation */}
          <nav className="flex-1 overflow-y-auto p-2">
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

                  if (collapsed) {
                    return (
                      <Tooltip key={item.href}>
                        <TooltipTrigger asChild>
                          <Link
                            to={item.href}
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
            {collapsed ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="w-full"
                    onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
                  >
                    {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">Toggle theme</TooltipContent>
              </Tooltip>
            ) : (
              <Button
                variant="ghost"
                className="w-full justify-start gap-2"
                onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
              >
                {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                {theme === 'dark' ? 'Light Mode' : 'Dark Mode'}
              </Button>
            )}
          </div>
        </aside>

        {/* Main content */}
        <div className="flex flex-1 flex-col overflow-hidden">
          {/* Header */}
          <header className="flex h-14 items-center justify-between border-b bg-card px-6">
            <Breadcrumbs />
            <div className="flex items-center gap-4">
              {/* WebSocket indicator */}
              <span
                className={cn(
                  'h-2 w-2 rounded-full',
                  connected ? 'bg-green-500' : 'bg-muted-foreground',
                )}
                title={connected ? 'Live connection active' : 'Disconnected'}
              />
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" className="gap-2">
                    <div className="flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground text-xs font-bold">
                      {(user?.display_name || user?.username || 'U').charAt(0).toUpperCase()}
                    </div>
                    {user?.username}
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
          <main className="flex-1 overflow-y-auto p-6">
            <Outlet />
          </main>
        </div>

        {/* Toast notifications */}
        <ToastContainer />
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
    <nav className="flex items-center gap-1 text-sm text-muted-foreground">
      {breadcrumbs.map((crumb, i) => (
        <span key={crumb.href} className="flex items-center gap-1">
          {i > 0 && <span>/</span>}
          {i === breadcrumbs.length - 1 ? (
            <span className="font-medium text-foreground">{crumb.label}</span>
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
