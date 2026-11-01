// The Parking page (docs/PARKING.md §4): a tenant's administrator makes a
// lot numbered as they like, sees its slots on the board, is told when a
// range clashes with another lot, renames the lot and deletes it — each
// step checked through the API.
import { test, expect, tenantAdmin, signIn, settled, type Page } from './fixtures'

/** A five-digit range no earlier run used: five slots. */
function freshRange() {
  const first = 70000 + Math.floor(Math.random() * 5000) * 5
  return { first: String(first), last: String(first + 4) }
}

async function fillLot(page: Page, name: string, first: string, last: string) {
  await page.getByLabel('Name').fill(name)
  await page.getByLabel('First slot').fill(first)
  await page.getByLabel('Last slot').fill(last)
}

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a parking lot is made, shown on the board, refused a clash, renamed and deleted', async ({ page, api, troubles }) => {
  await signIn(page, tenantAdmin())
  await settled(page)
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await nav.getByRole('link', { name: 'Parking' }).click()
  await expect(page).toHaveURL(/\/admin\/parking$/)
  await settled(page)
  await expect(page.getByRole('heading', { name: 'Parking', exact: true })).toBeVisible()

  const name = `Lobby-${Date.now().toString(36)}`
  const { first, last } = freshRange()
  await page.getByRole('button', { name: 'New Lot' }).click()
  await fillLot(page, name, first, last)
  await page.getByLabel('Waits (seconds)').fill('90')
  await page.getByRole('button', { name: 'Save' }).click()
  await settled(page)
  const row = page.getByRole('row').filter({ hasText: name })
  await expect(row).toContainText(`${first}–${last}`)
  await expect(row).toContainText('(5)')

  // Its five slots on the board, empty.
  const lots = await (await api.get('/api/v1/parking/lots')).json()
  const made = lots.data.find((l: { name: string }) => l.name === name)
  expect(made.first).toBe(first)
  expect(made.timeout_secs).toBe(90)
  const board = page.getByTestId(`board-${made.id}`)
  await expect(board.getByText('Empty')).toHaveCount(5)
  await expect(board).toContainText(first)

  // A lot overlapping it is refused, and the form says why.
  await page.getByRole('button', { name: 'New Lot' }).click()
  await fillLot(page, `${name}-over`, String(Number(first) + 2), String(Number(last) + 2))
  await page.getByRole('button', { name: 'Save' }).click()
  await expect(page.getByRole('dialog')).toContainText(name)
  await page.getByRole('button', { name: 'Cancel' }).click()
  // That one refusal is the form's to show, not a failure of the page.
  const refusal = troubles.findIndex((t) => t.what === 'POST /api/v1/parking/lots answered 409')
  expect(refusal).toBeGreaterThanOrEqual(0)
  troubles.splice(refusal, 1)

  // Renamed.
  await row.getByRole('button', { name: 'Edit' }).click()
  await page.getByLabel('Name').fill(`${name}-renamed`)
  await page.getByRole('button', { name: 'Save' }).click()
  await settled(page)
  await expect(page.getByRole('row').filter({ hasText: `${name}-renamed` })).toHaveCount(1)

  // Deleted.
  page.once('dialog', (d) => void d.accept())
  await page.getByRole('row').filter({ hasText: `${name}-renamed` }).getByRole('button', { name: 'Delete' }).click()
  await settled(page)
  await expect(page.getByRole('row').filter({ hasText: name })).toHaveCount(0)
  const after = await (await api.get('/api/v1/parking/lots')).json()
  expect(after.data.some((l: { id: string }) => l.id === made.id)).toBe(false)
})
