import { Resampler } from '../dsp/resample'
import { RATE } from '../manifest'

export interface OutputOptions {
  format: 'f32' | 's16'
  rate: number
  chunkMs: number
}

export class Packer {
  private resampler: Resampler
  private buf: Float32Array
  private fill = 0

  constructor(
    readonly opts: OutputOptions,
    private send: (chunk: ArrayBuffer) => void,
  ) {
    this.resampler = new Resampler(RATE, opts.rate)
    this.buf = new Float32Array(Math.round((opts.rate * opts.chunkMs) / 1000))
  }

  push(pcm: Float32Array): void {
    this.resampler.push(pcm, (x) => {
      this.buf[this.fill++] = x
      if (this.fill === this.buf.length) {
        this.send(encode(this.buf, this.opts.format))
        this.fill = 0
      }
    })
  }
}

export function encode(pcm: Float32Array, format: 'f32' | 's16'): ArrayBuffer {
  if (format === 'f32') return pcm.slice().buffer
  const out = new Int16Array(pcm.length)
  for (let i = 0; i < pcm.length; i++) {
    const x = Math.max(-1, Math.min(1, pcm[i]!))
    out[i] = x < 0 ? x * 32768 : x * 32767
  }
  return out.buffer
}
