import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { ArrowUpDown, Download, Play, Trash2, Video, Volume2 } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { recordings } from '@/api/endpoints'
import type { ConferenceRecording, RecordingQueryParams } from '@/types'

/** Unix seconds → a local date and time. */
const when = (secs: number | null) =>
  secs ? new Date(secs * 1000).toLocaleString() : '-'

const duration = (secs: number | null) => {
  if (secs === null) return '-'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const s = secs % 60
  return h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${m}:${String(s).padStart(2, '0')}`
}

const size = (bytes: number | null) => {
  if (bytes === null) return '-'
  const units = ['B', 'KB', 'MB', 'GB']
  let n = bytes
  let i = 0
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024
    i += 1
  }
  return `${n.toFixed(i === 0 ? 0 : 1)} ${units[i]}`
}

/** A date input's value as unix seconds, or undefined when it is empty. */
const dayStart = (value: string) =>
  value ? Math.floor(new Date(`${value}T00:00:00`).getTime() / 1000) : undefined
const dayEnd = (value: string) =>
  value ? Math.floor(new Date(`${value}T23:59:59`).getTime() / 1000) : undefined

export function RecordingsPage() {
  const [filters, setFilters] = useState<RecordingQueryParams>({ limit: 100, offset: 0 })
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [playing, setPlaying] = useState<ConferenceRecording | null>(null)
  const [mediaUrl, setMediaUrl] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<ConferenceRecording | null>(null)
  const [error, setError] = useState<string | null>(null)
  const objectUrl = useRef<string | null>(null)
  const queryClient = useQueryClient()

  const { data, isLoading } = useQuery({
    queryKey: ['recordings', filters],
    queryFn: () => recordings.list(filters),
    // A running recording grows; keep the list roughly current.
    refetchInterval: 30_000,
  })

  const rows = data?.recordings ?? []
  const total = data?.total ?? 0

  // The player holds an object URL; release it when it changes or goes.
  useEffect(() => {
    return () => {
      if (objectUrl.current) URL.revokeObjectURL(objectUrl.current)
    }
  }, [])

  const openPlayer = async (recording: ConferenceRecording) => {
    setPlaying(recording)
    setMediaUrl(null)
    setError(null)
    try {
      const url = await recordings.fetchMedia(recording.id)
      if (objectUrl.current) URL.revokeObjectURL(objectUrl.current)
      objectUrl.current = url
      setMediaUrl(url)
    } catch {
      setError('Could not load the recording; the node holding the file may be unreachable.')
      setPlaying(null)
    }
  }

  const download = async (recording: ConferenceRecording) => {
    setError(null)
    try {
      const url = await recordings.fetchMedia(recording.id)
      const a = document.createElement('a')
      a.href = url
      a.download = recording.file_name
      a.click()
      // Revoking straight away can cancel the save in some browsers.
      setTimeout(() => URL.revokeObjectURL(url), 30_000)
    } catch {
      setError('Download failed; the node holding the file may be unreachable.')
    }
  }

  const remove = useMutation({
    mutationFn: (id: string) => recordings.remove(id),
    onSuccess: (result) => {
      queryClient.invalidateQueries({ queryKey: ['recordings'] })
      setConfirmDelete(null)
      setError(
        result.file_deleted
          ? null
          : 'The record is gone, but its node could not be reached: the file stays until that node sweeps it.',
      )
    },
    onError: () => setError('Could not delete the recording.'),
  })

  const columns: ColumnDef<ConferenceRecording>[] = [
    {
      accessorKey: 'started_at',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Started <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => when(row.original.started_at),
    },
    {
      accessorKey: 'room_number',
      header: 'Room',
      cell: ({ row }) => (
        <div>
          <span className="font-mono text-sm">{row.original.room_number}</span>
          <p className="text-xs text-muted-foreground">{row.original.participants} in the room</p>
        </div>
      ),
    },
    {
      accessorKey: 'format',
      header: 'Format',
      cell: ({ row }) => (
        <Badge variant={row.original.format !== 'wav' ? 'default' : 'secondary'}>
          {row.original.format !== 'wav'
            ? <Video className="mr-1 h-3 w-3" />
            : <Volume2 className="mr-1 h-3 w-3" />}
          {row.original.format}
        </Badge>
      ),
    },
    {
      accessorKey: 'duration_secs',
      header: 'Length',
      cell: ({ row }) => (
        row.original.running
          ? <Badge variant="success">recording</Badge>
          : <span className="text-sm">{duration(row.original.duration_secs)}</span>
      ),
    },
    {
      accessorKey: 'size_bytes',
      header: 'Size',
      cell: ({ row }) => <span className="text-sm">{size(row.original.size_bytes)}</span>,
    },
    {
      accessorKey: 'trigger',
      header: 'Started by',
      cell: ({ row }) => (
        <div>
          <span className="text-sm">{row.original.trigger}</span>
          {row.original.consent !== 'not_required' && (
            <p className="text-xs text-muted-foreground">consent {row.original.consent}</p>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'node_id',
      header: 'Node',
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.node_id}</span>
      ),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <div className="flex justify-end gap-1">
          <Button
            variant="ghost"
            size="sm"
            disabled={row.original.running}
            onClick={() => openPlayer(row.original)}
            title={row.original.running ? 'Still recording' : 'Play'}
          >
            <Play className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={row.original.running}
            onClick={() => download(row.original)}
            title="Download"
          >
            <Download className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={row.original.running}
            onClick={() => setConfirmDelete(row.original)}
            title={row.original.running ? 'Stop it first' : 'Delete'}
          >
            <Trash2 className="h-4 w-4 text-destructive" />
          </Button>
        </div>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Conference Recordings</h1>
        <p className="text-muted-foreground">
          What each room said and showed
          {total > 0 && <span className="ml-1">({total} recordings)</span>}
        </p>
      </div>

      {/* Filters */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 rounded-lg border bg-card">
        <div className="space-y-1">
          <Label className="text-xs">Room</Label>
          <Input
            placeholder="Room number"
            value={filters.room_number ?? ''}
            onChange={(e) => setFilters((f) => ({
              ...f, room_number: e.target.value || undefined, offset: 0,
            }))}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">State</Label>
          <Select
            value={filters.running === undefined ? '__all__' : String(filters.running)}
            onValueChange={(v) => setFilters((f) => ({
              ...f, running: v === '__all__' ? undefined : v === 'true', offset: 0,
            }))}
          >
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All</SelectItem>
              <SelectItem value="true">Recording now</SelectItem>
              <SelectItem value="false">Finished</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs">From</Label>
          <Input
            type="date"
            value={from}
            onChange={(e) => {
              setFrom(e.target.value)
              setFilters((f) => ({ ...f, since: dayStart(e.target.value), offset: 0 }))
            }}
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">To</Label>
          <Input
            type="date"
            value={to}
            onChange={(e) => {
              setTo(e.target.value)
              setFilters((f) => ({ ...f, until: dayEnd(e.target.value), offset: 0 }))
            }}
          />
        </div>
      </div>

      {error && (
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </div>
      )}

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={rows} />
      )}

      {/* Pagination */}
      {total >= (filters.limit ?? 100) && (
        <div className="flex items-center justify-center gap-4">
          <Button
            variant="outline"
            size="sm"
            disabled={(filters.offset ?? 0) === 0}
            onClick={() => setFilters((f) => ({
              ...f, offset: Math.max(0, (f.offset ?? 0) - (f.limit ?? 100)),
            }))}
          >
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            {(filters.offset ?? 0) + 1}-{(filters.offset ?? 0) + rows.length}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={rows.length < (filters.limit ?? 100)}
            onClick={() => setFilters((f) => ({
              ...f, offset: (f.offset ?? 0) + (f.limit ?? 100),
            }))}
          >
            Next
          </Button>
        </div>
      )}

      {/* Player */}
      <Dialog
        open={!!playing}
        onOpenChange={(open) => {
          if (!open) {
            setPlaying(null)
            setMediaUrl(null)
          }
        }}
      >
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{playing?.room_number}</DialogTitle>
            <DialogDescription>
              {playing && `${when(playing.started_at)} · ${duration(playing.duration_secs)} · ${size(playing.size_bytes)}`}
            </DialogDescription>
          </DialogHeader>
          {!mediaUrl ? (
            <div className="flex h-32 items-center justify-center text-muted-foreground">
              Loading the recording...
            </div>
          ) : playing && playing.format !== 'wav' ? (
            <video src={mediaUrl} controls autoPlay className="w-full rounded" />
          ) : (
            <audio src={mediaUrl} controls autoPlay className="w-full" />
          )}
          {playing && (
            <p className="text-xs text-muted-foreground font-mono">{playing.file_name}</p>
          )}
        </DialogContent>
      </Dialog>

      {/* Delete confirmation */}
      <Dialog open={!!confirmDelete} onOpenChange={() => setConfirmDelete(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Delete this recording?</DialogTitle>
            <DialogDescription>
              {confirmDelete && `${confirmDelete.room_number}, ${when(confirmDelete.started_at)}.`}
              {' '}The file and its record both go, and neither comes back.
            </DialogDescription>
          </DialogHeader>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={() => setConfirmDelete(null)}>Cancel</Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => confirmDelete && remove.mutate(confirmDelete.id)}
            >
              Delete
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
