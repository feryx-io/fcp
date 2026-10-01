// A flow that listens (docs/DEVELOPER_GUIDE.md §4): a menu answered by
// speech or digits, the caller's words posted to your service, and a
// number that runs it.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({
  url, token, flowId = 'front-desk', number = '+15125550170', crm = 'https://crm.example.com',
}) {
  const fcp = createFcpClient({ baseUrl: url, token })

  const flow = await ok(fcp.POST('/api/v1/flows', {
    body: {
      id: flowId,
      name: 'Front desk',
      initial_node: 'answer',
      nodes: {
        answer: { type: 'answer', next: 'menu' },
        menu: {
          type: 'gather',
          input: 'dtmf_speech',
          prompt: { type: 'say', text: 'Say sales or support, or press 1 or 2.' },
          num_digits: 1,
          timeout: 5,
          hints: ['sales', 'support'],
          transitions: { 1: 'sales', 2: 'support' },
          speech_transitions: { sales: 'sales', support: 'support' },
          on_timeout: 'menu',
        },
        sales: {
          type: 'http',
          method: 'POST',
          url: `${crm}/leads`,
          send_variables: true, // caller, speech, speech_confidence, …
          response_variable: 'crm',
          next: 'to-sales',
          on_error: 'to-sales',
        },
        'to-sales': { type: 'dial', destination: '{{crm.owner_extension}}', next: 'bye', on_error: 'support' },
        support: { type: 'queue', queue_name: 'support', next_on_answer: 'bye' },
        bye: { type: 'hangup' },
      },
    },
  }))

  // A number whose calls run the flow.
  const phone = await ok(fcp.POST('/api/v1/phone-numbers', { body: { e164: number } }))
  await ok(fcp.POST('/api/v1/line-bindings', {
    body: { phone_number_id: phone.id, binding_type: 'ivr', flow_id: flow.id },
  }))
  return { flow: flow.id, version: flow.version, number: phone.e164 }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
