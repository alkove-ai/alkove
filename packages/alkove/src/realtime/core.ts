import { exp } from '../dsp/math'
import { Resampler } from '../dsp/resample'

// the audio-thread side, kept free of worklet globals so Node can drive it with a fake clock
export const HOP = 256
// a hop on the wire: 256 samples, then the far end's quiet time, then the activation it belongs to
export const WIRE = HOP + 2
// in place of the activation: a raw-mode hop, for the app's audio only (agent input)
export const RAW_HOP = -1
const RATE16 = 16000
// output primed with this many hops: the worker's round trip
const PRIME_HOPS = 2
// starved this long without a break, or this much in a 10 s window: the worker can't keep up
const STARVED_CONTINUOUS_MS = 250
const STARVED_WINDOW_MS = 10_000
const STARVED_BUDGET_MS = 400
// raw after an overload, then the models again after 10 s, 20 s, 40 s…; back to 10 s once they held a minute
const RETRY_MS = 10_000
const RETRY_MAX_MS = 160_000
const STABLE_MS = 60_000
// a block under this peak may be dropped to pay back the latency starvation added
const QUIET = 0.005
const FADE_MS = 10
// far end heard over this RMS
const FAR_LEVEL = 0.003

export type CoreMessage =
  | { type: 'overload'; queueMs: number; sent: number; received: number }
  | { type: 'retry' }
  | { type: 'timing'; queueMs: [number, number]; debtMs: number; droppedMs: number; latencyMs: number }
  | { type: 'raw'; pcm: Float32Array }

class Queue {
  buf: Float32Array
  start = 0
  size = 0
  lost = 0

  constructor(capacity: number) {
    this.buf = new Float32Array(capacity)
  }

  push(x: number): void {
    // overflow only in pathological states: dropping the backlog keeps latency bounded
    if (this.size === this.buf.length) {
      this.lost += this.size
      this.start = this.size = 0
    }
    this.buf[(this.start + this.size) % this.buf.length] = x
    this.size++
  }

  pull(out: Float32Array, n: number): void {
    for (let i = 0; i < n; i++) out[i] = this.buf[(this.start + i) % this.buf.length]!
    this.skip(n)
  }

  skip(n: number): void {
    this.start = (this.start + n) % this.buf.length
    this.size -= n
  }

  peak(n: number): number {
    let m = 0
    for (let i = 0; i < n; i++) m = Math.max(m, Math.abs(this.buf[(this.start + i) % this.buf.length]!))
    return m
  }
}

export class WorkletCore {
  mode: 'raw' | 'active' = 'raw'
  private send: ((hop: Float32Array) => void) | null = null
  private down!: Resampler
  private up!: Resampler
  private in16 = new Queue(4096)
  private out!: Queue
  private hop = new Float32Array(WIRE)
  private epoch = 0
  private raw: Float32Array
  private rawPos = 0
  private mix = 0
  private fade: number
  private debt = 0
  private floor: number
  private starvedAt = 0
  private windowStart = 0
  private windowStarved = 0
  private sent = 0
  private received = 0
  private inFlight = 0
  private retryMs = RETRY_MS
  private retryAt = 0
  private activatedAt = 0
  private stopped = false
  private quanta = 0
  private stats = { min: Infinity, max: 0, dropped: 0 }
  private mins: number[] = []
  private windowMin = Infinity
  private farActive = 0
  farQuietMs = Infinity
  private frames = 0
  private silence = new Float32Array(0)
  private tap: { down: Resampler; in16: Queue; hop: Float32Array } | null = null
  // the 16 kHz sample the next hop starts at: hops pause in raw mode, the worker's clock must not
  onActivate: ((at16: number) => void) | null = null

  // tapRaw: in raw mode the 16 kHz mic still goes out, to the worker when it is there, else to the main thread
  constructor(
    readonly rate: number,
    readonly delay16: number,
    private now: () => number,
    private emit: (m: CoreMessage) => void,
    tapRaw = false,
  ) {
    if (tapRaw) this.tap = { down: new Resampler(rate, RATE16), in16: new Queue(4096), hop: new Float32Array(WIRE) }
    this.fade = 1 - exp(-1 / ((FADE_MS / 1000) * rate))
    this.floor = Math.round(HOP * (rate / RATE16))
    this.raw = new Float32Array(Math.max(1, this.latency))
  }

  // the processed path's delay at the context rate (priming, the chain, half of each resampler's window), which the raw path copies so a switch moves nothing in time
  get latency(): number {
    const resamplers = this.rate === RATE16 ? 0 : Resampler.HALF + Resampler.HALF * (this.rate / RATE16)
    return Math.round((PRIME_HOPS * HOP + this.delay16) * (this.rate / RATE16) + resamplers)
  }

  get latencyMs(): number {
    return ((this.latency + this.debt) / this.rate) * 1000
  }

  connect(send: (hop: Float32Array) => void): void {
    this.send = send
    this.activate()
  }

  // the worker builds a session: raw at the same delay, then the models again, without counting it as an overload
  pause(on: boolean): void {
    if (on) {
      if (this.mode === 'active') this.mode = 'raw'
    } else if (this.mode === 'raw' && !this.stopped && !this.retryAt && this.send) this.activate()
  }

  // the engine crashed: raw until a new connect
  stop(): void {
    this.stopped = true
    this.mode = 'raw'
    this.retryAt = 0
  }

  private activate(): void {
    this.down = new Resampler(this.rate, RATE16)
    this.up = new Resampler(RATE16, this.rate)
    this.in16 = new Queue(4096)
    this.out = new Queue(Math.ceil(this.rate * 2))
    // the chain's own delay comes back in the worker's hops: only the round trip is primed
    const prime = Math.round(PRIME_HOPS * HOP * (this.rate / RATE16))
    for (let i = 0; i < prime; i++) this.out.push(0)
    this.epoch++
    this.inFlight = 0
    this.debt = 0
    this.starvedAt = 0
    this.windowStart = this.now()
    this.windowStarved = 0
    this.activatedAt = this.now()
    this.stopped = false
    this.mode = 'active'
    this.onActivate?.(Math.round((this.frames * RATE16) / this.rate))
  }

  // a hop from before the last activation (a worker that hung, then caught up) is stale
  receive(wire: Float32Array): void {
    this.received++
    if (wire[HOP + 1] !== this.epoch || this.mode !== 'active') return
    if (this.inFlight > 0) this.inFlight--
    this.up.push(wire.subarray(0, HOP), (x) => this.out.push(x))
  }

  private fail(): void {
    if (this.now() - this.activatedAt > STABLE_MS) this.retryMs = RETRY_MS
    this.mode = 'raw'
    this.retryAt = this.now() + this.retryMs
    this.retryMs = Math.min(2 * this.retryMs, RETRY_MAX_MS)
    this.emit({ type: 'overload', queueMs: Math.round((this.out.size / this.rate) * 1000), sent: this.sent, received: this.received })
  }

  process(input: Float32Array | null, far: Float32Array | null, output: Float32Array): void {
    const n = output.length
    this.frames += n
    this.listenFar(far, n)
    // Firefox gives no channel at all while nothing plays: silence, so the clock and the retries go on
    if (!input || !input.length) input = this.silence.length === n ? this.silence : (this.silence = new Float32Array(n))
    // raw path: the mic delayed by the processed path's latency
    const raw = this.raw
    const L = raw.length
    if (this.mode === 'raw' && this.retryAt && this.now() >= this.retryAt && this.send) {
      this.retryAt = 0
      this.activate()
      this.emit({ type: 'retry' })
    }
    if (this.mode === 'active') {
      this.down.push(input, (x) => this.in16.push(x))
      while (this.in16.size >= HOP) {
        this.in16.pull(this.hop, HOP)
        this.hop[HOP] = this.farQuietMs
        this.hop[HOP + 1] = this.epoch
        this.send!(this.hop)
        this.inFlight++
        this.sent++
      }
      this.drain(output)
    } else if (this.tap) this.tapRaw(input)
    for (let i = 0; i < n; i++) {
      const x = input[i] ?? 0
      const late = raw[this.rawPos]!
      raw[this.rawPos] = x
      this.rawPos = (this.rawPos + 1) % L
      this.mix += ((this.mode === 'active' ? 1 : 0) - this.mix) * this.fade
      output[i] = this.mode === 'active' || this.mix > 1e-4 ? this.mix * (this.mode === 'active' ? output[i]! : 0) + (1 - this.mix) * late : late
    }
    if (this.mode === 'active') this.track()
  }

  private tapRaw(input: Float32Array): void {
    const t = this.tap!
    t.down.push(input, (x) => t.in16.push(x))
    while (t.in16.size >= HOP) {
      t.in16.pull(t.hop, HOP)
      if (this.send && !this.stopped) {
        t.hop[HOP] = this.farQuietMs
        t.hop[HOP + 1] = RAW_HOP
        this.send(t.hop)
      } else this.emit({ type: 'raw', pcm: t.hop.slice(0, HOP) })
    }
  }

  private drain(output: Float32Array): void {
    const n = output.length
    const out = this.out
    if (out.lost) {
      this.debt = Math.max(0, this.debt - out.lost)
      out.lost = 0
    }
    if (out.size >= n) {
      // pay the starvation debt back on quiet blocks, once the queue stayed deep enough for ~3 s
      if (this.debt >= n && out.size >= 2 * n && this.lowWater() - n >= this.floor && out.peak(n) < QUIET) {
        out.skip(n)
        this.debt -= n
        this.stats.dropped += n
      }
      out.pull(output, n)
      if (this.starvedAt) {
        this.windowStarved += this.now() - this.starvedAt
        this.starvedAt = 0
      }
      return
    }
    // measured in wall-clock time, not quanta: the renderer catches up in bursts during which no reply can land
    output.fill(0)
    if (!this.sent) return
    this.debt += n
    const now = this.now()
    if (!this.starvedAt) this.starvedAt = now
    if (now - this.windowStart > STARVED_WINDOW_MS) {
      this.windowStart = now
      this.windowStarved = 0
    }
    if (now - this.starvedAt >= STARVED_CONTINUOUS_MS || this.windowStarved + (now - this.starvedAt) >= STARVED_BUDGET_MS) this.fail()
  }

  private lowWater(): number {
    return Math.min(this.windowMin, ...this.mins)
  }

  private track(): void {
    const size = this.out.size
    this.windowMin = Math.min(this.windowMin, size)
    this.stats.min = Math.min(this.stats.min, size)
    this.stats.max = Math.max(this.stats.max, size)
    const second = Math.round(this.rate / 128)
    if (++this.quanta % second === 0) {
      this.mins.push(this.windowMin)
      if (this.mins.length > 2) this.mins.shift()
      this.windowMin = Infinity
    }
    if (this.quanta % (2 * second) === 0) {
      const ms = (v: number) => Math.round((v / this.rate) * 1000)
      this.emit({ type: 'timing', queueMs: [ms(this.stats.min), ms(this.stats.max)], debtMs: ms(this.debt), droppedMs: ms(this.stats.dropped), latencyMs: Math.round(this.latencyMs) })
      this.stats = { min: Infinity, max: 0, dropped: 0 }
    }
  }

  // the far end's level, or what the app said with farEndActive
  private listenFar(far: Float32Array | null, n: number): void {
    const ms = (n / this.rate) * 1000
    let loud = this.farActive > 0
    if (far && far.length) {
      let s = 0
      for (let i = 0; i < far.length; i++) s += far[i]! * far[i]!
      loud ||= Math.sqrt(s / far.length) > FAR_LEVEL
    }
    this.farQuietMs = loud ? 0 : this.farQuietMs + ms
  }

  setFarActive(on: boolean): void {
    this.farActive = on ? 1 : 0
    if (on) this.farQuietMs = 0
  }
}
