import { useState, useRef, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, Upload, Download, CheckCircle2, XCircle, Loader2, FileUp, AlertCircle } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { phoneNumbers, locations } from '@/api/endpoints'
import type { PhoneNumber, CreatePhoneNumberRequest } from '@/types'
import { TenantField, useEffectiveTenantId } from '@/components/TenantSelector'

const createSchema = z.object({
  e164: z.string().min(1, 'E.164 number is required').regex(/^\+?[0-9]+$/, 'Invalid phone number format'),
  cnam: z.string().optional(),
  carrier: z.string().optional(),
  location_id: z.string().optional(),
  tenant_id: z.string().optional(),
})

const editSchema = z.object({
  cnam: z.string().optional(),
  recording_enabled: z.boolean().optional(),
  block_anonymous: z.boolean().optional(),
  location_id: z.string().optional(),
  tenant_id: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

const stateBadge = (state: string) => {
  switch (state) {
    case 'active': return 'success' as const
    case 'available': case 'inventory': return 'secondary' as const
    case 'reserved': case 'pending': return 'warning' as const
    case 'porting': return 'default' as const
    case 'released': case 'disconnected': return 'destructive' as const
    default: return 'outline' as const
  }
}

export function PhoneNumbersPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [editItem, setEditItem] = useState<PhoneNumber | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['phone-numbers'],
    queryFn: phoneNumbers.list,
  })

  const { data: locationList = [] } = useQuery({
    queryKey: ['locations'],
    queryFn: locations.list,
  })

  const locationName = (id: string | null) => {
    if (!id) return '-'
    const loc = locationList.find((l) => l.id === id)
    return loc ? loc.name : id
  }

  const createMutation = useMutation({
    mutationFn: phoneNumbers.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      phoneNumbers.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: phoneNumbers.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['phone-numbers'] }),
  })

  const columns: ColumnDef<PhoneNumber>[] = [
    {
      accessorKey: 'e164',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Number <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'cnam',
      header: 'CNAM',
      cell: ({ row }) => row.original.cnam || '-',
    },
    {
      accessorKey: 'state',
      header: 'State',
      cell: ({ row }) => <Badge variant={stateBadge(row.original.state)}>{row.original.state}</Badge>,
    },
    {
      accessorKey: 'carrier',
      header: 'Carrier',
      cell: ({ row }) => row.original.carrier || '-',
    },
    {
      accessorKey: 'location_id',
      header: 'Location',
      cell: ({ row }) => locationName(row.original.location_id),
    },
    {
      accessorKey: 'rate_center',
      header: 'Rate Center',
      cell: ({ row }) => row.original.rate_center || '-',
    },
    {
      accessorKey: 'e911_address',
      header: 'E911',
      cell: ({ row }) => row.original.e911_address ? (
        <Badge variant="success">Configured</Badge>
      ) : (
        <Badge variant="warning">Not set</Badge>
      ),
    },
    {
      accessorKey: 'tags',
      header: 'Tags',
      cell: ({ row }) => row.original.tags?.length
        ? row.original.tags.map((t) => <Badge key={t} variant="outline" className="mr-1">{t}</Badge>)
        : '-',
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
                if (confirm(`Delete number "${row.original.e164}"?`)) {
                  deleteMutation.mutate(row.original.id)
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

  const effectiveTenantId = useEffectiveTenantId()
  const createForm = useForm<CreateForm>({
    resolver: zodResolver(createSchema),
    defaultValues: { tenant_id: effectiveTenantId },
  })
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      cnam: editItem.cnam || '',
      recording_enabled: editItem.recording_enabled,
      block_anonymous: editItem.block_anonymous,
      location_id: editItem.location_id || '',
      tenant_id: editItem.tenant_id ?? '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      e164: data.e164,
      cnam: data.cnam || undefined,
      carrier: data.carrier || undefined,
      location_id: data.location_id || undefined,
      tenant_id: data.tenant_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      id: editItem.id,
      data: {
        cnam: data.cnam || undefined,
        recording_enabled: data.recording_enabled,
        block_anonymous: data.block_anonymous,
        location_id: data.location_id || undefined,
        tenant_id: data.tenant_id || undefined,
      },
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Phone Numbers</h1>
          <p className="text-muted-foreground">Manage DID numbers and E911 addresses</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => exportPhoneNumbers(items)}>
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload className="mr-2 h-4 w-4" />
            Bulk Import
          </Button>
          <Button onClick={() => { createForm.reset({ tenant_id: effectiveTenantId }); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Number
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="e164" searchPlaceholder="Search numbers..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add Phone Number</DialogTitle>
            <DialogDescription>Add a new DID number to the inventory.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="c-e164">E.164 Number *</Label>
              <Input id="c-e164" placeholder="+15551234567" {...createForm.register('e164')} />
              {createForm.formState.errors.e164 && <p className="text-sm text-destructive">{createForm.formState.errors.e164.message}</p>}
            </div>
            <TenantField value={createForm.watch('tenant_id') ?? ''} onChange={(v) => createForm.setValue('tenant_id', v)} />
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="c-cnam">CNAM</Label>
                <Input id="c-cnam" placeholder="Caller ID name" {...createForm.register('cnam')} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="c-carrier">Carrier</Label>
                <Input id="c-carrier" {...createForm.register('carrier')} />
              </div>
            </div>
            <div className="space-y-2">
              <Label>Location</Label>
              <Select onValueChange={(v) => createForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc) => (
                    <SelectItem key={loc.id} value={loc.id}>{loc.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Adding...' : 'Add Number'}
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
            <DialogTitle>Edit Number: {editItem?.e164}</DialogTitle>
            <DialogDescription>Update phone number settings.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="e-cnam">CNAM</Label>
              <Input id="e-cnam" placeholder="Caller ID name" {...editForm.register('cnam')} />
            </div>
            <TenantField value={editForm.watch('tenant_id') ?? ''} onChange={(v) => editForm.setValue('tenant_id', v)} />
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" {...editForm.register('recording_enabled')} className="rounded" />
                Recording enabled
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" {...editForm.register('block_anonymous')} className="rounded" />
                Block anonymous
              </label>
            </div>
            <div className="space-y-2">
              <Label>Location</Label>
              <Select defaultValue={editItem?.location_id || '__none__'} onValueChange={(v) => editForm.setValue('location_id', v === '__none__' ? '' : v)}>
                <SelectTrigger><SelectValue placeholder="Select location" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">None</SelectItem>
                  {locationList.map((loc) => (
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
      <PhoneNumberImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        existingItems={items}
        onComplete={() => queryClient.invalidateQueries({ queryKey: ['phone-numbers'] })}
      />
    </div>
  )
}

// --- CSV Export / Import Helpers ---

const EXPORT_COLUMNS = ['e164', 'cnam', 'carrier', 'state', 'rate_center', 'location_id'] as const

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

function exportPhoneNumbers(items: PhoneNumber[]) {
  const header = EXPORT_COLUMNS.join(',')
  const rows = items.map((n) =>
    EXPORT_COLUMNS.map((col) => escapeCSVField(String(n[col] ?? ''))).join(',')
  )
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `phone_numbers_export_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}

const IMPORT_COLUMNS = ['e164', 'cnam', 'carrier', 'location_id'] as const

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

  if (!headers.includes('e164')) {
    return { error: 'CSV must contain an "e164" column' }
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

function validatePhoneRow(row: ParsedRow): string[] {
  const errors: string[] = []
  if (!row.e164?.trim()) {
    errors.push('E.164 number is required')
  } else if (!/^\+?[0-9]+$/.test(row.e164.trim())) {
    errors.push('Invalid phone number format')
  }
  return errors
}

function downloadPhoneTemplate() {
  const header = IMPORT_COLUMNS.join(',')
  const sample = '+15551234567,ACME Corp,Bandwidth,'
  const csv = `${header}\n${sample}\n`
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'phone_numbers_import_template.csv'
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

function PhoneNumberImportDialog({
  open,
  onOpenChange,
  existingItems,
  onComplete,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  existingItems: PhoneNumber[]
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
      if (isImporting) { abortRef.current = true; return }
      if (step === 'import' && (importedCount > 0 || updatedCount > 0)) onComplete()
      reset()
    }
    onOpenChange(v)
  }

  const processFile = (file: File) => {
    setFileError(null)
    if (!file.name.endsWith('.csv')) { setFileError('Please select a .csv file'); return }
    const reader = new FileReader()
    reader.onload = (e) => {
      const text = e.target?.result as string
      const result = parseCSV(text)
      if ('error' in result) { setFileError(result.error); return }
      if (result.rows.length === 0) { setFileError('CSV contains no data rows'); return }
      const validated: ImportRowState[] = result.rows.map((data) => ({
        data,
        validationErrors: validatePhoneRow(data),
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
      r.validationErrors.length === 0
        ? { ...r, status: 'pending' as const }
        : { ...r, status: 'error' as const, apiError: 'Skipped (validation errors)' }
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

      const e164 = row.data.e164.trim()
      const updateData: Record<string, unknown> = {}
      if (row.data.cnam?.trim()) updateData.cnam = row.data.cnam.trim()
      if (row.data.carrier?.trim()) updateData.carrier = row.data.carrier.trim()
      if (row.data.location_id?.trim()) updateData.location_id = row.data.location_id.trim()

      try {
        const req: CreatePhoneNumberRequest = { e164, ...updateData as { cnam?: string; carrier?: string } }
        await phoneNumbers.create(req)
        created++
        setImportedCount(created)
        setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success' } : r))
      } catch (createErr: unknown) {
        const createMsg = createErr instanceof Error ? createErr.message : 'Unknown error'
        const isConflict = (createErr as { status?: number }).status === 409

        if (overrideExisting && isConflict) {
          const existing = existingItems.find((n) => n.e164 === e164)
          if (existing) {
            try {
              await phoneNumbers.update(existing.id, updateData)
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
            setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: 'Already exists (not found in current list to update)' } : r))
          }
        } else {
          failed++
          setFailedCount(failed - invalidRows.length)
          setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: createMsg } : r))
        }
      }
    }

    setIsImporting(false)
  }

  const handleDone = () => {
    if (importedCount > 0 || updatedCount > 0) onComplete()
    reset()
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {step === 'upload' && 'Bulk Import Phone Numbers'}
            {step === 'preview' && 'Preview Import'}
            {step === 'import' && 'Importing Phone Numbers'}
          </DialogTitle>
          <DialogDescription>
            {step === 'upload' && 'Upload a CSV file to add multiple phone numbers at once.'}
            {step === 'preview' && `Review ${rows.length} rows before importing.`}
            {step === 'import' && 'Adding phone numbers...'}
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
              <Button variant="link" size="sm" className="px-0" onClick={downloadPhoneTemplate}>
                <Download className="mr-1 h-3 w-3" />
                Download Template
              </Button>
              <p className="text-xs text-muted-foreground">
                Required column: e164
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
                    <TableHead>E.164</TableHead>
                    <TableHead>CNAM</TableHead>
                    <TableHead>Carrier</TableHead>
                    <TableHead>Location</TableHead>
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
                      <TableCell className="font-medium">{row.data.e164 || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.cnam || '-'}</TableCell>
                      <TableCell>{row.data.carrier || '-'}</TableCell>
                      <TableCell>{row.data.location_id || '-'}</TableCell>
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
                    Override existing numbers
                  </Label>
                </div>
                <Button onClick={startImport} disabled={validRows.length === 0}>
                  Import {validRows.length} Number{validRows.length !== 1 ? 's' : ''}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Step 3: Import */}
        {step === 'import' && (
          <div className="space-y-4 min-h-0 flex flex-col">
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>{isImporting ? 'Importing...' : 'Import complete'}</span>
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
                    <TableHead>E.164</TableHead>
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
                      <TableCell className="font-medium">{row.data.e164 || '(empty)'}</TableCell>
                      <TableCell className="text-sm">
                        {row.status === 'success' && row.apiError === 'Updated' && <span className="text-blue-600">Updated</span>}
                        {row.status === 'success' && row.apiError !== 'Updated' && <span className="text-green-600">Created</span>}
                        {row.status === 'error' && (
                          <span className="text-destructive">{row.apiError || row.validationErrors.join('; ')}</span>
                        )}
                        {row.status === 'importing' && <span className="text-muted-foreground">Adding...</span>}
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
