
export interface SpeechParams {
  confidence: number
  exitConfidence: number
  startSecs: number
  stopSecs: number
}

// tuned behind Focus on the 24 test streams of the VAD bench
export const SPEECH_DEFAULTS: SpeechParams = { confidence: 0.5, exitConfidence: 0.1, startSecs: 0.2, stopSecs: 0.6 }

export type SpeechEvent = { type: 'speechstart'; start: number } | { type: 'speechend'; start: number; end: number }
