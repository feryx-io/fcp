import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, KeyRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { devices, provisioning } from '@/api/endpoints'

/** The tenant's provisioning URL (docs/PROVISIONING.md): shown, copied, and
 * replaced — which stops the old one working. */
export function ProvisioningUrlCard() {
  const queryClient = useQueryClient()
  const [copied, setCopied] = useState(false)
  const { data: key, isLoading } = useQuery({ queryKey: ['provisioning-key'], queryFn: provisioning.getKey })
  const rotate = useMutation({
    mutationFn: provisioning.rotateKey,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['provisioning-key'] }),
  })
  const shown = key ? (key.url ?? key.path) : null
  return (
    <Card>
      <CardContent className="flex flex-wrap items-center justify-between gap-4 p-4">
        <div className="min-w-0">
          <p className="text-sm font-medium">Provisioning URL</p>
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : shown ? (
            <p className="break-all font-mono text-sm">{shown}</p>
          ) : (
            <p className="text-sm text-muted-foreground">
              None yet. Phones fetch their files under it, as their vendor names them (e.g.{' '}
              <span className="font-mono">&lt;mac&gt;.cfg</span>); HTTPS only.
            </p>
          )}
        </div>
        <div className="flex gap-2">
          {shown && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard?.writeText(shown)
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              }}
            >
              <Copy className="mr-2 h-4 w-4" /> {copied ? 'Copied' : 'Copy'}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={rotate.isPending}
            onClick={() => {
              if (!key || confirm('Make a new provisioning URL? Phones using the old one stop getting their files until they are given the new one.')) {
                rotate.mutate()
              }
            }}
          >
            <KeyRound className="mr-2 h-4 w-4" /> {key ? 'New URL' : 'Make URL'}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

const OUTCOMES: Record<string, string> = {
  served: 'Served',
  unauthorized: 'Wrong or missing device credentials',
  network_refused: 'From outside the tenant’s networks',
  not_served: 'Disabled, or no template',
}

/** A device's provisioning fetches, newest first. */
export function DeviceFetchesDialog({ device, onClose }: { device: { id: string; name: string } | null; onClose: () => void }) {
  const { data: fetches = [], isLoading } = useQuery({
    queryKey: ['device-fetches', device?.id],
    queryFn: () => devices.fetches(device!.id),
    enabled: !!device,
  })
  return (
    <Dialog open={!!device} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Provisioning fetches: {device?.name}</DialogTitle>
          <DialogDescription>What the phone asked for, from where, and what it was told.</DialogDescription>
        </DialogHeader>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>When</TableHead>
              <TableHead>File</TableHead>
              <TableHead>From</TableHead>
              <TableHead>Outcome</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && <TableRow><TableCell colSpan={4}>Loading…</TableCell></TableRow>}
            {!isLoading && fetches.length === 0 && (
              <TableRow><TableCell colSpan={4} className="text-muted-foreground">No fetches yet.</TableCell></TableRow>
            )}
            {fetches.map((f, i) => (
              <TableRow key={i}>
                <TableCell className="whitespace-nowrap text-sm">{new Date(f.at).toLocaleString()}</TableCell>
                <TableCell className="font-mono text-xs">{f.file}</TableCell>
                <TableCell className="text-xs">
                  {f.address ?? '-'}
                  {f.user_agent && <div className="text-muted-foreground">{f.user_agent}</div>}
                </TableCell>
                <TableCell className="text-sm">{OUTCOMES[f.outcome] ?? f.outcome}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </DialogContent>
    </Dialog>
  )
}
