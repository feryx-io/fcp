// The guide's examples (docs/DEVELOPER_GUIDE.md), run against the live
// stack fcp-admin's tests/sdks.rs starts: an admin, and a call manager
// with a call up whose id is FCP_SDK_CALL_ID.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { run as token } from '../examples/scoped_token.js'
import { run as webhook } from '../examples/webhook_receiver.js'
import { run as flow } from '../examples/listening_flow.js'
import { run as call } from '../examples/live_call.js'

const url = process.env.FCP_SDK_URL
const key = process.env.FCP_SDK_TOKEN
const callId = process.env.FCP_SDK_CALL_ID
const live = url && key ? test : test.skip
const withCall = url && key && callId ? test : test.skip

live('example: a scoped token', async () => {
  const out = await token({ url, token: key })
  assert.equal(typeof out.token, 'string')
  assert.ok(Array.isArray(out.users))
})

live('example: a webhook, verified', async () => {
  const out = await webhook({ url, token: key })
  assert.equal(out.delivered, true, JSON.stringify(out))
  assert.deepEqual(out.received, ['webhook.test'])
})

live('example: a flow that listens, and its number', async () => {
  const out = await flow({ url, token: key, flowId: 'front-desk-ts', number: '+15125550171' })
  assert.equal(out.flow, 'front-desk-ts')
  assert.equal(out.version, 1)
  assert.equal(out.number, '+15125550171')
})

withCall('example: a live call held, resumed and ended', async () => {
  assert.deepEqual(await call({ url, token: key, callId }), ['held', 'resumed', 'ended'])
})
