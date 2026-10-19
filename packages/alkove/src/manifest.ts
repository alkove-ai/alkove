export type ModelName = 'focus' | 'cue' | 'spot' | 'timbre' | 'lex'

export interface ModelFile {
  name: string
  file: string
  bytes: number
  sha256: string
  // the Hugging Face commit a model is read at (sha256 checked either way); the wasm has none, it comes with the package
  revision?: string
}

export { ENGINE_WASM } from './engine.generated'

export const RATE = 16000

export const HF_ORG = 'alkove-ai'

export const MODELS: Record<ModelName, ModelFile> = {
  focus: {
    name: 'alkove-focus-1-preview',
    file: 'alkove-focus-1-preview.alk',
    bytes: 1111696,
    sha256: '55318cf0a463d523e249a8d4fe3b83e205b83bdb3f275468bcd0707f52d8b97c',
  },
  cue: {
    name: 'alkove-cue-1-preview',
    file: 'alkove-cue-1-preview.alk',
    bytes: 1066500,
    sha256: 'e054958b133befc2bff36ad4ceae98a8b25d19c3fefdcc50df8139634132fe4f',
  },
  spot: {
    name: 'alkove-spot-1-preview',
    file: 'alkove-spot-1-preview.alk',
    bytes: 2774380,
    sha256: 'd7cbb73a4a027144040ebabba7ec361201e9b7ec2cbdb7a6c8e0e3b15e97de66',
  },
  timbre: {
    name: 'alkove-timbre-1-preview',
    file: 'alkove-timbre-1-preview.alk',
    bytes: 739616,
    sha256: '696dd4d8657e252665574a900e738e678f7b7dbf15fb45a53965492915c91c74',
  },
  lex: {
    name: 'alkove-lex-1-preview',
    file: 'alkove-lex-1-preview.alk',
    bytes: 1940212,
    sha256: '2c83ca72eb09b1fdc9e0f3cca83cb2d132c11f7d93a137e793520bfda9e86092',
  },
}

export const FOCUS = {
  chunk: 256,
  print: 192,
  lag: 256,
}

export const CUE = {
  chunk: 480,
  lagSteps: 1,
}

export const SPOT = {
  chunk: 480,
  classes: 59,
  lagSteps: 3,
  speechLagSteps: 1,
  frame: 0.03,
}

// what a print or a word is bound to: a model republished under the same name with other weights does not match
export const modelId = (m: ModelName) => `${MODELS[m].name}@${MODELS[m].sha256.slice(0, 12)}`

export const TIMBRE = {
  model: modelId('timbre'),
  window: 24000,
  min: 280,
  dim: 192,
}

export const LEX = {
  langs: ['fr', 'en', 'es', 'pt', 'it', 'ca', 'ro'] as const,
  beams: 3,
  bos: 59,
  eos: 60,
  vocab: 61,
  word: 33,
  prefix: 39,
}

export type Lang = (typeof LEX.langs)[number]
