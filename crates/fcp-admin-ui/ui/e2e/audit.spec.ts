// The audit log by the tenant acted on (docs/TENANT_SCOPE.md §3.3): a
// platform administrator sees each row's tenant and can list the platform's
// own; a tenant's administrator sees what the platform did to their tenant,
// as "platform administrator".
import { test, expect, admin, tenantAdmin, signIn, settled } from './fixtures'

const unique = (prefix: string) => `${prefix}-${Date.now().toString(36)}`

test('the audit log shows whose each request was', async ({ page, api, troubles }) => {
  // A platform action (a firmware image, refused or not) and the static
  // token's action in the default tenant.
  await api.post('/api/v1/firmware?vendor=yealink&version=ui', { data: 'not firmware' })
  const made = await api.post('/api/v1/schedules', { data: { name: unique('ui-audit'), timezone: 'UTC' } })
  expect(made.status()).toBe(201)

  await signIn(page, admin())
  await page.goto('/admin/audit')
  await settled(page)
  await expect(page.getByRole('columnheader', { name: 'Tenant' })).toBeVisible()
  await page.getByRole('combobox', { name: 'Rows' }).click()
  await page.getByRole('option', { name: 'The platform only' }).click()
  const firmware = page.getByRole('row').filter({ hasText: '/api/v1/firmware' }).first()
  await expect(firmware).toContainText('Platform')
  await expect(page.getByRole('row').filter({ hasText: '/api/v1/schedules' })).toHaveCount(0)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test("a tenant's administrator sees the platform's actions on their tenant, unnamed", async ({ page, api, troubles }) => {
  const made = await api.post('/api/v1/schedules', { data: { name: unique('ui-audit-t'), timezone: 'UTC' } })
  expect(made.status()).toBe(201)

  await signIn(page, tenantAdmin())
  await page.goto('/admin/audit')
  await settled(page)
  await expect(page.getByRole('columnheader', { name: 'Tenant' })).toHaveCount(0)
  await expect(page.getByRole('combobox', { name: 'Rows' })).toHaveCount(0)
  await page.getByPlaceholder('DELETE /api/v1/users').fill('POST /api/v1/schedules')
  const row = page.getByRole('row').filter({ hasText: '/api/v1/schedules' }).first()
  await expect(row).toContainText('platform administrator')
  await expect(page.getByRole('row').filter({ hasText: 'static_token' })).toHaveCount(0)
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

// #342: the log pages by cursor, takes an Until on deletions and exports
// them, and shows a platform administrator the configuration's changes
// across services; a tenant's administrator has no such tab.
test('the audit log pages, exports deletions and shows configuration changes', async ({ page, api, troubles }) => {
  const section = unique('ui-audit-section')
  const put = await api.put(`/api/v1/config/media/${section}`, {
    data: { value: { enabled: true }, updated_by: 'ui-audit' },
  })
  expect(put.ok(), await put.text()).toBeTruthy()

  await signIn(page, admin())
  await page.goto('/admin/audit')
  await settled(page)
  await expect(page.getByRole('columnheader', { name: 'Source IP' })).toBeVisible()

  await page.getByRole('button', { name: 'Deletions', exact: true }).click()
  await settled(page)
  await expect(page.getByLabel('Until')).toBeVisible()
  const download = page.waitForEvent('download')
  await page.getByRole('button', { name: 'CSV' }).click()
  expect((await download).suggestedFilename()).toBe('deletion-log.csv')

  await page.getByRole('button', { name: 'Config changes' }).click()
  await settled(page)
  await page.getByPlaceholder('auth').fill(section)
  const row = page.getByRole('row').filter({ hasText: section })
  await expect(row).toContainText('Media Server')
  await expect(row).toContainText('ui-audit')
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])

  await signIn(page, tenantAdmin())
  await page.goto('/admin/audit')
  await settled(page)
  await expect(page.getByRole('button', { name: 'Config changes' })).toHaveCount(0)
})
