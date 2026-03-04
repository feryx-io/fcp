import { useState, useEffect } from 'react'
import { Link, Outlet, useLocation, useNavigate } from 'react-router-dom'
import {
  LayoutDashboard, Voicemail, PhoneForwarded, Settings, LogOut,
  Moon, Sun, Monitor, Menu, X, Boxes,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'
import { me } from '@/api/endpoints'

interface NavItem {
  label: string
  href: string
  icon: React.ElementType
}

const navItems: NavItem[] = [
  { label: 'Dashboard', href: '/user', icon: LayoutDashboard },
  { label: 'Voicemail', href: '/user/voicemail', icon: Voicemail },
  { label: 'Call Forwarding', href: '/user/forwarding', icon: PhoneForwarded },
  { label: 'Settings', href: '/user/settings', icon: Settings },
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

export function UserLayout() {
  const [mobileOpen, setMobileOpen] = useState(false)
  const location = useLocation()
  const navigate = useNavigate()
  const { user, logout } = useAuthStore()
  const { theme, setTheme } = useThemeStore()
  const [headerAvatarUrl, setHeaderAvatarUrl] = useState<string | null>(null)

  // Fetch avatar for header
  useEffect(() => {
    me.getProfile().then((p) => {
      if (p.has_profile_picture) {
        me.fetchAvatar().then(setHeaderAvatarUrl).catch(() => {})
      }
    }).catch(() => {})
  }, [])

  // Close mobile drawer on navigation
  useEffect(() => {
    setMobileOpen(false)
  }, [location.pathname])

  const isActive = (href: string) => {
    if (href === '/user') return location.pathname === '/user'
    return location.pathname.startsWith(href)
  }

  const handleLogout = () => {
    logout()
    navigate('/login')
  }

  const ThemeIcon = themeIcon(theme)

  const sidebarContent = (
    <>
      <nav className="flex-1 overflow-y-auto p-2" aria-label="Main navigation">
        {navItems.map((item) => {
          const Icon = item.icon
          const active = isActive(item.href)
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
      </nav>
      <div className="border-t p-2">
        <Button
          variant="ghost"
          className="w-full justify-start gap-2"
          aria-label={`Switch theme (current: ${themeLabel(theme)})`}
          onClick={() => setTheme(nextTheme(theme))}
        >
          <ThemeIcon className="h-4 w-4" />
          {themeLabel(theme)} Mode
        </Button>
      </div>
    </>
  )

  return (
    <>
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
        <aside className="hidden md:flex w-64 flex-col border-r bg-card">
          <div className="flex h-14 items-center border-b px-4">
            <Link to="/user" className="flex items-center gap-2">
              <Boxes className="h-6 w-6 text-primary" />
              <span className="text-lg font-bold">FCP Portal</span>
            </Link>
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
            <Link to="/user" className="flex items-center gap-2">
              <Boxes className="h-6 w-6 text-primary" />
              <span className="text-lg font-bold">FCP Portal</span>
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
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" className="gap-2" aria-label="User menu">
                  {headerAvatarUrl ? (
                    <img src={headerAvatarUrl} alt="" className="h-8 w-8 rounded-full object-cover" />
                  ) : (
                    <div className="flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground text-xs font-bold">
                      {(user?.display_name || user?.username || 'U').charAt(0).toUpperCase()}
                    </div>
                  )}
                  <span className="hidden md:inline">{user?.display_name || user?.username}</span>
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuLabel>
                  {user?.display_name || user?.username}
                  <p className="text-xs font-normal text-muted-foreground">{user?.role}</p>
                </DropdownMenuLabel>
                <DropdownMenuSeparator />
                <DropdownMenuItem asChild>
                  <Link to="/user/settings">
                    <Settings className="mr-2 h-4 w-4" />
                    Settings
                  </Link>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem onClick={handleLogout}>
                  <LogOut className="mr-2 h-4 w-4" />
                  Log out
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </header>

          {/* Page content */}
          <main id="main-content" className="flex-1 overflow-y-auto p-4 md:p-6">
            <Outlet />
          </main>
        </div>
      </div>
    </>
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
