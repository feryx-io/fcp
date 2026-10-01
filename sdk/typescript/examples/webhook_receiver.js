// A webhook (docs/DEVELOPER_GUIDE.md §3): the tenant's events posted to
// your URL, each signed, and checked with verifyWebhook before it is
// believed.
import { createServer } from 'node:http'
import { createFcpClient, ok, verifyWebhook } from '../dist/index.js'

export async function run({ url, token, listen = { host: '127.0.0.1', port: 0 } }) {
  const fcp = createFcpClient({ baseUrl: url, token })
  const received = []
  let secret

  // Your receiver: refuse anything whose signature does not verify, or
  // that is older than five minutes (a replay).
  const server = createServer((req, res) => {
    let body = ''
    req.on('data', (chunk) => { body += chunk })
    req.on('end', () => {
      if (!verifyWebhook(secret, req.headers['x-fcp-signature'], body)) {
        res.statusCode = 401
        return res.end()
      }
      received.push(JSON.parse(body))
      res.end('ok')
    })
  })
  await new Promise((resolve) => server.listen(listen.port, listen.host, resolve))

  try {
    const made = await ok(fcp.POST('/api/v1/webhooks', {
      body: {
        url: `http://${listen.host}:${server.address().port}/fcp`,
        events: ['call.*', 'voicemail.transcribed'],
      },
    }))
    secret = made.secret // shown once, like a token's

    // Ask for a test delivery, signed like every other.
    const id = made.webhook.id
    const outcome = await ok(fcp.POST('/api/v1/webhooks/{id}/test', {
      params: { path: { id } },
      body: {},
    }))
    return { webhook: id, delivered: outcome.delivered, received: received.map((e) => e.event_type) }
  } finally {
    server.close()
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
