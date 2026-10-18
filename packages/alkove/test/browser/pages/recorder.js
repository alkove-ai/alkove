// records its input in batches, each with the context frame it starts at
const BATCH = 4096

class Recorder extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buf = new Float32Array(BATCH)
    this.fill = 0
    this.frame = -1
  }

  process(inputs) {
    const x = inputs[0] && inputs[0][0]
    if (this.frame < 0) this.frame = currentFrame
    for (let i = 0; i < 128; i++) {
      this.buf[this.fill++] = x ? x[i] : 0
      if (this.fill === BATCH) {
        this.port.postMessage({ frame: this.frame, pcm: this.buf }, [this.buf.buffer])
        this.buf = new Float32Array(BATCH)
        this.fill = 0
        this.frame = currentFrame + i + 1
      }
    }
    return true
  }
}

registerProcessor('alkove-test-recorder', Recorder)
