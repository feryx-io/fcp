// The Speech page (docs/TENANT_SPEECH.md, S2b): a tenant's administrator
// sets the language and a Spanish voice, saves, finds them kept, and goes
// back to the platform's defaults. The admin under test has no TTS
// service: the page says so and lists no voices.
import { test, expect, tenantAdmin, signIn, settled } from './fixtures'

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test("a tenant's speech settings are saved, kept and taken back", async ({ page, api }) => {
  await signIn(page, tenantAdmin())
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await nav.getByRole('link', { name: 'Speech' }).click()
  await expect(page).toHaveURL(/\/admin\/speech$/)
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Speech', exact: true })).toBeVisible()
  await expect(page.getByText('No TTS service answers this admin')).toBeVisible()

  await page.getByLabel('Language', { exact: true }).first().fill('en-GB')
  await page.getByRole('button', { name: 'Add' }).click()
  await page.getByLabel('Language').nth(1).fill('es')
  await page.getByRole('button', { name: 'Save' }).click()
  await settled(page)
  await expect(page.getByRole('button', { name: 'Saved' })).toBeVisible()

  // Kept: what the API holds, and what the page shows again.
  const kept = await (await api.get('/api/v1/speech/settings')).json()
  expect(kept.language).toBe('en-GB')
  expect(Object.keys(kept.voices)).toEqual([])
  await page.reload()
  await settled(page)
  await expect(page.getByLabel('Language', { exact: true }).first()).toHaveValue('en-GB')

  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Defaults' }).click()
  await settled(page)
  await expect(page.getByLabel('Language', { exact: true }).first()).toHaveValue('')
  const back = await (await api.get('/api/v1/speech/settings')).json()
  expect(back.configured).toBe(false)
})
