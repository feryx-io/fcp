import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Eye, Library, Pencil, Plus, Trash2 } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { configTemplates } from '@/api/endpoints'
import type { ConfigTemplate } from '@/types'

const VENDORS = ['yealink', 'polycom', 'grandstream', 'cisco', 'fanvil', 'snom', 'generic']

interface Draft {
  id?: string
  name: string
  vendor: string
  model_pattern: string
  description: string
  template_body: string
}

const EMPTY: Draft = { name: '', vendor: 'yealink', model_pattern: '', description: '', template_body: '' }

/** Phone templates (docs/PROVISIONING.md §5): Jinja bodies, picked by vendor
 * and model when a device names none. */
export function TemplatesPage() {
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState<Draft | null>(null)
  const [preview, setPreview] = useState<{ content?: string; error?: string } | null>(null)
  const { data: templates = [], isLoading } = useQuery({ queryKey: ['config-templates'], queryFn: configTemplates.list })
  const { data: library = [] } = useQuery({ queryKey: ['config-template-library'], queryFn: configTemplates.library })
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['config-templates'] })

  const save = useMutation({
    mutationFn: (d: Draft) => {
      const body = {
        name: d.name.trim(),
        vendor: d.vendor,
        model_pattern: d.model_pattern.trim(),
        description: d.description.trim() || undefined,
        template_body: d.template_body,
      }
      return d.id ? configTemplates.update(d.id, body) : configTemplates.create(body)
    },
    onSuccess: () => {
      refresh()
      setDraft(null)
      setPreview(null)
    },
  })
  const fromLibrary = useMutation({ mutationFn: configTemplates.fromLibrary, onSuccess: refresh })
  const remove = useMutation({ mutationFn: configTemplates.delete, onSuccess: refresh })
  const tryIt = useMutation({
    mutationFn: (d: Draft) => configTemplates.preview(d.template_body, d.vendor),
    onSuccess: (res) => setPreview({ content: res.content }),
    onError: (e) => setPreview({ error: (e as Error).message }),
  })

  const edit = (t: ConfigTemplate) => {
    setPreview(null)
    setDraft({
      id: t.id,
      name: t.name,
      vendor: t.vendor,
      model_pattern: t.model_pattern ?? '',
      description: t.description ?? '',
      template_body: t.template_body,
    })
  }
  const have = new Set(templates.map((t) => t.name))

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Phone templates</h1>
          <p className="text-muted-foreground">
            What a phone is given when it fetches its file. A device that names none gets its vendor's template whose model pattern matches it.
          </p>
        </div>
        <Button onClick={() => { setPreview(null); setDraft({ ...EMPTY }) }}>
          <Plus className="mr-2 h-4 w-4" /> New template
        </Button>
      </div>

      <Card>
        <CardContent className="space-y-2 p-4">
          <p className="text-sm font-medium">Starters</p>
          <div className="flex flex-wrap gap-2">
            {library.map((l) => (
              <Button
                key={l.name}
                variant="outline"
                size="sm"
                disabled={have.has(l.name) || fromLibrary.isPending}
                title={l.description}
                onClick={() => fromLibrary.mutate(l.vendor)}
              >
                <Library className="mr-2 h-4 w-4" /> {l.vendor}
                {l.model_pattern && <span className="ml-1 text-muted-foreground">{l.model_pattern}</span>}
              </Button>
            ))}
          </div>
          {fromLibrary.isError && <p className="text-sm text-destructive">{(fromLibrary.error as Error).message}</p>}
        </CardContent>
      </Card>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Vendor</TableHead>
            <TableHead>Models</TableHead>
            <TableHead>Description</TableHead>
            <TableHead />
          </TableRow>
        </TableHeader>
        <TableBody>
          {isLoading && <TableRow><TableCell colSpan={5}>Loading…</TableCell></TableRow>}
          {!isLoading && templates.length === 0 && (
            <TableRow><TableCell colSpan={5} className="text-muted-foreground">No templates yet: start from a starter above.</TableCell></TableRow>
          )}
          {templates.map((t) => (
            <TableRow key={t.id}>
              <TableCell className="font-medium">{t.name}</TableCell>
              <TableCell><Badge variant="outline">{t.vendor}</Badge></TableCell>
              <TableCell className="font-mono text-xs">{t.model_pattern || 'any'}</TableCell>
              <TableCell className="text-sm text-muted-foreground">{t.description}</TableCell>
              <TableCell className="whitespace-nowrap text-right">
                <Button variant="ghost" size="icon" aria-label="Edit" onClick={() => edit(t)}><Pencil className="h-4 w-4" /></Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label="Delete"
                  onClick={() => { if (confirm(`Delete the template "${t.name}"?`)) remove.mutate(t.id) }}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>

      <Dialog open={!!draft} onOpenChange={(open) => { if (!open) { setDraft(null); setPreview(null) } }}>
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle>{draft?.id ? `Edit ${draft.name}` : 'New template'}</DialogTitle>
            <DialogDescription>
              Jinja: <span className="font-mono">{'{{ sip_username }}'}</span>,{' '}
              <span className="font-mono">{'{% for account in accounts %}'}</span>,{' '}
              <span className="font-mono">{'{% for key in line_keys %}'}</span>; an older{' '}
              <span className="font-mono">{'${name}'}</span> still works. Values are escaped for XML vendors.
            </DialogDescription>
          </DialogHeader>
          {draft && (
            <div className="space-y-3">
              <div className="grid grid-cols-3 gap-3">
                <div className="space-y-1">
                  <Label htmlFor="t-name">Name</Label>
                  <Input id="t-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
                </div>
                <div className="space-y-1">
                  <Label>Vendor</Label>
                  <Select value={draft.vendor} onValueChange={(v) => setDraft({ ...draft, vendor: v })}>
                    <SelectTrigger><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {VENDORS.map((v) => <SelectItem key={v} value={v}>{v}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label htmlFor="t-pattern">Model pattern</Label>
                  <Input
                    id="t-pattern"
                    placeholder="T5*, VVX ?50 — empty for any"
                    value={draft.model_pattern}
                    onChange={(e) => setDraft({ ...draft, model_pattern: e.target.value })}
                  />
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="t-desc">Description</Label>
                <Input id="t-desc" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="t-body">Body</Label>
                <textarea
                  id="t-body"
                  rows={14}
                  spellCheck={false}
                  value={draft.template_body}
                  onChange={(e) => setDraft({ ...draft, template_body: e.target.value })}
                  className="flex w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                />
              </div>
              {preview?.content !== undefined && (
                <div className="space-y-1">
                  <p className="text-sm font-medium">With sample values</p>
                  <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 font-mono text-xs">{preview.content}</pre>
                </div>
              )}
              {preview?.error && <p className="text-sm text-destructive">{preview.error}</p>}
              {save.isError && <p className="text-sm text-destructive">{(save.error as Error).message}</p>}
              <div className="flex justify-end gap-2">
                <Button variant="outline" disabled={tryIt.isPending} onClick={() => tryIt.mutate(draft)}>
                  <Eye className="mr-2 h-4 w-4" /> Try with sample values
                </Button>
                <Button disabled={save.isPending || !draft.name.trim()} onClick={() => save.mutate(draft)}>Save</Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
