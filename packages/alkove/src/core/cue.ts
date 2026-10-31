
export interface SpeechParams {
  confidence: number
  exitConfidence: number
  startSecs: number
  stopSecs: number
}

// tuned behind Focus on the 24 test streams of the VAD bench
export const SPEECH_DEFAULTS: SpeechParams = { confidence: 0.5, exitConfidence: 0.1, startSecs: 0.2, stopSecs: 0.6 }

export type SpeechEvent = { type: 'speechstart'; start: number } | { type: 'speechend'; start: number; end: number }

// the end of the user's turn: Cue's head read every 0.1 s of each pause, `turnend` on the first read over `threshold` past `minPause` seconds, or at `maxPause`
export interface TurnOptions {
  threshold: number
  minPause: number
  maxPause: number
}

// speech that starts while the far end speaks: `interruptstart`, then `interrupt` once it spans `minDuration` seconds, or `interruptcancel`
export interface InterruptOptions {
  minDuration: number
}
