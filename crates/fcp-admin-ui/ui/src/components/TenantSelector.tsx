import { useEffect, useMemo } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
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

/** Returns true when list tables should render an extra "Tenant" column.
 *
 * The column only carries information for platform admins who are viewing
 * "All Tenants" in the top-bar picker — in every other context, every row
 * in a given table already belongs to the same tenant (either the pinned
 * one, or the tenant-scoped user's own tenant) so the column would just be
 * visual noise.
 *
 * - super_admin on "All Tenants"  → true
 * - super_admin with a tenant pinned → false (all rows are in the pin)
 * - tenant_admin / telecom_admin / etc → false (backend clamps to one tenant)
 */
export function useShowTenantColumn(): boolean {
  const isPlatformAdmin = useIsPlatformAdmin()
  const { selectedTenantId } = useTenantStore()
  return isPlatformAdmin && selectedTenantId === null
}

/** Returns true when a create-form TenantField should be locked (disabled).
 *
 * The create field is locked whenever there is a single, unambiguous tenant
 * the new resource must belong to: either the platform admin pinned a tenant
 * in the top bar, or the user is not a platform admin and can only operate
 * inside their own tenant.
 *
 * This hook is only consulted for create forms. Edit forms always display
 * the resource's actual tenant and ignore the pin.
 */
export function useTenantFieldLocked(): boolean {
  const { user } = useAuthStore()
  const { selectedTenantId } = useTenantStore()
  const isPlatformAdmin = user?.role === 'super_admin'
  if (!isPlatformAdmin) return true
  return selectedTenantId !== null
}

/**
 * Client-side filter hook: clamps a resource list to the tenant pinned in
 * the top-bar picker.
 *
 * Use this in management list pages so that selecting a tenant in the
 * top-bar picker hides everything that does not belong to that tenant:
 *
 *   const { data: devices = [] } = useQuery(...)
 *   const visibleDevices = useTenantListFilter(devices)
 *
 * Semantics:
 * - Non-platform admins: the backend already clamps to their tenant, so the
 *   hook is a pass-through.
 * - Platform admins with a pinned tenant: only items whose `tenant_id`
 *   equals the pin are returned.
 * - Platform admins viewing "All Tenants": pass-through (they see every
 *   tenant's resources).
 *
 * Items without a `tenant_id` field are always passed through (they are
 * typically platform-scoped resources).
 */
export function useTenantListFilter<T extends { tenant_id?: string | null }>(
  data: T[] | undefined,
): T[] {
  const isPlatformAdmin = useIsPlatformAdmin()
  const { selectedTenantId } = useTenantStore()
  return useMemo(() => {
    if (!data) return []
    if (!isPlatformAdmin || !selectedTenantId) return data
    return data.filter(
      (item) => item.tenant_id == null || item.tenant_id === selectedTenantId,
    )
  }, [data, isPlatformAdmin, selectedTenantId])
}

/**
 * Global tenant context picker for the admin header.
 * Only visible to super_admin users. Others see their own tenant.
 */
export function TenantContextPicker() {
  const { user } = useAuthStore()
  const { selectedTenantId, setSelectedTenantId } = useTenantStore()
  const queryClient = useQueryClient()
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
        onValueChange={(v) => {
          setSelectedTenantId(v === '__all__' ? null : v)
          // Everything on screen was the last tenant's: read it again.
          void queryClient.resetQueries()
        }}
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
 * Behavior depends on `mode`:
 *
 * - `mode="create"` (default): used in New-resource dialogs.
 *   - Hidden for non-platform-admins (their tenant is fixed by JWT).
 *   - When the top-bar picker has a tenant pinned, the field auto-fills with
 *     that tenant via an onChange effect and is disabled with a lock icon.
 *     A hint line tells the user the field is locked and how to unlock it.
 *   - When the picker is on "All Tenants", the field is a free-choice
 *     dropdown of every tenant plus "Use my tenant" (the JWT fallback).
 *
 * - `mode="edit"`: used in Edit dialogs.
 *   - Always shows the resource's actual tenant (the `value` prop), never
 *     the top-bar pin. The top-bar picker drives list filtering and new
 *     resource defaults, but an existing resource can never be reassigned
 *     to a different tenant from this form.
 *   - The dropdown is always disabled. Tenant changes (tenant moves) must
 *     be done via an explicit admin workflow, not an incidental edit.
 *   - Hidden for non-platform-admins.
 */
export function TenantField({
  value,
  onChange,
  mode = 'create',
}: {
  value: string
  onChange: (tenantId: string) => void
  mode?: 'create' | 'edit'
}) {
  const isPlatformAdmin = useIsPlatformAdmin()
  const { selectedTenantId } = useTenantStore()

  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })

  // Only create forms are affected by the top-bar pin. Edit forms always
  // show the resource's own tenant.
  const createLocked = mode === 'create' && isPlatformAdmin && !!selectedTenantId

  // Auto-mirror the pinned tenant into the form's tenant_id for create
  // dialogs. This is skipped entirely for edit dialogs so the resource's
  // existing tenant is never silently overwritten.
  useEffect(() => {
    if (createLocked && selectedTenantId && value !== selectedTenantId) {
      onChange(selectedTenantId)
    }
  }, [createLocked, selectedTenantId, value, onChange])

  if (!isPlatformAdmin) return null

  const currentTenantName =
    tenantList.find((t: Tenant) => t.id === value)?.name ?? value
  const lockedTenantName =
    tenantList.find((t: Tenant) => t.id === selectedTenantId)?.name ??
    selectedTenantId

  // Edit mode: render a disabled select tied to the resource's tenant.
  if (mode === 'edit') {
    return (
      <div>
        <label className="text-sm font-medium flex items-center gap-1.5">
          Tenant
          <Lock className="h-3 w-3 text-muted-foreground" />
        </label>
        <Select value={value || '__none__'} disabled>
          <SelectTrigger>
            <SelectValue placeholder="Unassigned" />
          </SelectTrigger>
          <SelectContent>
            {!value && <SelectItem value="__none__">Unassigned</SelectItem>}
            {tenantList.map((t: Tenant) => (
              <SelectItem key={t.id} value={t.id}>
                {t.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground mt-1">
          {value ? (
            <>
              This resource belongs to{' '}
              <span className="font-medium">{currentTenantName}</span>.
              Tenant cannot be changed after creation.
            </>
          ) : (
            <>This resource has no tenant assigned.</>
          )}
        </p>
      </div>
    )
  }

  // Create mode.
  return (
    <div>
      <label className="text-sm font-medium flex items-center gap-1.5">
        Tenant
        {createLocked && <Lock className="h-3 w-3 text-muted-foreground" />}
      </label>
      <Select
        value={value || '__inherit__'}
        onValueChange={(v) => onChange(v === '__inherit__' ? '' : v)}
        disabled={createLocked}
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
      {createLocked && (
        <p className="text-xs text-muted-foreground mt-1">
          Locked to <span className="font-medium">{lockedTenantName}</span> from
          the top-bar selection. Switch the tenant picker to change.
        </p>
      )}
    </div>
  )
}

/**
 * A tenant-owned page's frame (docs/TENANT_SCOPE.md §3.1): for a platform
 * administrator, the tenant the page shows ("Tenant: Acme"); with "All
 * tenants" picked, a page without an all-tenant view asks for one instead
 * of showing the administrator's own tenant's. Anyone else sees the page as
 * it is: their tenant is the only one.
 */
export function TenantScope({ what, children }: { what: string; children: React.ReactNode }) {
  const isPlatformAdmin = useIsPlatformAdmin()
  const { selectedTenantId } = useTenantStore()
  const { data: tenantList = [] } = useQuery({
    queryKey: ['tenants'],
    queryFn: tenants.list,
    enabled: isPlatformAdmin,
  })
  if (!isPlatformAdmin) return <>{children}</>
  if (!selectedTenantId) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-24 text-center" data-testid="pick-tenant">
        <Building2 className="h-8 w-8 text-muted-foreground" />
        <p className="font-medium">Pick a tenant</p>
        <p className="max-w-md text-sm text-muted-foreground">
          {what} belong to a tenant. Choose one in the top bar to see and manage its {what.toLowerCase()}.
        </p>
      </div>
    )
  }
  const name = tenantList.find((t: Tenant) => t.id === selectedTenantId)?.name ?? selectedTenantId
  return (
    <div className="space-y-2">
      <div
        className="inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs text-muted-foreground"
        data-testid="tenant-badge"
      >
        <Building2 className="h-3 w-3" />
        Tenant: <span className="font-medium text-foreground">{name}</span>
      </div>
      {children}
    </div>
  )
}
