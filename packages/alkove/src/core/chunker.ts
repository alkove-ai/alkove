// any-size input cut into the model's step; the remainder waits for the next push
export class Chunker {
  private buf: Float32Array
  private fill = 0

  constructor(readonly size: number) {
    this.buf = new Float32Array(size)
  }

  push(pcm: Float32Array, step: (chunk: Float32Array) => void): void {
    let i = 0
    while (i < pcm.length) {
      const n = Math.min(this.size - this.fill, pcm.length - i)
      this.buf.set(pcm.subarray(i, i + n), this.fill)
      this.fill += n
      i += n
      if (this.fill === this.size) {
        this.fill = 0
        step(this.buf)
      }
    }
  }

  get pending(): number {
    return this.fill
  }

  reset(): void {
    this.fill = 0
  }
}

// a growable list of Float32 samples, for results whose length is not known ahead
export class Samples {
  private buf = new Float32Array(16384)
  length = 0

  push(pcm: ArrayLike<number>): void {
    if (this.length + pcm.length > this.buf.length) {
      const next = new Float32Array(Math.max(2 * this.buf.length, this.length + pcm.length))
      next.set(this.buf.subarray(0, this.length))
      this.buf = next
    }
    this.buf.set(pcm, this.length)
    this.length += pcm.length
  }

  take(): Float32Array {
    return this.buf.slice(0, this.length)
  }
}
