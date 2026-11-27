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

// Recognition (docs/TENANT_RECOGNITION.md R2b): the language a tenant is
// heard in saved and kept; a Deepgram account listed as one that hears; a
// recording chosen here sent to be heard (the admin under test has no
// speech server, so it says it could not be).
test("a tenant's recognition is chosen, kept and tried", async ({ page, api }) => {
  const made = await api.post('/api/v1/speech/accounts', {
    data: { name: 'Our ears', kind: 'deepgram', secret: 'dg-never-shown' },
  })
  expect(made.ok()).toBe(true)

  await signIn(page, tenantAdmin())
  await page.goto('/admin/speech')
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Recognition' })).toBeVisible()
  await expect(page.getByRole('row', { name: /Our ears/ })).toContainText('Hears')

  await page.getByLabel('Recognition language').fill('fr-CA')
  await page.getByRole('button', { name: 'Save' }).click()
  await settled(page)
  await expect(page.getByRole('button', { name: 'Saved' })).toBeVisible()
  const kept = await (await api.get('/api/v1/speech/settings')).json()
  expect(kept.recognition_language).toBe('fr-CA')
  await page.reload()
  await settled(page)
  await expect(page.getByLabel('Recognition language')).toHaveValue('fr-CA')

  // A second of a 440 Hz tone as a WAV, chosen as the recording.
  const rate = 16000
  const samples = new Int16Array(rate).map((_, i) => Math.round(8000 * Math.sin((2 * Math.PI * 440 * i) / rate)))
  const header = Buffer.alloc(44)
  header.write('RIFF', 0); header.writeUInt32LE(36 + samples.length * 2, 4); header.write('WAVE', 8)
  header.write('fmt ', 12); header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(1, 22)
  header.writeUInt32LE(rate, 24); header.writeUInt32LE(rate * 2, 28); header.writeUInt16LE(2, 32); header.writeUInt16LE(16, 34)
  header.write('data', 36); header.writeUInt32LE(samples.length * 2, 40)
  const wav = Buffer.concat([header, Buffer.from(samples.buffer)])
  const sent = page.waitForRequest((r) => r.url().includes('/api/v1/speech/recognize') && r.method() === 'POST')
  await page.locator('#speech-recognize-file').setInputFiles({ name: 'tone.wav', mimeType: 'audio/wav', buffer: wav })
  // Sent, converted to WAV on the way (a Blob body Playwright cannot read).
  await sent
  await expect(page.getByText('The recording could not be heard')).toBeVisible()

  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Defaults' }).click()
  await settled(page)
  page.once('dialog', (d) => d.accept())
  await page.getByRole('button', { name: 'Delete Our ears' }).click()
  await expect(page.getByRole('row', { name: /Our ears/ })).toHaveCount(0)
})
