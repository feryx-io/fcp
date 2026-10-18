// Every page in the navigation opens without an error (docs/DEEP_TESTING.md D7).
import { test, expect, admin, person, signIn, settled, type Page, type Trouble } from './fixtures'

/**
 * The page drew something, and not the error boundary's apology; and every
 * button on it has a name a screen reader can say.
 */
async function opened(page: Page, troubles: Trouble[]) {
  await expect(page.locator('main')).not.toBeEmpty()
  await expect(page.getByText('Something went wrong')).toHaveCount(0)
  const unnamed = await page.locator('button:visible').evaluateAll((buttons) =>
    buttons
      .filter((b) => {
        const name = (b.getAttribute('aria-label') || b.getAttribute('title') || b.textContent || '').trim()
        const labelled = (b as HTMLButtonElement).labels?.length || b.getAttribute('aria-labelledby')
        return !name && !labelled
      })
      .map((b) => b.outerHTML.replace(/ class="[^"]*"/g, '').slice(0, 300)),
  )
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
  for (const href of ['/user', '/user/voicemail', '/user/calling', '/user/devices', '/user/api-tokens', '/user/conferences', '/user/settings']) {
    await page.goto(href)
    await settled(page)
    await opened(page, troubles)
  }
  await page.goto('/user/voicemail')
  await expect(page.getByText('No voicemail mailbox is configured for your account.')).toBeVisible()
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})
