import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Plus, Trash2, Pencil, Stethoscope } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { identityProviders } from '@/api/endpoints'
import type { IdentityProvider, IdentityProviderRequest } from '@/types'

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Request failed'
}

const ROLES = ['tenant_admin', 'telecom_admin', 'user_admin', 'department_admin', 'read_only', 'end_user']

interface Form {
  name: string
  enabled: boolean
  issuer: string
  client_id: string
  client_secret: string
  scopes: string
  username_claim: string
  email_claim: string
  name_claim: string
  groups_claim: string
  group_roles: string
  jit: boolean
  link_by_email: boolean
  email_verified_claim: string
  required_claims: string
}

const blank: Form = {
  name: '', enabled: true, issuer: '', client_id: '', client_secret: '', scopes: 'profile email',
  username_claim: 'preferred_username', email_claim: 'email', name_claim: 'name', groups_claim: 'groups',
  group_roles: '', jit: false, link_by_email: true, email_verified_claim: 'email_verified', required_claims: '',
}

function formOf(p: IdentityProvider): Form {
  const s = p.settings
  return {
    name: p.name, enabled: p.enabled, issuer: s.issuer, client_id: s.client_id, client_secret: '',
    scopes: (s.scopes ?? []).join(' '),
    username_claim: s.claims?.username ?? 'preferred_username', email_claim: s.claims?.email ?? 'email',
    name_claim: s.claims?.display_name ?? 'name', groups_claim: s.claims?.groups ?? 'groups',
    group_roles: (s.group_roles ?? []).map((g) => `${g.group} ${g.role}`).join('\n'),
    jit: s.jit ?? false, link_by_email: s.link_by_email ?? true,
    email_verified_claim: s.email_verified_claim ?? 'email_verified',
    required_claims: (s.required_claims ?? []).map((r) => `${r.claim} ${r.value}`).join('\n'),
  }
}

/** "claim value" per line. */
function parseRequiredClaims(text: string) {
  return text.split('\n').map((l) => l.trim()).filter(Boolean).map((line) => {
    const at = line.indexOf(' ')
    if (at < 0) throw new Error(`"${line}": write the claim, a space, then the value`)
    return { claim: line.slice(0, at), value: line.slice(at + 1).trim() }
  })
}

/** "group role" per line; a group with spaces keeps them, the role is the last word. */
function parseGroupRoles(text: string) {
  return text.split('\n').map((l) => l.trim()).filter(Boolean).map((line) => {
    const at = line.lastIndexOf(' ')
    if (at < 0) throw new Error(`"${line}": write the group, a space, then the role`)
    const role = line.slice(at + 1)
    if (!ROLES.includes(role)) throw new Error(`"${role}" is not a role (${ROLES.join(', ')})`)
    return { group: line.slice(0, at).trim(), role }
  })
}

function requestOf(f: Form, editing: boolean): IdentityProviderRequest {
  return {
    name: f.name.trim(),
    enabled: f.enabled,
    settings: {
      issuer: f.issuer.trim(),
      client_id: f.client_id.trim(),
      scopes: f.scopes.split(/\s+/).filter(Boolean),
      claims: { username: f.username_claim, email: f.email_claim, display_name: f.name_claim, groups: f.groups_claim },
      group_roles: parseGroupRoles(f.group_roles),
      jit: f.jit,
      link_by_email: f.link_by_email,
      email_verified_claim: f.email_verified_claim.trim() || 'email_verified',
      required_claims: parseRequiredClaims(f.required_claims),
    },
    // Editing, a blank secret keeps the one the provider has.
    client_secret: f.client_secret ? f.client_secret : editing ? undefined : null,
  }
}

/**
 * The tenant's identity providers (docs/IDENTITY_AND_DEVICES.md phase 3):
 * OpenID Connect sign-in — Entra ID, Okta, Google Workspace or any other.
 * Register the redirect URI shown with the provider, then its issuer,
 * client id and secret here. The secret is kept sealed and never shown.
 */
export function IdentityProvidersPage() {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<IdentityProvider | 'new' | null>(null)
  const [form, setForm] = useState<Form>(blank)
  const [formError, setFormError] = useState<string | null>(null)
  const [checks, setChecks] = useState<Record<string, string>>({})
  const { data: list = [], isLoading } = useQuery({ queryKey: ['identity-providers'], queryFn: identityProviders.list })
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['identity-providers'] })

  const save = useMutation({
    mutationFn: async () => {
      const req = requestOf(form, editing !== 'new')
      if (editing === 'new') await identityProviders.create(req)
      else if (editing) await identityProviders.update(editing.id, req)
    },
    onSuccess: () => { invalidate(); setEditing(null) },
    onError: (e) => setFormError(errorText(e)),
  })
  const remove = useMutation({
    mutationFn: (id: string) => identityProviders.delete(id),
    onSuccess: invalidate,
    onError: (e) => alert(errorText(e)),
  })
  const check = async (p: IdentityProvider) => {
    try {
      const r = await identityProviders.check(p.id)
      setChecks((c) => ({ ...c, [p.id]: r.ok ? 'Reached the provider' : `Failed: ${r.error}` }))
    } catch (e) {
      setChecks((c) => ({ ...c, [p.id]: `Failed: ${errorText(e)}` }))
    }
  }
  const open = (p: IdentityProvider | 'new') => {
    setForm(p === 'new' ? blank : formOf(p))
    setFormError(null)
    setEditing(p)
  }
  const field = (key: keyof Form) => ({
    value: form[key] as string,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setForm({ ...form, [key]: e.target.value }),
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Single sign-on</h1>
          <p className="text-muted-foreground">
            People sign in through your identity provider. A provider's groups can set their roles, and it can create
            them at their first sign-in.
          </p>
        </div>
        <Button onClick={() => open('new')}><Plus className="mr-2 h-4 w-4" />Add provider</Button>
      </div>

      {isLoading && <p className="text-muted-foreground">Loading…</p>}
      {!isLoading && list.length === 0 && (
        <Card><CardContent className="py-8 text-center text-muted-foreground">No identity providers yet.</CardContent></Card>
      )}
      {list.map((p) => (
        <Card key={p.id}>
          <CardHeader className="flex flex-row items-start justify-between space-y-0">
            <div>
              <CardTitle className="flex items-center gap-2">
                {p.name}
                <Badge variant={p.enabled ? 'default' : 'secondary'}>{p.enabled ? 'Enabled' : 'Disabled'}</Badge>
                {p.settings.jit && <Badge variant="outline">Creates users</Badge>}
              </CardTitle>
              <CardDescription className="break-all font-mono text-xs">{p.settings.issuer}</CardDescription>
            </div>
            <div className="flex gap-1">
              <Button variant="ghost" size="icon" title="Check" onClick={() => void check(p)}><Stethoscope className="h-4 w-4" /></Button>
              <Button variant="ghost" size="icon" title="Edit" onClick={() => open(p)}><Pencil className="h-4 w-4" /></Button>
              <Button
                variant="ghost" size="icon" title="Delete"
                onClick={() => { if (confirm(`Delete ${p.name}? Its users' links to it go too.`)) remove.mutate(p.id) }}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </CardHeader>
          <CardContent className="space-y-1 text-sm">
            <div>Client id: <span className="font-mono">{p.settings.client_id}</span>{p.has_client_secret ? ' · secret set' : ' · public client'}</div>
            {p.redirect_uri
              ? <div>Redirect URI to register: <span className="break-all font-mono">{p.redirect_uri}</span></div>
              : <div className="text-amber-600">Single sign-on is off on this server: set [api.sso] public_url.</div>}
            {p.backchannel_logout_uri && (
              <div>Back-channel logout URI: <span className="break-all font-mono">{p.backchannel_logout_uri}</span></div>
            )}
            {(p.settings.required_claims ?? []).length > 0 && (
              <div>Only tokens with: {(p.settings.required_claims ?? []).map((r) => `${r.claim} = ${r.value}`).join(', ')}</div>
            )}
            {(p.settings.group_roles ?? []).length > 0 && (
              <div>Roles by group: {(p.settings.group_roles ?? []).map((g) => `${g.group} → ${g.role}`).join(', ')}</div>
            )}
            {checks[p.id] && <div className="text-muted-foreground">{checks[p.id]}</div>}
          </CardContent>
        </Card>
      ))}

      <Dialog open={editing !== null} onOpenChange={(o) => !o && setEditing(null)}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing === 'new' ? 'Add identity provider' : 'Edit identity provider'}</DialogTitle>
            <DialogDescription>OpenID Connect: the issuer, and the client you registered there.</DialogDescription>
          </DialogHeader>
          <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); setFormError(null); save.mutate() }}>
            {formError && <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">{formError}</div>}
            <div className="space-y-1"><Label htmlFor="idp-name">Name (the login button says "Continue with …")</Label><Input id="idp-name" required {...field('name')} /></div>
            <div className="space-y-1"><Label htmlFor="idp-issuer">Issuer</Label><Input id="idp-issuer" required placeholder="https://login.microsoftonline.com/<tenant>/v2.0" {...field('issuer')} /></div>
            <div className="space-y-1"><Label htmlFor="idp-client">Client id</Label><Input id="idp-client" required {...field('client_id')} /></div>
            <div className="space-y-1">
              <Label htmlFor="idp-secret">Client secret</Label>
              <Input id="idp-secret" type="password" autoComplete="off" placeholder={editing !== 'new' && editing?.has_client_secret ? 'Unchanged' : ''} {...field('client_secret')} />
            </div>
            <div className="space-y-1"><Label htmlFor="idp-scopes">Scopes besides openid</Label><Input id="idp-scopes" {...field('scopes')} /></div>
            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1"><Label htmlFor="idp-uc">Username claim</Label><Input id="idp-uc" {...field('username_claim')} /></div>
              <div className="space-y-1"><Label htmlFor="idp-ec">Email claim</Label><Input id="idp-ec" {...field('email_claim')} /></div>
              <div className="space-y-1"><Label htmlFor="idp-nc">Name claim</Label><Input id="idp-nc" {...field('name_claim')} /></div>
              <div className="space-y-1"><Label htmlFor="idp-gc">Groups claim</Label><Input id="idp-gc" {...field('groups_claim')} /></div>
            </div>
            <div className="space-y-1">
              <Label htmlFor="idp-roles">Roles by group</Label>
              <textarea
                id="idp-roles" rows={3} placeholder="fcp-admins tenant_admin"
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
                {...field('group_roles')}
              />
              <p className="text-xs text-muted-foreground">
                One per line: the group, then the role. With any set, a user's role follows their groups at every
                sign-in (the most privileged wins; none mapped is end_user).
              </p>
            </div>
            <div className="space-y-1">
              <Label htmlFor="idp-required">Required claims</Label>
              <textarea
                id="idp-required" rows={2} placeholder="hd acme.com"
                className="flex w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
                {...field('required_claims')}
              />
              <p className="text-xs text-muted-foreground">
                One per line: the claim, then the value every token must carry — Google's <code>hd</code> (your Workspace
                domain), Entra's <code>tid</code> (your directory).
              </p>
            </div>
            <div className="space-y-1">
              <Label htmlFor="idp-ev">Verified-email claim</Label>
              <Input id="idp-ev" {...field('email_verified_claim')} />
              <p className="text-xs text-muted-foreground"><code>email_verified</code>; <code>xms_edov</code> for Entra ID.</p>
            </div>
            <div className="flex items-center justify-between"><Label htmlFor="idp-jit">Create users at their first sign-in</Label><Switch id="idp-jit" checked={form.jit} onCheckedChange={(v) => setForm({ ...form, jit: v })} /></div>
            <div className="flex items-center justify-between"><Label htmlFor="idp-link">Link existing users by verified email</Label><Switch id="idp-link" checked={form.link_by_email} onCheckedChange={(v) => setForm({ ...form, link_by_email: v })} /></div>
            <div className="flex items-center justify-between"><Label htmlFor="idp-on">Enabled</Label><Switch id="idp-on" checked={form.enabled} onCheckedChange={(v) => setForm({ ...form, enabled: v })} /></div>
            <Button type="submit" className="w-full" disabled={save.isPending}>Save</Button>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
