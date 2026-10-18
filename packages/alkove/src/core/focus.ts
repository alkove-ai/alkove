import { FOCUS } from '../manifest'
import { Chunker, Samples } from './chunker'
import type { StreamNet } from './engine'

// keeps the closest voice, or the voice of a print; 16 kHz in, 16 kHz out, `delay` samples late
export class Focus {
  static readonly chunk = FOCUS.chunk
  static readonly delay = FOCUS.lag
  readonly delay = FOCUS.lag
  private chunker = new Chunker(FOCUS.chunk)

  constructor(private net: StreamNet) {}

  // the model never learned a print changing mid-stream: a new one starts from a fresh state
  setPrint(embedding: ArrayLike<number> | null): void {
    const p = this.net.f32('print')
    if (embedding) p.set(embedding)
    else p.fill(0)
    this.net.reset()
  }

  reset(): void {
    this.net.reset()
  }

  step(chunk: Float32Array, out: Float32Array): void {
    this.net.f32('audio').set(chunk)
    this.net.run()
    out.set(this.net.f32('audio_out'))
  }

  push(pcm: Float32Array): Float32Array {
    const out = new Samples()
    const hop = new Float32Array(FOCUS.chunk)
    this.chunker.push(pcm, (c) => {
      this.step(c, hop)
      out.push(hop)
    })
    return out.take()
  }

  release(): void {
    this.net.release()
  }
}
