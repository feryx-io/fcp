import { useState, useRef } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Trash2, Upload, Play, Pause, Pencil, Volume2 } from 'lucide-react'
import { DataTable } from '@/components/DataTable'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select'
import { prompts, type PromptResponse, type CreatePromptRequest, type UpdatePromptRequest } from '@/api/endpoints'

const CATEGORIES = ['system', 'ivr', 'voicemail', 'queue', 'greeting', 'moh', 'custom'] as const

const categoryColors: Record<string, string> = {
  system: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
  ivr: 'bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-200',
  voicemail: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
  queue: 'bg-orange-100 text-orange-800 dark:bg-orange-900 dark:text-orange-200',
  greeting: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200',
  moh: 'bg-pink-100 text-pink-800 dark:bg-pink-900 dark:text-pink-200',
  custom: 'bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200',
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export function PromptsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editTarget, setEditTarget] = useState<PromptResponse | null>(null)
  const [uploadTarget, setUploadTarget] = useState<PromptResponse | null>(null)
  const [categoryFilter, setCategoryFilter] = useState<string>('all')
  const [playingId, setPlayingId] = useState<string | null>(null)
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const queryClient = useQueryClient()

  // Create form state
  const [formName, setFormName] = useState('')
  const [formDescription, setFormDescription] = useState('')
  const [formCategory, setFormCategory] = useState('custom')
  const [formLanguage, setFormLanguage] = useState('en-US')
  const [formSourceType, setFormSourceType] = useState('tts')
  const [formTtsText, setFormTtsText] = useState('')

  // Edit form state
  const [editDescription, setEditDescription] = useState('')
  const [editCategory, setEditCategory] = useState('')
  const [editTtsText, setEditTtsText] = useState('')
  const [editTtsVoice, setEditTtsVoice] = useState('')

  const { data: items = [] } = useQuery({
    queryKey: ['prompts', categoryFilter],
    queryFn: () => prompts.list(categoryFilter !== 'all' ? { category: categoryFilter } : undefined),
  })

  const [createError, setCreateError] = useState<string | null>(null)
  const [editError, setEditError] = useState<string | null>(null)

  const createMutation = useMutation({
    mutationFn: prompts.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['prompts'] })
      setCreateOpen(false)
      setCreateError(null)
      resetForm()
    },
    onError: (err: Error) => {
      setCreateError(err.message || 'Failed to create prompt')
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdatePromptRequest }) => prompts.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['prompts'] })
      setEditTarget(null)
      setEditError(null)
    },
    onError: (err: Error) => {
      setEditError(err.message || 'Failed to update prompt')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: prompts.delete,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['prompts'] }),
  })

  const uploadMutation = useMutation({
    mutationFn: ({ id, file }: { id: string; file: File }) => prompts.uploadAudio(id, file),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['prompts'] })
      setUploadTarget(null)
    },
  })

  const synthesizeMutation = useMutation({
    mutationFn: prompts.synthesize,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['prompts'] })
    },
  })

  function resetForm() {
    setFormName('')
    setFormDescription('')
    setFormCategory('custom')
    setFormLanguage('en-US')
    setFormSourceType('tts')
    setFormTtsText('')
  }

  function handleCreate() {
    const data: CreatePromptRequest = {
      name: formName,
      description: formDescription || undefined,
      category: formCategory,
      language: formLanguage,
      source_type: formSourceType,
      tts_text: formTtsText || undefined,
    }
    createMutation.mutate(data)
  }

  function openEdit(p: PromptResponse) {
    setEditDescription(p.description || '')
    setEditCategory(p.category)
    setEditTtsText(p.tts_text || '')
    setEditTtsVoice(p.tts_voice || '')
    setEditError(null)
    setEditTarget(p)
  }

  function handleUpdate() {
    if (!editTarget) return
    const data: UpdatePromptRequest = {
      description: editDescription || undefined,
      category: editCategory,
      tts_text: editTtsText || undefined,
      tts_voice: editTtsVoice || undefined,
    }
    updateMutation.mutate({ id: editTarget.id, data })
  }

  function handleUpload(file: File) {
    if (!uploadTarget) return
    uploadMutation.mutate({ id: uploadTarget.id, file })
  }

  function togglePlay(prompt: PromptResponse) {
    if (playingId === prompt.id) {
      audioRef.current?.pause()
      setPlayingId(null)
    } else {
      if (audioRef.current) {
        audioRef.current.pause()
      }
      const audio = new Audio(prompts.audioUrl(prompt.id))
      audio.onended = () => setPlayingId(null)
      audio.onerror = () => setPlayingId(null)
      audio.play()
      audioRef.current = audio
      setPlayingId(prompt.id)
    }
  }

  const columns: ColumnDef<PromptResponse>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => (
        <Button variant="ghost" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
          Name <ArrowUpDown className="ml-2 h-4 w-4" />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.name}</div>
          {row.original.description && (
            <div className="text-xs text-muted-foreground">{row.original.description}</div>
          )}
        </div>
      ),
    },
    {
      accessorKey: 'category',
      header: 'Category',
      cell: ({ row }) => (
        <Badge variant="outline" className={categoryColors[row.original.category] || ''}>
          {row.original.category}
        </Badge>
      ),
    },
    {
      accessorKey: 'language',
      header: 'Language',
    },
    {
      accessorKey: 'source_type',
      header: 'Source',
      cell: ({ row }) => (
        <Badge variant={row.original.source_type === 'file' ? 'default' : 'secondary'}>
          {row.original.source_type === 'file' ? 'File' : 'TTS'}
        </Badge>
      ),
    },
    {
      id: 'audio',
      header: 'Audio',
      cell: ({ row }) => {
        const p = row.original
        if (!p.has_audio) return <span className="text-xs text-muted-foreground">No audio</span>
        return (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => togglePlay(p)}
            className="h-7 w-7 p-0"
          >
            {playingId === p.id ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}
          </Button>
        )
      },
    },
    {
      accessorKey: 'file_size',
      header: 'Size',
      cell: ({ row }) => row.original.file_size ? formatBytes(row.original.file_size) : '-',
    },
    {
      id: 'system',
      header: 'System',
      cell: ({ row }) =>
        row.original.is_system ? (
          <Badge variant="outline" className="bg-blue-50 dark:bg-blue-950">System</Badge>
        ) : null,
    },
    {
      id: 'actions',
      cell: ({ row }) => {
        const p = row.original
        return (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" className="h-8 w-8 p-0">
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => openEdit(p)}>
                <Pencil className="mr-2 h-4 w-4" /> Edit
              </DropdownMenuItem>
              {p.source_type === 'tts' && p.tts_text && (
                <DropdownMenuItem
                  onClick={() => synthesizeMutation.mutate(p.id)}
                  disabled={synthesizeMutation.isPending}
                >
                  <Volume2 className="mr-2 h-4 w-4" />
                  {synthesizeMutation.isPending ? 'Synthesizing...' : 'Synthesize Audio'}
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => setUploadTarget(p)}>
                <Upload className="mr-2 h-4 w-4" /> Upload Audio
              </DropdownMenuItem>
              {!p.is_system && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem
                    className="text-red-600"
                    onClick={() => {
                      if (window.confirm(`Delete prompt "${p.name}"?`)) {
                        deleteMutation.mutate(p.id)
                      }
                    }}
                  >
                    <Trash2 className="mr-2 h-4 w-4" /> Delete
                  </DropdownMenuItem>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )
      },
    },
  ]

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Audio Prompts</h1>
          <p className="text-sm text-muted-foreground">
            Manage pre-recorded and TTS-generated audio prompts for IVR and system use.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Select value={categoryFilter} onValueChange={setCategoryFilter}>
            <SelectTrigger className="w-[150px]">
              <SelectValue placeholder="All Categories" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All Categories</SelectItem>
              {CATEGORIES.map((cat) => (
                <SelectItem key={cat} value={cat}>
                  {cat.charAt(0).toUpperCase() + cat.slice(1)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button onClick={() => { resetForm(); setCreateError(null); setCreateOpen(true) }}>
            <Plus className="mr-2 h-4 w-4" /> New Prompt
          </Button>
        </div>
      </div>

      <DataTable columns={columns} data={items} searchKey="name" />

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Prompt</DialogTitle>
            <DialogDescription>Add a new audio prompt for IVR or system use.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label htmlFor="name">Name</Label>
              <Input id="name" value={formName} onChange={(e) => setFormName(e.target.value)} placeholder="vm_enter_pin" />
            </div>
            <div>
              <Label htmlFor="description">Description</Label>
              <Input id="description" value={formDescription} onChange={(e) => setFormDescription(e.target.value)} placeholder="PIN entry prompt" />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>Category</Label>
                <Select value={formCategory} onValueChange={setFormCategory}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {CATEGORIES.map((cat) => (
                      <SelectItem key={cat} value={cat}>
                        {cat.charAt(0).toUpperCase() + cat.slice(1)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label htmlFor="language">Language</Label>
                <Input id="language" value={formLanguage} onChange={(e) => setFormLanguage(e.target.value)} placeholder="en-US" />
              </div>
            </div>
            <div>
              <Label>Source Type</Label>
              <Select value={formSourceType} onValueChange={setFormSourceType}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="tts">TTS (Text-to-Speech)</SelectItem>
                  <SelectItem value="file">File (Upload Audio)</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {formSourceType === 'tts' && (
              <div>
                <Label htmlFor="tts_text">TTS Text</Label>
                <textarea
                  id="tts_text"
                  value={formTtsText}
                  onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setFormTtsText(e.target.value)}
                  placeholder="Enter your PIN followed by pound."
                  rows={3}
                  className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                />
              </div>
            )}
            {createError && (
              <p className="text-sm text-red-600 dark:text-red-400">{createError}</p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => { setCreateOpen(false); setCreateError(null) }}>Cancel</Button>
              <Button
                onClick={handleCreate}
                disabled={!formName || (formSourceType === 'tts' && !formTtsText) || createMutation.isPending}
              >
                {createMutation.isPending ? 'Creating...' : 'Create'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editTarget} onOpenChange={() => setEditTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Prompt</DialogTitle>
            <DialogDescription>
              Update prompt "{editTarget?.name}".
              {editTarget?.source_type === 'tts' && ' Changing TTS text will regenerate audio automatically.'}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label htmlFor="edit-description">Description</Label>
              <Input id="edit-description" value={editDescription} onChange={(e) => setEditDescription(e.target.value)} />
            </div>
            <div>
              <Label>Category</Label>
              <Select value={editCategory} onValueChange={setEditCategory}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {CATEGORIES.map((cat) => (
                    <SelectItem key={cat} value={cat}>
                      {cat.charAt(0).toUpperCase() + cat.slice(1)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {editTarget?.source_type === 'tts' && (
              <>
                <div>
                  <Label htmlFor="edit-tts-text">TTS Text</Label>
                  <textarea
                    id="edit-tts-text"
                    value={editTtsText}
                    onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setEditTtsText(e.target.value)}
                    rows={3}
                    className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  />
                </div>
                <div>
                  <Label htmlFor="edit-tts-voice">TTS Voice (optional)</Label>
                  <Input id="edit-tts-voice" value={editTtsVoice} onChange={(e) => setEditTtsVoice(e.target.value)} placeholder="af_heart" />
                </div>
              </>
            )}
            {editError && (
              <p className="text-sm text-red-600 dark:text-red-400">{editError}</p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => { setEditTarget(null); setEditError(null) }}>Cancel</Button>
              <Button
                onClick={handleUpdate}
                disabled={updateMutation.isPending}
              >
                {updateMutation.isPending ? 'Saving...' : 'Save'}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* Upload Audio Dialog */}
      <Dialog open={!!uploadTarget} onOpenChange={() => setUploadTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Upload Audio</DialogTitle>
            <DialogDescription>
              Upload a WAV audio file for prompt "{uploadTarget?.name}".
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <input
              type="file"
              ref={fileInputRef}
              accept="audio/wav,audio/x-wav,.wav"
              className="hidden"
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                const file = e.target.files?.[0]
                if (file) handleUpload(file)
              }}
            />
            <div
              className="flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed p-8 cursor-pointer hover:border-primary transition-colors"
              onClick={() => fileInputRef.current?.click()}
            >
              <Upload className="h-8 w-8 text-muted-foreground" />
              <p className="text-sm text-muted-foreground">Click to select a WAV file</p>
              {uploadMutation.isPending && <p className="text-sm text-primary">Uploading...</p>}
            </div>
            <div className="flex justify-end">
              <Button variant="outline" onClick={() => setUploadTarget(null)}>Cancel</Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
