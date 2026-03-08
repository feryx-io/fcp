import { useState, useRef, useEffect, useCallback, useMemo } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, X, Upload, Download, CheckCircle2, XCircle, Loader2, FileUp, AlertCircle } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Switch } from '@/components/ui/switch'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { extensions, users, locations, devices, phoneNumbers } from '@/api/endpoints'
import type { Extension, CreateExtensionRequest, UpdateExtensionRequest, User, Location } from '@/types'

const TARGET_TYPES = [
  { value: 'user', label: 'User' },
  { value: 'phone_number', label: 'Phone Number' },
  { value: 'device', label: 'Device' },
  { value: 'service', label: 'Service' },
]

function formatTargetType(type: string): string {
  const found = TARGET_TYPES.find(t => t.value === type)
  return found ? found.label : type
}

const createSchema = z.object({
  number: z.string().min(1, 'Extension number is required').regex(/^\d+$/, 'Must be numeric'),
  target_type: z.string().min(1, 'Target type is required'),
  target_id: z.string().min(1, 'Target is required'),
  description: z.string().optional(),
  location_id: z.string().optional(),
})

const editSchema = z.object({
  target_type: z.string().optional(),
  target_id: z.string().optional(),
  description: z.string().optional(),
  location_id: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function ExtensionsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [editItem, setEditItem] = useState<Extension | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['extensions'],
    queryFn: extensions.list,
  })

  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  const { data: locationList = [] } = useQuery({
    queryKey: ['locations'],
    queryFn: locations.list,
  })

  const { data: deviceList = [] } = useQuery({
    queryKey: ['devices'],
    queryFn: devices.list,
  })

  const { data: phoneNumberList = [] } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const userMap = useMemo(() => new Map(userList.map(u => [u.id, u])), [userList])
  const deviceMap = useMemo(() => new Map(deviceList.map(d => [d.id, d])), [deviceList])
  const phoneNumberMap = useMemo(() => new Map(phoneNumberList.map(p => [p.id, p])), [phoneNumberList])

  const createMutation = useMutation({
    mutationFn: extensions.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ number, data }: { number: string; data: UpdateExtensionRequest }) =>
      extensions.update(number, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: extensions.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['extensions'] }),
  })

  function targetDisplay(ext: Extension): string {
    switch (ext.target.type) {
      case 'user': {
        const user = userMap.get(ext.target.id)
        return user ? (user.display_name || user.username) : ext.target.id
      }
      case 'phone_number': {
        const pn = phoneNumberMap.get(ext.target.id)
        return pn ? pn.e164 : ext.target.id
      }
      case 'device': {
        const dev = deviceMap.get(ext.target.id)
        return dev ? dev.name : ext.target.id
      }
      case 'service':
        return ext.target.id
    }
  }

  const columns: ColumnDef<Extension>[] = [
    {
      accessorKey: 'number',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Number <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'description',
      header: 'Description',
      cell: ({ row }) => row.original.description || '-',
    },
    {
      id: 'target_type',
      header: 'Target Type',
      cell: ({ row }) => <Badge variant="secondary">{formatTargetType(row.original.target.type)}</Badge>,
    },
    {
      id: 'target',
      header: 'Target',
      cell: ({ row }) => targetDisplay(row.original),
    },
    {
      accessorKey: 'enabled',
      header: 'Status',
      cell: ({ row }) => (
        <Badge variant={row.original.enabled ? 'success' : 'destructive'}>
          {row.original.enabled ? 'Enabled' : 'Disabled'}
        </Badge>
      ),
    },
    {
      accessorKey: 'location_id',
      header: 'Location',
      cell: ({ row }) => locationName(row.original.location_id),
    },
    {
      id: 'actions',
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon"><MoreHorizontal className="h-4 w-4" /></Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setEditItem(row.original)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => {
                if (confirm(`Delete extension "${row.original.number}"?`)) {
                  deleteMutation.mutate(row.original.number)
                }
              }}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ]

  const createForm = useForm<CreateForm>({ resolver: zodResolver(createSchema) })
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      target_type: editItem.target.type,
      target_id: editItem.target.id,
      description: editItem.description || '',
      location_id: editItem.location_id || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      number: data.number,
      target: { type: data.target_type as CreateExtensionRequest['target']['type'], id: data.target_id },
      description: data.description || undefined,
      location_id: data.location_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    const updateData: UpdateExtensionRequest = {
      description: data.description || undefined,
      location_id: data.location_id || undefined,
    }
    if (data.target_type && data.target_id) {
      updateData.target = { type: data.target_type as UpdateExtensionRequest['target'] extends undefined ? never : NonNullable<UpdateExtensionRequest['target']>['type'], id: data.target_id }
    }
    updateMutation.mutate({ number: editItem.number, data: updateData })
  }

  const locationName = (id: string | null) => {
    if (!id) return '-'
    const loc = locationList.find((l: Location) => l.id === id)
    return loc ? loc.name : id
  }

  const watchCreateTargetType = createForm.watch('target_type')
  const watchEditTargetType = editForm.watch('target_type')

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Extensions</h1>
          <p className="text-muted-foreground">Manage telephone extensions</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => exportExtensions(items)}>
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload className="mr-2 h-4 w-4" />
            Bulk Import
          </Button>
          <Button onClick={() => { createForm.reset({ number: '', target_type: '', target_id: '', description: '', location_id: '' }); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Extension
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="number" searchPlaceholder="Search extensions..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Create Extension</DialogTitle>
            <DialogDescription>Add a new telephone extension.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-number">Extension Number *</Label>
              <Input id="c-number" placeholder="1001" {...createForm.register('number')} />
              {createForm.formState.errors.number && <p className="text-sm text-destructive">{createForm.formState.errors.number.message}</p>}
            </div>
            <div className="space-y-2">
              <Label htmlFor="c-description">Description</Label>
              <Input id="c-description" placeholder="Front desk, conference room, etc." {...createForm.register('description')} />
            </div>
            <div className="space-y-2">
              <Label>Target Type *</Label>
              <Select
                value={watchCreateTargetType || ''}
                onValueChange={(v) => { createForm.setValue('target_type', v); createForm.setValue('target_id', '') }}
              >
                <SelectTrigger><SelectValue placeholder="Select target type" /></SelectTrigger>
                <SelectContent>
                  {TARGET_TYPES.map(t => (
                    <SelectItem key={t.value} value={t.value}>{t.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {createForm.formState.errors.target_type && <p className="text-sm text-destructive">{createForm.formState.errors.target_type.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Target *</Label>
              <TargetSelector
                targetType={watchCreateTargetType || ''}
                value={createForm.watch('target_id') || ''}
                onChange={(v) => createForm.setValue('target_id', v)}
                userList={userList}
                deviceList={deviceList}
                phoneNumberList={phoneNumberList}
              />
              {createForm.formState.errors.target_id && <p className="text-sm text-destructive">{createForm.formState.errors.target_id.message}</p>}
            </div>
            <div className="space-y-2">
              <Label>Location</Label>
              <Select onValueChange={(v) => createForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc: Location) => (
                    <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Creating...' : 'Create Extension'}
              </Button>
            </div>
            {createMutation.isError && (
              <p className="text-sm text-destructive">{(createMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editItem} onOpenChange={(v) => { if (!v) { setEditItem(null); setLastEditId(null); editForm.reset() } }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit Extension: {editItem?.number}</DialogTitle>
            <DialogDescription>Update extension details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-description">Description</Label>
              <Input id="e-description" {...editForm.register('description')} />
            </div>
            <div className="space-y-2">
              <Label>Target Type</Label>
              <Select
                value={watchEditTargetType || ''}
                onValueChange={(v) => { editForm.setValue('target_type', v); editForm.setValue('target_id', '') }}
              >
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {TARGET_TYPES.map(t => (
                    <SelectItem key={t.value} value={t.value}>{t.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Target</Label>
              <TargetSelector
                targetType={watchEditTargetType || ''}
                value={editForm.watch('target_id') || ''}
                onChange={(v) => editForm.setValue('target_id', v)}
                userList={userList}
                deviceList={deviceList}
                phoneNumberList={phoneNumberList}
              />
            </div>
            <div className="space-y-2">
              <Label>Location</Label>
              <Select defaultValue={editItem?.location_id || '__none__'} onValueChange={(v) => editForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc: Location) => (
                    <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setEditItem(null)}>Cancel</Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {updateMutation.isPending ? 'Saving...' : 'Save Changes'}
              </Button>
            </div>
            {updateMutation.isError && (
              <p className="text-sm text-destructive">{(updateMutation.error as Error).message}</p>
            )}
          </form>
        </DialogContent>
      </Dialog>

      {/* Bulk Import Dialog */}
      <BulkImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onComplete={() => queryClient.invalidateQueries({ queryKey: ['extensions'] })}
      />
    </div>
  )
}

// --- Target Selector Component ---

function TargetSelector({
  targetType,
  value,
  onChange,
  userList,
  deviceList,
  phoneNumberList,
}: {
  targetType: string
  value: string
  onChange: (value: string) => void
  userList: User[]
  deviceList: { id: string; name: string }[]
  phoneNumberList: { id: string; e164: string; cnam: string | null }[]
}) {
  if (targetType === 'user') {
    return (
      <UserAutocomplete
        users={userList}
        value={value}
        onChange={onChange}
        placeholder="Search by username or name..."
      />
    )
  }
  if (targetType === 'phone_number') {
    return (
      <Select value={value || undefined} onValueChange={onChange}>
        <SelectTrigger><SelectValue placeholder="Select phone number" /></SelectTrigger>
        <SelectContent>
          {phoneNumberList.map(pn => (
            <SelectItem key={pn.id} value={pn.id}>
              {pn.cnam ? `${pn.e164} - ${pn.cnam}` : pn.e164}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    )
  }
  if (targetType === 'device') {
    return (
      <Select value={value || undefined} onValueChange={onChange}>
        <SelectTrigger><SelectValue placeholder="Select device" /></SelectTrigger>
        <SelectContent>
          {deviceList.map(d => (
            <SelectItem key={d.id} value={d.id}>{d.name}</SelectItem>
          ))}
        </SelectContent>
      </Select>
    )
  }
  if (targetType === 'service') {
    return <Input value={value} onChange={(e) => onChange(e.target.value)} placeholder="Service ID" />
  }
  return <Input disabled placeholder="Select a target type first" />
}

// --- CSV Export / Import Helpers ---

const EXPORT_COLUMNS = ['number', 'description', 'target_type', 'target_id', 'enabled', 'location_id'] as const

function escapeCSVField(value: string): string {
  // Neutralize spreadsheet formula injection (CWE-1236):
  // check first non-whitespace/control char to catch " =CMD", "\t@SUM", etc.
  const first = value.match(/[^\s\x00-\x1f]/)?.[0]
  if (first && '=+-@'.includes(first)) {
    value = "'" + value
  }
  if (value.includes(',') || value.includes('"') || value.includes('\n')) {
    return `"${value.replace(/"/g, '""')}"`
  }
  return value
}

function exportExtensions(extList: Extension[]) {
  const header = EXPORT_COLUMNS.join(',')
  const rows = extList.map((ext) => {
    const values: Record<string, string> = {
      number: ext.number,
      description: ext.description || '',
      target_type: ext.target.type,
      target_id: ext.target.id,
      enabled: String(ext.enabled),
      location_id: ext.location_id || '',
    }
    return EXPORT_COLUMNS.map((col) => escapeCSVField(values[col] ?? '')).join(',')
  })
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `extensions_export_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}

const IMPORT_COLUMNS = ['number', 'description', 'target_type', 'target_id', 'location_id'] as const

interface ParsedRow {
  [key: string]: string
}

function parseCSV(text: string): { headers: string[]; rows: ParsedRow[] } | { error: string } {
  const lines = text.split(/\r?\n/).filter((line) => line.trim() !== '')
  if (lines.length === 0) return { error: 'File is empty' }

  const parseLine = (line: string): string[] => {
    const fields: string[] = []
    let current = ''
    let inQuotes = false
    for (let i = 0; i < line.length; i++) {
      const ch = line[i]
      if (inQuotes) {
        if (ch === '"') {
          if (i + 1 < line.length && line[i + 1] === '"') {
            current += '"'
            i++
          } else {
            inQuotes = false
          }
        } else {
          current += ch
        }
      } else {
        if (ch === '"') {
          inQuotes = true
        } else if (ch === ',') {
          fields.push(current.trim())
          current = ''
        } else {
          current += ch
        }
      }
    }
    fields.push(current.trim())
    return fields
  }

  const headers = parseLine(lines[0]).map((h) => h.toLowerCase().replace(/\s+/g, '_'))

  if (!headers.includes('number')) {
    return { error: 'CSV must contain at least a "number" column' }
  }

  const rows: ParsedRow[] = []
  for (let i = 1; i < lines.length; i++) {
    const values = parseLine(lines[i])
    const row: ParsedRow = {}
    headers.forEach((h, idx) => {
      row[h] = values[idx] || ''
    })
    rows.push(row)
  }

  return { headers, rows }
}

function validateImportRow(row: ParsedRow): string[] {
  const errors: string[] = []
  if (!row.number?.trim()) errors.push('Number is required')
  else if (!/^\d+$/.test(row.number.trim())) errors.push('Number must be all digits')
  if (!row.target_type?.trim()) errors.push('target_type is required')
  else if (!['user', 'phone_number', 'device', 'service'].includes(row.target_type.trim())) {
    errors.push('target_type must be user, phone_number, device, or service')
  }
  if (!row.target_id?.trim()) errors.push('target_id is required')
  return errors
}

function downloadTemplate() {
  const header = IMPORT_COLUMNS.join(',')
  const sample = '1001,Front Desk,user,user-00000000-0000-0000-0000-000000000001,'
  const csv = `${header}\n${sample}\n`
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'extensions_import_template.csv'
  a.click()
  URL.revokeObjectURL(url)
}

type ImportStep = 'upload' | 'preview' | 'import'
type RowStatus = 'pending' | 'importing' | 'success' | 'error'

interface ImportRowState {
  data: ParsedRow
  validationErrors: string[]
  status: RowStatus
  apiError?: string
}

function BulkImportDialog({
  open,
  onOpenChange,
  onComplete,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: () => void
}) {
  const [step, setStep] = useState<ImportStep>('upload')
  const [rows, setRows] = useState<ImportRowState[]>([])
  const [importedCount, setImportedCount] = useState(0)
  const [failedCount, setFailedCount] = useState(0)
  const [updatedCount, setUpdatedCount] = useState(0)
  const [isImporting, setIsImporting] = useState(false)
  const [fileError, setFileError] = useState<string | null>(null)
  const [isDragOver, setIsDragOver] = useState(false)
  const [overrideExisting, setOverrideExisting] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const abortRef = useRef(false)

  const reset = useCallback(() => {
    setStep('upload')
    setRows([])
    setImportedCount(0)
    setFailedCount(0)
    setUpdatedCount(0)
    setIsImporting(false)
    setFileError(null)
    setIsDragOver(false)
    setOverrideExisting(false)
    abortRef.current = false
  }, [])

  const handleClose = (v: boolean) => {
    if (!v) {
      if (isImporting) {
        abortRef.current = true
        return
      }
      if (step === 'import' && importedCount > 0) {
        onComplete()
      }
      reset()
    }
    onOpenChange(v)
  }

  const processFile = (file: File) => {
    setFileError(null)
    if (!file.name.endsWith('.csv')) {
      setFileError('Please select a .csv file')
      return
    }
    const reader = new FileReader()
    reader.onload = (e) => {
      const text = e.target?.result as string
      const result = parseCSV(text)
      if ('error' in result) {
        setFileError(result.error)
        return
      }
      if (result.rows.length === 0) {
        setFileError('CSV contains no data rows')
        return
      }
      const validated: ImportRowState[] = result.rows.map((data) => ({
        data,
        validationErrors: validateImportRow(data),
        status: 'pending' as const,
      }))
      setRows(validated)
      setStep('preview')
    }
    reader.readAsText(file)
  }

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) processFile(file)
  }

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragOver(false)
    const file = e.dataTransfer.files[0]
    if (file) processFile(file)
  }

  const validRows = rows.filter((r) => r.validationErrors.length === 0)
  const invalidRows = rows.filter((r) => r.validationErrors.length > 0)

  const startImport = async () => {
    setStep('import')
    setIsImporting(true)
    setImportedCount(0)
    setFailedCount(0)
    setUpdatedCount(0)
    abortRef.current = false

    const updatedRows = rows.map((r) =>
      r.validationErrors.length === 0 ? { ...r, status: 'pending' as const } : { ...r, status: 'error' as const, apiError: 'Skipped (validation errors)' }
    )
    setRows(updatedRows)

    let created = 0
    let updated = 0
    let failed = invalidRows.length

    for (let i = 0; i < updatedRows.length; i++) {
      if (abortRef.current) break
      const row = updatedRows[i]
      if (row.validationErrors.length > 0) continue

      setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'importing' } : r))

      const number = row.data.number.trim()
      const targetType = row.data.target_type?.trim() || 'user'
      const targetId = row.data.target_id?.trim() || ''

      try {
        const req: CreateExtensionRequest = {
          number,
          target: { type: targetType as CreateExtensionRequest['target']['type'], id: targetId },
          description: row.data.description?.trim() || undefined,
          location_id: row.data.location_id?.trim() || undefined,
        }
        await extensions.create(req)
        created++
        setImportedCount(created)
        setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success' } : r))
      } catch (createErr: unknown) {
        const createMsg = createErr instanceof Error ? createErr.message : 'Unknown error'
        const isConflict = (createErr as { status?: number }).status === 409

        if (overrideExisting && isConflict) {
          try {
            const updateData: UpdateExtensionRequest = {
              target: { type: targetType as NonNullable<UpdateExtensionRequest['target']>['type'], id: targetId },
              description: row.data.description?.trim() || undefined,
              location_id: row.data.location_id?.trim() || undefined,
            }
            await extensions.update(number, updateData)
            updated++
            setUpdatedCount(updated)
            setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success', apiError: 'Updated' } : r))
          } catch (updateErr) {
            failed++
            setFailedCount(failed - invalidRows.length)
            const message = updateErr instanceof Error ? updateErr.message : 'Unknown error'
            setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: `Update failed: ${message}` } : r))
          }
        } else {
          failed++
          setFailedCount(failed - invalidRows.length)
          setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: createMsg || 'Unknown error' } : r))
        }
      }
    }

    setIsImporting(false)
  }

  const handleDone = () => {
    if (importedCount > 0) onComplete()
    reset()
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {step === 'upload' && 'Bulk Import Extensions'}
            {step === 'preview' && 'Preview Import'}
            {step === 'import' && 'Importing Extensions'}
          </DialogTitle>
          <DialogDescription>
            {step === 'upload' && 'Upload a CSV file to create multiple extensions at once.'}
            {step === 'preview' && `Review ${rows.length} rows before importing.`}
            {step === 'import' && 'Creating extensions...'}
          </DialogDescription>
        </DialogHeader>

        {/* Step 1: Upload */}
        {step === 'upload' && (
          <div className="space-y-4">
            <div
              className={`border-2 border-dashed rounded-lg p-8 text-center cursor-pointer transition-colors ${
                isDragOver ? 'border-primary bg-primary/5' : 'border-muted-foreground/25 hover:border-primary/50'
              }`}
              onClick={() => fileInputRef.current?.click()}
              onDragOver={(e) => { e.preventDefault(); setIsDragOver(true) }}
              onDragLeave={() => setIsDragOver(false)}
              onDrop={handleDrop}
            >
              <FileUp className="mx-auto h-10 w-10 text-muted-foreground mb-3" />
              <p className="text-sm font-medium">Drop a CSV file here or click to browse</p>
              <p className="text-xs text-muted-foreground mt-1">Accepts .csv files</p>
              <input
                ref={fileInputRef}
                type="file"
                accept=".csv"
                className="hidden"
                onChange={handleFileChange}
              />
            </div>
            {fileError && (
              <div className="flex items-center gap-2 text-sm text-destructive">
                <AlertCircle className="h-4 w-4 shrink-0" />
                {fileError}
              </div>
            )}
            <div className="flex items-center justify-between">
              <Button variant="link" size="sm" className="px-0" onClick={downloadTemplate}>
                <Download className="mr-1 h-3 w-3" />
                Download Template
              </Button>
              <p className="text-xs text-muted-foreground">
                Required columns: number, target_type, target_id
              </p>
            </div>
          </div>
        )}

        {/* Step 2: Preview */}
        {step === 'preview' && (
          <div className="space-y-4 min-h-0 flex flex-col">
            <div className="flex items-center gap-3 text-sm">
              <Badge variant="default">{validRows.length} valid</Badge>
              {invalidRows.length > 0 && (
                <Badge variant="destructive">{invalidRows.length} with errors</Badge>
              )}
            </div>

            <div className="border rounded-md overflow-auto max-h-[400px] min-h-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-8">#</TableHead>
                    <TableHead>Number</TableHead>
                    <TableHead>Description</TableHead>
                    <TableHead>Target Type</TableHead>
                    <TableHead>Target ID</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row, idx) => (
                    <TableRow
                      key={idx}
                      className={row.validationErrors.length > 0 ? 'bg-destructive/5' : ''}
                    >
                      <TableCell className="text-muted-foreground">{idx + 1}</TableCell>
                      <TableCell className="font-medium">{row.data.number || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.description || '-'}</TableCell>
                      <TableCell>{row.data.target_type || '-'}</TableCell>
                      <TableCell className="max-w-[120px] truncate">{row.data.target_id || '-'}</TableCell>
                      <TableCell>
                        {row.validationErrors.length > 0 ? (
                          <span className="text-xs text-destructive">{row.validationErrors.join('; ')}</span>
                        ) : (
                          <span className="text-xs text-green-600">OK</span>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            <div className="flex items-center justify-between">
              <Button variant="outline" onClick={() => { setRows([]); setStep('upload') }}>
                Back
              </Button>
              <div className="flex items-center gap-4">
                <div className="flex items-center gap-2">
                  <Switch
                    id="override-existing"
                    checked={overrideExisting}
                    onCheckedChange={setOverrideExisting}
                  />
                  <Label htmlFor="override-existing" className="text-sm cursor-pointer">
                    Override existing extensions
                  </Label>
                </div>
                <Button onClick={startImport} disabled={validRows.length === 0}>
                  Import {validRows.length} Extension{validRows.length !== 1 ? 's' : ''}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Step 3: Import */}
        {step === 'import' && (
          <div className="space-y-4 min-h-0 flex flex-col">
            {/* Progress bar */}
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>
                  {isImporting ? 'Importing...' : 'Import complete'}
                </span>
                <span className="text-muted-foreground">
                  {importedCount + updatedCount + failedCount} / {validRows.length}
                </span>
              </div>
              <div className="h-2 rounded-full bg-muted overflow-hidden">
                <div
                  className="h-full bg-primary rounded-full transition-all duration-300"
                  style={{ width: `${validRows.length > 0 ? ((importedCount + updatedCount + failedCount) / validRows.length) * 100 : 0}%` }}
                />
              </div>
              {!isImporting && (
                <div className="flex gap-4 text-sm">
                  <span className="text-green-600">{importedCount} created</span>
                  {updatedCount > 0 && <span className="text-blue-600">{updatedCount} updated</span>}
                  {failedCount > 0 && <span className="text-destructive">{failedCount} failed</span>}
                  {invalidRows.length > 0 && <span className="text-muted-foreground">{invalidRows.length} skipped</span>}
                </div>
              )}
            </div>

            <div className="border rounded-md overflow-auto max-h-[350px] min-h-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-8"></TableHead>
                    <TableHead>Number</TableHead>
                    <TableHead>Result</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row, idx) => (
                    <TableRow key={idx}>
                      <TableCell>
                        {row.status === 'importing' && <Loader2 className="h-4 w-4 animate-spin text-primary" />}
                        {row.status === 'success' && <CheckCircle2 className="h-4 w-4 text-green-600" />}
                        {row.status === 'error' && <XCircle className="h-4 w-4 text-destructive" />}
                        {row.status === 'pending' && <span className="block h-4 w-4 rounded-full border-2 border-muted-foreground/25" />}
                      </TableCell>
                      <TableCell className="font-medium">{row.data.number || '(empty)'}</TableCell>
                      <TableCell className="text-sm">
                        {row.status === 'success' && row.apiError === 'Updated' && <span className="text-blue-600">Updated</span>}
                        {row.status === 'success' && row.apiError !== 'Updated' && <span className="text-green-600">Created</span>}
                        {row.status === 'error' && (
                          <span className="text-destructive">{row.apiError || row.validationErrors.join('; ')}</span>
                        )}
                        {row.status === 'importing' && <span className="text-muted-foreground">Creating...</span>}
                        {row.status === 'pending' && <span className="text-muted-foreground">Waiting</span>}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            <div className="flex justify-end">
              {isImporting ? (
                <Button variant="outline" onClick={() => { abortRef.current = true }}>
                  Cancel
                </Button>
              ) : (
                <Button onClick={handleDone}>Done</Button>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function UserAutocomplete({
  users,
  value,
  onChange,
  placeholder,
}: {
  users: User[]
  value: string
  onChange: (value: string) => void
  placeholder?: string
}) {
  const selectedUser = users.find(u => u.id === value)
  const [inputValue, setInputValue] = useState(selectedUser ? (selectedUser.display_name || selectedUser.username) : '')
  const [isOpen, setIsOpen] = useState(false)
  const [focusedIndex, setFocusedIndex] = useState(-1)
  const wrapperRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLUListElement>(null)

  // Sync inputValue when form value changes externally (e.g. on form reset)
  useEffect(() => {
    const user = users.find(u => u.id === value)
    if (user) {
      setInputValue(user.display_name || user.username)
    } else if (!value) {
      setInputValue('')
    }
  }, [value, users])

  // Close dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (wrapperRef.current && !wrapperRef.current.contains(e.target as Node)) {
        setIsOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  const query = inputValue.toLowerCase()
  const isSelected = selectedUser && (inputValue === (selectedUser.display_name || selectedUser.username))
  const filtered = (!isSelected && query)
    ? users.filter(
        (u) =>
          u.username.toLowerCase().includes(query) ||
          (u.display_name && u.display_name.toLowerCase().includes(query)) ||
          (u.email && u.email.toLowerCase().includes(query))
      ).slice(0, 8)
    : users.slice(0, 8)

  const selectUser = (user: User) => {
    setInputValue(user.display_name || user.username)
    onChange(user.id)
    setIsOpen(false)
    setFocusedIndex(-1)
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!isOpen) {
      if (e.key === 'ArrowDown') {
        setIsOpen(true)
        setFocusedIndex(0)
        e.preventDefault()
      }
      return
    }

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        setFocusedIndex((prev) => Math.min(prev + 1, filtered.length - 1))
        break
      case 'ArrowUp':
        e.preventDefault()
        setFocusedIndex((prev) => Math.max(prev - 1, 0))
        break
      case 'Enter':
        e.preventDefault()
        if (focusedIndex >= 0 && focusedIndex < filtered.length) {
          selectUser(filtered[focusedIndex])
        }
        break
      case 'Escape':
        setIsOpen(false)
        setFocusedIndex(-1)
        break
    }
  }

  // Scroll focused item into view
  useEffect(() => {
    if (focusedIndex >= 0 && listRef.current) {
      const item = listRef.current.children[focusedIndex] as HTMLElement | undefined
      item?.scrollIntoView({ block: 'nearest' })
    }
  }, [focusedIndex])

  return (
    <div ref={wrapperRef} className="relative">
      <div className="relative">
        <Input
          value={inputValue}
          onChange={(e) => {
            const v = e.target.value
            setInputValue(v)
            // Clear the selected user ID when typing
            if (selectedUser && v !== (selectedUser.display_name || selectedUser.username)) {
              onChange('')
            }
            setIsOpen(true)
            setFocusedIndex(-1)
          }}
          onFocus={() => setIsOpen(true)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          autoComplete="off"
        />
        {inputValue && (
          <button
            type="button"
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            onClick={() => {
              setInputValue('')
              onChange('')
              setIsOpen(false)
            }}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
      {isOpen && filtered.length > 0 && (
        <ul
          ref={listRef}
          className="absolute z-50 mt-1 w-full max-h-48 overflow-auto rounded-md border bg-popover shadow-md"
        >
          {filtered.map((u, idx) => (
            <li
              key={u.id}
              className={`flex cursor-pointer items-center gap-2 px-3 py-2 text-sm hover:bg-accent ${
                idx === focusedIndex ? 'bg-accent' : ''
              }`}
              onMouseDown={(e) => { e.preventDefault(); selectUser(u) }}
              onMouseEnter={() => setFocusedIndex(idx)}
            >
              <span className="font-medium">{u.username}</span>
              {u.display_name && (
                <span className="text-muted-foreground">{u.display_name}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
