// Each page's main object made, changed and deleted through its form, and
// checked through the API afterwards (docs/DEEP_TESTING.md D7). As a tenant
// administrator, who keeps these day to day.
import { test, expect, admin, tenantAdmin, signIn, settled, pickTenant, type Page } from './fixtures'

/** A name no earlier run used. */
const unique = (prefix: string) => `${prefix}-${Date.now().toString(36)}`

/** Accept the browser's confirm() the next click raises. */
function acceptConfirm(page: Page) {
  page.once('dialog', (d) => void d.accept())
}

/** A DataTable row by its text, found through the table's search. */
async function row(page: Page, text: string) {
  const search = page.getByPlaceholder(/search/i).first()
  if (await search.isVisible()) await search.fill(text)
  const found = page.getByRole('row').filter({ hasText: text })
  await expect(found).toHaveCount(1)
  return found
}

/** A row's actions menu item. */
async function rowAction(page: Page, text: string, item: string) {
  const r = await row(page, text)
  await r.getByRole('button', { name: 'Actions' }).click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

// In an enterprise (the default deployment mode) trunks are the platform
// administrator's; the rest the tenant administrator's.
test.beforeEach(async ({ page }, info) => {
  await signIn(page, info.title.includes('trunk') ? admin() : tenantAdmin())
})

test.afterEach(async ({ troubles }) => {
  expect(troubles, JSON.stringify(troubles, null, 2)).toEqual([])
})

test('a user is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-user')
  await page.goto('/admin/users')
  await settled(page)
  await page.getByRole('button', { name: 'Add User' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#create-username').fill(name)
  await dialog.locator('#create-password').fill('Passw0rd!x-ui')
  // No display name: she reads as her first and last (#343).
  await dialog.locator('#create-first_name').fill('Ada')
  await dialog.locator('#create-last_name').fill('Lovelace')
  await dialog.locator('#create-email').fill(`${name}@example.com`)
  // A deleted user keeps their extension through the grace period, so one
  // no earlier run held.
  await dialog.locator('#create-extension').fill(`4${Math.floor(Math.random() * 900 + 100)}`)
  await dialog.getByRole('button', { name: 'Create User' }).click()
  await expect(dialog).toBeHidden()
  let made = await (await api.get(`/api/v1/users/${name}`)).json()
  expect(made.display_name).toBe('Ada Lovelace')
  expect(made.first_name).toBe('Ada')
  expect(made.email).toBe(`${name}@example.com`)

  // A new last name moves the name she reads as.
  await rowAction(page, name, 'Edit')
  let edit = page.getByRole('dialog')
  await expect(edit.locator('#edit-display_name')).toHaveValue('')
  await edit.locator('#edit-last_name').fill('King')
  await edit.locator('#edit-email').fill('')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await (await api.get(`/api/v1/users/${name}`)).json()
  expect(made.display_name).toBe('Ada King')
  expect(made.last_name).toBe('King')
  expect(made.email ?? '').toBe('')

  // A display name of her own.
  await rowAction(page, name, 'Edit')
  edit = page.getByRole('dialog')
  await edit.locator('#edit-display_name').fill('Countess')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await (await api.get(`/api/v1/users/${name}`)).json()
  expect(made.display_name).toBe('Countess')

  acceptConfirm(page)
  await rowAction(page, name, 'Delete')
  await expect.poll(async () => (await (await api.get(`/api/v1/users/${name}`)).json()).account_status).toBe('deprovisioned')
})

// #343: which greeting a mailbox's callers hear, chosen on the Voicemail
// page; the default reads as the greeting with the name.
test("a mailbox's greeting is chosen", async ({ page, api }) => {
  const name = unique('ui-vm')
  const last = unique('Hopper')
  const user = await api.post('/api/v1/users', {
    data: { username: name, password: 'Passw0rd!x-ui', first_name: 'Grace', last_name: last },
  })
  expect(user.status(), await user.text()).toBe(201)
  const made = await user.json()
  const mailbox = await api.post('/api/v1/voicemail/mailboxes', {
    data: { tenant_id: made.tenant_id, user_id: made.id, pin: '4321' },
  })
  expect(mailbox.status(), await mailbox.text()).toBe(201)
  const id = (await mailbox.json()).id

  await page.goto('/admin/voicemail')
  await settled(page)
  await rowAction(page, `Grace ${last}`, 'Edit')
  const edit = page.getByRole('dialog')
  const choice = edit.getByLabel('Greeting callers hear')
  await expect(choice).toHaveValue('')
  await choice.selectOption('personalized')
  await edit.getByLabel('The name it says').selectOption('display_name')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  let read = await (await api.get(`/api/v1/voicemail/mailboxes/${id}`)).json()
  expect(read.greeting_choice).toBe('personalized')
  expect(read.greeting_name).toBe('display_name')

  await rowAction(page, `Grace ${last}`, 'Edit')
  await page.getByRole('dialog').getByLabel('Greeting callers hear').selectOption('standard')
  await page.getByRole('dialog').getByRole('button', { name: 'Save Changes' }).click()
  await expect(page.getByRole('dialog')).toBeHidden()
  read = await (await api.get(`/api/v1/voicemail/mailboxes/${id}`)).json()
  expect(read.greeting_choice).toBe('standard')
  expect(read.greeting_heard).toBe('standard')
})

/** Every item of a paged list (`{data, next_cursor}`). */
async function all(api: import('@playwright/test').APIRequestContext, path: string) {
  const items: Record<string, unknown>[] = []
  let cursor: string | null = null
  do {
    const sep = path.includes('?') ? '&' : '?'
    const res = await api.get(`${path}${cursor ? `${sep}cursor=${encodeURIComponent(cursor)}` : ''}`)
    expect(res.status(), `GET ${path}`).toBe(200)
    const body = await res.json()
    if (Array.isArray(body)) return body as Record<string, unknown>[]
    items.push(...body.data)
    cursor = body.next_cursor ?? null
  } while (cursor)
  return items
}

/** The one item of a list whose `field` is `value`, or undefined. */
async function find(api: import('@playwright/test').APIRequestContext, path: string, field: string, value: string) {
  return (await all(api, path)).find((i) => i[field] === value)
}

/** Choose `option` in the radix Select that follows the label `label`. */
async function choose(scope: import('@playwright/test').Locator, label: string, option: string) {
  const field = scope.locator('div').filter({ has: scope.page().getByText(label, { exact: true }) }).last()
  await field.getByRole('combobox').click()
  await scope.page().getByRole('option', { name: option, exact: true }).click()
}

test('a device is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-phone')
  await page.goto('/admin/devices')
  await settled(page)
  await page.getByRole('button', { name: 'Add Device' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#c-name').fill(name)
  await choose(dialog, 'Device Type *', 'Softphone')
  await dialog.locator('#c-model').fill('T54W')
  await dialog.getByRole('button', { name: 'Add Device' }).click()
  const token = page.getByRole('dialog', { name: 'Provisioning token created' })
  if (await token.isVisible().catch(() => false)) await token.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByRole('dialog')).toBeHidden()
  let made = await find(api, '/api/v1/devices', 'name', name)
  expect(made, 'the device is listed').toBeTruthy()
  expect(made!.model).toBe('T54W')

  await rowAction(page, name, 'Edit')
  const edit = page.getByRole('dialog')
  await edit.locator('#e-name').fill(`${name}-desk`)
  await edit.locator('#e-model').fill('')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/devices', 'id', made!.id as string)
  expect(made!.name).toBe(`${name}-desk`)
  expect(made!.model ?? null).toBeNull()

  acceptConfirm(page)
  await rowAction(page, `${name}-desk`, 'Delete')
  await expect.poll(async () => find(api, '/api/v1/devices', 'id', made!.id as string)).toBeUndefined()
})

test('a number is made, changed and deleted', async ({ page, api }) => {
  const e164 = `+1555${Date.now().toString().slice(-7)}`
  await page.goto('/admin/phone-numbers')
  await settled(page)
  await page.getByRole('button', { name: 'Add Number' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#c-e164').fill(e164)
  await dialog.locator('#c-cnam').fill('FRONT DESK')
  await dialog.getByRole('button', { name: 'Add Number' }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/phone-numbers', 'e164', e164)
  expect(made, 'the number is listed').toBeTruthy()
  expect(made!.cnam).toBe('FRONT DESK')

  await rowAction(page, e164, 'Edit')
  const edit = page.getByRole('dialog')
  await edit.locator('#e-cnam').fill('')
  await edit.getByLabel('Block anonymous').check()
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/phone-numbers', 'e164', e164)
  expect(made!.cnam ?? null).toBeNull()
  expect(made!.block_anonymous).toBe(true)

  acceptConfirm(page)
  await rowAction(page, e164, 'Delete')
  await expect.poll(async () => find(api, '/api/v1/phone-numbers', 'e164', e164)).toBeUndefined()
})

/** The card (or row) holding `name` and an `Edit` button: the innermost. */
function holder(page: Page, name: string) {
  return page
    .locator('div, tr')
    .filter({ hasText: name })
    .filter({ has: page.getByTitle('Edit', { exact: true }) })
    .last()
}

test('a trunk is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-trunk')
  await page.goto('/admin/trunks')
  await settled(page)
  // A platform administrator picks the tenant the trunk is the tenant's in.
  await expect(page.getByTestId('pick-tenant')).toBeVisible()
  await pickTenant(page, api)
  await expect(page.getByTestId('tenant-badge')).toBeVisible()
  await expect(page.getByTestId('platform-trunks')).toBeVisible()
  await page.getByRole('button', { name: 'Add Trunk' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#name').fill(name)
  await dialog.locator('#hosts').fill('192.0.2.10:5060')
  await dialog.getByRole('button', { name: 'Create Trunk' }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/trunks', 'name', name)
  expect(made, 'the trunk is listed').toBeTruthy()

  await holder(page, name).getByTitle('Edit', { exact: true }).click()
  const edit = page.getByRole('dialog')
  await edit.locator('#description').fill('Carrier A, primary')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/trunks', 'name', name)
  expect(made!.description).toBe('Carrier A, primary')

  await holder(page, name).getByTitle('Delete', { exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/trunks', 'name', name)).toBeUndefined()
})

test('a queue is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-queue')
  await page.goto('/admin/queues')
  await settled(page)
  await page.getByRole('button', { name: 'Add Queue' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#name').fill(name)
  await dialog.locator('#extension').fill(`6${Math.floor(Math.random() * 900 + 100)}`)
  await dialog.getByRole('button', { name: 'Create Queue' }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/queue-configs', 'name', name)
  expect(made, 'the queue is listed').toBeTruthy()

  await holder(page, name).getByTitle('Edit', { exact: true }).click()
  const edit = page.getByRole('dialog')
  await choose(edit, 'Distribution Strategy', 'Longest Idle')
  await edit.locator('#max_wait_time_secs').fill('120')
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/queue-configs', 'name', name)
  expect(made!.max_wait_time_secs).toBe(120)
  expect(String(made!.strategy)).toMatch(/longest/i)

  await holder(page, name).getByTitle('Delete', { exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/queue-configs', 'name', name)).toBeUndefined()
})

test('a flow is made, changed and deleted', async ({ page, api }) => {
  const id = unique('ui-flow')
  await page.goto('/admin/flows')
  await settled(page)
  await page.getByRole('button', { name: 'New Flow' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#flow-id').fill(id)
  await dialog.locator('#flow-name').fill('Main menu')
  await dialog.getByRole('button', { name: 'Create & Edit' }).click()
  await expect(page).toHaveURL(new RegExp(`/admin/flows/${id}$`))
  await settled(page)
  expect((await (await api.get(`/api/v1/flows/${id}`)).json()).name).toBe('Main menu')

  await page.getByLabel('Flow name').fill('Main menu, after hours')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await expect.poll(async () => (await (await api.get(`/api/v1/flows/${id}`)).json()).name).toBe('Main menu, after hours')

  await page.getByTitle('Back to the flows').click()
  await settled(page)
  acceptConfirm(page)
  await rowAction(page, 'Main menu, after hours', 'Delete')
  await expect.poll(async () => (await api.get(`/api/v1/flows/${id}`)).status()).toBe(404)
})

test('a schedule is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-hours')
  await page.goto('/admin/schedules')
  await settled(page)
  await page.getByRole('button', { name: 'New Schedule' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('textbox').first().fill(name)
  await dialog.getByPlaceholder('America/New_York').fill('Europe/London')
  await dialog.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/schedules', 'name', name)
  expect(made, 'the schedule is listed').toBeTruthy()
  expect(made!.timezone).toBe('Europe/London')

  await holder(page, name).getByTitle('Edit', { exact: true }).click()
  const edit = page.getByRole('dialog')
  await edit.getByPlaceholder('America/New_York').fill('America/Chicago')
  await edit.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/schedules', 'name', name)
  expect(made!.timezone).toBe('America/Chicago')

  acceptConfirm(page)
  await holder(page, name).getByTitle('Delete', { exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/schedules', 'name', name)).toBeUndefined()
})

test('a prompt is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-prompt')
  await page.goto('/admin/prompts')
  await settled(page)
  await page.getByRole('button', { name: 'New Prompt' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#name').fill(name)
  await dialog.locator('#tts_text').fill('Thank you for calling.')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/prompts', 'name', name)
  expect(made, 'the prompt is listed').toBeTruthy()

  await rowAction(page, name, 'Edit')
  const edit = page.getByRole('dialog')
  await edit.locator('#edit-description').fill('Played before the menu')
  await edit.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/prompts', 'name', name)
  expect(made!.description).toBe('Played before the menu')

  acceptConfirm(page)
  await rowAction(page, name, 'Delete')
  await expect.poll(async () => find(api, '/api/v1/prompts', 'name', name)).toBeUndefined()
})

test('a system prompt is overridden and restored', async ({ page, api }) => {
  // The platform's prompt (the static token is a platform administrator's).
  const name = unique('ui-system')
  const made = await api.post('/api/v1/prompts', {
    data: { name, source_type: 'tts', tts_text: 'Goodbye.', system: true },
  })
  expect(made.status()).toBe(201)
  const system = await made.json()

  await page.goto('/admin/prompts')
  await settled(page)
  // A tenant administrator can only override it.
  const r = await row(page, name)
  await r.getByRole('button', { name: 'Actions' }).click()
  await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toHaveCount(0)
  await page.getByRole('menuitem', { name: 'Override', exact: true }).click()
  await expect.poll(async () => (await find(api, '/api/v1/prompts', 'overrides', system.id))?.name).toBe(name)

  // The tenant's copy is listed beside it; deleting the copy restores it.
  const copy = page.getByRole('row').filter({ hasText: name }).filter({ hasText: /Override(?!n)/ })
  await expect(copy).toHaveCount(1)
  acceptConfirm(page)
  await copy.getByRole('button', { name: 'Actions' }).click()
  await page.getByRole('menuitem', { name: 'Restore system prompt', exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/prompts', 'overrides', system.id)).toBeUndefined()
})

test('a webhook is made, changed and deleted', async ({ page, api }) => {
  const url = `http://127.0.0.1:9/${unique('ui-hook')}`
  await page.goto('/admin/webhooks')
  await settled(page)
  await page.getByRole('button', { name: 'Add Webhook' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#webhook-url').fill(url)
  await dialog.locator('#webhook-description').fill('The CRM')
  await dialog.getByRole('button', { name: 'Save', exact: true }).click()
  const secret = page.getByRole('dialog', { name: 'Webhook created' })
  await expect(secret).toBeVisible()
  await expect(secret.getByRole('textbox')).toHaveValue(/^whsec_/)
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toBeHidden()
  let made = await find(api, '/api/v1/webhooks', 'url', url)
  expect(made, 'the webhook is listed').toBeTruthy()

  await holder(page, url).getByTitle('Edit', { exact: true }).click()
  const edit = page.getByRole('dialog')
  await edit.locator('#webhook-description').fill('The CRM, staging')
  await edit.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/webhooks', 'url', url)
  expect(made!.description).toBe('The CRM, staging')

  acceptConfirm(page)
  await holder(page, url).getByTitle('Delete', { exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/webhooks', 'url', url)).toBeUndefined()
})

test('an identity provider is made, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-idp')
  await page.goto('/admin/sso')
  await settled(page)
  await page.getByRole('button', { name: 'Add provider' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#idp-name').fill(name)
  await dialog.locator('#idp-issuer').fill('https://accounts.google.com')
  await dialog.locator('#idp-client').fill('fcp-ui-test')
  await dialog.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/identity-providers', 'name', name)
  expect(made, 'the provider is listed').toBeTruthy()

  await holder(page, name).getByTitle('Edit', { exact: true }).click()
  const edit = page.getByRole('dialog')
  await edit.locator('#idp-client').fill('fcp-ui-test-2')
  await edit.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/identity-providers', 'name', name)
  expect(JSON.stringify(made)).toContain('fcp-ui-test-2')

  acceptConfirm(page)
  await holder(page, name).getByTitle('Delete', { exact: true }).click()
  await expect.poll(async () => find(api, '/api/v1/identity-providers', 'name', name)).toBeUndefined()
})

test('a location is made with its emergency contacts, changed and deleted', async ({ page, api }) => {
  const name = unique('ui-site')
  await page.goto('/admin/locations')
  await settled(page)
  await page.getByRole('button', { name: 'Add Location' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('#f-name').fill(name)
  // Security, rung and allowed to listen in, and the front desk by email.
  await dialog.getByRole('button', { name: 'Add contact' }).click()
  await dialog.getByRole('combobox', { name: 'Contact kind' }).click()
  await page.getByRole('option', { name: 'Call', exact: true }).click()
  await dialog.getByLabel('Contact target').fill('2001')
  await dialog.getByLabel('Contact name').fill('Security')
  await dialog.getByLabel('May listen in').click()
  await dialog.getByRole('button', { name: 'Add contact' }).click()
  await dialog.getByLabel('Contact target').nth(1).fill('desk@example.com')
  await dialog.getByLabel("Tell the tenant's contacts too").click()
  await dialog.getByRole('button', { name: 'Add Location' }).click()
  await expect(dialog).toBeHidden()
  let made = await find(api, '/api/v1/locations', 'name', name)
  expect(made, 'the location is listed').toBeTruthy()
  expect(made!.notify_tenant_contacts).toBe(false)
  expect(made!.emergency_contacts).toEqual([
    { kind: 'call', target: '2001', name: 'Security', listen: true },
    { kind: 'email', target: 'desk@example.com', listen: false },
  ])

  await rowAction(page, name, 'Edit')
  const edit = page.getByRole('dialog')
  await expect(edit.getByLabel('Contact target')).toHaveCount(2)
  await edit.getByRole('button', { name: 'Remove contact' }).nth(1).click()
  await edit.getByRole('button', { name: 'Save Changes' }).click()
  await expect(edit).toBeHidden()
  made = await find(api, '/api/v1/locations', 'name', name)
  expect(made!.emergency_contacts).toHaveLength(1)
  expect(made!.emergency_contacts[0].listen).toBe(true)

  acceptConfirm(page)
  await rowAction(page, name, 'Delete')
  await expect.poll(async () => find(api, '/api/v1/locations', 'name', name)).toBeUndefined()
})
