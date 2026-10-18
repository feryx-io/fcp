import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useForm } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import {
  Server, Plus, Pencil, Trash2, RefreshCw, ChevronRight,
  Power, PowerOff, Copy, Zap, History,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { lcrRoutes, trunks, sites } from '@/api/endpoints'
import { useEffectiveTenantId } from '@/components/TenantSelector'
import type { SipTrunk, CreateTrunkRequest, TrunkStatsResponse } from '@/types'

const healthBadge = (status: string) => {
  switch (status) {
    case 'healthy': return 'success' as const
    case 'degraded': return 'warning' as const
    case 'down': return 'destructive' as const
    case 'disabled': return 'secondary' as const
    default: return 'outline' as const
  }
}

function formatUptime(secs: number | null): string {
  if (secs == null) return '-'
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (h < 24) return `${h}h ${m}m`
  const d = Math.floor(h / 24)
  return `${d}d ${h % 24}h`
}

const directionLabel = (dir: string) => {
  switch (dir) {
    case 'upstream': return 'Outbound'
    case 'downstream': return 'Inbound'
    case 'bidirectional': return 'Both'
    default: return dir
  }
}

const directionBadge = (dir: string) => {
  switch (dir) {
    case 'upstream': return 'default' as const
    case 'downstream': return 'secondary' as const
    case 'bidirectional': return 'outline' as const
    default: return 'outline' as const
  }
}

const protocolLabel = (proto: string) => proto.toUpperCase()

interface TrunkFormData {
  name: string
  description: string
  enabled: boolean
  direction: string
  priority: number
  weight: number
  hosts: string
  inbound_sources: string
  transport_protocol: string
  transport_port: number
  verify_tls: boolean
  auth_username: string
  auth_password: string
  auth_realm: string
  register: boolean
  register_expiry: number
  max_concurrent_calls: number
  cps_limit: number
  codec_prefs: string
  caller_id_override: string
  from_domain: string
  strip_digits: number
  prepend_digits: string
  srtp_mode: string
  dtmf_mode: string
  session_timer_secs: number
  t38_enabled: boolean
  emergency_capable: boolean
  outbound_proxy: string
  trunk_group: string
  header_rules: string
  max_failures: number
  recovery_interval_secs: number
  health_check_enabled: boolean
  health_check_interval_secs: number
  /** The site this trunk runs at; empty for the cloud's own. */
  site_id: string
}

const defaultValues: TrunkFormData = {
  name: '',
  description: '',
  enabled: true,
  direction: 'bidirectional',
  priority: 10,
  weight: 100,
  hosts: '',
  inbound_sources: '',
  transport_protocol: 'udp',
  transport_port: 5060,
  verify_tls: true,
  auth_username: '',
  auth_password: '',
  auth_realm: '',
  register: false,
  register_expiry: 3600,
  max_concurrent_calls: 0,
  cps_limit: 0,
  codec_prefs: '',
  caller_id_override: '',
  from_domain: '',
  strip_digits: 0,
  prepend_digits: '',
  srtp_mode: 'disabled',
  dtmf_mode: 'rfc2833',
  session_timer_secs: 0,
  t38_enabled: false,
  emergency_capable: false,
  outbound_proxy: '',
  trunk_group: '',
  header_rules: '',
  max_failures: 5,
  recovery_interval_secs: 60,
  health_check_enabled: true,
  health_check_interval_secs: 30,
  site_id: '',
}

function trunkToForm(trunk: SipTrunk): TrunkFormData {
  return {
    name: trunk.name,
    description: trunk.description ?? '',
    enabled: trunk.enabled,
    direction: trunk.direction,
    priority: trunk.priority,
    weight: trunk.weight,
    hosts: trunk.hosts.join('\n'),
    inbound_sources: (trunk.inbound_sources ?? []).join('\n'),
    transport_protocol: trunk.transport_protocol,
    transport_port: trunk.transport_port,
    verify_tls: trunk.verify_tls,
    auth_username: trunk.auth_username ?? '',
    auth_password: '',
    auth_realm: trunk.auth_realm ?? '',
    register: trunk.register,
    register_expiry: trunk.register_expiry,
    max_concurrent_calls: trunk.max_concurrent_calls,
    cps_limit: trunk.cps_limit,
    codec_prefs: trunk.codec_prefs.join(', '),
    caller_id_override: trunk.caller_id_override ?? '',
    from_domain: trunk.from_domain ?? '',
    strip_digits: trunk.strip_digits,
    prepend_digits: trunk.prepend_digits ?? '',
    srtp_mode: trunk.srtp_mode,
    dtmf_mode: trunk.dtmf_mode,
    session_timer_secs: trunk.session_timer_secs,
    t38_enabled: trunk.t38_enabled,
    emergency_capable: trunk.emergency_capable,
    outbound_proxy: trunk.outbound_proxy ?? '',
    site_id: trunk.site_id ?? '',
    trunk_group: trunk.trunk_group ?? '',
    header_rules: trunk.header_rules ?? '',
    max_failures: trunk.max_failures,
    recovery_interval_secs: trunk.recovery_interval_secs,
    health_check_enabled: trunk.health_check_enabled,
    health_check_interval_secs: trunk.health_check_interval_secs,
  }
}

function formToRequest(form: TrunkFormData): CreateTrunkRequest {
  const hosts = form.hosts.split('\n').map((h) => h.trim()).filter(Boolean)
  const inboundSources = form.inbound_sources.split('\n').map((h) => h.trim()).filter(Boolean)
  const codecs = form.codec_prefs.split(',').map((c) => c.trim()).filter(Boolean)
  return {
    name: form.name,
    description: form.description || undefined,
    enabled: form.enabled,
    direction: form.direction,
    priority: form.priority,
    weight: form.weight,
    hosts,
    inbound_sources: inboundSources,
    transport_protocol: form.transport_protocol,
    transport_port: form.transport_port,
    verify_tls: form.verify_tls,
    auth_username: form.auth_username || undefined,
    auth_password: form.auth_password || undefined,
    auth_realm: form.auth_realm || undefined,
    register: form.register,
    register_expiry: form.register_expiry,
    max_concurrent_calls: form.max_concurrent_calls,
    cps_limit: form.cps_limit,
    codec_prefs: codecs.length > 0 ? codecs : undefined,
    caller_id_override: form.caller_id_override || undefined,
    from_domain: form.from_domain || undefined,
    strip_digits: form.strip_digits,
    prepend_digits: form.prepend_digits || undefined,
    srtp_mode: form.srtp_mode,
    dtmf_mode: form.dtmf_mode,
    session_timer_secs: form.session_timer_secs,
    t38_enabled: form.t38_enabled,
    emergency_capable: form.emergency_capable,
    outbound_proxy: form.outbound_proxy || undefined,
    // An empty site makes the trunk the cloud's own again.
    site_id: form.site_id,
    trunk_group: form.trunk_group || undefined,
    header_rules: form.header_rules || undefined,
    max_failures: form.max_failures,
    recovery_interval_secs: form.recovery_interval_secs,
    health_check_enabled: form.health_check_enabled,
    health_check_interval_secs: form.health_check_interval_secs,
  }
}

export function TrunksPage() {
  const queryClient = useQueryClient()
  const [createOpen, setCreateOpen] = useState(false)
  const [editTrunk, setEditTrunk] = useState<SipTrunk | null>(null)
  const [deleteTrunk, setDeleteTrunk] = useState<SipTrunk | null>(null)
  const [detailTrunk, setDetailTrunk] = useState<SipTrunk | null>(null)

  const createForm = useForm<TrunkFormData>({ defaultValues })
  const editForm = useForm<TrunkFormData>({ defaultValues })

  const { data: trunkStats } = useQuery({
    queryKey: ['trunk-stats', detailTrunk?.id],
    queryFn: () => trunks.stats(detailTrunk!.id),
    enabled: !!detailTrunk,
  })

  const { data: trunkList = [], isLoading, isFetching, refetch } = useQuery({
    queryKey: ['trunks'],
    queryFn: trunks.list,
    refetchInterval: 15000, // Auto-refresh every 15s for health status
  })

  const createMutation = useMutation({
    mutationFn: (data: CreateTrunkRequest) => trunks.create(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] })
      setCreateOpen(false)
      createForm.reset()
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateTrunkRequest> }) => trunks.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] })
      setEditTrunk(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => trunks.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] })
      setDeleteTrunk(null)
    },
  })

  const cloneMutation = useMutation({
    mutationFn: (id: string) => trunks.clone(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['trunks'] }),
  })

  const [testResult, setTestResult] = useState<{ name: string; success: boolean; message: string } | null>(null)
  const testMutation = useMutation({
    mutationFn: (id: string) => trunks.test(id),
    onSuccess: (data, id) => {
      const trunk = trunkList.find((t) => t.id === id)
      setTestResult({ name: trunk?.name ?? id, ...data })
    },
  })

  const navigate = useNavigate()

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      trunks.update(id, { enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['trunks'] }),
  })

  const onCreateSubmit = (data: TrunkFormData) => {
    createMutation.mutate(formToRequest(data))
  }

  const onEditSubmit = (data: TrunkFormData) => {
    if (!editTrunk) return
    updateMutation.mutate({ id: editTrunk.id, data: formToRequest(data) })
  }

  const openEdit = (trunk: SipTrunk) => {
    editForm.reset(trunkToForm(trunk))
    setEditTrunk(trunk)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">SIP Trunks</h1>
          <p className="text-muted-foreground">Manage SIP trunk connections to carriers and PBXes</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} />
            Refresh
          </Button>
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Trunk
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : trunkList.length === 0 ? (
        <div className="flex flex-col h-48 items-center justify-center text-muted-foreground gap-4">
          <Server className="h-12 w-12 opacity-30" />
          <p>No SIP trunks configured yet</p>
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Create Your First Trunk
          </Button>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {trunkList.map((trunk) => (
            <TrunkCard
              key={trunk.id}
              trunk={trunk}
              onSelect={() => setDetailTrunk(trunk)}
              onEdit={() => openEdit(trunk)}
              onDelete={() => setDeleteTrunk(trunk)}
              onToggle={() => toggleMutation.mutate({ id: trunk.id, enabled: !trunk.enabled })}
              onClone={() => cloneMutation.mutate(trunk.id)}
              onTest={() => testMutation.mutate(trunk.id)}
              onViewCdrs={() => navigate(`/admin/cdrs?trunk=${encodeURIComponent(trunk.name)}`)}
            />
          ))}
        </div>
      )}

      {/* Create Trunk Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Create SIP Trunk</DialogTitle>
            <DialogDescription>Configure a new SIP trunk connection.</DialogDescription>
          </DialogHeader>
          <TrunkForm form={createForm} onSubmit={onCreateSubmit} isSubmitting={createMutation.isPending} error={createMutation.error} />
        </DialogContent>
      </Dialog>

      {/* Edit Trunk Dialog */}
      <Dialog open={!!editTrunk} onOpenChange={() => setEditTrunk(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Trunk: {editTrunk?.name}</DialogTitle>
            <DialogDescription>Update trunk configuration.</DialogDescription>
          </DialogHeader>
          <TrunkForm form={editForm} onSubmit={onEditSubmit} isSubmitting={updateMutation.isPending} submitLabel="Save Changes" error={updateMutation.error} />
        </DialogContent>
      </Dialog>

      {/* Detail Dialog */}
      <Dialog open={!!detailTrunk} onOpenChange={() => setDetailTrunk(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Server className="h-5 w-5" />
              {detailTrunk?.name}
            </DialogTitle>
            <DialogDescription>Trunk configuration details</DialogDescription>
          </DialogHeader>
          {detailTrunk && <TrunkDetailView trunk={detailTrunk} stats={trunkStats} onEdit={() => { setDetailTrunk(null); openEdit(detailTrunk) }} />}
        </DialogContent>
      </Dialog>

      {/* Test Result */}
      <Dialog open={!!testResult} onOpenChange={() => setTestResult(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Connectivity Test: {testResult?.name}</DialogTitle>
          </DialogHeader>
          <div className="flex items-center gap-3 py-4">
            <Badge variant={testResult?.success ? 'success' : 'destructive'} className="text-base px-4 py-1">
              {testResult?.success ? 'PASS' : 'FAIL'}
            </Badge>
            <p className="text-sm text-muted-foreground">{testResult?.message}</p>
          </div>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={!!deleteTrunk} onOpenChange={() => setDeleteTrunk(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Trunk</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete the trunk <strong>{deleteTrunk?.name}</strong>? This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {deleteMutation.error && <p className="text-sm text-destructive">{deleteMutation.error.message}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTrunk(null)}>Cancel</Button>
            <Button
              variant="destructive"
              onClick={() => deleteTrunk && deleteMutation.mutate(deleteTrunk.id)}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function TrunkCard({ trunk, onSelect, onEdit, onDelete, onToggle, onClone, onTest, onViewCdrs }: {
  trunk: SipTrunk
  onSelect: () => void
  onEdit: () => void
  onDelete: () => void
  onToggle: () => void
  onClone: () => void
  onTest: () => void
  onViewCdrs: () => void
}) {
  return (
    <Card className={`hover:border-primary/30 transition-colors ${!trunk.enabled ? 'opacity-60' : ''}`}>
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center justify-between text-base">
          <button className="flex items-center gap-2 hover:underline" onClick={onSelect}>
            {trunk.name}
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </button>
          <div className="flex items-center gap-1">
            <Badge variant={healthBadge(trunk.health_status)}>
              {trunk.health_status}
            </Badge>
            <Badge variant={directionBadge(trunk.direction)}>
              {directionLabel(trunk.direction)}
            </Badge>
          </div>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {(trunk.description || trunk.trunk_group) && (
          <div className="flex items-center gap-2">
            {trunk.trunk_group && <Badge variant="outline" className="text-xs">{trunk.trunk_group}</Badge>}
            {trunk.description && <p className="text-xs text-muted-foreground truncate">{trunk.description}</p>}
          </div>
        )}
        <div className="grid grid-cols-4 gap-2 text-center text-sm">
          <div>
            <p className="text-lg font-bold">{trunk.hosts.length}</p>
            <p className="text-xs text-muted-foreground">Hosts</p>
          </div>
          <div>
            <p className="text-lg font-bold">{trunk.max_concurrent_calls || '\u221E'}</p>
            <p className="text-xs text-muted-foreground">Max Calls</p>
          </div>
          <div>
            <Badge variant="outline" className="text-xs">{protocolLabel(trunk.transport_protocol)}</Badge>
            <p className="text-xs text-muted-foreground mt-1">Transport</p>
          </div>
          <div>
            <p className="text-lg font-bold font-mono">{formatUptime(trunk.uptime_secs)}</p>
            <p className="text-xs text-muted-foreground">Uptime</p>
          </div>
        </div>

        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <span>P:{trunk.priority} W:{trunk.weight}</span>
            {trunk.register && <Badge variant="secondary" className="text-xs">REG</Badge>}
            {trunk.has_credentials && <Badge variant="secondary" className="text-xs">AUTH</Badge>}
          </div>
          <div className="flex items-center gap-1">
            <Button variant="ghost" size="icon" className="h-7 w-7" title="Test connectivity" onClick={(e) => { e.stopPropagation(); onTest() }}>
              <Zap className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" title="View CDRs" onClick={(e) => { e.stopPropagation(); onViewCdrs() }}>
              <History className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" title="Clone" onClick={(e) => { e.stopPropagation(); onClone() }}>
              <Copy className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" title={trunk.enabled ? 'Disable' : 'Enable'} onClick={(e) => { e.stopPropagation(); onToggle() }}>
              {trunk.enabled ? <PowerOff className="h-3.5 w-3.5" /> : <Power className="h-3.5 w-3.5" />}
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" title="Edit" onClick={(e) => { e.stopPropagation(); onEdit() }}>
              <Pencil className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7 text-destructive" title="Delete" onClick={(e) => { e.stopPropagation(); onDelete() }}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function Sparkline({ data, width = 120, height = 28 }: { data: number[]; width?: number; height?: number }) {
  if (!data || data.length === 0) return null
  const max = Math.max(...data, 1)
  const step = width / (data.length - 1 || 1)
  const points = data.map((v, i) => `${i * step},${height - (v / max) * height}`).join(' ')
  return (
    <svg width={width} height={height} className="inline-block">
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth={1.5} className="text-primary" />
      <polyline points={`0,${height} ${points} ${width},${height}`} fill="currentColor" className="text-primary/10" />
    </svg>
  )
}

function TrunkDetailView({ trunk, stats, onEdit }: { trunk: SipTrunk; stats?: TrunkStatsResponse; onEdit: () => void }) {
  // What this trunk is priced at (`docs/TENANT_TRUNKS.md` §12, phase 6c).
  const { data: allRoutes = [] } = useQuery({ queryKey: ['lcr-routes'], queryFn: lcrRoutes.list })
  const priced = allRoutes.filter((r) => r.trunk_id === trunk.id)
  const nodes = trunk.nodes ?? []
  return (
    <div className="space-y-4">
      {/* Stats bar */}
      {stats && stats.total_calls_24h > 0 && (
        <div className="grid grid-cols-4 gap-3 p-3 bg-muted rounded-lg text-center">
          <div>
            <p className="text-lg font-bold">{stats.total_calls_24h}</p>
            <p className="text-xs text-muted-foreground">Calls (24h)</p>
          </div>
          <div>
            <p className="text-lg font-bold">{stats.asr.toFixed(1)}%</p>
            <p className="text-xs text-muted-foreground">ASR</p>
          </div>
          <div>
            <p className="text-lg font-bold">{stats.acd_secs.toFixed(0)}s</p>
            <p className="text-xs text-muted-foreground">ACD</p>
          </div>
          <div>
            <p className="text-lg font-bold">{stats.pdd_ms.toFixed(0)}ms</p>
            <p className="text-xs text-muted-foreground">PDD</p>
          </div>
        </div>
      )}
      {stats && stats.hourly_volume.some((v) => v > 0) && (
        <div className="p-3 bg-muted rounded-lg">
          <p className="text-xs text-muted-foreground mb-1">Traffic (24h)</p>
          <Sparkline data={stats.hourly_volume} width={380} height={32} />
        </div>
      )}

      <div className="grid grid-cols-2 gap-4 text-sm">
        <Field label="Status" badge={trunk.enabled ? 'success' : 'secondary'} value={trunk.enabled ? 'Enabled' : 'Disabled'} />
        <Field label="Health" badge={healthBadge(trunk.health_status)} value={trunk.health_status} />
        <Field label="Uptime" value={formatUptime(trunk.uptime_secs)} />
        <Field label="Direction" value={directionLabel(trunk.direction)} />
        <Field label="Transport" value={`${protocolLabel(trunk.transport_protocol)}:${trunk.transport_port}`} />
        <Field label="Max Calls" value={trunk.max_concurrent_calls ? String(trunk.max_concurrent_calls) : 'Unlimited'} />
        <Field label="Priority / Weight" value={`${trunk.priority} / ${trunk.weight}`} />
        <Field label="Auth" value={trunk.has_credentials ? `${trunk.auth_username}` : 'None'} />
        <Field label="Registration" value={trunk.register ? `Enabled (${trunk.register_expiry}s)` : 'Disabled'} />
        <Field label="Health Check" value={trunk.health_check_enabled ? `Every ${trunk.health_check_interval_secs}s` : 'Disabled'} />
        <Field label="Failover" value={`${trunk.max_failures} failures / ${trunk.recovery_interval_secs}s recovery`} />
        {trunk.caller_id_override && <Field label="Caller ID" value={trunk.caller_id_override} />}
        {trunk.from_domain && <Field label="From Domain" value={trunk.from_domain} />}
        {(trunk.strip_digits > 0 || trunk.prepend_digits) && (
          <Field label="Number Rules" value={`Strip ${trunk.strip_digits}${trunk.prepend_digits ? `, prepend ${trunk.prepend_digits}` : ''}`} />
        )}
        {trunk.codec_prefs.length > 0 && <Field label="Codecs" value={trunk.codec_prefs.join(', ')} />}
        {trunk.cps_limit > 0 && <Field label="CPS Limit" value={String(trunk.cps_limit)} />}
        <Field label="SRTP" value={trunk.srtp_mode} />
        <Field label="DTMF" value={trunk.dtmf_mode.replace('_', ' ')} />
        {trunk.session_timer_secs > 0 && <Field label="Session Timer" value={`${trunk.session_timer_secs}s`} />}
        {trunk.t38_enabled && <Field label="T.38 Fax" value="Enabled" />}
        {trunk.emergency_capable && <Field label="Emergency calls" value="Capable" />}
        {trunk.outbound_proxy && <Field label="Outbound Proxy" value={trunk.outbound_proxy} />}
        {trunk.trunk_group && <Field label="Group" value={trunk.trunk_group} />}
        {trunk.verify_tls && trunk.transport_protocol === 'tls' && <Field label="TLS Verify" value="Enabled" />}
        {nodes.length > 0 && <Field label="Calls now" value={`${trunk.calls_in_flight ?? 0} across ${nodes.length} node${nodes.length === 1 ? '' : 's'}`} />}
      </div>

      {nodes.length > 0 && (
        <div>
          <p className="text-xs text-muted-foreground mb-1">
            What each node sees — each probes the carrier itself, so they can disagree
          </p>
          <div className="space-y-1">
            {nodes.map((view) => (
              <div key={view.node_id ?? view.trunk_id} className="flex flex-wrap items-center gap-2 text-sm bg-muted rounded px-2 py-1">
                <span className="font-mono">{view.node_id ?? 'node'}</span>
                <Badge variant={healthBadge(view.health)}>{view.health}</Badge>
                {!view.routable && <Badge variant="destructive">not routing to it</Badge>}
                <span className="text-xs text-muted-foreground">{view.calls_in_flight} call{view.calls_in_flight === 1 ? '' : 's'}</span>
                {view.registration && (
                  <span className="text-xs text-muted-foreground">
                    registration: {view.registration}{view.account ? ` as ${view.account}` : ''}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
      {nodes.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No node reports its view of this trunk: health above is this server's own probe.
        </p>
      )}

      {priced.length > 0 && (
        <div>
          <p className="text-xs text-muted-foreground mb-1">Least-cost routes on this trunk</p>
          <div className="space-y-1">
            {priced.map((r) => (
              <p key={r.id} className="font-mono text-sm bg-muted rounded px-2 py-1">
                {r.prefix} at {r.cost_per_minute}/min{r.enabled ? '' : ' (disabled)'}
              </p>
            ))}
          </div>
        </div>
      )}

      {trunk.hosts.length > 0 && (
        <div>
          <p className="text-xs text-muted-foreground mb-1">Hosts</p>
          <div className="space-y-1">
            {trunk.hosts.map((host) => (
              <p key={host} className="font-mono text-sm bg-muted rounded px-2 py-1">{host}</p>
            ))}
          </div>
        </div>
      )}

      {trunk.direction !== 'upstream' && (
        <div>
          <p className="text-xs text-muted-foreground mb-1">Takes calls from</p>
          <div className="space-y-1">
            {(trunk.inbound_sources?.length ? trunk.inbound_sources : ['the hosts above']).map((source) => (
              <p key={source} className="font-mono text-sm bg-muted rounded px-2 py-1">{source}</p>
            ))}
          </div>
        </div>
      )}

      <div className="flex justify-end">
        <Button onClick={onEdit}>
          <Pencil className="mr-2 h-4 w-4" /> Edit Trunk
        </Button>
      </div>
    </div>
  )
}

function TrunkForm({ form, onSubmit, isSubmitting, submitLabel = 'Create Trunk', error }: {
  form: ReturnType<typeof useForm<TrunkFormData>>
  onSubmit: (data: TrunkFormData) => void
  isSubmitting: boolean
  submitLabel?: string
  /** Why the API refused the last save. */
  error?: Error | null
}) {
  const { register, handleSubmit, watch, setValue, formState: { errors } } = form
  const tenantForSites = useEffectiveTenantId()
  // The tenant's sites: a trunk may run at one, as a carrier of the
  // site's own (docs/HYBRID_CONTROL_PLANE.md §3.6).
  const { data: siteList = [] } = useQuery({
    queryKey: ['sites', tenantForSites],
    queryFn: () => sites.list(tenantForSites || undefined),
  })
  const transportProtocol = watch('transport_protocol')
  const registerEnabled = watch('register')
  const healthCheckEnabled = watch('health_check_enabled')

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
      {/* Basic */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Basic</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="name">Trunk Name</Label>
            <Input id="name" placeholder="e.g. twilio-us-east" {...register('name', { required: true })} />
            {errors.name && <p className="mt-1 text-xs text-destructive">A trunk needs a name</p>}
          </div>
          <div className="flex items-end">
            <div className="flex items-center gap-2">
              <Switch id="enabled" checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
              <Label htmlFor="enabled">Enabled</Label>
            </div>
          </div>
          <div className="col-span-2">
            <Label htmlFor="description">Description</Label>
            <Input id="description" placeholder="e.g. Twilio US East - account #12345" {...register('description')} />
          </div>
          <div>
            <Label>Direction</Label>
            <Select value={watch('direction')} onValueChange={(v) => setValue('direction', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="upstream">Upstream (Outbound)</SelectItem>
                <SelectItem value="downstream">Downstream (Inbound)</SelectItem>
                <SelectItem value="bidirectional">Bidirectional</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {siteList.length > 0 && (
            <div>
              <Label>Runs at</Label>
              <Select value={watch('site_id') || '__none__'} onValueChange={(v) => setValue('site_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">The cloud (here)</SelectItem>
                  {siteList.map((site) => (
                    <SelectItem key={site.id} value={site.id}>Site {site.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
        </div>
      </div>

      {/* Hosts */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Hosts</h3>
        <div>
          <Label htmlFor="hosts">Remote Hosts (one per line, host:port)</Label>
          <textarea
            id="hosts"
            rows={3}
            className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            placeholder={'sip.carrier.com:5060\n192.168.1.100:5060'}
            {...register('hosts')}
          />
        </div>
        <div>
          <Label htmlFor="inbound_sources">Inbound Sources (one per line, IP or CIDR)</Label>
          <textarea
            id="inbound_sources"
            rows={3}
            className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            placeholder={'203.0.113.0/24\n198.51.100.7'}
            {...register('inbound_sources')}
          />
          <p className="text-xs text-muted-foreground mt-1">
            Where the carrier's calls come from. Leave empty to take calls from the hosts. A call arriving on
            this trunk is this tenant's call.
          </p>
        </div>
      </div>

      {/* Transport */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Transport</h3>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label>Protocol</Label>
            <Select value={transportProtocol} onValueChange={(v) => {
              setValue('transport_protocol', v)
              if (v === 'tls') setValue('transport_port', 5061)
              else setValue('transport_port', 5060)
            }}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="udp">UDP</SelectItem>
                <SelectItem value="tcp">TCP</SelectItem>
                <SelectItem value="tls">TLS</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="transport_port">Port</Label>
            <Input id="transport_port" type="number" {...register('transport_port', { valueAsNumber: true })} />
          </div>
          {transportProtocol === 'tls' && (
            <div className="flex items-end">
              <div className="flex items-center gap-2">
                <Switch id="verify_tls" checked={watch('verify_tls')} onCheckedChange={(v) => setValue('verify_tls', v)} />
                <Label htmlFor="verify_tls">Verify TLS</Label>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Authentication */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Authentication</h3>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label htmlFor="auth_username">Username</Label>
            <Input id="auth_username" placeholder="Optional" {...register('auth_username')} />
          </div>
          <div>
            <Label htmlFor="auth_password">Password</Label>
            <Input id="auth_password" type="password" placeholder="Optional" {...register('auth_password')} />
          </div>
          <div>
            <Label htmlFor="auth_realm">Realm</Label>
            <Input id="auth_realm" placeholder="Optional" {...register('auth_realm')} />
          </div>
        </div>
      </div>

      {/* Registration */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Registration</h3>
        <div className="grid grid-cols-2 gap-4">
          <div className="flex items-center gap-2">
            <Switch id="register" checked={registerEnabled} onCheckedChange={(v) => setValue('register', v)} />
            <Label htmlFor="register">Send REGISTER to trunk</Label>
          </div>
          {registerEnabled && (
            <div>
              <Label htmlFor="register_expiry">Expiry (seconds)</Label>
              <Input id="register_expiry" type="number" {...register('register_expiry', { valueAsNumber: true })} />
            </div>
          )}
        </div>
      </div>

      {/* Caller ID & Domain */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Caller ID & Domain</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="caller_id_override">Caller ID Override</Label>
            <Input id="caller_id_override" placeholder="e.g. +15551234567" {...register('caller_id_override')} />
          </div>
          <div>
            <Label htmlFor="from_domain">From Domain</Label>
            <Input id="from_domain" placeholder="e.g. sip.carrier.com" {...register('from_domain')} />
          </div>
        </div>
      </div>

      {/* Number Manipulation */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Number Manipulation</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="strip_digits">Strip Leading Digits</Label>
            <Input id="strip_digits" type="number" min={0} {...register('strip_digits', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="prepend_digits">Prepend Digits</Label>
            <Input id="prepend_digits" placeholder="e.g. +1" {...register('prepend_digits')} />
          </div>
        </div>
      </div>

      {/* Codecs & Rate Limiting */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Codecs & Limits</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="codec_prefs">Codec Preferences (comma-separated)</Label>
            <Input id="codec_prefs" placeholder="e.g. PCMU, PCMA, G722" {...register('codec_prefs')} />
          </div>
          <div>
            <Label htmlFor="cps_limit">CPS Limit (0 = no limit)</Label>
            <Input id="cps_limit" type="number" {...register('cps_limit', { valueAsNumber: true })} />
          </div>
        </div>
      </div>

      {/* Media */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Media</h3>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label>SRTP Mode</Label>
            <Select value={watch('srtp_mode')} onValueChange={(v) => setValue('srtp_mode', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="disabled">Disabled</SelectItem>
                <SelectItem value="optional">Optional</SelectItem>
                <SelectItem value="mandatory">Mandatory</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label>DTMF Mode</Label>
            <Select value={watch('dtmf_mode')} onValueChange={(v) => setValue('dtmf_mode', v)}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="rfc2833">RFC 2833</SelectItem>
                <SelectItem value="sip_info">SIP INFO</SelectItem>
                <SelectItem value="inband">Inband</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-end">
            <div className="flex items-center gap-2">
              <Switch id="t38_enabled" checked={watch('t38_enabled')} onCheckedChange={(v) => setValue('t38_enabled', v)} />
              <Label htmlFor="t38_enabled">T.38 Fax</Label>
            </div>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="session_timer_secs">Session Timer (0 = disabled)</Label>
            <Input id="session_timer_secs" type="number" {...register('session_timer_secs', { valueAsNumber: true })} />
          </div>
          <div className="flex items-end">
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Switch
                  id="emergency_capable"
                  checked={watch('emergency_capable')}
                  onCheckedChange={(v) => setValue('emergency_capable', v)}
                />
                <Label htmlFor="emergency_capable">Emergency calls</Label>
              </div>
              <p className="text-xs text-muted-foreground">
                Your emergency-capable trunks carry your 911 calls first; the platform's follow if
                none of yours can take one. No routing rule applies to an emergency call.
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* Advanced */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Advanced</h3>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <Label htmlFor="outbound_proxy">Outbound Proxy</Label>
            <Input id="outbound_proxy" placeholder="e.g. sip:proxy.carrier.com:5060" {...register('outbound_proxy')} />
          </div>
          <div>
            <Label htmlFor="trunk_group">Trunk Group</Label>
            <Input id="trunk_group" placeholder="e.g. US Carriers" {...register('trunk_group')} />
          </div>
          <div className="col-span-2">
            <Label htmlFor="header_rules">Header Rules (JSON)</Label>
            <textarea
              id="header_rules"
              rows={2}
              className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              placeholder='[{"action":"add","name":"X-Custom","value":"foo"}]'
              {...register('header_rules')}
            />
          </div>
        </div>
      </div>

      {/* Load Balancing */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Load Balancing</h3>
        <div className="grid grid-cols-3 gap-4">
          <div>
            <Label htmlFor="priority">Priority (lower = higher)</Label>
            <Input id="priority" type="number" {...register('priority', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="weight">Weight</Label>
            <Input id="weight" type="number" {...register('weight', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="max_concurrent_calls">Max Calls (0 = no limit)</Label>
            <Input id="max_concurrent_calls" type="number" {...register('max_concurrent_calls', { valueAsNumber: true })} />
          </div>
        </div>
      </div>

      {/* Health & Failover */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Health & Failover</h3>
        <div className="grid grid-cols-2 gap-4">
          <div className="flex items-center gap-2">
            <Switch id="health_check_enabled" checked={healthCheckEnabled} onCheckedChange={(v) => setValue('health_check_enabled', v)} />
            <Label htmlFor="health_check_enabled">OPTIONS Health Checks</Label>
          </div>
          {healthCheckEnabled && (
            <div>
              <Label htmlFor="health_check_interval_secs">Check Interval (seconds)</Label>
              <Input id="health_check_interval_secs" type="number" {...register('health_check_interval_secs', { valueAsNumber: true })} />
            </div>
          )}
          <div>
            <Label htmlFor="max_failures">Max Failures Before Down</Label>
            <Input id="max_failures" type="number" {...register('max_failures', { valueAsNumber: true })} />
          </div>
          <div>
            <Label htmlFor="recovery_interval_secs">Recovery Interval (seconds)</Label>
            <Input id="recovery_interval_secs" type="number" {...register('recovery_interval_secs', { valueAsNumber: true })} />
          </div>
        </div>
      </div>

      {error && <p className="text-sm text-destructive">{error.message}</p>}
      <DialogFooter>
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}

function Field({ label, value, badge }: {
  label: string; value: string; badge?: 'success' | 'warning' | 'destructive' | 'outline' | 'default' | 'secondary'
}) {
  return (
    <div>
      <p className="text-muted-foreground text-xs">{label}</p>
      {badge ? <Badge variant={badge}>{value}</Badge> : <p className="font-medium">{value}</p>}
    </div>
  )
}
