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

live('a webhook delivery verifies with verifyWebhook and a replay later is stale', async () => {
  const { createServer } = await import('node:http')
  const { verifyWebhook } = await import('../dist/index.js')
  const seen = []
  const server = createServer((req, res) => {
    let body = ''
    req.on('data', (chunk) => { body += chunk })
    req.on('end', () => {
      seen.push({ signature: req.headers['x-fcp-signature'], body })
      res.end('ok')
    })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  try {
    const fcp = createFcpClient({ baseUrl: url, token })
    const made = await ok(fcp.POST('/api/v1/webhooks', {
      body: { url: `http://127.0.0.1:${server.address().port}/hook`, events: ['call.*'] },
    }))
    assert.ok(made.secret.startsWith('whsec_'))
    const id = made.webhook.id
    const outcome = await ok(fcp.POST('/api/v1/webhooks/{id}/test', { params: { path: { id } }, body: {} }))
    assert.equal(outcome.delivered, true, JSON.stringify(outcome))
    const { signature, body } = seen.at(-1)
    assert.ok(verifyWebhook(made.secret, signature, body), signature)
    assert.equal(JSON.parse(body).event_type, 'webhook.test')
    const t = Number(/t=(\d+)/.exec(signature)[1])
    assert.equal(verifyWebhook(made.secret, signature, body, { now: t + 600 }), false, 'stale')
    assert.equal(verifyWebhook('whsec_wrong', signature, body), false)
    await ok(fcp.DELETE('/api/v1/webhooks/{id}', { params: { path: { id } } }))
  } finally {
    server.close()
  }
})
