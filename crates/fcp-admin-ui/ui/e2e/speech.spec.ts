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
  // Usage (S4): nothing spoken for this tenant yet.
  await expect(page.getByRole('heading', { name: 'Usage' })).toBeVisible()
  await expect(page.getByText('Nothing spoken in the last twelve months.')).toBeVisible()

  await page.getByLabel('Language', { exact: true }).first().fill('en-GB')
  await page.getByRole('button', { name: 'Add', exact: true }).click()
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

test("a tenant's own speech account is added, edited and deleted, its key never shown", async ({ page, api }) => {
  await signIn(page, tenantAdmin())
  await page.goto('/admin/speech')
  await settled(page)
  await page.getByRole('button', { name: 'Add account' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Name').fill('Our OpenAI')
  await dialog.getByLabel('API key').fill('sk-never-shown')
  await dialog.getByRole('button', { name: 'Save account' }).click()
  await expect(dialog).toBeHidden()
  const row = page.getByRole('row', { name: /Our OpenAI/ })
  await expect(row).toBeVisible()
  await expect(row.getByLabel('Key kept')).toBeVisible()
  await expect(page.getByText('sk-never-shown')).toHaveCount(0)

  // Edited without its key: the key is kept.
  await page.getByRole('button', { name: 'Edit Our OpenAI' }).click()
  await expect(dialog.getByLabel('API key')).toHaveValue('')
  await dialog.getByLabel('Name').fill('Voices')
  await dialog.getByRole('button', { name: 'Save account' }).click()
  await expect(dialog).toBeHidden()
  const kept = await (await api.get('/api/v1/speech/accounts')).json()
  const mine = (kept.data ?? kept).find((a: { name: string }) => a.name === 'Voices')
  expect(mine?.has_key).toBe(true)

  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Delete Voices' }).click()
  await expect(page.getByRole('row', { name: /Voices/ })).toHaveCount(0)
})

// Caps (docs/SPEECH_CAPS.md C2): a tenant's administrator caps its own
// account from the page, sees where it stands this month, and takes the
// cap off again.
test("a tenant caps its own account's speech and takes the cap off", async ({ page, api }) => {
  const made = await api.post('/api/v1/speech/accounts', {
    data: { name: 'Capped voices', kind: 'openai', secret: 'sk-capped', price_per_million: 15 },
  })
  expect(made.ok()).toBe(true)
  const account = await made.json()
  expect(account.price_per_million).toBe(15)

  await signIn(page, tenantAdmin())
  await page.goto('/admin/speech')
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Caps' })).toBeVisible()
  await expect(page.getByText('No caps: speech is not limited.')).toBeVisible()

  await page.getByRole('button', { name: 'Add a cap' }).click()
  await page.getByLabel('Characters a month').fill('1000')
  await page.getByRole('button', { name: 'Save cap' }).click()
  await settled(page)
  const row = page.getByTestId(`cap-${account.provider}`)
  await expect(row.getByText('0 of 1,000 characters this month', { exact: false })).toBeVisible()
  await expect(row.getByRole('progressbar', { name: 'Capped voices this month' })).toBeVisible()
  const kept = await (await api.get('/api/v1/speech/caps')).json()
  const mine = (kept.data ?? kept).find((c: { provider: string }) => c.provider === account.provider)
  expect(mine?.own_cap).toBe(1000)
  expect(mine?.state).toBe('ok')

  // Emptied and saved: no cap.
  await row.getByLabel('Your cap', { exact: true }).fill('')
  await row.getByRole('button', { name: 'Save your cap on Capped voices' }).click()
  await settled(page)
  await expect(page.getByText('No caps: speech is not limited.')).toBeVisible()

  // The account's price is in its form.
  await page.getByRole('button', { name: 'Edit Capped voices' }).click()
  await expect(page.getByRole('dialog').getByLabel('Price per million characters (optional)')).toHaveValue('15')
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Delete Capped voices' }).click()
  await expect(page.getByRole('row', { name: /Capped voices/ })).toHaveCount(0)
})
