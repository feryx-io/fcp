// The Backups page (docs/SCHEDULED_BACKUPS.md B2, B3a): a platform
// administrator adds an SFTP destination, keeps the password made for it, asks for a test
// and a backup (which wait for fcp-backup: none runs under this test), edits
// it without its key, downloads a snapshot, and deletes it. Encryption at rest is off for the
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

  // B3a: its snapshots, and one made a download. The backup service is not
  // run under this test, so its three answers are stood in for here.
  await page.route(/\/api\/v1\/system\/backups\/destinations\/[^/]+\/snapshots(\?|$)/, (r) => r.fulfill({
    json: { data: [{
      run: 'bkr-6c2a7f0e-1d2b-4c3a-9f00-000000000001', time: '2026-11-27T02:00:00Z', release: '0.25.672',
      kek_id: null, host: 'fcp-backup', database: 'aaaa', files: 'bbbb', volumes: ['fcp-certs', 'voicemail-data'],
    }], next_cursor: null, total: 1 },
  }))
  let asked: unknown = null
  await page.route('**/api/v1/system/backups/destinations/*/snapshots/*/download', (r) => {
    asked = r.request().postDataJSON()
    return r.fulfill({ status: 202, json: { id: 'dl.tok', state: 'building', size: null, error: null, file_url: null } })
  })
  await page.route('**/api/v1/system/backups/downloads/*', (r) => r.fulfill({
    json: { id: 'dl.tok', state: 'ready', size: 2048, error: null, file_url: '/backups/downloads/dl/tok' },
  }))
  await page.getByRole('button', { name: 'Snapshots in Off site, Ohio' }).click()
  const snaps = page.getByRole('dialog', { name: 'Snapshots in Off site, Ohio' })
  await expect(snaps.getByRole('row', { name: /0\.25\.672/ })).toContainText('database, fcp-certs, voicemail-data')
  await snaps.getByRole('button', { name: /Choose the snapshot/ }).click()
  await snaps.getByRole('button', { name: 'Make the download' }).click()
  await expect(snaps.getByRole('link', { name: 'save it' })).toHaveAttribute('href', '/backups/downloads/dl/tok')
  await expect(snaps).toContainText('2.0 KiB')
  expect(asked).toEqual({})
  await snaps.getByRole('button', { name: 'Close' }).first().click()
  await expect(snaps).toBeHidden()

  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Delete Off site, Ohio' }).click()
  await expect(page.getByText('None yet: nothing is backed up off this machine.')).toBeVisible()
})
