import { WIRE, WorkletCore } from './core'

declare const sampleRate: number
declare function registerProcessor(name: string, ctor: unknown): void
declare class AudioWorkletProcessor {
  readonly port: MessagePort
}

interface Options {
  processorOptions: { delay16: number; tapRaw?: boolean }
}

// resampling and queues only: the models run in a worker reached by a direct MessagePort, never the main thread
class AlkoveProcessor extends AudioWorkletProcessor {
  private core: WorkletCore
  private worker: MessagePort | null = null
  private pool: ArrayBuffer[] = []
  private dead = false

  constructor(options: Options) {
    super()
    this.core = new WorkletCore(sampleRate, options.processorOptions.delay16, () => Date.now(), (m) => this.port.postMessage(m), !!options.processorOptions.tapRaw)
    // the raw path already runs at the processed path's delay
    this.port.postMessage({ type: 'latency', ms: this.core.latencyMs })
    this.port.onmessage = (e: MessageEvent) => {
      const m = e.data
      if (m.type === 'connect') {
        this.worker = m.port as MessagePort
        this.worker.onmessage = (ev: MessageEvent) => {
          if (!(ev.data instanceof ArrayBuffer)) return this.core.pause(!!ev.data?.pause)
          const buf = ev.data
          this.core.receive(new Float32Array(buf))
          this.pool.push(buf)
        }
        this.core.onActivate = (at16) => this.worker!.postMessage({ clock: at16 })
        this.core.connect((hop) => {
          const buf = this.pool.pop() ?? new ArrayBuffer(WIRE * 4)
          new Float32Array(buf).set(hop)
          this.worker!.postMessage(buf, [buf])
        })
        this.port.postMessage({ type: 'latency', ms: this.core.latencyMs })
      } else if (m.type === 'stop') this.core.stop()
      else if (m.type === 'far') this.core.setFarActive(!!m.on)
      else if (m.type === 'dispose') this.dead = true
    }
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.dead) return false
    const out = outputs[0]?.[0]
    if (!out) return true
    this.core.process(inputs[0]?.[0] ?? null, inputs[1]?.[0] ?? null, out)
    for (let c = 1; c < (outputs[0]?.length ?? 0); c++) outputs[0]![c]!.set(out)
    return true
  }
}

registerProcessor('alkove', AlkoveProcessor)
