import { lazy, Suspense } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { AdminLayout } from '@/layouts/AdminLayout'
import { UserLayout } from '@/layouts/UserLayout'
import { useAuthStore } from '@/stores/auth'

// Lazy-loaded pages
const LoginPage = lazy(() => import('@/pages/LoginPage').then((m) => ({ default: m.LoginPage })))
const DashboardPage = lazy(() => import('@/pages/DashboardPage').then((m) => ({ default: m.DashboardPage })))
const UsersPage = lazy(() => import('@/pages/UsersPage').then((m) => ({ default: m.UsersPage })))
const ExtensionsPage = lazy(() => import('@/pages/ExtensionsPage').then((m) => ({ default: m.ExtensionsPage })))
const DevicesPage = lazy(() => import('@/pages/DevicesPage').then((m) => ({ default: m.DevicesPage })))
const PhoneNumbersPage = lazy(() => import('@/pages/PhoneNumbersPage').then((m) => ({ default: m.PhoneNumbersPage })))
const LineBindingsPage = lazy(() => import('@/pages/LineBindingsPage').then((m) => ({ default: m.LineBindingsPage })))
const TenantsPage = lazy(() => import('@/pages/TenantsPage').then((m) => ({ default: m.TenantsPage })))
const ActiveCallsPage = lazy(() => import('@/pages/ActiveCallsPage').then((m) => ({ default: m.ActiveCallsPage })))
const CDRsPage = lazy(() => import('@/pages/CDRsPage').then((m) => ({ default: m.CDRsPage })))
const QueuesPage = lazy(() => import('@/pages/QueuesPage').then((m) => ({ default: m.QueuesPage })))
const VoicemailPage = lazy(() => import('@/pages/VoicemailPage').then((m) => ({ default: m.VoicemailPage })))
const PresencePage = lazy(() => import('@/pages/PresencePage').then((m) => ({ default: m.PresencePage })))
const TrunksPage = lazy(() => import('@/pages/TrunksPage').then((m) => ({ default: m.TrunksPage })))
const RoutingPage = lazy(() => import('@/pages/RoutingPage').then((m) => ({ default: m.RoutingPage })))
const FlowsPage = lazy(() => import('@/pages/FlowsPage').then((m) => ({ default: m.FlowsPage })))
const FlowBuilderPage = lazy(() => import('@/pages/FlowBuilderPage').then((m) => ({ default: m.FlowBuilderPage })))
const RegistrationsPage = lazy(() => import('@/pages/RegistrationsPage').then((m) => ({ default: m.RegistrationsPage })))
const EventsPage = lazy(() => import('@/pages/EventsPage').then((m) => ({ default: m.EventsPage })))
const ClusterPage = lazy(() => import('@/pages/ClusterPage').then((m) => ({ default: m.ClusterPage })))
const PortingPage = lazy(() => import('@/pages/PortingPage').then((m) => ({ default: m.PortingPage })))
const AuditLogPage = lazy(() => import('@/pages/AuditLogPage').then((m) => ({ default: m.AuditLogPage })))
const ApiTokensPage = lazy(() => import('@/pages/ApiTokensPage').then((m) => ({ default: m.ApiTokensPage })))
const SettingsPage = lazy(() => import('@/pages/SettingsPage').then((m) => ({ default: m.SettingsPage })))
const SystemSettingsPage = lazy(() => import('@/pages/SystemSettingsPage').then((m) => ({ default: m.SystemSettingsPage })))
const SipLadderPage = lazy(() => import('@/pages/SipLadderPage').then((m) => ({ default: m.SipLadderPage })))

// User portal pages
const UserDashboardPage = lazy(() => import('@/pages/user/UserDashboardPage').then((m) => ({ default: m.UserDashboardPage })))
const UserVoicemailPage = lazy(() => import('@/pages/user/UserVoicemailPage').then((m) => ({ default: m.UserVoicemailPage })))
const UserForwardingPage = lazy(() => import('@/pages/user/UserForwardingPage').then((m) => ({ default: m.UserForwardingPage })))
const UserSettingsPage = lazy(() => import('@/pages/user/UserSettingsPage').then((m) => ({ default: m.UserSettingsPage })))

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
})

function PageLoader() {
  return (
    <div className="flex h-32 items-center justify-center text-muted-foreground">
      <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
    </div>
  )
}

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuthStore()
  if (!isAuthenticated) return <Navigate to="/admin/login" replace />
  return <>{children}</>
}

function UserProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuthStore()
  if (!isAuthenticated) return <Navigate to="/admin/login" replace />
  return <>{children}</>
}

function DefaultRedirect() {
  const { isAuthenticated, user } = useAuthStore()
  if (!isAuthenticated) return <Navigate to="/admin/login" replace />
  if (user?.role === 'user') return <Navigate to="/user" replace />
  return <Navigate to="/admin" replace />
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Suspense fallback={<PageLoader />}>
          <Routes>
            <Route path="/admin/login" element={<LoginPage />} />
            <Route
              path="/admin"
              element={
                <ProtectedRoute>
                  <AdminLayout />
                </ProtectedRoute>
              }
            >
              <Route index element={<Suspense fallback={<PageLoader />}><DashboardPage /></Suspense>} />
              {/* Management */}
              <Route path="users" element={<Suspense fallback={<PageLoader />}><UsersPage /></Suspense>} />
              <Route path="extensions" element={<Suspense fallback={<PageLoader />}><ExtensionsPage /></Suspense>} />
              <Route path="devices" element={<Suspense fallback={<PageLoader />}><DevicesPage /></Suspense>} />
              <Route path="phone-numbers" element={<Suspense fallback={<PageLoader />}><PhoneNumbersPage /></Suspense>} />
              <Route path="line-bindings" element={<Suspense fallback={<PageLoader />}><LineBindingsPage /></Suspense>} />
              <Route path="tenants" element={<Suspense fallback={<PageLoader />}><TenantsPage /></Suspense>} />
              {/* Operations */}
              <Route path="registrations" element={<Suspense fallback={<PageLoader />}><RegistrationsPage /></Suspense>} />
              <Route path="calls" element={<Suspense fallback={<PageLoader />}><ActiveCallsPage /></Suspense>} />
              <Route path="cdrs" element={<Suspense fallback={<PageLoader />}><CDRsPage /></Suspense>} />
              <Route path="queues" element={<Suspense fallback={<PageLoader />}><QueuesPage /></Suspense>} />
              <Route path="voicemail" element={<Suspense fallback={<PageLoader />}><VoicemailPage /></Suspense>} />
              <Route path="presence" element={<Suspense fallback={<PageLoader />}><PresencePage /></Suspense>} />
              <Route path="events" element={<Suspense fallback={<PageLoader />}><EventsPage /></Suspense>} />
              {/* Advanced */}
              <Route path="trunks" element={<Suspense fallback={<PageLoader />}><TrunksPage /></Suspense>} />
              <Route path="routing" element={<Suspense fallback={<PageLoader />}><RoutingPage /></Suspense>} />
              <Route path="flows" element={<Suspense fallback={<PageLoader />}><FlowsPage /></Suspense>} />
              <Route path="flows/:id" element={<Suspense fallback={<PageLoader />}><FlowBuilderPage /></Suspense>} />
              <Route path="cluster" element={<Suspense fallback={<PageLoader />}><ClusterPage /></Suspense>} />
              <Route path="porting" element={<Suspense fallback={<PageLoader />}><PortingPage /></Suspense>} />
              <Route path="audit" element={<Suspense fallback={<PageLoader />}><AuditLogPage /></Suspense>} />
              <Route path="tokens" element={<Suspense fallback={<PageLoader />}><ApiTokensPage /></Suspense>} />
              <Route path="settings" element={<Suspense fallback={<PageLoader />}><SettingsPage /></Suspense>} />
              <Route path="system-settings" element={<Suspense fallback={<PageLoader />}><SystemSettingsPage /></Suspense>} />
              <Route path="calls/:callId/sip-trace" element={<Suspense fallback={<PageLoader />}><SipLadderPage /></Suspense>} />
            </Route>
            {/* User portal routes */}
            <Route
              path="/user"
              element={
                <UserProtectedRoute>
                  <UserLayout />
                </UserProtectedRoute>
              }
            >
              <Route index element={<Suspense fallback={<PageLoader />}><UserDashboardPage /></Suspense>} />
              <Route path="voicemail" element={<Suspense fallback={<PageLoader />}><UserVoicemailPage /></Suspense>} />
              <Route path="forwarding" element={<Suspense fallback={<PageLoader />}><UserForwardingPage /></Suspense>} />
              <Route path="settings" element={<Suspense fallback={<PageLoader />}><UserSettingsPage /></Suspense>} />
            </Route>
            <Route path="*" element={<DefaultRedirect />} />
          </Routes>
        </Suspense>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
