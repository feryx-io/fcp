import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { createFcpClient, ok, FcpError } from '../dist/index.js'

/** A server answering from `answers` in turn, remembering what it was sent. */
async function serve(answers) {
  const seen = []
  const server = createServer((req, res) => {
    let body = ''
    req.on('data', (c) => (body += c))
    req.on('end', () => {
      seen.push({ method: req.method, url: req.url, body, headers: req.headers })
      const [status, payload, headers = {}] = answers.shift() ?? [500, {}]
      res.writeHead(status, { 'content-type': 'application/json', 'x-request-id': 'req-1', ...headers })
      res.end(JSON.stringify(payload))
    })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address()
  return { url: `http://127.0.0.1:${port}`, seen, close: () => server.close() }
}

const LIMITED = [429, { error: 'Too Many Requests', code: 'rate_limited', details: 'slow down' }, { 'retry-after': '0' }]

test('a 429 is retried as Retry-After says, the body sent again each time', async () => {
  const srv = await serve([LIMITED, LIMITED, [201, { id: 'user-1', username: 'ada' }]])
  try {
    const fcp = createFcpClient({ baseUrl: srv.url, token: 'fcp_t', tenantId: 'tenant-x' })
    const made = await ok(fcp.POST('/api/v1/users', { body: { username: 'ada', password: 'pw-123456789' } }))
    assert.equal(made.username, 'ada')
    assert.equal(srv.seen.length, 3)
    for (const s of srv.seen) {
      assert.equal(JSON.parse(s.body).username, 'ada')
      assert.equal(s.headers.authorization, 'Bearer fcp_t')
      assert.equal(s.headers['x-fcp-tenant-id'], 'tenant-x')
    }
  } finally {
    srv.close()
  }
})

test('past its retries, the API error is thrown with its code and request id', async () => {
  const srv = await serve([LIMITED, LIMITED])
  try {
    const fcp = createFcpClient({ baseUrl: srv.url, token: 'fcp_t', retries: 1 })
    await assert.rejects(ok(fcp.GET('/api/v1/users')), (e) => {
      assert.ok(e instanceof FcpError)
      assert.equal(e.status, 429)
      assert.equal(e.code, 'rate_limited')
      assert.equal(e.details, 'slow down')
      assert.equal(e.requestId, 'req-1')
      return true
    })
    assert.equal(srv.seen.length, 2)
  } finally {
    srv.close()
  }
})

test('any other error is not retried', async () => {
  const srv = await serve([[404, { error: 'Not Found', code: 'not_found', details: 'no such user' }]])
  try {
    const fcp = createFcpClient({ baseUrl: srv.url, token: 'fcp_t' })
    await assert.rejects(
      ok(fcp.GET('/api/v1/users/{username}', { params: { path: { username: 'nobody' } } })),
      (e) => e instanceof FcpError && e.code === 'not_found',
    )
    assert.equal(srv.seen.length, 1)
  } finally {
    srv.close()
  }
})
