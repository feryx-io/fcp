import { useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import { tenants } from '@/api/endpoints'
import { useAuthStore } from '@/stores/auth'
import { useTenantStore } from '@/stores/tenant'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { Building2, Lock } from 'lucide-react'
import type { Tenant } from '@/types'

/** Returns true if the current user is a platform-level admin (super_admin).
 *
 * Platform admins see the top-bar tenant picker and can switch between tenants.
 * Tenant admins (tenant_admin) are NOT platform admins — they are locked to
 * their own tenant and should not see the cross-tenant picker.
 */
export function useIsPlatformAdmin(): boolean {
  const { user } = useAuthStore()
  return user?.role === 'super_admin'
}

/** Returns the effective tenant ID for new resource creation:
 * - If the platform admin has pinned a tenant in the top bar, use that.
 * - Otherwise fall back to the user's own tenant from their JWT session.
 *
 * An empty string means "no default" (only possible for platform admins
 * viewing "All Tenants" — the user must then pick one explicitly in the form).
 */
export function useEffectiveTenantId(): string {
  const { user } = useAuthStore()
  const { selectedTenantId } = useTenantStore()
  const isPlatformAdmin = user?.role === 'super_admin'

  if (isPlatformAdmin && selectedTenantId) {
    return selectedTenantId
  }
  return user?.tenant_id ?? ''
}

/** Returns true when the TenantField should be locked (disabled) in forms.
 *
 * The field is locked whenever there is a single, unambiguous tenant the
 * resource should belong to: either the platform admin pinned a tenant in the
 * top bar, or the user is not a platform admin (tenant_admin / end_user) and
 * can only operate inside their own tenant.
 */
export function useTenantFieldLocked(): boolean {
  const { user } = useAuthStore()
  const { selectedTenantId } = useTenantStore()
  const isPlatformAdmin = user?.role === 'super_admin'
  if (!isPlatformAdmin) return true
  return selectedTenantId !== null
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
 *
 * Behavior:
 * - Hidden entirely for non-platform-admins (their tenant is already fixed
 *   by the JWT and the backend will enforce it).
 * - When a tenant is pinned in the top-bar picker, the field auto-fills
 *   with that tenant and is locked (disabled) so the form cannot accidentally
 *   create resources under a different tenant. An onChange is fired so the
 *   enclosing form state tracks the pinned tenant.
 * - When the picker is on "All Tenants" the field is fully editable.
 */
export function TenantField({ value, onChange }: {
  value: string
  onChange: (tenantId: string) => void
}) {
  const isPlatformAdmin = useIsPlatformAdmin()
  const { selectedTenantId } = useTenantStore()
  const locked = useTenantFieldLocked() && isPlatformAdmin && !!selectedTenantId

  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })

  // Whenever a tenant is pinned in the top bar, mirror that pin into the
  // form's tenant_id. This is safe even for edit dialogs because list
  // endpoints are already tenant-scoped when the pin is set — an edit
  // dialog can only open for a resource that already belongs to the pinned
  // tenant, so the sync is a no-op in that case. The effect also handles
  // the "stale mount" case: a long-lived create dialog that was mounted
  // under one pin will re-sync to the new pin when the user changes it.
  useEffect(() => {
    if (locked && selectedTenantId && value !== selectedTenantId) {
      onChange(selectedTenantId)
    }
  }, [locked, selectedTenantId, value, onChange])

  if (!isPlatformAdmin) return null

  const lockedTenantName =
    tenantList.find((t: Tenant) => t.id === selectedTenantId)?.name ?? selectedTenantId

  return (
    <div>
      <label className="text-sm font-medium flex items-center gap-1.5">
        Tenant
        {locked && <Lock className="h-3 w-3 text-muted-foreground" />}
      </label>
      <Select
        value={value || '__inherit__'}
        onValueChange={(v) => onChange(v === '__inherit__' ? '' : v)}
        disabled={locked}
      >
        <SelectTrigger>
          <SelectValue placeholder="Use my tenant" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__inherit__">Use my tenant</SelectItem>
          {tenantList.map((t: Tenant) => (
            <SelectItem key={t.id} value={t.id}>{t.name}</SelectItem>
          ))}
        </SelectContent>
      </Select>
      {locked && (
        <p className="text-xs text-muted-foreground mt-1">
          Locked to <span className="font-medium">{lockedTenantName}</span> from the top-bar selection.
          Switch the tenant picker to change.
        </p>
      )}
    </div>
  )
}
