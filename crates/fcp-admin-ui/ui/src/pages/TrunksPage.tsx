import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useForm } from 'react-hook-form'
import {
  Server, Plus, Pencil, Trash2, RefreshCw, ChevronRight,
  Power, PowerOff,
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
import { trunks } from '@/api/endpoints'
import type { SipTrunk, CreateTrunkRequest } from '@/types'

const healthBadge = (status: string) => {
  switch (status) {
    case 'healthy': return 'success' as const
    case 'degraded': return 'warning' as const
    case 'down': return 'destructive' as const
    default: return 'outline' as const
  }
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
  enabled: boolean
  direction: string
  priority: number
  weight: number
  hosts: string
  transport_protocol: string
  transport_port: number
  verify_tls: boolean
  auth_username: string
  auth_password: string
  auth_realm: string
  register: boolean
  register_expiry: number
  max_concurrent_calls: number
  max_failures: number
  recovery_interval_secs: number
  health_check_enabled: boolean
  health_check_interval_secs: number
}

const defaultValues: TrunkFormData = {
  name: '',
  enabled: true,
  direction: 'upstream',
  priority: 10,
  weight: 100,
  hosts: '',
  transport_protocol: 'udp',
  transport_port: 5060,
  verify_tls: true,
  auth_username: '',
  auth_password: '',
  auth_realm: '',
  register: false,
  register_expiry: 3600,
  max_concurrent_calls: 0,
  max_failures: 5,
  recovery_interval_secs: 60,
  health_check_enabled: true,
  health_check_interval_secs: 30,
}

function trunkToForm(trunk: SipTrunk): TrunkFormData {
  return {
    name: trunk.name,
    enabled: trunk.enabled,
    direction: trunk.direction,
    priority: trunk.priority,
    weight: trunk.weight,
    hosts: trunk.hosts.join('\n'),
    transport_protocol: trunk.transport_protocol,
    transport_port: trunk.transport_port,
    verify_tls: trunk.verify_tls,
    auth_username: trunk.auth_username ?? '',
    auth_password: '',
    auth_realm: trunk.auth_realm ?? '',
    register: trunk.register,
    register_expiry: trunk.register_expiry,
    max_concurrent_calls: trunk.max_concurrent_calls,
    max_failures: trunk.max_failures,
    recovery_interval_secs: trunk.recovery_interval_secs,
    health_check_enabled: trunk.health_check_enabled,
    health_check_interval_secs: trunk.health_check_interval_secs,
  }
}

function formToRequest(form: TrunkFormData): CreateTrunkRequest {
  const hosts = form.hosts.split('\n').map((h) => h.trim()).filter(Boolean)
  return {
    name: form.name,
    enabled: form.enabled,
    direction: form.direction,
    priority: form.priority,
    weight: form.weight,
    hosts,
    transport_protocol: form.transport_protocol,
    transport_port: form.transport_port,
    verify_tls: form.verify_tls,
    auth_username: form.auth_username || undefined,
    auth_password: form.auth_password || undefined,
    auth_realm: form.auth_realm || undefined,
    register: form.register,
    register_expiry: form.register_expiry,
    max_concurrent_calls: form.max_concurrent_calls,
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
          <TrunkForm form={createForm} onSubmit={onCreateSubmit} isSubmitting={createMutation.isPending} />
        </DialogContent>
      </Dialog>

      {/* Edit Trunk Dialog */}
      <Dialog open={!!editTrunk} onOpenChange={() => setEditTrunk(null)}>
        <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Trunk: {editTrunk?.name}</DialogTitle>
            <DialogDescription>Update trunk configuration.</DialogDescription>
          </DialogHeader>
          <TrunkForm form={editForm} onSubmit={onEditSubmit} isSubmitting={updateMutation.isPending} submitLabel="Save Changes" />
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
          {detailTrunk && <TrunkDetailView trunk={detailTrunk} onEdit={() => { setDetailTrunk(null); openEdit(detailTrunk) }} />}
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

function TrunkCard({ trunk, onSelect, onEdit, onDelete, onToggle }: {
  trunk: SipTrunk
  onSelect: () => void
  onEdit: () => void
  onDelete: () => void
  onToggle: () => void
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
        <div className="grid grid-cols-3 gap-2 text-center text-sm">
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
        </div>

        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <span>P:{trunk.priority} W:{trunk.weight}</span>
            {trunk.register && <Badge variant="secondary" className="text-xs">REG</Badge>}
            {trunk.has_credentials && <Badge variant="secondary" className="text-xs">AUTH</Badge>}
          </div>
          <div className="flex items-center gap-1">
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={(e) => { e.stopPropagation(); onToggle() }}>
              {trunk.enabled ? <PowerOff className="h-3.5 w-3.5" /> : <Power className="h-3.5 w-3.5" />}
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={(e) => { e.stopPropagation(); onEdit() }}>
              <Pencil className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7 text-destructive" onClick={(e) => { e.stopPropagation(); onDelete() }}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function TrunkDetailView({ trunk, onEdit }: { trunk: SipTrunk; onEdit: () => void }) {
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-4 text-sm">
        <Field label="Status" badge={trunk.enabled ? 'success' : 'secondary'} value={trunk.enabled ? 'Enabled' : 'Disabled'} />
        <Field label="Health" badge={healthBadge(trunk.health_status)} value={trunk.health_status} />
        <Field label="Direction" value={directionLabel(trunk.direction)} />
        <Field label="Transport" value={`${protocolLabel(trunk.transport_protocol)}:${trunk.transport_port}`} />
        <Field label="Max Calls" value={trunk.max_concurrent_calls ? String(trunk.max_concurrent_calls) : 'Unlimited'} />
        <Field label="Priority / Weight" value={`${trunk.priority} / ${trunk.weight}`} />
        <Field label="Auth" value={trunk.has_credentials ? `${trunk.auth_username}` : 'None'} />
        <Field label="Registration" value={trunk.register ? `Enabled (${trunk.register_expiry}s)` : 'Disabled'} />
        <Field label="Health Check" value={trunk.health_check_enabled ? `Every ${trunk.health_check_interval_secs}s` : 'Disabled'} />
        <Field label="Failover" value={`${trunk.max_failures} failures / ${trunk.recovery_interval_secs}s recovery`} />
        {trunk.verify_tls && trunk.transport_protocol === 'tls' && <Field label="TLS Verify" value="Enabled" />}
      </div>

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

      <div className="flex justify-end">
        <Button onClick={onEdit}>
          <Pencil className="mr-2 h-4 w-4" /> Edit Trunk
        </Button>
      </div>
    </div>
  )
}

function TrunkForm({ form, onSubmit, isSubmitting, submitLabel = 'Create Trunk' }: {
  form: ReturnType<typeof useForm<TrunkFormData>>
  onSubmit: (data: TrunkFormData) => void
  isSubmitting: boolean
  submitLabel?: string
}) {
  const { register, handleSubmit, watch, setValue } = form
  const transportProtocol = watch('transport_protocol')
  const registerEnabled = watch('register')
  const healthCheckEnabled = watch('health_check_enabled')

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
      {/* Basic */}
      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">Basic</h3>
        <div className="grid grid-cols-2 gap-4">
          <div className="col-span-2">
            <Label htmlFor="name">Trunk Name</Label>
            <Input id="name" placeholder="e.g. twilio-us-east" {...register('name', { required: true })} />
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
          <div className="flex items-end gap-4">
            <div className="flex items-center gap-2">
              <Switch id="enabled" checked={watch('enabled')} onCheckedChange={(v) => setValue('enabled', v)} />
              <Label htmlFor="enabled">Enabled</Label>
            </div>
          </div>
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
            <Label htmlFor="max_concurrent_calls">Max Concurrent (0 = unlimited)</Label>
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
