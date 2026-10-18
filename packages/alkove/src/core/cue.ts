import { CUE, RATE } from '../manifest'
import { Chunker } from './chunker'
import type { StreamNet } from './engine'

// speech probability every 30 ms; `frame` k covers samples [480k, 480k + 480) of what Cue was given
export class Cue {
  static readonly chunk = CUE.chunk
  static readonly delay = CUE.chunk * CUE.lagSteps
  readonly delay = Cue.delay
  private chunker = new Chunker(CUE.chunk)
  private steps = 0

  constructor(private net: StreamNet) {}

  step(chunk: Float32Array): number | null {
    this.net.f32('audio').set(chunk)
    this.net.run()
    return this.steps++ >= CUE.lagSteps ? this.net.f32('speech')[0]! : null
  }

  push(pcm: Float32Array, onFrame: (p: number, frame: number) => void): void {
    this.chunker.push(pcm, (c) => {
      const p = this.step(c)
      if (p !== null) onFrame(p, this.steps - 1 - CUE.lagSteps)
    })
  }

  reset(): void {
    this.net.reset()
    this.chunker.reset()
    this.steps = 0
  }

  release(): void {
    this.net.release()
  }
}

export interface SpeechParams {
  confidence: number
  exitConfidence: number
  startSecs: number
  stopSecs: number
}

// tuned behind Focus on the 24 test streams of the VAD bench
export const SPEECH_DEFAULTS: SpeechParams = { confidence: 0.5, exitConfidence: 0.1, startSecs: 0.2, stopSecs: 0.6 }

export type SpeechEvent = { type: 'speechstart'; start: number } | { type: 'speechend'; start: number; end: number }

// a VAD state machine on frames of `frameSecs`: open on one frame over `confidence`, confirmed after `startSecs` of such frames, closed after `stopSecs` under `exitConfidence`
export class SpeechTracker {
  active = false
  confirmed = false
  private onset = 0
  private lastSpeech = 0
  private speechFrames = 0
  private quietFrames = 0
  private startFrames: number
  private stopFrames: number

  constructor(
    readonly params: SpeechParams = SPEECH_DEFAULTS,
    readonly frameSecs = CUE.chunk / RATE,
  ) {
    this.startFrames = Math.max(1, Math.ceil(params.startSecs / frameSecs - 1e-9))
    this.stopFrames = Math.max(1, Math.round(params.stopSecs / frameSecs))
  }

  // frame k's probability; times in seconds from the frame clock
  push(p: number, k: number): SpeechEvent | null {
    const t0 = k * this.frameSecs
    const { confidence, exitConfidence } = this.params
    if (p >= confidence) {
      this.quietFrames = 0
      this.lastSpeech = t0 + this.frameSecs
      if (!this.active) {
        this.active = true
        this.confirmed = false
        this.onset = t0
        this.speechFrames = 0
      }
      if (++this.speechFrames >= this.startFrames && !this.confirmed) {
        this.confirmed = true
        return { type: 'speechstart', start: this.onset }
      }
      return null
    }
    if (this.active && p < exitConfidence && ++this.quietFrames >= this.stopFrames) {
      const was = this.confirmed
      this.active = this.confirmed = false
      this.quietFrames = 0
      if (was) return { type: 'speechend', start: this.onset, end: this.lastSpeech }
    }
    return null
  }

  reset(): void {
    this.active = this.confirmed = false
    this.speechFrames = this.quietFrames = 0
  }
}
