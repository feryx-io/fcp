// The Logs page (docs/LOG_VIEWER.md, L4): a platform administrator follows
// the services' lines (the admin's own, shipped to Redis), finds one by its
// text, opens it, and raises a service's level for an hour and takes it
// back; a tenant's administrator sees the page without the levels.
import { test, expect, admin, tenantAdmin, signIn, settled } from './fixtures'

const withRedis = !!process.env.FCP_UI_SERVICES

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a platform administrator follows, finds and opens a line, and sets a level', async ({ page }) => {
  test.skip(!withRedis, 'the admin under test keeps no logs (no Redis)')
  await signIn(page, admin())
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await nav.getByRole('link', { name: 'Logs' }).click()
  await expect(page).toHaveURL(/\/admin\/logs$/)
  await expect(page.getByRole('heading', { name: 'Logs', exact: true })).toBeVisible()

  // The admin's own lines, followed as they come.
  const lines = page.getByTestId('log-lines')
  await expect(lines).toContainText('ui-admin', { timeout: 20_000 })

  // Found by text, and opened.
  await page.getByLabel('Text').fill('API server listening')
  const found = lines.getByRole('button').filter({ hasText: 'API server listening' }).first()
  await expect(found).toBeVisible({ timeout: 15_000 })
  await found.click()
  await expect(lines).toContainText('target')

  // The admin's level for an hour, then back.
  const level = page.getByRole('combobox', { name: 'Level of ui-admin' })
  await level.click()
  await page.getByRole('option', { name: 'debug' }).click()
  await settled(page)
  await expect(page.getByText(/for \d+ more minutes/)).toBeVisible()
  await page.getByRole('button', { name: 'Back to configured for ui-admin' }).click()
  await settled(page)
  await expect(page.getByText(/for \d+ more minutes/)).toHaveCount(0)
})

test("a tenant's administrator sees the page without the levels", async ({ page }) => {
  test.skip(!withRedis, 'the admin under test keeps no logs (no Redis)')
  await signIn(page, tenantAdmin())
  await settled(page)
  await page.goto('/admin/logs')
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Logs', exact: true })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Levels' })).toHaveCount(0)
  await expect(page.getByLabel('Tenant')).toHaveCount(0)
})
