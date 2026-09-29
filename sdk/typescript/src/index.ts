/**
 * The FCP admin API's TypeScript client (docs/DEVELOPER_SURFACE.md §3,
 * phase 4): a typed client generated from the OpenAPI contract, with
 * authentication, retries on 429 and 503, the API's errors, list paging and
 * webhook verification.
 */
export { createFcpClient, ok, retrying, type FcpClient, type FcpClientOptions } from './client.js'
export { FcpError, type ErrorBody } from './errors.js'
export { paginate, all, type PageOf } from './paginate.js'
export { verifyWebhook } from './webhook.js'
export type { paths, components, operations } from './schema.js'

import type { components } from './schema.js'
/** The API's schemas by name: `Schemas['UserResponse']`. */
export type Schemas = components['schemas']
