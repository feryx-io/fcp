import { lazy, Suspense } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { AdminLayout } from '@/layouts/AdminLayout'
import { UserLayout } from '@/layouts/UserLayout'
import { useAuthStore } from '@/stores/auth'

// Lazy-loaded pages
const LoginPage = lazy(() => import('@/pages/LoginPage').then((m) => ({ default: m.LoginPage })))
const ChangePasswordPage = lazy(() => import('@/pages/ChangePasswordPage').then((m) => ({ default: m.ChangePasswordPage })))
const DashboardPage = lazy(() => import('@/pages/DashboardPage').then((m) => ({ default: m.DashboardPage })))
const UsersPage = lazy(() => import('@/pages/UsersPage').then((m) => ({ default: m.UsersPage })))
const ExtensionsPage = lazy(() => import('@/pages/ExtensionsPage').then((m) => ({ default: m.ExtensionsPage })))
const DevicesPage = lazy(() => import('@/pages/DevicesPage').then((m) => ({ default: m.DevicesPage })))
const PhoneNumbersPage = lazy(() => import('@/pages/PhoneNumbersPage').then((m) => ({ default: m.PhoneNumbersPage })))
const LineBindingsPage = lazy(() => import('@/pages/LineBindingsPage').then((m) => ({ default: m.LineBindingsPage })))
const TenantsPage = lazy(() => import('@/pages/TenantsPage').then((m) => ({ default: m.TenantsPage })))
const LocationsPage = lazy(() => import('@/pages/LocationsPage').then((m) => ({ default: m.LocationsPage })))
const ActiveCallsPage = lazy(() => import('@/pages/ActiveCallsPage').then((m) => ({ default: m.ActiveCallsPage })))
const CDRsPage = lazy(() => import('@/pages/CDRsPage').then((m) => ({ default: m.CDRsPage })))
const QueuesPage = lazy(() => import('@/pages/QueuesPage').then((m) => ({ default: m.QueuesPage })))
const VoicemailPage = lazy(() => import('@/pages/VoicemailPage').then((m) => ({ default: m.VoicemailPage })))
const PresencePage = lazy(() => import('@/pages/PresencePage').then((m) => ({ default: m.PresencePage })))
const ConferencesPage = lazy(() => import('@/pages/ConferencesPage').then((m) => ({ default: m.ConferencesPage })))
const TrunksPage = lazy(() => import('@/pages/TrunksPage').then((m) => ({ default: m.TrunksPage })))
const RoutingPage = lazy(() => import('@/pages/RoutingPage').then((m) => ({ default: m.RoutingPage })))
const HotelingPage = lazy(() => import('@/pages/HotelingPage').then((m) => ({ default: m.HotelingPage })))
const ClassesOfServicePage = lazy(() => import('@/pages/ClassesOfServicePage').then((m) => ({ default: m.ClassesOfServicePage })))
const GroupsPage = lazy(() => import('@/pages/GroupsPage').then((m) => ({ default: m.GroupsPage })))
const SchedulesPage = lazy(() => import('@/pages/SchedulesPage').then((m) => ({ default: m.SchedulesPage })))
const LcrRoutesPage = lazy(() => import('@/pages/LcrRoutesPage').then((m) => ({ default: m.LcrRoutesPage })))
const FlowsPage = lazy(() => import('@/pages/FlowsPage').then((m) => ({ default: m.FlowsPage })))
const FlowBuilderPage = lazy(() => import('@/pages/FlowBuilderPage').then((m) => ({ default: m.FlowBuilderPage })))
const FlowTracesPage = lazy(() => import('@/pages/FlowTracesPage').then((m) => ({ default: m.FlowTracesPage })))
const RegistrationsPage = lazy(() => import('@/pages/RegistrationsPage').then((m) => ({ default: m.RegistrationsPage })))
const EventsPage = lazy(() => import('@/pages/EventsPage').then((m) => ({ default: m.EventsPage })))
const SitesPage = lazy(() => import('@/pages/SitesPage').then((m) => ({ default: m.SitesPage })))
const ClusterPage = lazy(() => import('@/pages/ClusterPage').then((m) => ({ default: m.ClusterPage })))
const AuditLogPage = lazy(() => import('@/pages/AuditLogPage').then((m) => ({ default: m.AuditLogPage })))
const EmergencyPage = lazy(() => import('@/pages/EmergencyPage').then((m) => ({ default: m.EmergencyPage })))
const RecordingsPage = lazy(() => import('@/pages/RecordingsPage').then((m) => ({ default: m.RecordingsPage })))
const WebhooksPage = lazy(() => import('@/pages/WebhooksPage').then((m) => ({ default: m.WebhooksPage })))
const IdentityProvidersPage = lazy(() => import('@/pages/IdentityProvidersPage').then((m) => ({ default: m.IdentityProvidersPage })))
const ApiTokensPage = lazy(() => import('@/pages/ApiTokensPage').then((m) => ({ default: m.ApiTokensPage })))
const SettingsPage = lazy(() => import('@/pages/SettingsPage').then((m) => ({ default: m.SettingsPage })))
const SystemSettingsPage = lazy(() => import('@/pages/SystemSettingsPage').then((m) => ({ default: m.SystemSettingsPage })))
const PromptsPage = lazy(() => import('@/pages/PromptsPage').then((m) => ({ default: m.PromptsPage })))
const SipLadderPage = lazy(() => import('@/pages/SipLadderPage').then((m) => ({ default: m.SipLadderPage })))

// User portal pages
const UserDashboardPage = lazy(() => import('@/pages/user/UserDashboardPage').then((m) => ({ default: m.UserDashboardPage })))
const UserVoicemailPage = lazy(() => import('@/pages/user/UserVoicemailPage').then((m) => ({ default: m.UserVoicemailPage })))
const UserCallingPage = lazy(() => import('@/pages/user/UserCallingPage').then((m) => ({ default: m.UserCallingPage })))
const UserApiTokensPage = lazy(() => import('@/pages/user/UserApiTokensPage').then((m) => ({ default: m.UserApiTokensPage })))
const UserSettingsPage = lazy(() => import('@/pages/user/UserSettingsPage').then((m) => ({ default: m.UserSettingsPage })))
const UserConferencesPage = lazy(() => import('@/pages/user/UserConferencesPage').then((m) => ({ default: m.UserConferencesPage })))

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
  const { isAuthenticated, user, restoring } = useAuthStore()
  if (restoring) return <PageLoader />
  if (!isAuthenticated) return <Navigate to="/login" replace />
  // End users cannot access admin routes
  if (user?.role === 'end_user') return <Navigate to="/user" replace />
  return <>{children}</>
}

function UserProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, restoring } = useAuthStore()
  if (restoring) return <PageLoader />
  if (!isAuthenticated) return <Navigate to="/login" replace />
  return <>{children}</>
}

function DefaultRedirect() {
  const { isAuthenticated, user, restoring } = useAuthStore()
  if (restoring) return <PageLoader />
  if (!isAuthenticated) return <Navigate to="/login" replace />
  if (user?.role === 'end_user') return <Navigate to="/user" replace />
  return <Navigate to="/admin" replace />
}

// A reload keeps you signed in: the session's cookie is asked once, at
// start (docs/IDENTITY_AND_DEVICES.md phase 1).
void useAuthStore.getState().restore()

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Suspense fallback={<PageLoader />}>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/change-password" element={<ChangePasswordPage />} />
            {/* Backward compat: redirect old login path */}
            <Route path="/admin/login" element={<Navigate to="/login" replace />} />
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
              <Route path="locations" element={<Suspense fallback={<PageLoader />}><LocationsPage /></Suspense>} />
              <Route path="emergency" element={<Suspense fallback={<PageLoader />}><EmergencyPage /></Suspense>} />
              {/* Operations */}
              <Route path="registrations" element={<Suspense fallback={<PageLoader />}><RegistrationsPage /></Suspense>} />
              <Route path="calls" element={<Suspense fallback={<PageLoader />}><ActiveCallsPage /></Suspense>} />
              <Route path="cdrs" element={<Suspense fallback={<PageLoader />}><CDRsPage /></Suspense>} />
              <Route path="queues" element={<Suspense fallback={<PageLoader />}><QueuesPage /></Suspense>} />
              <Route path="voicemail" element={<Suspense fallback={<PageLoader />}><VoicemailPage /></Suspense>} />
              <Route path="presence" element={<Suspense fallback={<PageLoader />}><PresencePage /></Suspense>} />
              <Route path="conferences" element={<Suspense fallback={<PageLoader />}><ConferencesPage /></Suspense>} />
              <Route path="prompts" element={<Suspense fallback={<PageLoader />}><PromptsPage /></Suspense>} />
              <Route path="events" element={<Suspense fallback={<PageLoader />}><EventsPage /></Suspense>} />
              {/* Advanced */}
              <Route path="trunks" element={<Suspense fallback={<PageLoader />}><TrunksPage /></Suspense>} />
              <Route path="routing" element={<Suspense fallback={<PageLoader />}><RoutingPage /></Suspense>} />
              <Route path="schedules" element={<Suspense fallback={<PageLoader />}><SchedulesPage /></Suspense>} />
              <Route path="groups" element={<Suspense fallback={<PageLoader />}><GroupsPage /></Suspense>} />
              <Route path="classes-of-service" element={<Suspense fallback={<PageLoader />}><ClassesOfServicePage /></Suspense>} />
              <Route path="hoteling" element={<Suspense fallback={<PageLoader />}><HotelingPage /></Suspense>} />
              <Route path="lcr" element={<Suspense fallback={<PageLoader />}><LcrRoutesPage /></Suspense>} />
              <Route path="flows" element={<Suspense fallback={<PageLoader />}><FlowsPage /></Suspense>} />
              <Route path="flows/:id" element={<Suspense fallback={<PageLoader />}><FlowBuilderPage /></Suspense>} />
              <Route path="flow-traces" element={<Suspense fallback={<PageLoader />}><FlowTracesPage /></Suspense>} />
              <Route path="cluster" element={<Suspense fallback={<PageLoader />}><ClusterPage /></Suspense>} />
              <Route path="sites" element={<Suspense fallback={<PageLoader />}><SitesPage /></Suspense>} />
              <Route path="audit" element={<Suspense fallback={<PageLoader />}><AuditLogPage /></Suspense>} />
              <Route path="recordings" element={<Suspense fallback={<PageLoader />}><RecordingsPage /></Suspense>} />
              <Route path="tokens" element={<Suspense fallback={<PageLoader />}><ApiTokensPage /></Suspense>} />
              <Route path="webhooks" element={<Suspense fallback={<PageLoader />}><WebhooksPage /></Suspense>} />
              <Route path="sso" element={<Suspense fallback={<PageLoader />}><IdentityProvidersPage /></Suspense>} />
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
              <Route path="calling" element={<Suspense fallback={<PageLoader />}><UserCallingPage /></Suspense>} />
              <Route path="api-tokens" element={<Suspense fallback={<PageLoader />}><UserApiTokensPage /></Suspense>} />
              <Route path="conferences" element={<Suspense fallback={<PageLoader />}><UserConferencesPage /></Suspense>} />
              <Route path="settings" element={<Suspense fallback={<PageLoader />}><UserSettingsPage /></Suspense>} />
            </Route>
            <Route path="*" element={<DefaultRedirect />} />
          </Routes>
        </Suspense>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
