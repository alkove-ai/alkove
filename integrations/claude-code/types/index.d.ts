export type Lang = 'en' | 'fr' | 'es' | 'it'

export type Action = 'abort' | 'send' | 'clear' | 'run'

export type Command = {
  id: string
  action: Action
  phrases: string[]
  // what `run` does: a slash command when it starts with `/`, else a prompt sent to Claude
  text: string | null
}

export type VoiceSettings = {
  lang: Lang
  // empty: "OK Claude"
  trigger: string
  // what a phrase that is no command does: written in the prompt box, or sent
  dictation: 'fill' | 'submit'
  // off, or a short spoken summary of each answer
  readAloud: 'off' | 'summary'
  // Supertonic's voice, 0–9: F1–F5 then M1–M5
  voice: number
  // Supertonic's denoising steps: more is better and slower
  steps: number
  // device ids as alkove-voice lists them; null: the system's default
  mic: string | null
  speaker: string | null
  // seconds of silence that end what follows the trigger
  endSecs: number
  // off: the microphone is closed, for every session
  isListening: boolean
  maxSessions: number
  // null: the defaults of `lang`
  commands: Command[] | null
  isSetUp: boolean
}

export type Device = { id: string; name: string }

export type Live = {
  number: number | null
  state: 'starting' | 'listening' | 'heard' | 'paused' | 'error'
  trigger: string
  warnings: string[]
  error: string | null
  last: string | null
  devices: { inputs: Device[]; outputs: Device[] } | null
}

declare module 'claude-code' {
  interface PluginState {
    'voice-commands': { settings: Shaped<VoiceSettings>; live: Shaped<Live> }
  }
}
