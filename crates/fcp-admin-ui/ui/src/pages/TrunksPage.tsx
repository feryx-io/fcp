import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  Server, ChevronRight, Power, PowerOff, RefreshCw,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { trunks } from '@/api/endpoints'
import type { TrunkSummary } from '@/types'

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

export function TrunksPage() {
  const [selectedTrunk, setSelectedTrunk] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: trunkList = [], isLoading } = useQuery({
    queryKey: ['trunks'],
    queryFn: trunks.list,
    refetchInterval: 10000,
  })

  const { data: trunkDetail } = useQuery({
    queryKey: ['trunk-detail', selectedTrunk],
    queryFn: () => trunks.get(selectedTrunk!),
    enabled: !!selectedTrunk,
  })

  const enableMutation = useMutation({
    mutationFn: trunks.enable,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['trunks'] }),
  })

  const disableMutation = useMutation({
    mutationFn: trunks.disable,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['trunks'] }),
  })

  const registerMutation = useMutation({
    mutationFn: trunks.register,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['trunks'] }),
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">SIP Trunks</h1>
          <p className="text-muted-foreground">Manage SIP trunk connections and monitor health</p>
        </div>
        <Button
          variant="outline"
          onClick={() => queryClient.invalidateQueries({ queryKey: ['trunks'] })}
        >
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : trunkList.length === 0 ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">
          No trunks configured
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {trunkList.map((trunk) => (
            <TrunkCard
              key={trunk.name}
              trunk={trunk}
              onSelect={() => setSelectedTrunk(trunk.name)}
              onEnable={() => enableMutation.mutate(trunk.name)}
              onDisable={() => disableMutation.mutate(trunk.name)}
            />
          ))}
        </div>
      )}

      {/* Trunk Detail Dialog */}
      <Dialog open={!!selectedTrunk} onOpenChange={() => setSelectedTrunk(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Server className="h-5 w-5" />
              Trunk: {selectedTrunk}
            </DialogTitle>
            <DialogDescription>Detailed trunk configuration and status.</DialogDescription>
          </DialogHeader>
          {trunkDetail && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <Field label="Health" badge={healthBadge(trunkDetail.health_status)} value={trunkDetail.health_status} />
                <Field label="Direction" value={directionLabel(trunkDetail.direction)} />
                <Field label="Enabled" value={trunkDetail.enabled && !trunkDetail.admin_disabled ? 'Yes' : 'No'} />
                <Field label="Transport" value={`${trunkDetail.transport_protocol}${trunkDetail.transport_port ? `:${trunkDetail.transport_port}` : ''}`} />
                <Field label="Concurrent Calls" value={`${trunkDetail.concurrent_calls} / ${trunkDetail.max_concurrent_calls}`} />
                <Field label="Priority / Weight" value={`${trunkDetail.priority} / ${trunkDetail.weight}`} />
                <Field label="Consecutive Failures" value={String(trunkDetail.consecutive_failures)} />
                <Field label="Credentials" value={trunkDetail.has_credentials ? 'Configured' : 'None'} />
              </div>

              {trunkDetail.hosts.length > 0 && (
                <div>
                  <p className="text-xs text-muted-foreground mb-1">Hosts</p>
                  <div className="space-y-1">
                    {trunkDetail.hosts.map((host) => (
                      <p key={host} className="font-mono text-sm bg-muted rounded px-2 py-1">{host}</p>
                    ))}
                  </div>
                </div>
              )}

              {trunkDetail.register && (
                <div>
                  <p className="text-xs text-muted-foreground mb-1">Registration</p>
                  <div className="flex items-center gap-2">
                    <Badge variant={trunkDetail.registration_state === 'registered' ? 'success' : 'warning'}>
                      {trunkDetail.registration_state ?? 'unknown'}
                    </Badge>
                    {trunkDetail.registration_error && (
                      <span className="text-xs text-destructive">{trunkDetail.registration_error}</span>
                    )}
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => registerMutation.mutate(selectedTrunk!)}
                    >
                      Re-register
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}

function TrunkCard({ trunk, onSelect, onEnable, onDisable }: {
  trunk: TrunkSummary
  onSelect: () => void
  onEnable: () => void
  onDisable: () => void
}) {
  const isUp = trunk.enabled && !trunk.admin_disabled

  return (
    <Card className="hover:border-primary/30 transition-colors">
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center justify-between text-base">
          <button className="flex items-center gap-2 hover:underline" onClick={onSelect}>
            {trunk.name}
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </button>
          <Badge variant={healthBadge(trunk.health_status)}>
            {trunk.health_status}
          </Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-3 gap-2 text-center text-sm">
          <div>
            <p className="text-lg font-bold">{trunk.concurrent_calls}</p>
            <p className="text-xs text-muted-foreground">Active</p>
          </div>
          <div>
            <p className="text-lg font-bold">{trunk.max_concurrent_calls}</p>
            <p className="text-xs text-muted-foreground">Max</p>
          </div>
          <div>
            <Badge variant="outline" className="text-xs">{directionLabel(trunk.direction)}</Badge>
            <p className="text-xs text-muted-foreground mt-1">Direction</p>
          </div>
        </div>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            {trunk.registration_state && (
              <Badge variant="secondary" className="text-xs">
                {trunk.registration_state}
              </Badge>
            )}
            <span>P:{trunk.priority} W:{trunk.weight}</span>
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => { e.stopPropagation(); isUp ? onDisable() : onEnable() }}
          >
            {isUp ? <PowerOff className="h-4 w-4 mr-1" /> : <Power className="h-4 w-4 mr-1" />}
            {isUp ? 'Disable' : 'Enable'}
          </Button>
        </div>
      </CardContent>
    </Card>
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
