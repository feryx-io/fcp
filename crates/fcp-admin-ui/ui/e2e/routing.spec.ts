// A routing rule for one location's callers (docs/TENANT_SCOPE.md §3.6): a
// tenant's administrator makes one on the Routing page, sees its location
// in the list, and filters the list by it — each step checked through the
// API.
import { test, expect, tenantAdmin, signIn, settled } from './fixtures'

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test("a routing rule is made for a location's callers and the list filters by it", async ({ page, api }) => {
  const suffix = Date.now().toString(36)
  const made = await api.post('/api/v1/locations', {
    data: { name: `Branch-${suffix}`, city: 'Troy', country: 'US' },
  })
  expect(made.status()).toBe(201)
  const branch = await made.json()
  const everyone = await api.post('/api/v1/routing/rules', {
    data: { name: `everyone-${suffix}`, action: 'allow' },
  })
  expect(everyone.status()).toBe(201)

  await signIn(page, tenantAdmin())
  await page.goto('/admin/routing')
  await settled(page)

  const name = `branch-only-${suffix}`
  await page.getByRole('button', { name: 'Add Rule' }).click()
  await page.getByLabel('Rule Name').fill(name)
  await page.getByLabel('For callers at').selectOption({ label: branch.name })
  await page.getByRole('button', { name: 'Create Rule' }).click()
  await settled(page)
  await expect(page.getByRole('row').filter({ hasText: name })).toContainText(branch.name)
  await expect(page.getByRole('row').filter({ hasText: `everyone-${suffix}` })).toContainText('Every caller')

  const rules = await (await api.get(`/api/v1/routing/rules?location_id=${branch.id}`)).json()
  const listed = (Array.isArray(rules) ? rules : rules.data) as { id: string; name: string; location_id: string }[]
  const rule = listed.find((r) => r.name === name)
  expect(rule?.location_id).toBe(branch.id)

  // The list for the branch, then for every caller.
  const filter = page.getByLabel('Rules for')
  await filter.selectOption({ label: branch.name })
  await expect(page.getByRole('row').filter({ hasText: name })).toHaveCount(1)
  await expect(page.getByRole('row').filter({ hasText: `everyone-${suffix}` })).toHaveCount(0)
  await filter.selectOption({ label: 'Every caller' })
  await expect(page.getByRole('row').filter({ hasText: name })).toHaveCount(0)
  await expect(page.getByRole('row').filter({ hasText: `everyone-${suffix}` })).toHaveCount(1)

  expect((await api.delete(`/api/v1/routing/rules/${rule!.id}`)).status()).toBe(204)
  expect((await api.delete(`/api/v1/routing/rules/${(await everyone.json()).id}`)).status()).toBe(204)
  expect((await api.delete(`/api/v1/locations/${branch.id}`)).status()).toBe(204)
})
