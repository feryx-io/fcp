import { test } from 'node:test'
import assert from 'node:assert/strict'
import { verifyWebhook } from '../dist/index.js'

// The vector every FCP webhook implementation checks against
// (sdk/python/tests/test_webhook.py, and the dispatcher's own tests).
const SECRET = 'whsec_test_vector'
const T = 1790000000
const BODY = '{"event":"call.started"}'
const V1 = '008976dcc4d8a2d92dbfe2c643bdb8f498825fa20a43baa8d0e7a0358ffa159e'

test('a genuine delivery verifies', () => {
  assert.equal(verifyWebhook(SECRET, `t=${T},v1=${V1}`, BODY, { now: T + 60 }), true)
})

test('a stale delivery does not, however genuine', () => {
  assert.equal(verifyWebhook(SECRET, `t=${T},v1=${V1}`, BODY, { now: T + 301 }), false)
  assert.equal(verifyWebhook(SECRET, `t=${T},v1=${V1}`, BODY, { now: T + 900, toleranceSecs: 1000 }), true)
})

test('a changed body, a wrong secret or a bad header does not', () => {
  assert.equal(verifyWebhook(SECRET, `t=${T},v1=${V1}`, BODY + ' ', { now: T }), false)
  assert.equal(verifyWebhook('whsec_other', `t=${T},v1=${V1}`, BODY, { now: T }), false)
  assert.equal(verifyWebhook(SECRET, `v1=${V1}`, BODY, { now: T }), false)
  assert.equal(verifyWebhook(SECRET, `t=${T},v1=zz`, BODY, { now: T }), false)
})

test('while a secret rotates, either signature does', () => {
  const header = `t=${T},v1=${'0'.repeat(64)},v1=${V1}`
  assert.equal(verifyWebhook(SECRET, header, BODY, { now: T }), true)
})
