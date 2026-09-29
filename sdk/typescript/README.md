# @fcp/sdk (TypeScript)

The FCP admin API's client for Node 20+ and browsers. It has two parts:
- a typed client over every route, generated from the API's OpenAPI
  contract (`docs/openapi.json`, via `openapi-typescript` and
  `openapi-fetch`);
- a thin hand-written layer:
  - the token and the tenant;
  - retries on 429 and 503;
  - the API's errors as `FcpError`;
  - list paging;
  - webhook verification.

```ts
import { createFcpClient, ok, paginate, FcpError, verifyWebhook } from '@fcp/sdk'

const fcp = createFcpClient({ baseUrl: 'https://admin.example.com', token: 'fcp_…' })

await ok(fcp.POST('/api/v1/users', { body: { username: 'ada', password: '…', role: 'end_user' } }))

for await (const user of paginate((cursor) =>
  ok(fcp.GET('/api/v1/users', { params: { query: { cursor, limit: 500 } } })))) {
  console.log(user.username)
}

try {
  await ok(fcp.GET('/api/v1/users/{username}', { params: { path: { username: 'nobody' } } }))
} catch (e) {
  if (e instanceof FcpError) console.log(e.status, e.code, e.details, e.requestId)
}
```

Paths, parameters, bodies and responses are all typed from the contract.
`Schemas['UserResponse']` names any schema.

## How it behaves

- **Tokens.** An `fcp_…` token reaches only what its permissions name
  (docs/API_TOKENS.md).
- **Tenants.** `tenantId` picks the tenant for the static token or a
  platform admin's session.
- **Retries.** A 429 or a 503 is retried up to `retries` times (3 by
  default), waiting as `Retry-After` says.
- **Webhooks.** `verifyWebhook(secret, signatureHeader, rawBody)` is true
  only for a delivery signed with the endpoint's secret in the last five
  minutes. It uses `node:crypto`.

## Developing it

- `npm test` builds and runs the tests.
- The live ones need a running admin, which `cargo test -p fcp-admin --test
  sdks` starts for them.
- When the API changes, `npm run gen` regenerates `src/schema.ts` from the
  contract. CI fails if the committed file is stale.
- The version's major and minor follow the FCP release. The patch is set
  when it is published.
