// A recording (whatever the browser made it: WebM/Opus from MediaRecorder,
// or a file) as 16 kHz mono 16-bit WAV, what the speech server hears
// (docs/TENANT_RECOGNITION.md R2b).

const RATE = 16_000

/** `blob` decoded, mixed to mono, resampled to 16 kHz and written as WAV. */
export async function toWav16k(blob: Blob): Promise<Blob> {
  const bytes = await blob.arrayBuffer()
  const decoder = new AudioContext()
  let decoded: AudioBuffer
  try {
    decoded = await decoder.decodeAudioData(bytes)
  } finally {
    void decoder.close()
  }
  const frames = Math.max(1, Math.ceil(decoded.duration * RATE))
  const offline = new OfflineAudioContext(1, frames, RATE)
  const source = offline.createBufferSource()
  source.buffer = decoded
  source.connect(offline.destination)
  source.start()
  const rendered = await offline.startRendering()
  return wavOf(rendered.getChannelData(0))
}

/** 16 kHz mono float samples as a 16-bit PCM WAV file. */
export function wavOf(samples: Float32Array): Blob {
  const data = samples.length * 2
  const out = new DataView(new ArrayBuffer(44 + data))
  const text = (at: number, s: string) => { for (let i = 0; i < s.length; i++) out.setUint8(at + i, s.charCodeAt(i)) }
  text(0, 'RIFF'); out.setUint32(4, 36 + data, true); text(8, 'WAVE')
  text(12, 'fmt '); out.setUint32(16, 16, true); out.setUint16(20, 1, true); out.setUint16(22, 1, true)
  out.setUint32(24, RATE, true); out.setUint32(28, RATE * 2, true); out.setUint16(32, 2, true); out.setUint16(34, 16, true)
  text(36, 'data'); out.setUint32(40, data, true)
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]))
    out.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true)
  }
  return new Blob([out.buffer], { type: 'audio/wav' })
}
