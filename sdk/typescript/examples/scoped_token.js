// A scoped API token (docs/DEVELOPER_GUIDE.md §2): made with your own
// token, holding only what it names, and used by the integration it is for.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({ url, token }) {
  const fcp = createFcpClient({ baseUrl: url, token })

  // The secret is shown once: store it where the integration reads it.
  const made = await ok(fcp.POST('/api/v1/api-tokens', {
    body: { name: 'crm-sync', permissions: ['user_read'] },
  }))

  const crm = createFcpClient({ baseUrl: url, token: made.secret })
  const users = await ok(crm.GET('/api/v1/users', { params: { query: { limit: 50 } } }))
  return { token: made.token.id, users: users.data.map((u) => u.username) }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
