// A call, live (docs/DEVELOPER_GUIDE.md §5): put the caller on hold, take
// them back, and end the call. The call's id comes from its events: the
// `call_id` of the `call.initiated` webhook delivery.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({ url, token, callId }) {
  const fcp = createFcpClient({ baseUrl: url, token })
  const call = { params: { path: { call_id: callId } } }

  // The caller hears the hold music; the other party no longer hears them.
  const held = await ok(fcp.POST('/api/v1/calls/{call_id}/hold', { ...call, body: { party: 'caller' } }))
  await new Promise((resolve) => setTimeout(resolve, 1000))
  const resumed = await ok(fcp.POST('/api/v1/calls/{call_id}/resume', call))

  // Elsewhere: transfer, redirect into a flow, play a prompt, say text —
  // each one POST to /api/v1/calls/{call_id}/<action> (docs/CALL_CONTROL.md).
  const ended = await ok(fcp.POST('/api/v1/calls/{call_id}/hangup', call))
  return [held.status, resumed.status, ended.status]
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN, callId: process.argv[2] }))
}
