import { useQuery } from '@tanstack/react-query'
import { tenants } from '@/api/endpoints'
import { useAuthStore } from '@/stores/auth'
import { useTenantStore } from '@/stores/tenant'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { Building2 } from 'lucide-react'
import type { Tenant } from '@/types'

/** Returns true if the current user is a platform-level admin (super_admin). */
export function useIsPlatformAdmin(): boolean {
  const { user } = useAuthStore()
  return user?.role === 'super_admin'
}

/** Returns the effective tenant ID: selected override for platform admins, or the user's own tenant. */
export function useEffectiveTenantId(): string {
  const { user } = useAuthStore()
  const { selectedTenantId } = useTenantStore()
  const isPlatformAdmin = user?.role === 'super_admin'

  if (isPlatformAdmin && selectedTenantId) {
    return selectedTenantId
  }
  return user?.tenant_id ?? ''
}

/**
 * Global tenant context picker for the admin header.
 * Only visible to super_admin users. Others see their own tenant.
 */
export function TenantContextPicker() {
  const { user } = useAuthStore()
  const { selectedTenantId, setSelectedTenantId } = useTenantStore()
  const isPlatformAdmin = user?.role === 'super_admin'

  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })

  if (!isPlatformAdmin) return null

  return (
    <div className="flex items-center gap-2">
      <Building2 className="h-4 w-4 text-muted-foreground" />
      <Select
        value={selectedTenantId ?? '__all__'}
        onValueChange={(v) => setSelectedTenantId(v === '__all__' ? null : v)}
      >
        <SelectTrigger className="w-48 h-8 text-xs">
          <SelectValue placeholder="All Tenants" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__all__">All Tenants</SelectItem>
          {tenantList.map((t: Tenant) => (
            <SelectItem key={t.id} value={t.id}>
              {t.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

/**
 * Inline tenant selector for create/edit forms.
 * Only visible to super_admin users.
 */
export function TenantField({ value, onChange }: {
  value: string
  onChange: (tenantId: string) => void
}) {
  const isPlatformAdmin = useIsPlatformAdmin()

  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })

  if (!isPlatformAdmin) return null

  return (
    <div>
      <label className="text-sm font-medium">Tenant</label>
      <Select value={value || '__default__'} onValueChange={(v) => onChange(v === '__default__' ? '' : v)}>
        <SelectTrigger><SelectValue placeholder="Default tenant" /></SelectTrigger>
        <SelectContent>
          <SelectItem value="__default__">Default (from auth)</SelectItem>
          {tenantList.map((t: Tenant) => (
            <SelectItem key={t.id} value={t.id}>{t.name}</SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}
