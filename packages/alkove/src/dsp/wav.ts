// 16-bit PCM WAV, one channel per array
export function encodeWav(channels: Float32Array[], rate: number): Uint8Array {
  const n = channels[0]?.length ?? 0
  const ch = channels.length
  const data = n * ch * 2
  const out = new Uint8Array(44 + data)
  const v = new DataView(out.buffer)
  const text = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)))
  text(0, 'RIFF')
  v.setUint32(4, 36 + data, true)
  text(8, 'WAVE')
  text(12, 'fmt ')
  v.setUint32(16, 16, true)
  v.setUint16(20, 1, true)
  v.setUint16(22, ch, true)
  v.setUint32(24, rate, true)
  v.setUint32(28, rate * ch * 2, true)
  v.setUint16(32, ch * 2, true)
  v.setUint16(34, 16, true)
  text(36, 'data')
  v.setUint32(40, data, true)
  let o = 44
  for (let i = 0; i < n; i++)
    for (let c = 0; c < ch; c++) {
      const x = Math.max(-1, Math.min(1, channels[c]![i]!))
      v.setInt16(o, x < 0 ? x * 32768 : x * 32767, true)
      o += 2
    }
  return out
}

// PCM 8 / 16 / 24 / 32 bits or float 32 / 64, channels averaged to mono
export function decodeWav(bytes: Uint8Array): { pcm: Float32Array; rate: number } {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const tag = (o: number) => String.fromCharCode(...bytes.subarray(o, o + 4))
  if (tag(0) !== 'RIFF' || tag(8) !== 'WAVE') throw new TypeError('audio.format: not a WAV file')
  let o = 12
  let fmt: { format: number; ch: number; rate: number; bits: number } | null = null
  while (o + 8 <= bytes.length) {
    const id = tag(o)
    const size = v.getUint32(o + 4, true)
    const body = o + 8
    if (id === 'fmt ') {
      let format = v.getUint16(body, true)
      if (format === 0xfffe) format = v.getUint16(body + 24, true)
      fmt = { format, ch: v.getUint16(body + 2, true), rate: v.getUint32(body + 4, true), bits: v.getUint16(body + 14, true) }
    } else if (id === 'data' && fmt) {
      const { format, ch, rate, bits } = fmt
      const width = bits / 8
      const n = Math.floor(Math.min(size, bytes.length - body) / (width * ch))
      const pcm = new Float32Array(n)
      for (let i = 0; i < n; i++) {
        let s = 0
        for (let c = 0; c < ch; c++) {
          const p = body + (i * ch + c) * width
          if (format === 3) s += bits === 64 ? v.getFloat64(p, true) : v.getFloat32(p, true)
          else if (bits === 8) s += (v.getUint8(p) - 128) / 128
          else if (bits === 16) s += v.getInt16(p, true) / 32768
          else if (bits === 24) s += ((v.getUint8(p) | (v.getUint8(p + 1) << 8) | (v.getInt8(p + 2) << 16)) / 8388608)
          else if (bits === 32) s += v.getInt32(p, true) / 2147483648
          else throw new TypeError(`audio.format: ${bits}-bit WAV`)
        }
        pcm[i] = s / ch
      }
      return { pcm, rate }
    }
    o = body + size + (size & 1)
  }
  throw new TypeError('audio.format: WAV without data')
}
