// The Backups page (docs/SCHEDULED_BACKUPS.md B2): a platform administrator
// adds an SFTP destination, keeps the password made for it, asks for a test
// and a backup (which wait for fcp-backup: none runs under this test), edits
// it without its key, and deletes it. Encryption at rest is off for the
// admin under test, so the key card says nothing needs downloading.
import { test, expect, admin, signIn, settled } from './fixtures'

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a platform administrator sets up an off-site destination', async ({ page, api }) => {
  await signIn(page, admin())
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await nav.getByRole('link', { name: 'Backups' }).click()
  await expect(page).toHaveURL(/\/admin\/backups$/)
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Backups' })).toBeVisible()
  await expect(page.getByText('Encryption at rest is off')).toBeVisible()
  await expect(page.getByText('None yet: nothing is backed up off this machine.')).toBeVisible()

  await page.getByRole('button', { name: 'Add a destination' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Name').fill('Off site')
  await dialog.getByLabel('Host').fill('backup.example.com')
  await dialog.getByLabel('User').fill('fcp')
  await dialog.getByLabel('Directory').fill('/srv/fcp')
  await dialog.getByLabel('Private key').fill('-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n')
  await dialog.getByRole('button', { name: 'Save destination' }).click()

  // The password made for it, shown once.
  const kept = page.getByRole('dialog', { name: 'Keep the repository password' })
  await expect(kept).toBeVisible()
  await expect(kept.getByLabel('Repository password')).toHaveText(/^[0-9a-f]{64}$/)
  await kept.getByRole('button', { name: 'Copy and close' }).click()

  const row = page.getByRole('row', { name: /Off site/ })
  await expect(row).toContainText('sftp:fcp@backup.example.com:/srv/fcp')
  await expect(row).toContainText('Daily at 02:00 UTC')
  await expect(row).toContainText('Never run')

  // Asked for: waiting for the service.
  await page.getByRole('button', { name: 'Test Off site' }).click()
  await page.getByRole('button', { name: 'Back up Off site now' }).click()
  await settled(page)
  const runs = await (await api.get('/api/v1/system/backups/runs')).json()
  const list = runs.data ?? runs
  expect(list.map((r: { kind: string }) => r.kind).sort()).toEqual(['backup', 'test'])
  await expect(page.getByRole('row', { name: /Test · test/ })).toContainText('Waiting')

  // Edited without its key: the key stays.
  await page.getByRole('button', { name: 'Edit Off site' }).click()
  await expect(dialog.getByLabel('Private key')).toHaveValue('')
  await dialog.getByLabel('Name').fill('Off site, Ohio')
  await dialog.getByRole('button', { name: 'Save destination' }).click()
  await expect(dialog).toBeHidden()
  const after = await (await api.get('/api/v1/system/backups/destinations')).json()
  const dest = (after.data ?? after)[0]
  expect(dest.name).toBe('Off site, Ohio')
  expect(dest.target.has_private_key).toBe(true)

  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Delete Off site, Ohio' }).click()
  await expect(page.getByText('None yet: nothing is backed up off this machine.')).toBeVisible()
})
