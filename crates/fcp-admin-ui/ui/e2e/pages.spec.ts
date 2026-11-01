// Every page in the navigation opens without an error (docs/DEEP_TESTING.md D7).
import { readFileSync } from 'node:fs'
import { test, expect, admin, person, signIn, settled, type Page, type Trouble } from './fixtures'

/**
 * The page drew something, and not the error boundary's apology; and every
 * button on it has a name a screen reader can say.
 */
async function opened(page: Page, troubles: Trouble[]) {
  await expect(page.locator('main')).not.toBeEmpty()
  await expect(page.getByText('Something went wrong')).toHaveCount(0)
  const scan = () =>
    page.locator('button:visible').evaluateAll((buttons) =>
      buttons
        .filter((b) => {
          const name = (b.getAttribute('aria-label') || b.getAttribute('title') || b.textContent || '').trim()
          const labelled = (b as HTMLButtonElement).labels?.length || b.getAttribute('aria-labelledby')
          return !name && !labelled
        })
        .map((b) => b.outerHTML.replace(/ class="[^"]*"/g, '').slice(0, 300)),
    )
  // The page before can still be in the DOM, its labels already gone, just
  // after the route changes: a button is unnamed only if it still is a
  // moment later.
  let unnamed = await scan()
  if (unnamed.length > 0) {
    await page.waitForTimeout(500)
    const again = await scan()
    unnamed = unnamed.filter((b) => again.includes(b))
  }
  const here = new URL(page.url()).pathname
  for (const button of unnamed) troubles.push({ page: here, what: `a button without a name: ${button}` })
}

test('an administrator opens every page in the navigation', async ({ page, troubles }) => {
  await signIn(page, admin())
  await expect(page).toHaveURL(/\/admin/)
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  const hrefs = await nav.locator('a').evaluateAll((links) =>
    links.map((a) => (a as HTMLAnchorElement).pathname),
  )
  expect(hrefs.length).toBeGreaterThan(30)
  for (const href of hrefs) {
    await nav.locator(`a[href="${href}"]`).first().click()
    await expect(page).toHaveURL(new RegExp(`${href}$`))
    await settled(page)
    await opened(page, troubles)
  }
  await page.goto('/admin/settings')
  await settled(page)
  await opened(page, troubles)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a person opens every page of their portal', async ({ page, troubles }) => {
  await signIn(page, person())
  await expect(page).toHaveURL(/\/user/)
  await settled(page)
  for (const href of ['/user', '/user/voicemail', '/user/calling', '/user/contacts', '/user/devices', '/user/api-tokens', '/user/conferences', '/user/settings']) {
    await page.goto(href)
    await settled(page)
    await opened(page, troubles)
  }
  await page.goto('/user/voicemail')
  await expect(page.getByText('No voicemail mailbox is configured for your account.')).toBeVisible()
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a person keeps their own contacts', async ({ page, troubles }) => {
  const name = `UI Contact ${Date.now().toString(36)}`
  await signIn(page, person())
  await page.goto('/user/contacts')
  await settled(page)
  await page.getByRole('button', { name: 'New contact' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#c-name').fill(name)
  await dialog.locator('#c-company').fill('Acme')
  // The first number is a mobile unless chosen; the second, Work, is
  // changed to a label of the person's own.
  await dialog.getByLabel('Number 1', { exact: true }).fill('0412345678')
  await dialog.getByRole('button', { name: 'Add a number' }).click()
  await dialog.getByLabel('Number 2', { exact: true }).fill('0298765432')
  await dialog.getByLabel('Label 2', { exact: true }).click()
  await page.getByRole('option', { name: 'Custom…' }).click()
  await dialog.getByLabel('Label 2', { exact: true }).fill('Boat')
  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog).toBeHidden()

  // Kept by FCP: there after a reload, and found by its number.
  await page.reload()
  await settled(page)
  await page.getByLabel('Search contacts').fill('0298765432')
  await expect(page.getByText(name)).toBeVisible()
  await expect(page.getByText('0412345678 (Mobile)', { exact: false })).toBeVisible()
  await expect(page.getByText('0298765432 (Boat)', { exact: false })).toBeVisible()

  // Out as a vCard, and that file in again: nothing new.
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('button', { name: 'Export' }).click().then(() =>
      page.getByRole('menuitem', { name: /vCard/ }).click()),
  ])
  expect(download.suggestedFilename()).toBe('contacts.vcf')
  const vcard = readFileSync(await download.path(), 'utf8')
  expect(vcard).toContain(`FN:${name}`)
  await page.getByLabel('Contacts file to import').setInputFiles({
    name: 'contacts.vcf',
    mimeType: 'text/vcard',
    buffer: Buffer.from(vcard),
  })
  await expect(page.getByRole('status')).toContainText('Imported 0 contacts')
  await expect(page.getByRole('status')).toContainText('you already had')

  await page.getByRole('button', { name: `Edit ${name}` }).click()
  await dialog.getByRole('button', { name: 'Remove number 1' }).click()
  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.getByText('0412345678', { exact: false })).toHaveCount(0)

  page.once('dialog', (d) => void d.accept())
  await page.getByRole('button', { name: `Delete ${name}` }).click()
  await expect(page.getByText(name)).toHaveCount(0)
  await page.reload()
  await settled(page)
  await expect(page.getByText(name)).toHaveCount(0)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})
