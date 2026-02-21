import { useState } from 'react'
import { Link, Outlet, useLocation } from 'react-router-dom'
import {
  LayoutDashboard, Users, Phone, Boxes, MonitorSmartphone,
  Link2, Building2, Settings, LogOut, Moon, Sun, Menu,
  ChevronLeft,
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

interface NavItem {
  label: string
  href: string
  icon: React.ElementType
}

const navItems: NavItem[] = [
  { label: 'Dashboard', href: '/admin', icon: LayoutDashboard },
  { label: 'Users', href: '/admin/users', icon: Users },
  { label: 'Extensions', href: '/admin/extensions', icon: Phone },
  { label: 'Devices', href: '/admin/devices', icon: MonitorSmartphone },
  { label: 'Phone Numbers', href: '/admin/phone-numbers', icon: Phone },
  { label: 'Line Bindings', href: '/admin/line-bindings', icon: Link2 },
  { label: 'Tenants', href: '/admin/tenants', icon: Building2 },
]

export function AdminLayout() {
  const [collapsed, setCollapsed] = useState(false)
  const location = useLocation()
  const { user, logout } = useAuthStore()
  const { theme, setTheme } = useThemeStore()

  const isActive = (href: string) => {
    if (href === '/admin') return location.pathname === '/admin'
    return location.pathname.startsWith(href)
  }

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
            {navItems.map((item) => {
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
