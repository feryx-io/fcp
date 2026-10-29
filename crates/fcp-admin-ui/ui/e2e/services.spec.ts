// The System section and the Services page (docs/SERVICES.md §3): a platform
// administrator sees every service, the admin itself serving and a stopped
// voicemail server, opens it and forgets it; the dashboard's tile counts
// them; a tenant's administrator sees neither Services nor Cluster.
import { test, expect, admin, tenantAdmin, signIn, settled } from './fixtures'

const withServices = !!process.env.FCP_UI_SERVICES

test('a platform administrator sees every service and forgets one that stopped', async ({ page, troubles }) => {
  test.skip(!withServices, 'the admin under test keeps no services (no Redis)')
  await signIn(page, admin())
  await settled(page)

  // The dashboard counts them, and leads to the ones not serving.
  await expect(page.getByText(/\d+ of \d+ serving/)).toBeVisible()

  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await nav.getByRole('link', { name: 'Services' }).click()
  await expect(page).toHaveURL(/\/admin\/services$/)
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Services' })).toBeVisible()

  // The admin itself, serving; the voicemail server, stopped.
  const me = page.getByRole('row').filter({ hasText: 'ui-admin' })
  await expect(me).toContainText('serving')
  await expect(me).toContainText('Admin')
  const vm = page.getByRole('row').filter({ hasText: 'ui-vm-1' })
  await expect(vm).toContainText('stopped')

  // Filtered to what is not serving: the voicemail server, not the admin.
  await page.getByRole('combobox', { name: 'State' }).click()
  await page.getByRole('option', { name: 'Not serving' }).click()
  await expect(page).toHaveURL(/state=attention/)
  await expect(page.getByRole('row').filter({ hasText: 'ui-vm-1' })).toBeVisible()
  await expect(page.getByRole('row').filter({ hasText: 'ui-admin' })).toHaveCount(0)

  // Its own page, and forgotten.
  await page.getByRole('link', { name: 'ui-vm-1' }).click()
  await expect(page).toHaveURL(/\/admin\/services\/ui-vm-1/)
  await expect(page.getByRole('heading', { name: 'ui-vm-1' })).toBeVisible()
  await expect(page.getByText('It said it was stopping')).toBeVisible()
  await expect(page.getByText('10.0.0.5:8083')).toBeVisible()
  page.once('dialog', (d) => void d.accept())
  await page.getByRole('button', { name: 'Forget' }).click()
  await expect(page).toHaveURL(/\/admin\/services$/)
  await settled(page)
  await expect(page.getByRole('row').filter({ hasText: 'ui-vm-1' })).toHaveCount(0)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test("a tenant's administrator sees none of the platform's pages", async ({ page, troubles }) => {
  await signIn(page, tenantAdmin())
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await expect(nav.getByRole('link', { name: 'Sites' })).toBeVisible()
  await expect(nav.getByRole('link', { name: 'Services' })).toHaveCount(0)
  await expect(nav.getByRole('link', { name: 'Cluster' })).toHaveCount(0)
  await expect(page.getByText(/\d+ of \d+ serving/)).toHaveCount(0)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})
