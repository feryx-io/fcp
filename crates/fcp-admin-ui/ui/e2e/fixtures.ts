// What every spec shares: who signs in, the API beside the UI, and a page
// that remembers what went wrong in it — console errors, uncaught
// exceptions, and API answers a page did not expect.
import { test as base, expect, type Page, type APIRequestContext, request } from '@playwright/test'

export type Account = { name: string; password: string }

function account(variable: string): Account {
  const value = process.env[variable]
  if (!value) throw new Error(`${variable} is not set (crates/fcp-admin/tests/ui.rs sets it)`)
  const [name, ...rest] = value.split(':')
  return { name, password: rest.join(':') }
}

export const admin = () => account('FCP_UI_ADMIN')
export const tenantAdmin = () => account('FCP_UI_TENANT_ADMIN')
export const person = () => account('FCP_UI_USER')

/**
 * Reads that answer 404 when there is nothing yet, which the page shows as
 * nothing yet. Any other answer of 400 or more is a trouble.
 */
const NOTHING_YET = [
  '/api/v1/branding', // a tenant without a brand
  '/api/v1/branding/deployment-profile', // a platform administrator with no tenant chosen
  '/api/v1/provisioning/key', // no provisioning key made
  '/api/v1/config/call-manager/b2bua', // no call manager has written its configuration
  '/api/v1/me/voicemail', // a user without a mailbox
  '/api/v1/me/voicemail/stats',
]

/** Something a page did that a person would call broken. */
export type Trouble = { page: string; what: string }

export const test = base.extend<{ troubles: Trouble[]; api: APIRequestContext }>({
  troubles: async ({ page }, use) => {
    const troubles: Trouble[] = []
    const here = () => new URL(page.url()).pathname
    page.on('console', (msg) => {
      // A failed request is reported below, by its address.
      if (msg.type() === 'error' && !msg.text().startsWith('Failed to load resource')) {
        troubles.push({ page: here(), what: `console: ${msg.text()}` })
      }
    })
    page.on('pageerror', (err) => troubles.push({ page: here(), what: `exception: ${err.message}` }))
    // Before a sign-in the session's cookie is asked for and refused (401).
    page.on('response', (res) => {
      const url = new URL(res.url())
      const expected =
        (res.status() === 401 && url.pathname === '/api/v1/auth/refresh') ||
        (res.status() === 404 && NOTHING_YET.includes(url.pathname))
      if (url.pathname.startsWith('/api/') && res.status() >= 400 && !expected) {
        troubles.push({ page: here(), what: `${res.request().method()} ${url.pathname} answered ${res.status()}` })
      }
    })
    await use(troubles)
  },
  api: async ({}, use) => {
    const ctx = await request.newContext({
      baseURL: process.env.FCP_UI_URL,
      extraHTTPHeaders: { Authorization: `Bearer ${process.env.FCP_UI_TOKEN}` },
    })
    await use(ctx)
    await ctx.dispose()
  },
})

export { expect }
export type { Page }

/** Sign in through the form, landing where the role lands. */
export async function signIn(page: Page, who: Account) {
  await page.goto('/login')
  await page.locator('#username').fill(who.name)
  await page.locator('#username').blur()
  await page.locator('#password').fill(who.password)
  await page.getByRole('button', { name: 'Sign in' }).click()
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
}

/** The default tenant: the static token's, and where the suite's people are. */
export const DEFAULT_TENANT = 'tenant-00000000-0000-0000-0000-000000000000'

/**
 * As a platform administrator, work in a tenant: pick it in the top bar
 * (docs/TENANT_SCOPE.md §3.1). Its name is read through the API.
 */
export async function pickTenant(page: Page, api: APIRequestContext, id = DEFAULT_TENANT) {
  const tenant = await (await api.get(`/api/v1/tenants/${id}`)).json()
  await page.getByRole('combobox').filter({ hasText: /All Tenants/i }).click()
  await page.getByRole('option', { name: tenant.name, exact: true }).click()
}

/** Wait until a page has loaded what it asks for. */
export async function settled(page: Page) {
  await page.waitForLoadState('networkidle')
}
