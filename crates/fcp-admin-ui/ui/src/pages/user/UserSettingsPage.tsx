import { useEffect, useRef, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Moon, Sun, Monitor, Camera, Trash2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'
import { me } from '@/api/endpoints'
import type { MeProfile } from '@/types'

function getInitials(profile: MeProfile | null, username?: string): string {
  if (profile?.display_name) {
    return profile.display_name
      .split(' ')
      .map((w) => w[0])
      .slice(0, 2)
      .join('')
      .toUpperCase()
  }
  return (profile?.username || username || '?').slice(0, 2).toUpperCase()
}

export function UserSettingsPage() {
  const { user } = useAuthStore()
  const { theme, setTheme } = useThemeStore()

  const [profile, setProfile] = useState<MeProfile | null>(null)
  const [loading, setLoading] = useState(true)

  // Profile form
  const [displayName, setDisplayName] = useState('')
  const [email, setEmail] = useState('')
  const [phone, setPhone] = useState('')
  const [timezone, setTimezone] = useState('')
  const [language, setLanguage] = useState('')
  const [profileSaving, setProfileSaving] = useState(false)
  const [profileMsg, setProfileMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null)

  // Avatar
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [avatarUrl, setAvatarUrl] = useState<string | null>(null)
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null)
  const [avatarUploading, setAvatarUploading] = useState(false)

  // Password form
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [passwordSaving, setPasswordSaving] = useState(false)
  const [passwordMsg, setPasswordMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null)

  useEffect(() => {
    async function load() {
      try {
        const p = await me.getProfile()
        setProfile(p)
        setDisplayName(p.display_name || '')
        setEmail(p.email || '')
        setPhone(p.phone_number || '')
        setTimezone(p.timezone)
        setLanguage(p.language)
        if (p.has_profile_picture) {
          try {
            const url = await me.fetchAvatar()
            setAvatarUrl(url)
          } catch {
            // Avatar fetch failed, show initials
          }
        }
      } catch {
        // Profile might not be available
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const handleAvatarSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return

    // Validate type
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) {
      setProfileMsg({ type: 'error', text: 'Please select a JPEG, PNG, or WebP image.' })
      return
    }

    // Validate size (2MB)
    if (file.size > 2 * 1024 * 1024) {
      setProfileMsg({ type: 'error', text: 'Image must be under 2 MB.' })
      return
    }

    // Show preview
    const previewUrl = URL.createObjectURL(file)
    setAvatarPreview(previewUrl)

    // Upload immediately
    handleAvatarUpload(file)
  }

  const handleAvatarUpload = async (file: File) => {
    setAvatarUploading(true)
    setProfileMsg(null)
    try {
      await me.uploadAvatar(file)
      const url = await me.fetchAvatar()
      setAvatarUrl(url)
      setAvatarPreview(null)
      setProfile((prev) => prev ? { ...prev, has_profile_picture: true } : prev)
      setProfileMsg({ type: 'success', text: 'Profile picture updated.' })
    } catch (err) {
      setAvatarPreview(null)
      setProfileMsg({ type: 'error', text: err instanceof Error ? err.message : 'Failed to upload picture' })
    } finally {
      setAvatarUploading(false)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  const handleAvatarRemove = async () => {
    setAvatarUploading(true)
    setProfileMsg(null)
    try {
      await me.deleteAvatar()
      if (avatarUrl) URL.revokeObjectURL(avatarUrl)
      setAvatarUrl(null)
      setAvatarPreview(null)
      setProfile((prev) => prev ? { ...prev, has_profile_picture: false } : prev)
      setProfileMsg({ type: 'success', text: 'Profile picture removed.' })
    } catch (err) {
      setProfileMsg({ type: 'error', text: err instanceof Error ? err.message : 'Failed to remove picture' })
    } finally {
      setAvatarUploading(false)
    }
  }

  const handleProfileSave = async (e: React.FormEvent) => {
    e.preventDefault()
    setProfileSaving(true)
    setProfileMsg(null)
    try {
      const updated = await me.updateProfile({
        display_name: displayName || undefined,
        email: email || undefined,
        phone_number: phone || undefined,
        timezone: timezone || undefined,
        language: language || undefined,
      })
      setProfile(updated)
      setProfileMsg({ type: 'success', text: 'Profile updated successfully.' })
    } catch (err) {
      setProfileMsg({ type: 'error', text: err instanceof Error ? err.message : 'Failed to update profile' })
    } finally {
      setProfileSaving(false)
    }
  }

  const handlePasswordChange = async (e: React.FormEvent) => {
    e.preventDefault()
    setPasswordMsg(null)

    if (newPassword !== confirmPassword) {
      setPasswordMsg({ type: 'error', text: 'New passwords do not match.' })
      return
    }
    if (newPassword.length < 8) {
      setPasswordMsg({ type: 'error', text: 'Password must be at least 8 characters.' })
      return
    }

    setPasswordSaving(true)
    try {
      await me.changePassword({ current_password: currentPassword, new_password: newPassword })
      setPasswordMsg({ type: 'success', text: 'Password changed successfully.' })
      setCurrentPassword('')
      setNewPassword('')
      setConfirmPassword('')
    } catch (err) {
      setPasswordMsg({ type: 'error', text: err instanceof Error ? err.message : 'Failed to change password' })
    } finally {
      setPasswordSaving(false)
    }
  }

  const displayedAvatar = avatarPreview || avatarUrl

  if (loading) {
    return (
      <div className="flex h-32 items-center justify-center text-muted-foreground">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-muted-foreground">Manage your account preferences.</p>
      </div>

      {/* Profile */}
      <Card>
        <CardHeader>
          <CardTitle>Profile</CardTitle>
          <CardDescription>Update your display name and contact information.</CardDescription>
        </CardHeader>
        <CardContent>
          {/* Avatar */}
          <div className="mb-6 flex items-center gap-4">
            <div className="relative">
              {displayedAvatar ? (
                <img
                  src={displayedAvatar}
                  alt="Profile picture"
                  className="h-20 w-20 rounded-full object-cover border-2 border-muted"
                />
              ) : (
                <div className="flex h-20 w-20 items-center justify-center rounded-full bg-primary/10 text-primary text-xl font-semibold border-2 border-muted">
                  {getInitials(profile, user?.username)}
                </div>
              )}
              {avatarUploading && (
                <div className="absolute inset-0 flex items-center justify-center rounded-full bg-black/40">
                  <div className="h-5 w-5 animate-spin rounded-full border-2 border-white border-t-transparent" />
                </div>
              )}
            </div>
            <div className="flex flex-col gap-2">
              <input
                ref={fileInputRef}
                type="file"
                accept="image/jpeg,image/png,image/webp"
                className="hidden"
                onChange={handleAvatarSelect}
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="gap-2"
                disabled={avatarUploading}
                onClick={() => fileInputRef.current?.click()}
              >
                <Camera className="h-4 w-4" />
                Upload Photo
              </Button>
              {profile?.has_profile_picture && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="gap-2 text-destructive hover:text-destructive"
                  disabled={avatarUploading}
                  onClick={handleAvatarRemove}
                >
                  <Trash2 className="h-4 w-4" />
                  Remove
                </Button>
              )}
            </div>
          </div>

          <form onSubmit={handleProfileSave} className="space-y-4">
            {profileMsg && (
              <div className={cn(
                'rounded-md p-3 text-sm',
                profileMsg.type === 'success' ? 'bg-green-500/15 text-green-700 dark:text-green-400' : 'bg-destructive/15 text-destructive',
              )}>
                {profileMsg.text}
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="displayName">Display Name</Label>
                <Input id="displayName" value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="email">Email</Label>
                <Input id="email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="phone">Phone Number</Label>
                <Input id="phone" value={phone} onChange={(e) => setPhone(e.target.value)} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="timezone">Timezone</Label>
                <Input id="timezone" value={timezone} onChange={(e) => setTimezone(e.target.value)} placeholder="America/New_York" />
              </div>
              <div className="space-y-2">
                <Label htmlFor="language">Language</Label>
                <Input id="language" value={language} onChange={(e) => setLanguage(e.target.value)} placeholder="en-US" />
              </div>
            </div>
            <Button type="submit" disabled={profileSaving}>
              {profileSaving ? 'Saving...' : 'Save Profile'}
            </Button>
          </form>
        </CardContent>
      </Card>

      {/* Password */}
      <Card>
        <CardHeader>
          <CardTitle>Change Password</CardTitle>
          <CardDescription>Update your login password.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handlePasswordChange} className="space-y-4">
            {passwordMsg && (
              <div className={cn(
                'rounded-md p-3 text-sm',
                passwordMsg.type === 'success' ? 'bg-green-500/15 text-green-700 dark:text-green-400' : 'bg-destructive/15 text-destructive',
              )}>
                {passwordMsg.text}
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-3">
              <div className="space-y-2">
                <Label htmlFor="currentPassword">Current Password</Label>
                <Input id="currentPassword" type="password" value={currentPassword} onChange={(e) => setCurrentPassword(e.target.value)} required />
              </div>
              <div className="space-y-2">
                <Label htmlFor="newPassword">New Password</Label>
                <Input id="newPassword" type="password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} required />
              </div>
              <div className="space-y-2">
                <Label htmlFor="confirmPassword">Confirm New Password</Label>
                <Input id="confirmPassword" type="password" value={confirmPassword} onChange={(e) => setConfirmPassword(e.target.value)} required />
              </div>
            </div>
            <Button type="submit" disabled={passwordSaving}>
              {passwordSaving ? 'Changing...' : 'Change Password'}
            </Button>
          </form>
        </CardContent>
      </Card>

      {/* Appearance */}
      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
          <CardDescription>Choose your preferred theme.</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex gap-2">
            {(['light', 'dark', 'system'] as const).map((t) => {
              const Icon = t === 'light' ? Sun : t === 'dark' ? Moon : Monitor
              return (
                <Button
                  key={t}
                  variant={theme === t ? 'default' : 'outline'}
                  className="gap-2"
                  onClick={() => setTheme(t)}
                >
                  <Icon className="h-4 w-4" />
                  {t.charAt(0).toUpperCase() + t.slice(1)}
                </Button>
              )
            })}
          </div>
        </CardContent>
      </Card>

      {/* Account Info (read-only) */}
      <Card>
        <CardHeader>
          <CardTitle>Account Information</CardTitle>
          <CardDescription>Your account details (read-only).</CardDescription>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-3 sm:grid-cols-2 text-sm">
            <div>
              <dt className="text-muted-foreground">Username</dt>
              <dd className="font-medium">{profile?.username || user?.username}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Role</dt>
              <dd className="font-medium">{profile?.role || user?.role}</dd>
            </div>
            {profile?.extension && (
              <div>
                <dt className="text-muted-foreground">Extension</dt>
                <dd className="font-medium">{profile.extension}</dd>
              </div>
            )}
            {profile?.department && (
              <div>
                <dt className="text-muted-foreground">Department</dt>
                <dd className="font-medium">{profile.department}</dd>
              </div>
            )}
            {profile?.job_title && (
              <div>
                <dt className="text-muted-foreground">Job Title</dt>
                <dd className="font-medium">{profile.job_title}</dd>
              </div>
            )}
            {profile?.location && (
              <div>
                <dt className="text-muted-foreground">Location</dt>
                <dd className="font-medium">{profile.location}</dd>
              </div>
            )}
          </dl>
        </CardContent>
      </Card>
    </div>
  )
}
