import type { SpeechParams } from '../core/cue'
import type { Take } from '../core/spot/spot'
import { type Context, type WakeEvent, type Word, type WordJSON, wordOf } from '../core/spot/spot'
import type { Warning } from '../core/lex'
import { Voiceprint, type VoiceprintJSON } from '../core/timbre'
import { AlkoveError, type Diag } from '../errors'
import { Emitter } from '../events'
import { onDiag } from '../loader'
import { FOCUS, type Lang, RATE } from '../manifest'
import { printOf } from '../process'
import { type OutputOptions, Packer } from '../realtime/packer'
import type { RealtimeOptions } from '../realtime/worker'
import { Host, scriptUrl, type WorkerLink } from '../runtime/host'
import { Enrollment, type EnrollTarget } from './enroll'
import { CONSTRAINTS, openMic, processed, supported } from './mic'

export type Mode = 'raw' | 'focus' | 'focus+print' | 'spot' | 'spot+print'
export type RawReason = 'off' | 'loading' | 'failed' | 'overload' | 'unsupported'
export type Status = 'starting' | 'running' | 'blocked' | 'interrupted' | 'disposed'
// Focus's output before the leveling over the phrase (null for a held phrase without voice), the mic's noise floor, the session's main voice (null until it has heard it twice), in dBFS
export interface PhraseLevel {
  voiceDb: number | null
  noiseDb: number
  refDb: number | null
}

export interface InputEvents {
  speechstart: { start: number }
  speechend: { start: number; end: number; level?: PhraseLevel }
  speechcancel: { start: number }
  mode: { mode: Mode; reason?: RawReason }
  status: Status
  voiceprint: Voiceprint
  learn: { progress: number }
  diag: Diag
  mic: { state: 'lost' | 'reopened' | 'denied' | 'muted' | 'unmuted' }
  trackchange: MediaStreamTrack
  wake: WakeEvent
  heard: { speech: number; scores: { word: string; score: number }[] }
  trace: Record<string, unknown>[]
  audio: ArrayBuffer
  utterance: { pcm: ArrayBuffer; start: number; end: number; level?: PhraseLevel }
  take: Take
  enrolled: { word?: WordJSON; takes?: Take[]; warnings?: Warning[]; audio?: Float32Array; error?: string }
}

export interface CommonOptions {
  stream?: MediaStream
  track?: MediaStreamTrack
  audioContext?: AudioContext
  deviceId?: string
  followDefault?: boolean
  signal?: AbortSignal
  trace?: boolean
  keepRecent?: number
}

export interface FocusOptions extends CommonOptions {
  voiceprint?: Voiceprint | VoiceprintJSON | string | null
  learn?: boolean
  farEnd?: MediaStreamTrack | AudioNode | 'manual'
  strength?: number
  normalize?: boolean
  stats?: boolean
  speech?: Partial<SpeechParams>
  levels?: boolean
  near?: boolean | { gapDb?: number }
}

const RETRY_MS = 5000
const RETRY_MAX_MS = 60_000
const STALL_MS = 1500
const KEEP_ALIVE_HZ = 30
const KEEP_ALIVE_GAIN = 0.002
// raw this long on a mic the SDK opened: the browser's noise suppression and gain come back (models in the cache load faster)
const RAW_BROWSER_MS = 2000

let ids = 0
const contexts = new WeakMap<BaseAudioContext, Promise<void>>()
const inputs = new Set<AlkoveInput>()
// inputs created and not disposed, the ones still opening included: the workers close when none is left
let alive = 0
const STALE_PRINT = 'This print comes from another Timbre: ignored, learn it again.'

export abstract class AlkoveInput extends Emitter<InputEvents> {
  protected readonly id = ++ids
  protected ctx!: AudioContext
  protected ownCtx = false
  protected stream!: MediaStream
  protected ownStream = false
  protected source!: MediaStreamAudioSourceNode
  protected node!: AudioWorkletNode
  protected dest: MediaStreamAudioDestinationNode | null = null
  protected farSource: AudioNode | null = null
  protected rt: WorkerLink | null = null
  protected bg: WorkerLink | null = null
  protected offs: (() => void)[] = []
  private _mode: { mode: Mode; reason?: RawReason } = { mode: 'raw', reason: 'loading' }
  private _status: Status = 'starting'
  private _latency = 0
  private _voiceprint: Voiceprint | null = null
  private retryMs = RETRY_MS
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private started = 0
  private counted = false
  private enabled = true
  private printed = false
  private sender: RTCRtpSender | null = null
  private stalled = false
  protected spotWords: { words: WordJSON[]; context: Context } | null = null
  private running: Promise<void>
  private markRunning!: () => void
  private ran = false
  private notReady: AlkoveError | null = null
  private waiting: [() => void, (e: AlkoveError) => void][] = []
  private warned = new Set<string>()
  // what the mode asks of the browser's processing, and what the mic has
  private browserWant = false
  private browserHas = false
  private browserBusy: Promise<void> = Promise.resolve()
  private browserTimer: ReturnType<typeof setTimeout> | undefined
  // sent again to every new pipeline: the app's listen() and hold() hold across loads and crashes
  private listening = true
  private holding = false
  protected abstract readonly kind: 'call' | 'agent' | 'wake'

  constructor(protected opts: FocusOptions) {
    super()
    this.running = new Promise((r) => (this.markRunning = r))
  }

  protected get disposed(): boolean {
    return this._status === 'disposed'
  }

  get mode(): Mode {
    return this._mode.mode
  }

  get status(): Status {
    return this._status
  }

  // ms between the mic and the output: Focus's window, the worklet's queue, the gate
  get latency(): number {
    return this._latency
  }

  get voiceprint(): Voiceprint | null {
    return this._voiceprint
  }

  // once the models run; a failed try rejects the calls waiting on it, a later call waits for the next try; rejects at once without SIMD or once disposed
  get ready(): Promise<void> {
    if (this.ran) return Promise.resolve()
    if (this.notReady) return Promise.reject(this.notReady)
    return new Promise((resolve, reject) => this.waiting.push([resolve, reject]))
  }

  private settle(error: AlkoveError | null, final = false): void {
    if (this.ran) return
    this.ran = !error
    if (final) this.notReady = error
    const waiting = this.waiting
    this.waiting = []
    for (const [resolve, reject] of waiting) {
      if (error) reject(error)
      else resolve()
    }
  }

  get track(): MediaStreamTrack | null {
    if (this.stalled) return this.stream.getAudioTracks()[0] ?? null
    return this.dest?.stream.getAudioTracks()[0] ?? null
  }

  now(): number {
    return this.ctx ? this.ctx.currentTime - this.started : 0
  }

  protected abstract realtimeOptions(): RealtimeOptions
  protected abstract models(): ('focus' | 'cue' | 'spot')[]
  protected get delay16(): number {
    return 0
  }

  // a creation that fails leaves nothing behind: no mic, no context, no listener
  protected async open(): Promise<void> {
    alive++
    this.counted = true
    try {
      await this.opening()
    } catch (e) {
      this.dispose()
      throw e
    }
  }

  private async opening(): Promise<void> {
    // first, so diags raised while opening (mic.waiting) reach the input too
    this.offs.push(onDiag((d) => this.diag(d)))
    if (this.opts.voiceprint && !printOf(this.opts.voiceprint)) this.diag({ code: 'voiceprint.reset', level: 'warn', message: STALE_PRINT })
    const s = supported()
    if (!s.secure) throw new AlkoveError('page.insecure', 'The microphone needs HTTPS (or localhost).')
    if (!s.worklet) throw new AlkoveError('browser.unsupported', 'No AudioWorklet in this browser (Safari 14.1 or later).')
    const o = this.opts
    if (o.stream || o.track) this.stream = o.stream ?? new MediaStream([o.track!])
    else {
      this.stream = await openMic(o.deviceId, o.signal)
      this.ownStream = true
    }
    const track = this.stream.getAudioTracks()[0]
    if (!track) throw new AlkoveError('mic.not-found', 'The stream has no audio track.')
    if (!this.ownStream) {
      const on = processed(track)
      if (on.length) this.diag({ code: 'mic.processed', level: 'warn', message: `The browser processes the stream before the models (${on.join(', ')}): open it with these off.`, settings: on })
    }
    for (const other of inputs) if (other.stream.getAudioTracks()[0]?.getSettings().deviceId === track.getSettings().deviceId && other !== this)
      this.diag({ code: 'mic.twice', level: 'warn', message: 'Two inputs on the same microphone run everything twice: use call.spot() for words on a call input.' })
    // created after the capture: Safari can pick a wrong rate for a context opened before
    if (o.audioContext) this.ctx = o.audioContext
    else {
      this.ctx = new AudioContext()
      this.ownCtx = true
    }
    await this.build()
    inputs.add(this)
    this.watchContext()
    this.watchTrack(track)
    if (this.ownStream && o.followDefault !== false && !o.deviceId) this.followDefault()
    void this.startModels()
    this.flushHeld()
  }

  private async build(): Promise<void> {
    const ctx = this.ctx
    let loading = contexts.get(ctx)
    if (!loading) {
      loading = scriptUrl('worklet').then((url) => ctx.audioWorklet.addModule(url))
      // Chrome keeps a failed addModule per URL: the next try gets a new blob URL
      loading.catch(() => contexts.delete(ctx))
      contexts.set(ctx, loading)
    }
    await loading
    this.node = new AudioWorkletNode(ctx, 'alkove', { numberOfInputs: 2, numberOfOutputs: 1, outputChannelCount: [1], processorOptions: { delay16: this.delay16, tapRaw: this.kind === 'agent' } })
    this.node.port.onmessage = (e) => this.onWorklet(e.data)
    this.source = ctx.createMediaStreamSource(this.stream)
    this.source.connect(this.node, 0, 0)
    const far = this.opts.farEnd
    if (far && far !== 'manual') {
      this.farSource = far instanceof MediaStreamTrack ? ctx.createMediaStreamSource(new MediaStream([far])) : far
      this.farSource.connect(this.node, 0, 1)
    }
    this.connectOutput()
    this.started = ctx.currentTime
    await this.resume()
  }

  protected connectOutput(): void {
    this.dest = this.ctx.createMediaStreamDestination()
    this.node.connect(this.dest)
  }

  private onWorklet(m: { type: string; [k: string]: any }): void {
    if (m.type === 'overload') {
      this.diag({ code: 'overload', level: 'warn', message: 'The device is too slow for the models: raw microphone for a while.', queueMs: m.queueMs, sent: m.sent, received: m.received })
      this.setMode('raw', 'overload')
    } else if (m.type === 'retry') this.setMode(this.activeMode())
    else if (m.type === 'latency') this._latency = Math.round(m.ms)
    else if (m.type === 'raw') this.rawAudio(m.pcm)
    else if (m.type === 'timing') {
      this._latency = m.latencyMs
      this.diag({ code: 'queue', level: 'info', ...m })
    }
  }

  // raw mode without a worker (models loading or failed, engine crashed): the agent's audio still goes out
  protected rawAudio(_pcm: Float32Array): void {}

  protected activeMode(): Mode {
    if (!this.enabled) return 'raw'
    if (this.kind === 'wake') return this.reference() ? 'spot+print' : 'spot'
    return this.printed ? 'focus+print' : 'focus'
  }

  // events raised while the input is created wait for the app's first on(): it can only call it once the promise resolved
  private held: [keyof InputEvents, unknown][] | null = []

  protected emit<K extends keyof InputEvents>(type: K, value: InputEvents[K]): void {
    if (this.held) this.held.push([type, value])
    else this.deliver(type, value)
  }

  protected flushHeld(): void {
    setTimeout(() => {
      const q = this.held ?? []
      this.held = null
      for (const [t, v] of q) this.deliver(t, v as never)
    }, 0)
  }

  // a warning the app does not listen to goes to the console, once per code: what degrades the input is seen
  private deliver<K extends keyof InputEvents>(type: K, value: InputEvents[K]): void {
    super.emit(type, value)
    if (type !== 'diag') return
    const d = value as Diag
    if (d.level !== 'warn' || this.listens('diag') || this.warned.has(d.code)) return
    this.warned.add(d.code)
    console.warn(`alkove: ${d.message ?? d.code}`, d)
  }

  protected setMode(mode: Mode, reason?: RawReason): void {
    if (this._mode.mode === mode && this._mode.reason === reason) return
    this._mode = reason ? { mode, reason } : { mode }
    this.emit('mode', this._mode)
    this.followRaw()
  }

  // raw for a while on a mic the SDK opened: the browser's noise suppression and gain, off again once the models run
  private followRaw(): void {
    clearTimeout(this.browserTimer)
    if (!this.ownStream || this.disposed) return
    const raw = this._mode.mode === 'raw' && this._mode.reason !== 'off'
    if (!raw) this.wantBrowser(false)
    else if (!this.browserWant) this.browserTimer = setTimeout(() => this.wantBrowser(true), RAW_BROWSER_MS)
  }

  // one change of the mic at a time, each to what the mode asks by then
  private wantBrowser(on: boolean): void {
    this.browserWant = on
    this.browserBusy = this.browserBusy.then(() => this.browser())
  }

  private async browser(): Promise<void> {
    const on = this.browserWant
    const track = this.stream.getAudioTracks()[0]
    if (this.disposed || !this.ownStream || on === this.browserHas || !track) return
    try {
      await track.applyConstraints({ ...CONSTRAINTS, noiseSuppression: on, autoGainControl: on })
    } catch {}
    const told = track.getSettings().noiseSuppression
    // Chrome keeps a source's processing as it was opened, for every new track on it too: the mic closed, then opened again; a browser that does not tell is left as it is
    if (typeof told === 'boolean' && told !== on) {
      if (this.disposed || !this.ownStream) return
      const deviceId = track.getSettings().deviceId ?? this.opts.deviceId
      track.stop()
      try {
        const stream = await openMic(deviceId, undefined, on)
        if (this.disposed || !this.ownStream) {
          for (const t of stream.getTracks()) t.stop()
          return
        }
        await this.setStream(stream, true)
      } catch {
        void this.onTrackEnded()
        return
      }
    }
    this.browserHas = on
    const applied = this.stream.getAudioTracks()[0]?.getSettings().noiseSuppression
    this.diag({ code: 'mic.browser', level: 'info', noiseSuppression: on, autoGainControl: on, applied: typeof applied === 'boolean' ? applied : null })
  }

  protected setStatus(s: Status): void {
    if (this._status === s) return
    this._status = s
    this.emit('status', s)
  }

  protected diag(d: Diag): void {
    this.emit('diag', d)
  }

  private async startModels(): Promise<void> {
    if (this.disposed) return
    if (!supported().simd) {
      this.diag({ code: 'engine.unsupported', level: 'warn', message: 'No WebAssembly SIMD: raw microphone for this session.' })
      this.setMode('raw', 'unsupported')
      this.settle(new AlkoveError('browser.unsupported', 'No WebAssembly SIMD: the models cannot run in this browser.'), true)
      return
    }
    this.setMode('raw', 'loading')
    // the input starts raw and loading: setMode saw no change
    this.followRaw()
    try {
      const host = Host.get()
      const rt = await host.models('realtime', [...this.models(), ...(this.spotWords?.words.length ? (['spot'] as const) : [])])
      let bgPort: MessagePort | undefined
      if (this.wantsBackground()) {
        const bg = await host.models('background', ['timbre'])
        const channel = new MessageChannel()
        bg.post({ type: 'rt-port', input: this.id, port: channel.port2, ref: this.reference()?.toJSON() ?? null }, [channel.port2])
        bgPort = channel.port1
        this.bg = bg
        this.offs.push(bg.listen((m) => m.input === this.id && m.type === 'event' && this.onEvent(m.event)))
        const start = printOf(this.opts.voiceprint)
        if (this.opts.learn) await bg.call('learner', [this.id, start?.toJSON() ?? null])
      }
      if (this.disposed) return
      this.rt = rt
      this.offs.push(rt.listen((m) => this.onRealtime(m)))
      const out = this.appPort()
      const transfer: Transferable[] = [...(bgPort ? [bgPort] : []), ...(out ? [out] : [])]
      if (out) this.portGiven = true
      await rt.request({ type: 'realtime', input: this.id, opts: this.realtimeOptions(), bg: bgPort, out }, transfer)
      if (!this.listening || this.holding) rt.post({ type: 'set', input: this.id, ...(!this.listening && { listen: false }), ...(this.holding && { hold: true }) })
      if (this.spotWords?.words.length) {
        await host.models('realtime', ['spot'])
        rt.post({ type: 'set', input: this.id, ...this.spotWords })
      }
      this.markRunning()
      this.settle(null)
      const channel = new MessageChannel()
      rt.post({ type: 'worklet', input: this.id, port: channel.port2 }, [channel.port2])
      this.node.port.postMessage({ type: 'connect', port: channel.port1 }, [channel.port1])
      this.printed = !!printOf(this.opts.voiceprint)
      this.retryMs = RETRY_MS
      this.setMode(this.activeMode(), this.enabled ? undefined : 'off')
    } catch (e) {
      if (this.disposed) return
      this.diag({ code: 'models.failed', level: 'warn', message: `Models not loaded, raw microphone; retry in ${this.retryMs / 1000} s.`, error: String((e as Error)?.message ?? e) })
      this.setMode('raw', 'failed')
      this.settle(e instanceof AlkoveError ? e : new AlkoveError('engine.failed', `Models not loaded: ${String((e as Error)?.message ?? e)}`, e))
      this.retryTimer = setTimeout(() => void this.startModels(), this.retryMs)
      this.retryMs = Math.min(2 * this.retryMs, RETRY_MAX_MS)
    }
  }

  // the app's own port for audio and utterances, until it is handed to a worker (once: a crashed worker takes it along)
  protected portGiven = false
  protected appPort(): MessagePort | undefined {
    return this.portGiven ? undefined : (this.opts as { output?: { port?: MessagePort } }).output?.port
  }

  // the background worker: Timbre for learning the print, or for checking the voice of a wake
  protected wantsBackground(): boolean {
    return !!this.opts.learn
  }

  protected reference(): Voiceprint | null {
    return null
  }

  private onRealtime(m: { type: string; input?: number; [k: string]: any }): void {
    if (m.input !== this.id) return
    if (m.type === 'events') {
      const traces: Record<string, unknown>[] = []
      for (const e of m.list) {
        if (e.type === 'trace') traces.push(e)
        else this.onEvent(e)
      }
      if (traces.length) this.emit('trace', traces)
    } else if (m.type === 'pcm') {
      if (m.kind === 'audio') this.emit('audio', m.pcm)
      else this.emit('utterance', { pcm: m.pcm, start: m.start, end: m.end, ...(m.level && { level: m.level }) })
    } else if (m.type === 'crash') {
      this.node.port.postMessage({ type: 'stop' })
      this.rt = null
      this.diag({ code: 'engine.crash', level: 'warn', message: `The engine stopped: raw microphone; retry in ${this.retryMs / 1000} s.`, error: m.message })
      this.setMode('raw', 'failed')
      clearTimeout(this.retryTimer)
      this.retryTimer = setTimeout(() => void this.startModels(), this.retryMs)
      this.retryMs = Math.min(2 * this.retryMs, RETRY_MAX_MS)
    }
  }

  protected onEvent(e: { type: string; [k: string]: any }): void {
    switch (e.type) {
      case 'speechstart':
        this.emit('speechstart', { start: e.start })
        break
      case 'speechcancel':
        this.emit('speechcancel', { start: e.start })
        break
      case 'speechend':
        this.emit('speechend', { start: e.start, end: e.end, ...('voiceDb' in e && { level: { voiceDb: e.voiceDb, noiseDb: e.noiseDb, refDb: e.refDb } }) })
        break
      case 'wake':
        this.emit('wake', e as unknown as WakeEvent)
        break
      case 'take': {
        const { type: _, ...take } = e
        this.emit('take', take as unknown as Take)
        break
      }
      case 'enrolled': {
        const { type: _, ...r } = e
        this.emit('enrolled', r)
        break
      }
      case 'heard':
        this.emit('heard', { speech: e.speech, scores: e.scores })
        break
      case 'learn':
        this.emit('learn', { progress: e.progress })
        break
      case 'voiceprint': {
        const p = Voiceprint.fromJSON(e.json)
        if (p) {
          this._voiceprint = p
          this.emit('voiceprint', p)
        }
        break
      }
      case 'idle':
        this.diag({ code: 'idle', level: 'info', asleep: e.asleep, at: e.at })
        break
      case 'diag':
        if (e.code === 'focus.print') {
          this.printed = !!e.data?.print
          if (this._mode.mode !== 'raw') this.setMode(this.activeMode())
        }
        this.diag({ code: e.code, level: e.level, ...e.data })
        break
    }
  }

  async setEnabled(on: boolean): Promise<boolean> {
    this.enabled = on
    this.rt?.post({ type: 'set', input: this.id, enabled: on })
    if (!on) this.setMode('raw', 'off')
    else if (this.rt && this._mode.reason === 'off') this.setMode(this.activeMode())
    return on && !!this.rt
  }

  // off: no phrase until it is on again, the one being said dropped (speechcancel); back on, the speech under way is no phrase
  listen(on: boolean): void {
    this.listening = on
    this.rt?.post({ type: 'set', input: this.id, listen: on })
  }

  // a phrase from now until release(), whatever Cue hears: hold to talk
  hold(): void {
    this.holding = true
    this.rt?.post({ type: 'set', input: this.id, hold: true })
  }

  release(): void {
    this.holding = false
    this.rt?.post({ type: 'set', input: this.id, hold: false })
  }

  setStrength(s: number): void {
    this.rt?.post({ type: 'set', input: this.id, strength: s })
    this.opts.strength = s
  }

  setVoiceprint(p: Voiceprint | VoiceprintJSON | string | null): void {
    const print = printOf(p)
    if (p && !print) this.diag({ code: 'voiceprint.reset', level: 'warn', message: 'This print comes from another Timbre: ignored, learn it again.' })
    this.opts.voiceprint = print
    this.rt?.post({ type: 'set', input: this.id, print: print?.embedding ?? null })
    if (this.opts.learn) void this.bg?.call('learner', [this.id, print?.toJSON() ?? null])
  }

  forgetVoiceprint(): void {
    this._voiceprint = null
    this.rt?.post({ type: 'set', input: this.id, print: null })
    void this.bg?.call('forget', [this.id])
  }

  // the app's own TTS starts or stops
  farEndActive(on: boolean): void {
    this.node?.port.postMessage({ type: 'far', on })
  }

  async stats(): Promise<{ speechSeconds: number; seconds: number; noiseRemovedDb: number | null } | null> {
    return this.rt ? this.rt.call('stats', [this.id]) : null
  }

  // the next `seconds`: stereo WAV, left the mic, right the output (16 kHz)
  async record(seconds: number): Promise<Blob> {
    if (!this.rt) throw new AlkoveError('engine.failed', 'The models are not running yet.')
    const wav = await this.rt.call<Uint8Array>('record', [this.id, seconds])
    return new Blob([wav as Uint8Array<ArrayBuffer>], { type: 'audio/wav' })
  }

  async recent(): Promise<Blob | null> {
    const wav = this.rt ? await this.rt.call<Uint8Array | null>('recent', [this.id]) : null
    return wav ? new Blob([wav as Uint8Array<ArrayBuffer>], { type: 'audio/wav' }) : null
  }

  // in a click, when the browser left the context suspended
  async resume(): Promise<void> {
    try {
      await Promise.race([this.ctx.resume(), new Promise((r) => setTimeout(r, 300))])
    } catch {}
    this.setStatus(this.ctx.state === 'running' ? 'running' : (this.ctx.state as string) === 'interrupted' ? 'interrupted' : 'blocked')
  }

  private watchContext(): void {
    const ctx = this.ctx
    const on = () => {
      if (this.disposed) return
      const state = ctx.state as string
      if (state === 'running') this.setStatus('running')
      else if (state === 'interrupted') this.setStatus('interrupted')
      else if (state === 'suspended') {
        // Android suspends the context on audio-mode changes: resume, blocked only if that fails
        void ctx.resume().then(
          () => this.setStatus(ctx.state === 'running' ? 'running' : 'blocked'),
          () => this.setStatus('blocked'),
        )
      }
    }
    ctx.addEventListener('statechange', on)
    this.offs.push(() => ctx.removeEventListener('statechange', on))
    // an audio clock that stops or drags (Chrome on Windows): the raw mic goes out instead, back once it runs again
    let last = ctx.currentTime
    let lastAt = performance.now()
    let slowSince = 0
    const timer = setInterval(() => {
      const now = performance.now()
      const advanced = (ctx.currentTime - last) * 1000
      const elapsed = now - lastAt
      last = ctx.currentTime
      lastAt = now
      const slow = ctx.state !== 'running' || advanced < 0.9 * elapsed
      slowSince = slow ? slowSince || now : 0
      const stalled = !!slowSince && now - slowSince >= STALL_MS
      if (stalled !== this.stalled) {
        this.stalled = stalled
        this.diag({ code: stalled ? 'audio.stalled' : 'audio.resumed', level: stalled ? 'warn' : 'info' })
        const t = this.track
        if (t) {
          if (this.sender) void this.sender.replaceTrack(t)
          else this.emit('trackchange', t)
        }
      }
    }, 500)
    this.offs.push(() => clearInterval(timer))
  }

  // the SDK swaps the sender's track itself when the audio clock stalls
  bind(sender: RTCRtpSender): void {
    this.sender = sender
    const t = this.track
    if (t && sender.track !== t) void sender.replaceTrack(t)
  }

  private watchTrack(track: MediaStreamTrack): void {
    const onEnded = () => void this.onTrackEnded()
    const onMute = () => this.emit('mic', { state: 'muted' })
    const onUnmute = () => this.emit('mic', { state: 'unmuted' })
    track.addEventListener('ended', onEnded)
    track.addEventListener('mute', onMute)
    track.addEventListener('unmute', onUnmute)
    this.offs.push(() => {
      track.removeEventListener('ended', onEnded)
      track.removeEventListener('mute', onMute)
      track.removeEventListener('unmute', onUnmute)
    })
  }

  private reopenMs = RETRY_MS

  private async onTrackEnded(): Promise<void> {
    if (this.disposed) return
    this.emit('mic', { state: 'lost' })
    if (!this.ownStream) return
    try {
      const want = this.browserWant
      await this.setStream(await openMic(this.opts.deviceId, undefined, want), true)
      this.browserHas = want
      this.reopenMs = RETRY_MS
      this.emit('mic', { state: 'reopened' })
    } catch (e) {
      if ((e as AlkoveError).code === 'mic.denied') {
        this.emit('mic', { state: 'denied' })
        return
      }
      setTimeout(() => void this.onTrackEnded(), this.reopenMs)
      this.reopenMs = Math.min(2 * this.reopenMs, RETRY_MAX_MS)
    }
  }

  // a new source in the same graph: the output track stays the same
  async setStream(stream: MediaStream, own = false): Promise<void> {
    if (this.ownStream) for (const t of this.stream.getTracks()) if (t !== stream.getAudioTracks()[0]) t.stop()
    this.source.disconnect()
    this.stream = stream
    this.ownStream = own
    this.source = this.ctx.createMediaStreamSource(stream)
    this.source.connect(this.node, 0, 0)
    const track = stream.getAudioTracks()[0]
    if (track) this.watchTrack(track)
  }

  async setDevice(deviceId: string): Promise<void> {
    this.opts.deviceId = deviceId
    this.opts.followDefault = false
    const want = this.browserWant
    await this.setStream(await openMic(deviceId, undefined, want), true)
    this.browserHas = want
  }

  private followDefault(): void {
    const md = navigator.mediaDevices
    if (!md?.addEventListener) return
    let current = this.stream.getAudioTracks()[0]?.getSettings().groupId
    const on = async () => {
      if (this.opts.followDefault === false || this.disposed) return
      const devices = await md.enumerateDevices().catch(() => [])
      const def = devices.find((d) => d.kind === 'audioinput' && d.deviceId === 'default')
      if (def && def.groupId && def.groupId !== current) {
        current = def.groupId
        const want = this.browserWant
        await this.setStream(await openMic(undefined, undefined, want), true)
          .then(() => void (this.browserHas = want))
          .catch(() => {})
        this.emit('mic', { state: 'reopened' })
      }
    }
    md.addEventListener('devicechange', on)
    this.offs.push(() => md.removeEventListener('devicechange', on))
  }

  // a new track or context (LiveKit's restart): the graph is built again
  async restart({ track, audioContext }: { track?: MediaStreamTrack; audioContext?: AudioContext } = {}): Promise<void> {
    const before = this.track
    if (audioContext && audioContext !== this.ctx) {
      this.teardownGraph()
      if (this.ownCtx) void this.ctx.close().catch(() => {})
      this.ctx = audioContext
      this.ownCtx = false
      if (track) {
        if (this.ownStream) for (const t of this.stream.getTracks()) t.stop()
        this.stream = new MediaStream([track])
        this.ownStream = false
      }
      await this.build()
      this.watchContext()
      this.rt?.post({ type: 'dispose', input: this.id })
      this.rt = null
      void this.startModels()
    } else if (track) await this.setStream(new MediaStream([track]))
    const after = this.track
    if (after && after !== before) this.emit('trackchange', after)
  }

  private teardownGraph(): void {
    this.node?.port.postMessage({ type: 'dispose' })
    for (const n of [this.source, this.node, this.dest, this.farSource]) n?.disconnect()
  }

  // words read in Focus's output, in the same worker: no second microphone, no second Focus; with a print Focus already keeps only that voice (onlyMyVoice)
  spot(words: readonly (Word | WordJSON | string)[], opts: { context?: Context; onlyMyVoice?: boolean } = {}): SpotHandle {
    if (this.kind === 'wake') throw new TypeError('spot() reads Focus: use the wake listener itself')
    if (opts.onlyMyVoice && !printOf(this.opts.voiceprint) && !this.opts.learn) this.diag({ code: 'spot.no-print', level: 'warn', message: "onlyMyVoice relies on this input's print: give it a voiceprint (or learn: true), else the closest voice wakes it." })
    return new SpotHandle(this, words.map((w) => wordOf(w).toJSON()), opts.context ?? 'anywhere')
  }

  // a word enrolled on this call, read behind Focus like call.spot() reads it
  enrollWord(text: string, opts: { lang?: Lang | Lang[]; takes?: number } = {}): Enrollment {
    return new Enrollment({
      compile: async () => (await import('../api')).Spot.compile(text, { lang: opts.lang, chain: 'focus' }),
      open: async (word) => {
        await this.running
        const rt = await Host.get().models('realtime', ['spot'])
        rt.post({ type: 'set', input: this.id, enroll: { word, takes: opts.takes ?? 3 } })
        return { on: this.on.bind(this) as EnrollTarget['on'], stop: () => rt.post({ type: 'set', input: this.id, enroll: null }), source: 'focus' }
      },
      embed: async (pcm, source) => (await (await import('../api')).Timbre.create()).embed(pcm, { source }),
    })
  }

  // kept here and sent when the pipeline starts: spot() may come before the models are loaded
  setWords(words: WordJSON[], context: Context): void {
    this.spotWords = { words, context }
    if (!this.rt) return
    void Host.get()
      .models('realtime', ['spot'])
      .then((rt) => rt.post({ type: 'set', input: this.id, words, context }))
  }

  dispose(): void {
    if (this.disposed) return
    clearTimeout(this.retryTimer)
    clearTimeout(this.browserTimer)
    this.setStatus('disposed')
    this.settle(new AlkoveError('engine.failed', 'The input was disposed before its models ran.'), true)
    if (this.counted && --alive === 0) void Host.get().idle()
    for (const off of this.offs) off()
    this.offs = []
    this.rt?.post({ type: 'dispose', input: this.id })
    this.bg?.post({ type: 'dispose', input: this.id })
    this.teardownGraph()
    if (this.ownStream) for (const t of this.stream.getTracks()) t.stop()
    if (this.ownCtx) void this.ctx.close().catch(() => {})
    inputs.delete(this)
    this.clear()
  }
}

// ---- call: two humans, WebRTC

export interface CallOptions extends FocusOptions {
  keepAliveWhenHidden?: boolean
}

class CallInput extends AlkoveInput {
  protected readonly kind = 'call' as const
  private osc: OscillatorNode | null = null

  constructor(protected opts: CallOptions) {
    super(opts)
  }

  get track(): MediaStreamTrack {
    return super.track!
  }

  protected get delay16(): number {
    return FOCUS.lag + 0.1 * RATE
  }

  protected models(): ('focus' | 'cue' | 'spot')[] {
    return ['focus', 'cue']
  }

  protected chainOptions(): NonNullable<RealtimeOptions['chain']> {
    const print = printOf(this.opts.voiceprint)
    return { gate: true, normalize: this.opts.normalize ?? true, strength: this.opts.strength ?? 1, print: print?.embedding ?? null, speech: this.opts.speech, levels: this.opts.levels, near: this.opts.near }
  }

  protected realtimeOptions(): RealtimeOptions {
    return { pipeline: 'call', chain: this.chainOptions(), learn: !!this.opts.learn, keepRecent: this.opts.keepRecent, trace: this.opts.trace, stats: this.opts.stats }
  }

  async start(): Promise<this> {
    if (this.opts.learn && !this.opts.farEnd) this.diag({ code: 'learn.no-far-end', level: 'warn', message: 'learn without farEnd: the other voice may get into the print.' })
    await this.open()
    // Chrome freezes a hidden page after ~1 min without sound: a 30 Hz tone at -54 dBFS, which phone speakers can't play
    if (this.opts.keepAliveWhenHidden !== false) {
      this.osc = this.ctx.createOscillator()
      this.osc.frequency.value = KEEP_ALIVE_HZ
      const g = this.ctx.createGain()
      g.gain.value = KEEP_ALIVE_GAIN
      this.osc.connect(g).connect(this.ctx.destination)
      this.osc.start()
      this.offs.push(() => this.osc?.stop())
    }
    return this
  }

}

export class SpotHandle extends Emitter<{ wake: WakeEvent }> {
  private off: () => void

  constructor(
    private input: AlkoveInput,
    words: WordJSON[],
    context: Context,
  ) {
    super()
    input.setWords(words, context)
    this.off = input.on('wake', (e) => this.emit('wake', e))
  }

  dispose(): void {
    this.off()
    this.input.setWords([], 'anywhere')
    this.clear()
  }
}

export function createCallInput(opts: CallOptions = {}): Promise<CallInput> {
  return new CallInput(opts).start()
}

// ---- agent: voice → transcription / LLM

export interface AgentOptions extends FocusOptions {
  output?: Partial<OutputOptions> & { port?: MessagePort }
  withTrack?: boolean
  wake?: never
}

class AgentInput extends AlkoveInput {
  protected readonly kind = 'agent' as const
  private packer: Packer | null = null

  constructor(protected opts: AgentOptions) {
    super(opts)
  }

  protected rawAudio(pcm: Float32Array): void {
    // with the app's own port, never on the main thread: once the port went to a worker, these few raw packets are dropped
    this.packer ??= new Packer(this.output(), (chunk) => {
      const port = this.appPort()
      if (port) port.postMessage({ type: 'audio', pcm: chunk }, [chunk])
      else if (!this.opts.output?.port) this.emit('audio', chunk)
    })
    this.packer.push(pcm)
  }

  private output(): OutputOptions {
    const o = this.opts.output ?? {}
    return { format: o.format ?? 'f32', rate: o.rate ?? RATE, chunkMs: Math.max(16, Math.round((o.chunkMs ?? 32) / 16) * 16) }
  }

  protected get delay16(): number {
    return FOCUS.lag
  }

  protected models(): ('focus' | 'cue' | 'spot')[] {
    return ['focus', 'cue']
  }

  protected connectOutput(): void {
    if (this.opts.withTrack) return super.connectOutput()
    // the worklet only runs while something pulls it
    const mute = this.ctx.createGain()
    mute.gain.value = 0
    this.node.connect(mute).connect(this.ctx.destination)
  }

  protected realtimeOptions(): RealtimeOptions {
    const print = printOf(this.opts.voiceprint)
    return {
      pipeline: 'agent',
      chain: { gate: false, normalize: this.opts.normalize ?? true, strength: this.opts.strength ?? 1, print: print?.embedding ?? null, speech: this.opts.speech, levels: this.opts.levels, near: this.opts.near },
      learn: !!this.opts.learn,
      keepRecent: this.opts.keepRecent,
      trace: this.opts.trace,
      stats: this.opts.stats,
      output: { ...this.output(), audio: true, utterance: true },
    }
  }

  async start(): Promise<this> {
    if (this.opts.learn && !this.opts.farEnd) throw new TypeError("createAgentInput: learn needs farEnd (the agent's voice would teach the print)")
    if ((this.opts as { wake?: unknown }).wake) throw new TypeError('createAgentInput: wake is reserved, not in this preview (use createWakeListener)')
    await this.open()
    return this
  }
}

export function createAgentInput(opts: AgentOptions = {}): Promise<AgentInput> {
  return new AgentInput(opts).start()
}

// ---- wake word: Spot alone on the raw mic

export interface WakeOptions extends CommonOptions {
  words: readonly (Word | WordJSON | string)[]
  context?: Context
  // only this voice wakes: Timbre on the word + 0.5 s once Spot heard it (+0.5 s of latency)
  onlyMyVoice?: Voiceprint | VoiceprintJSON | string | null
}

class WakeListener extends AlkoveInput {
  protected readonly kind = 'wake' as const
  private words: WordJSON[]

  constructor(protected opts: WakeOptions & FocusOptions) {
    super(opts)
    this.words = opts.words.map((w) => wordOf(w).toJSON())
    if (!this.words.length) throw new TypeError('createWakeListener: no word')
    for (const w of this.words) if (w.chain === 'focus') this.diag({ code: 'word.chain', level: 'warn', message: `"${w.text}" was enrolled behind Focus: its thresholds are off on the raw microphone.` })
    if (opts.onlyMyVoice && !this.reference()) this.diag({ code: 'voiceprint.reset', level: 'warn', message: `onlyMyVoice: ${STALE_PRINT}` })
  }

  protected models(): ('focus' | 'cue' | 'spot')[] {
    return ['spot']
  }

  protected connectOutput(): void {
    const mute = this.ctx.createGain()
    mute.gain.value = 0
    this.node.connect(mute).connect(this.ctx.destination)
  }

  protected wantsBackground(): boolean {
    return !!this.reference()
  }

  protected reference(): Voiceprint | null {
    return printOf(this.opts.onlyMyVoice)
  }

  protected realtimeOptions(): RealtimeOptions {
    return { pipeline: 'wake', words: this.words, context: this.opts.context ?? 'pause', trace: this.opts.trace, verify: !!this.reference() }
  }

  async start(): Promise<this> {
    await this.open()
    return this
  }
}

export function createWakeListener(opts: WakeOptions): Promise<WakeListener> {
  return new WakeListener(opts).start()
}

// ---- word enrollment: Spot on the raw mic, the takes counted in the worker

class EnrollInput extends AlkoveInput {
  protected readonly kind = 'wake' as const

  constructor(
    protected opts: CommonOptions & FocusOptions,
    private word: WordJSON,
    private takes: number,
  ) {
    super(opts)
  }

  protected models(): ('focus' | 'cue' | 'spot')[] {
    return ['spot']
  }

  protected connectOutput(): void {
    const mute = this.ctx.createGain()
    mute.gain.value = 0
    this.node.connect(mute).connect(this.ctx.destination)
  }

  protected realtimeOptions(): RealtimeOptions {
    return { pipeline: 'enroll', enroll: { word: this.word, takes: this.takes } }
  }

  async start(): Promise<this> {
    await this.open()
    return this
  }
}

export function createEnrollInput(opts: CommonOptions, word: WordJSON, takes: number): Promise<EnrollInput> {
  return new EnrollInput(opts, word, takes).start()
}

export type { AgentInput, CallInput, EnrollInput, WakeListener }
