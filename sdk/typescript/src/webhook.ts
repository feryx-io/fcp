import { createHmac, timingSafeEqual } from 'node:crypto'

/**
 * Whether a webhook delivery is genuine: `X-FCP-Signature` is
 * `t=<unix seconds>,v1=<hex HMAC-SHA256 of "<t>.<body>">` under the
 * endpoint's secret, and `t` is within `toleranceSecs` of now, so a
 * captured delivery cannot be replayed later. Pass the body exactly as
 * received, before parsing it.
 */
export function verifyWebhook(
  secret: string,
  signatureHeader: string,
  body: string,
  options: { toleranceSecs?: number; now?: number } = {},
): boolean {
  const parts = new Map<string, string[]>()
  for (const piece of signatureHeader.split(',')) {
    const i = piece.indexOf('=')
    if (i < 0) continue
    const key = piece.slice(0, i).trim()
    parts.set(key, [...(parts.get(key) ?? []), piece.slice(i + 1).trim()])
  }
  const t = Number(parts.get('t')?.[0])
  if (!Number.isInteger(t)) return false
  const now = options.now ?? Math.floor(Date.now() / 1000)
  if (Math.abs(now - t) > (options.toleranceSecs ?? 300)) return false
  const expected = createHmac('sha256', secret).update(`${t}.${body}`).digest()
  // Any v1 signature may match: a secret being rotated signs with both.
  return (parts.get('v1') ?? []).some((hex) => {
    const given = Buffer.from(hex, 'hex')
    return given.length === expected.length && timingSafeEqual(given, expected)
  })
}
