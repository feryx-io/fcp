import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { AdminLayout } from '@/layouts/AdminLayout'
import { LoginPage } from '@/pages/LoginPage'
import { DashboardPage } from '@/pages/DashboardPage'
import { UsersPage } from '@/pages/UsersPage'
import { ExtensionsPage } from '@/pages/ExtensionsPage'
import { DevicesPage } from '@/pages/DevicesPage'
import { PhoneNumbersPage } from '@/pages/PhoneNumbersPage'
import { LineBindingsPage } from '@/pages/LineBindingsPage'
import { TenantsPage } from '@/pages/TenantsPage'
import { ActiveCallsPage } from '@/pages/ActiveCallsPage'
import { CDRsPage } from '@/pages/CDRsPage'
import { QueuesPage } from '@/pages/QueuesPage'
import { VoicemailPage } from '@/pages/VoicemailPage'
import { PresencePage } from '@/pages/PresencePage'
import { useAuthStore } from '@/stores/auth'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
})

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuthStore()
  if (!isAuthenticated) return <Navigate to="/admin/login" replace />
  return <>{children}</>
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
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
            <Route index element={<DashboardPage />} />
            <Route path="users" element={<UsersPage />} />
            <Route path="extensions" element={<ExtensionsPage />} />
            <Route path="devices" element={<DevicesPage />} />
            <Route path="phone-numbers" element={<PhoneNumbersPage />} />
            <Route path="line-bindings" element={<LineBindingsPage />} />
            <Route path="tenants" element={<TenantsPage />} />
            <Route path="calls" element={<ActiveCallsPage />} />
            <Route path="cdrs" element={<CDRsPage />} />
            <Route path="queues" element={<QueuesPage />} />
            <Route path="voicemail" element={<VoicemailPage />} />
            <Route path="presence" element={<PresencePage />} />
          </Route>
          <Route path="*" element={<Navigate to="/admin" replace />} />
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  )
}
