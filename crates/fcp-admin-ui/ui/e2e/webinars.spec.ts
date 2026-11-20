// A webinar's pages (docs/WEBINARS.md §2.6, W4.3): an administrator turns
// on registration in the meeting's form, registers a person and is handed
// their join link, mints another, revokes the registration, and finds no
// sitting yet — each step checked through the API.
import { test, expect, tenantAdmin, signIn, settled } from './fixtures'

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a webinar requires registration, registers a person, re-links and revokes them', async ({ page, api, troubles }) => {
  const room = `7${Date.now().toString().slice(-5)}`
  const name = `Town hall ${Date.now().toString(36)}`
  const made = await api.post('/api/v1/conferences/meetings', {
    data: { room_number: room, name, kind: 'webinar' },
  })
  expect(made.status()).toBe(201)
  const meeting = await made.json()

  await signIn(page, tenantAdmin())
  await page.goto('/admin/conferences')
  await settled(page)
  const row = page.getByRole('row').filter({ hasText: name })
  await expect(row).toContainText('Webinar')

  // Registration on, through the form.
  await row.getByRole('button', { name: 'Actions' }).click()
  await page.getByRole('menuitem', { name: 'Edit' }).click()
  const edit = page.getByRole('dialog')
  await edit.getByRole('switch', { name: 'Require registration' }).click()
  await edit.getByRole('button', { name: /Save/ }).click()
  await expect(edit).toBeHidden()
  const after = await (await api.get(`/api/v1/conferences/meetings/${meeting.id}`)).json()
  expect(after.registration).toBe(true)
  await expect(row).toContainText('Registration')

  // The webinar's own dialog. No conference server here, so no sitting:
  // the Live tab says nobody is in the room.
  await row.getByRole('button', { name: 'Actions' }).click()
  await page.getByRole('menuitem', { name: 'Webinar' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Nobody is in the room')
  for (let i = troubles.length - 1; i >= 0; i--) {
    if (troubles[i].what.startsWith(`GET /api/v1/conferences/live/${room}/webinar answered`)) troubles.splice(i, 1)
  }

  await dialog.getByRole('button', { name: 'Registrants' }).click()
  await expect(dialog).toContainText('requires registration')
  await dialog.getByPlaceholder('Ann Lee').fill('Ann Example')
  await dialog.getByPlaceholder('Optional').fill('ann@example.com')
  await dialog.getByRole('button', { name: 'Register' }).click()
  await expect(dialog).toContainText('for Ann Example')
  const regRow = dialog.getByRole('row').filter({ hasText: 'Ann Example' })
  await expect(regRow).toContainText('Registered')
  const listed = await (await api.get(`/api/v1/conferences/meetings/${meeting.id}/registrants`)).json()
  expect(listed.data).toHaveLength(1)
  expect(listed.data[0].email).toBe('ann@example.com')

  await regRow.getByRole('button', { name: 'New link' }).click()
  await expect(dialog).toContainText('for Ann Example')

  page.once('dialog', (d) => void d.accept())
  await regRow.getByRole('button', { name: 'Revoke' }).click()
  await expect(regRow).toContainText('Revoked')
  const revoked = await (await api.get(`/api/v1/conferences/meetings/${meeting.id}/registrants/${listed.data[0].id}`)).json()
  expect(revoked.revoked_at).toBeTruthy()

  await dialog.getByRole('button', { name: 'Attendance' }).click()
  await expect(dialog).toContainText('has not been used yet')

  expect((await api.delete(`/api/v1/conferences/meetings/${meeting.id}`)).status()).toBe(204)
})
