import { useState, useRef, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { MoreHorizontal, Plus, ArrowUpDown, Pencil, Trash2, X } from 'lucide-react'
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
import { extensions, users } from '@/api/endpoints'
import type { Extension, User } from '@/types'

const createSchema = z.object({
  number: z.string().min(1, 'Extension number is required').regex(/^\d+$/, 'Must be numeric'),
  display_name: z.string().optional(),
  user_id: z.string().optional(),
})

const editSchema = z.object({
  display_name: z.string().optional(),
  user_id: z.string().optional(),
  type: z.string().optional(),
})

type CreateForm = z.infer<typeof createSchema>
type EditForm = z.infer<typeof editSchema>

export function ExtensionsPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [editItem, setEditItem] = useState<Extension | null>(null)
  const queryClient = useQueryClient()

  const { data: items = [], isLoading } = useQuery({
    queryKey: ['extensions'],
    queryFn: extensions.list,
  })

  const createMutation = useMutation({
    mutationFn: extensions.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensions'] })
      setCreateOpen(false)
    },
  })

  const updateMutation = useMutation({
    mutationFn: ({ number, data }: { number: string; data: Partial<EditForm> }) =>
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
      accessorKey: 'display_name',
      header: 'Display Name',
      cell: ({ row }) => row.original.display_name || '-',
    },
    {
      accessorKey: 'type',
      header: 'Type',
      cell: ({ row }) => <Badge variant="secondary">{row.original.type}</Badge>,
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
      accessorKey: 'user_id',
      header: 'User',
      cell: ({ row }) => row.original.user_id || '-',
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
      display_name: editItem.display_name || '',
      user_id: editItem.user_id || '',
      type: editItem.type || '',
    })
  }

  const onCreateSubmit = (data: CreateForm) => {
    createMutation.mutate({
      ...data,
      display_name: data.display_name || undefined,
      user_id: data.user_id || undefined,
    })
  }

  const onEditSubmit = (data: EditForm) => {
    if (!editItem) return
    updateMutation.mutate({
      number: editItem.number,
      data: {
        display_name: data.display_name || undefined,
        user_id: data.user_id || undefined,
        type: data.type || undefined,
      },
    })
  }

  const { data: userList = [] } = useQuery({
    queryKey: ['users'],
    queryFn: users.list,
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Extensions</h1>
          <p className="text-muted-foreground">Manage telephone extensions</p>
        </div>
        <Button onClick={() => { createForm.reset(); setCreateOpen(true) }}>
          <Plus className="mr-2 h-4 w-4" /> Add Extension
        </Button>
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
              <Label htmlFor="c-display_name">Display Name</Label>
              <Input id="c-display_name" {...createForm.register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label>User</Label>
              <UserAutocomplete
                users={userList}
                value={createForm.watch('user_id') || ''}
                onChange={(v) => createForm.setValue('user_id', v)}
                placeholder="Search by username or name..."
              />
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
              <Label htmlFor="e-display_name">Display Name</Label>
              <Input id="e-display_name" {...editForm.register('display_name')} />
            </div>
            <div className="space-y-2">
              <Label>User</Label>
              <UserAutocomplete
                users={userList}
                value={editForm.watch('user_id') || ''}
                onChange={(v) => editForm.setValue('user_id', v)}
                placeholder="Search by username or name..."
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="e-type">Type</Label>
              <Input id="e-type" {...editForm.register('type')} />
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
    </div>
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
  const [inputValue, setInputValue] = useState(value)
  const [isOpen, setIsOpen] = useState(false)
  const [focusedIndex, setFocusedIndex] = useState(-1)
  const wrapperRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLUListElement>(null)

  // Sync inputValue when form value changes externally (e.g. on form reset)
  useEffect(() => {
    setInputValue(value)
  }, [value])

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
  const filtered = query
    ? users.filter(
        (u) =>
          u.username.toLowerCase().includes(query) ||
          (u.display_name && u.display_name.toLowerCase().includes(query)) ||
          (u.email && u.email.toLowerCase().includes(query))
      ).slice(0, 8)
    : users.slice(0, 8)

  const selectUser = (username: string) => {
    setInputValue(username)
    onChange(username)
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
          selectUser(filtered[focusedIndex].username)
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
            onChange(v)
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
              key={u.username}
              className={`flex cursor-pointer items-center gap-2 px-3 py-2 text-sm hover:bg-accent ${
                idx === focusedIndex ? 'bg-accent' : ''
              }`}
              onMouseDown={(e) => { e.preventDefault(); selectUser(u.username) }}
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
