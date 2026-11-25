// The Branding page (docs/BRANDING.md), D8 findings 72 and 73: a logo with
// a long file name used to widen its picker's select until the picker's
// buttons were pushed out over the next picker and could not be pressed;
// and nothing said how to take a logo off again.
import { test, expect, tenantAdmin, signIn, settled } from './fixtures'

// A 1×1 PNG.
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
)

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a logo with a long name leaves every picker usable, and is removed and deleted', async ({ page }) => {
  await signIn(page, tenantAdmin())
  await settled(page)
  await page.goto('/admin/branding')
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Branding', exact: true })).toBeVisible()
  // Save needs a name.
  await page.getByLabel('App name').fill('Acme Phone')

  const name = `a-very-long-logo-file-name-from-the-designers-final-final-v2-${Date.now().toString(36)}.png`
  const chooser = page.waitForEvent('filechooser')
  await page.getByRole('button', { name: 'Upload the logo', exact: true }).click()
  await (await chooser).setFiles({ name, mimeType: 'image/png', buffer: PNG })
  await settled(page)
  await expect(page.getByRole('combobox', { name: 'Logo', exact: true })).toContainText(name.slice(0, 20))

  // Every picker's buttons stay inside its own column, and still work.
  const logo = await page.getByRole('combobox', { name: 'Logo', exact: true }).boundingBox()
  const dark = await page.getByRole('combobox', { name: 'Logo on dark', exact: true }).boundingBox()
  const upload = await page.getByRole('button', { name: 'Upload the logo', exact: true }).boundingBox()
  expect(logo && dark && upload).toBeTruthy()
  expect(upload!.x + upload!.width).toBeLessThanOrEqual(dark!.x)
  const chooser2 = page.waitForEvent('filechooser')
  await page.getByRole('button', { name: 'Upload the logo on dark' }).click()
  await (await chooser2).setFiles({ name: `dark-${name}`, mimeType: 'image/png', buffer: PNG })
  await settled(page)

  await page.getByRole('button', { name: 'Save' }).first().click()
  await settled(page)

  // In use: its delete is off and says why.
  const row = page.getByRole('row').filter({ hasText: name }).first()
  await expect(row.getByRole('button', { name: 'Delete' })).toBeDisabled()

  // Removed from the brand and saved, it can be deleted.
  await page.getByRole('button', { name: 'Remove the logo', exact: true }).click()
  await page.getByRole('button', { name: 'Remove the logo on dark', exact: true }).click()
  await page.getByRole('button', { name: 'Save' }).first().click()
  await settled(page)
  page.once('dialog', (d) => d.accept())
  await row.getByRole('button', { name: 'Delete' }).click()
  await settled(page)
  await expect(page.getByRole('row').filter({ hasText: name })).toHaveCount(0)
})
