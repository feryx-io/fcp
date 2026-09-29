// Against a running admin: fcp-admin's tests/sdks.rs starts one on a
// throwaway database and runs this with FCP_SDK_URL and FCP_SDK_TOKEN.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createFcpClient, ok, paginate, all, FcpError } from '../dist/index.js'

const url = process.env.FCP_SDK_URL
const token = process.env.FCP_SDK_TOKEN
const live = url && token ? test : test.skip

live('lists page by cursor until every item is read', async () => {
  const fcp = createFcpClient({ baseUrl: url, token })
  const names = ['ts-a', 'ts-b', 'ts-c', 'ts-d', 'ts-e']
  for (const username of names) {
    await ok(fcp.POST('/api/v1/users', { body: { username, password: 'Str0ng-Passw0rd!', role: 'end_user' } }))
  }
  const seen = []
  let pages = 0
  for await (const user of paginate(async (cursor) => {
    pages += 1
    return ok(fcp.GET('/api/v1/users', { params: { query: { limit: 2, cursor } } }))
  })) {
    seen.push(user.username)
  }
  for (const n of names) assert.ok(seen.includes(n), `${n} in ${seen}`)
  assert.ok(pages >= 3, `paged ${pages} times`)
  const everyone = await all((cursor) => ok(fcp.GET('/api/v1/users', { params: { query: { cursor } } })))
  assert.equal(everyone.length, seen.length)
})

live('a refusal is an FcpError with the stable code and a request id', async () => {
  const fcp = createFcpClient({ baseUrl: url, token })
  await assert.rejects(
    ok(fcp.GET('/api/v1/users/{username}', { params: { path: { username: 'nobody-at-all' } } })),
    (e) => {
      assert.ok(e instanceof FcpError)
      assert.equal(e.status, 404)
      assert.equal(e.code, 'not_found')
      assert.ok(e.requestId && e.requestId.length > 8, 'a request id')
      return true
    },
  )
  const stranger = createFcpClient({ baseUrl: url, token: 'fcp_not-a-token' })
  await assert.rejects(ok(fcp.GET('/api/v1/cdrs')).then(() => ok(stranger.GET('/api/v1/users'))), (e) => e.code === 'unauthorized')
})

live('a scoped token made through the SDK reaches what it names', async () => {
  const fcp = createFcpClient({ baseUrl: url, token })
  const made = await ok(fcp.POST('/api/v1/api-tokens', { body: { name: 'ts-sdk', permissions: ['user_read'] } }))
  const scoped = createFcpClient({ baseUrl: url, token: made.secret })
  const users = await ok(scoped.GET('/api/v1/users'))
  assert.ok(Array.isArray(users.data))
  await assert.rejects(ok(scoped.GET('/api/v1/devices')), (e) => e.code === 'forbidden')
})
