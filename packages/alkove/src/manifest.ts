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
    sha256: 'a63cda32e023b8d76089f4c3df0cc2c69ba36017b8f6cdea1da1094ef8022b6a',
  },
  cue: {
    name: 'alkove-cue-1-preview',
    file: 'alkove-cue-1-preview.alk',
    bytes: 1066516,
    sha256: '977cd614996498f468536effbc2fb05138331751ca21572cf07b24184b4e7911',
  },
  spot: {
    name: 'alkove-spot-1-preview',
    file: 'alkove-spot-1-preview.alk',
    bytes: 2774380,
    sha256: 'fef6b59f0c31c5c742db17d758621f778b23cff5a04834e0c6ec47b313d2ce3d',
  },
  timbre: {
    name: 'alkove-timbre-1-preview',
    file: 'alkove-timbre-1-preview.alk',
    bytes: 739632,
    sha256: 'd498a7dd8b20528d783052b1e1d49e54ab7ee4e47aa12712d04f3d43edc68bd8',
  },
  lex: {
    name: 'alkove-lex-1-preview',
    file: 'alkove-lex-1-preview.alk',
    bytes: 1818020,
    sha256: '1e64ca9f5c2281cadae0f9c4a944cd1b1b433f0c0c29760c36340c97f7308957',
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
  steps: 39,
}

export type Lang = (typeof LEX.langs)[number]
