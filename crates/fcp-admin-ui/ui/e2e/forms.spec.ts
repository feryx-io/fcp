// The forms' validation through zod and its resolver: a bad field refused
// with its message on the form, and a typed number reaching the API as a
// number. A resolver that does not know the zod release throws instead of
// returning field errors, which only a submitted form shows.
import { test, expect, admin, tenantAdmin, signIn, settled } from './fixtures'

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a form with a bad field is refused with its message', async ({ page }) => {
  await signIn(page, admin())
  await page.goto('/admin/tenants')
  await settled(page)

  await page.getByRole('button', { name: 'Add Tenant' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Create Tenant' }).click()
  await expect(dialog.getByText('Name is required')).toBeVisible()
})

test('a number typed into a form reaches the API as a number', async ({ page, api }) => {
  await signIn(page, tenantAdmin())
  await page.goto('/admin/conferences')
  await settled(page)

  const room = `9${Date.now().toString().slice(-5)}`
  const name = `Forms-${Date.now().toString(36)}`
  await page.getByRole('button', { name: 'Create Bridge' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('8001').fill(room)
  await dialog.getByPlaceholder('Main Conference').fill(name)
  await dialog.getByPlaceholder('0 = unlimited').fill('12')
  await dialog.getByRole('button', { name: 'Create Bridge' }).click()
  await expect(dialog).toBeHidden()

  const listed = await (await api.get('/api/v1/conferences/meetings?limit=500')).json()
  const meetings = (Array.isArray(listed) ? listed : listed.data) as { id: string; name: string; max_participants: number }[]
  const made = meetings.find((m) => m.name === name)
  expect(made?.max_participants).toBe(12)

  expect((await api.delete(`/api/v1/conferences/meetings/${made!.id}`)).status()).toBe(204)
})
