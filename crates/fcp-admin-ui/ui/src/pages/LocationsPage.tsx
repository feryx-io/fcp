import { useState, useRef, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, Upload, Download, CheckCircle2, XCircle, Loader2, FileUp, AlertCircle } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Switch } from '@/components/ui/switch'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table'
import { locations } from '@/api/endpoints'
import type { Location, CreateLocationRequest } from '@/types'

const createSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  description: z.string().optional(),
  address_line1: z.string().optional(),
  address_line2: z.string().optional(),
  city: z.string().optional(),
  state: z.string().optional(),
  postal_code: z.string().optional(),
  country: z.string().optional(),
  floor: z.string().optional(),
  building: z.string().optional(),
  latitude: z.string().optional(),
  longitude: z.string().optional(),
  phone_number: z.string().optional(),
  timezone: z.string().optional(),
  emergency_contact_name: z.string().optional(),
  emergency_contact_phone: z.string().optional(),
})

const editSchema = createSchema

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function LocationsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [editItem, setEditItem] = useState<Location | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['locations'],
    queryFn: locations.list,
  })

  const createMutation = useMutation({
    mutationFn: locations.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['locations'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<CreateLocationRequest> }) =>
      locations.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['locations'] })
      setEditItem(null)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: locations.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['locations'] }),
  })

  const columns: ColumnDef<Location>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
    },
    {
      accessorKey: 'building',
      header: 'Building',
      cell: ({ row }) => row.original.building || '-',
    },
    {
      accessorKey: 'floor',
      header: 'Floor',
      cell: ({ row }) => row.original.floor || '-',
    },
    {
      accessorKey: 'city',
      header: 'City',
      cell: ({ row }) => row.original.city || '-',
    },
    {
      accessorKey: 'state',
      header: 'State',
      cell: ({ row }) => row.original.state || '-',
    },
    {
      accessorKey: 'phone_number',
      header: 'Phone',
      cell: ({ row }) => row.original.phone_number || '-',
    },
    {
      accessorKey: 'timezone',
      header: 'Timezone',
      cell: ({ row }) => row.original.timezone ? (
        <Badge variant="secondary">{row.original.timezone}</Badge>
      ) : '-',
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
                if (confirm(`Delete location "${row.original.name}"?`)) {
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

  const createForm = useForm<CreateForm>({ resolver: zodResolver(createSchema) })
  const editForm = useForm<EditForm>({ resolver: zodResolver(editSchema) })

  const [lastEditId, setLastEditId] = useState<string | null>(null)
  if (editItem && editItem.id !== lastEditId) {
    setLastEditId(editItem.id)
    editForm.reset({
      name: editItem.name,
      description: editItem.description || '',
      address_line1: editItem.address_line1 || '',
      address_line2: editItem.address_line2 || '',
      city: editItem.city || '',
      state: editItem.state || '',
      postal_code: editItem.postal_code || '',
      country: editItem.country || '',
      floor: editItem.floor || '',
      building: editItem.building || '',
      latitude: editItem.latitude != null ? String(editItem.latitude) : '',
      longitude: editItem.longitude != null ? String(editItem.longitude) : '',
      phone_number: editItem.phone_number || '',
      timezone: editItem.timezone || '',
      emergency_contact_name: editItem.emergency_contact_name || '',
      emergency_contact_phone: editItem.emergency_contact_phone || '',
    })
  }

  const formToRequest = (data: CreateForm): CreateLocationRequest => ({
    name: data.name,
    description: data.description || undefined,
    address_line1: data.address_line1 || undefined,
    address_line2: data.address_line2 || undefined,
    city: data.city || undefined,
    state: data.state || undefined,
    postal_code: data.postal_code || undefined,
    country: data.country || undefined,
    floor: data.floor || undefined,
    building: data.building || undefined,
    latitude: data.latitude ? parseFloat(data.latitude) : undefined,
    longitude: data.longitude ? parseFloat(data.longitude) : undefined,
    phone_number: data.phone_number || undefined,
    timezone: data.timezone || undefined,
    emergency_contact_name: data.emergency_contact_name || undefined,
    emergency_contact_phone: data.emergency_contact_phone || undefined,
  })

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate(formToRequest(data))
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({ id: editItem.id, data: formToRequest(data) })
  }

  const locationFormFields = (form: ReturnType<typeof useForm<CreateForm>>) => (
    <>
      {/* General */}
      <div className="space-y-2">
        <Label htmlFor="f-name">Name *</Label>
        <Input id="f-name" placeholder="e.g., Headquarters" {...form.register('name')} />
        {form.formState.errors.name && <p className="text-sm text-destructive">{form.formState.errors.name.message}</p>}
      </div>
      <div className="space-y-2">
        <Label htmlFor="f-description">Description</Label>
        <Input id="f-description" placeholder="Main office" {...form.register('description')} />
      </div>
      <div className="space-y-2">
        <Label htmlFor="f-phone_number">Phone Number</Label>
        <Input id="f-phone_number" placeholder="+1-555-0100" {...form.register('phone_number')} />
      </div>
      <div className="space-y-2">
        <Label htmlFor="f-timezone">Timezone</Label>
        <Input id="f-timezone" placeholder="America/New_York" {...form.register('timezone')} />
      </div>

      {/* Address */}
      <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wider pt-2">Address</p>
      <div className="space-y-2">
        <Label htmlFor="f-address_line1">Address Line 1</Label>
        <Input id="f-address_line1" placeholder="123 Main St" {...form.register('address_line1')} />
      </div>
      <div className="space-y-2">
        <Label htmlFor="f-address_line2">Address Line 2</Label>
        <Input id="f-address_line2" placeholder="Suite 100" {...form.register('address_line2')} />
      </div>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="f-city">City</Label>
          <Input id="f-city" {...form.register('city')} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="f-state">State</Label>
          <Input id="f-state" {...form.register('state')} />
        </div>
      </div>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="f-postal_code">Postal Code</Label>
          <Input id="f-postal_code" {...form.register('postal_code')} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="f-country">Country</Label>
          <Input id="f-country" {...form.register('country')} />
        </div>
      </div>

      {/* Building Details */}
      <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wider pt-2">Building Details</p>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="f-building">Building</Label>
          <Input id="f-building" {...form.register('building')} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="f-floor">Floor</Label>
          <Input id="f-floor" {...form.register('floor')} />
        </div>
      </div>

      {/* Emergency */}
      <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wider pt-2">Emergency Contact</p>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="f-emergency_contact_name">Contact Name</Label>
          <Input id="f-emergency_contact_name" {...form.register('emergency_contact_name')} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="f-emergency_contact_phone">Contact Phone</Label>
          <Input id="f-emergency_contact_phone" {...form.register('emergency_contact_phone')} />
        </div>
      </div>

      {/* Coordinates */}
      <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wider pt-2">Coordinates</p>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="f-latitude">Latitude</Label>
          <Input id="f-latitude" placeholder="40.7128" {...form.register('latitude')} />
        </div>
        <div className="space-y-2">
          <Label htmlFor="f-longitude">Longitude</Label>
          <Input id="f-longitude" placeholder="-74.0060" {...form.register('longitude')} />
        </div>
      </div>
    </>
  )

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Locations</h1>
          <p className="text-muted-foreground">Manage office locations and sites</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => exportLocations(items)}>
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload className="mr-2 h-4 w-4" />
            Bulk Import
          </Button>
          <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> Add Location
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center text-muted-foreground">Loading...</div>
      ) : (
        <DataTable columns={columns} data={items} searchKey="name" searchPlaceholder="Search locations..." />
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-lg max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Add Location</DialogTitle>
            <DialogDescription>Create a new office location or site.</DialogDescription>
          </DialogHeader>
          <form onSubmit={createForm.handleSubmit(onCreateSubmit)} className="space-y-4">
            {locationFormFields(createForm)}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? 'Adding...' : 'Add Location'}
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
        <DialogContent className="max-w-lg max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Location: {editItem?.name}</DialogTitle>
            <DialogDescription>Update location details.</DialogDescription>
          </DialogHeader>
          <form onSubmit={editForm.handleSubmit(onEditSubmit)} className="space-y-4">
            {locationFormFields(editForm)}
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
        onComplete={() => queryClient.invalidateQueries({ queryKey: ['locations'] })}
        existingItems={items}
      />
    </div>
  )
}

// --- CSV Export / Import Helpers ---

const EXPORT_COLUMNS = [
  'name', 'description', 'address_line1', 'address_line2', 'city', 'state',
  'postal_code', 'country', 'floor', 'building', 'latitude', 'longitude',
  'phone_number', 'timezone', 'emergency_contact_name', 'emergency_contact_phone',
] as const

function escapeCSVField(value: string): string {
  if (value.includes(',') || value.includes('"') || value.includes('\n')) {
    return `"${value.replace(/"/g, '""')}"`
  }
  return value
}

function exportLocations(locationList: Location[]) {
  const header = EXPORT_COLUMNS.join(',')
  const rows = locationList.map((loc) =>
    EXPORT_COLUMNS.map((col) => escapeCSVField(String(loc[col] ?? ''))).join(',')
  )
  const csv = [header, ...rows].join('\n') + '\n'
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `locations_export_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}

const IMPORT_COLUMNS = [
  'name', 'description', 'address_line1', 'address_line2', 'city', 'state',
  'postal_code', 'country', 'floor', 'building', 'latitude', 'longitude',
  'phone_number', 'timezone', 'emergency_contact_name', 'emergency_contact_phone',
] as const

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

  if (!headers.includes('name')) {
    return { error: 'CSV must contain at least a "name" column' }
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

function validateLocationImportRow(row: ParsedRow): string[] {
  const errors: string[] = []
  if (!row.name?.trim()) errors.push('Name is required')
  if (row.latitude?.trim() && isNaN(parseFloat(row.latitude))) errors.push('Latitude must be a number')
  if (row.longitude?.trim() && isNaN(parseFloat(row.longitude))) errors.push('Longitude must be a number')
  return errors
}

function downloadLocationTemplate() {
  const header = IMPORT_COLUMNS.join(',')
  const sample = 'Headquarters,Main office,123 Main St,,New York,NY,10001,US,,Building A,40.7128,-74.0060,+1-555-0100,America/New_York,John Doe,+1-555-0911'
  const csv = `${header}\n${sample}\n`
  const blob = new Blob([csv], { type: 'text/csv' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'locations_import_template.csv'
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
  existingItems,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: () => void
  existingItems: Location[]
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
        validationErrors: validateLocationImportRow(data),
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

  const rowToRequest = (row: ParsedRow): CreateLocationRequest => {
    const req: CreateLocationRequest = { name: row.name.trim() }
    if (row.description?.trim()) req.description = row.description.trim()
    if (row.address_line1?.trim()) req.address_line1 = row.address_line1.trim()
    if (row.address_line2?.trim()) req.address_line2 = row.address_line2.trim()
    if (row.city?.trim()) req.city = row.city.trim()
    if (row.state?.trim()) req.state = row.state.trim()
    if (row.postal_code?.trim()) req.postal_code = row.postal_code.trim()
    if (row.country?.trim()) req.country = row.country.trim()
    if (row.floor?.trim()) req.floor = row.floor.trim()
    if (row.building?.trim()) req.building = row.building.trim()
    if (row.latitude?.trim()) req.latitude = parseFloat(row.latitude.trim())
    if (row.longitude?.trim()) req.longitude = parseFloat(row.longitude.trim())
    if (row.phone_number?.trim()) req.phone_number = row.phone_number.trim()
    if (row.timezone?.trim()) req.timezone = row.timezone.trim()
    if (row.emergency_contact_name?.trim()) req.emergency_contact_name = row.emergency_contact_name.trim()
    if (row.emergency_contact_phone?.trim()) req.emergency_contact_phone = row.emergency_contact_phone.trim()
    return req
  }

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

      const reqData = rowToRequest(row.data)

      try {
        await locations.create(reqData)
        created++
        setImportedCount(created)
        setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'success' } : r))
      } catch (createErr: unknown) {
        const createMsg = createErr instanceof Error ? createErr.message : 'Unknown error'
        const isConflict = (createErr as { status?: number }).status === 409

        if (overrideExisting && isConflict) {
          const existing = existingItems.find((l) => l.name === reqData.name)
          if (existing) {
            try {
              await locations.update(existing.id, reqData)
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
            setRows((prev) => prev.map((r, idx) => idx === i ? { ...r, status: 'error', apiError: 'Conflict: location not found for override' } : r))
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
            {step === 'upload' && 'Bulk Import Locations'}
            {step === 'preview' && 'Preview Import'}
            {step === 'import' && 'Importing Locations'}
          </DialogTitle>
          <DialogDescription>
            {step === 'upload' && 'Upload a CSV file to create multiple locations at once.'}
            {step === 'preview' && `Review ${rows.length} rows before importing.`}
            {step === 'import' && 'Creating locations...'}
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
              <Button variant="link" size="sm" className="px-0" onClick={downloadLocationTemplate}>
                <Download className="mr-1 h-3 w-3" />
                Download Template
              </Button>
              <p className="text-xs text-muted-foreground">
                Required columns: name
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
                    <TableHead>Name</TableHead>
                    <TableHead>City</TableHead>
                    <TableHead>State</TableHead>
                    <TableHead>Building</TableHead>
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
                      <TableCell className="font-medium">{row.data.name || <span className="text-destructive italic">empty</span>}</TableCell>
                      <TableCell>{row.data.city || '-'}</TableCell>
                      <TableCell>{row.data.state || '-'}</TableCell>
                      <TableCell>{row.data.building || '-'}</TableCell>
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
                    Override existing locations
                  </Label>
                </div>
                <Button onClick={startImport} disabled={validRows.length === 0}>
                  Import {validRows.length} Location{validRows.length !== 1 ? 's' : ''}
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
                    <TableHead>Name</TableHead>
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
                      <TableCell className="font-medium">{row.data.name || '(empty)'}</TableCell>
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
