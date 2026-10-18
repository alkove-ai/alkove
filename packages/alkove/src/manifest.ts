export type ModelName = 'focus' | 'cue' | 'spot' | 'timbre' | 'lex'

export interface ModelFile {
  name: string
  file: string
  bytes: number
  sha256: string
}

export const RATE = 16000

// the Hugging Face commit the files are read at, set when they are published (sha256 checked either way)
export const HF_REVISION = 'main'
export const HF_ORG = 'alkove-ai'

export const MODELS: Record<ModelName, ModelFile> = {
  focus: {
    name: 'alkove-focus-1-preview',
    file: 'alkove-focus-1-preview.onnx',
    bytes: 931249,
    sha256: '67753cbc5b1da4292e118782f5d77e39c4bf7d26e3421ac28905369b4559937d',
  },
  cue: {
    name: 'alkove-cue-1-preview',
    file: 'alkove-cue-1-preview.onnx',
    bytes: 1156915,
    sha256: '00a8a49b08653d067dba276a6bd8c3868a06b154d6cb72ef0d7211c3981f340a',
  },
  spot: {
    name: 'alkove-spot-1-preview',
    file: 'alkove-spot-1-preview.onnx',
    bytes: 2860508,
    sha256: 'aa8ce7259528a78db9427a8d970c35be72e0c070af705f8c668a0b1e3cb02dc6',
  },
  timbre: {
    name: 'alkove-timbre-1-preview',
    file: 'alkove-timbre-1-preview.onnx',
    bytes: 867280,
    sha256: '53032a255010770e232dcc43611132996a4db85fc9272978caaa9f9498bcb437',
  },
  lex: {
    name: 'alkove-lex-1-preview',
    file: 'alkove-lex-1-preview.onnx',
    bytes: 1775231,
    sha256: '9af6466a7def8916a81349c830eb25bd81f20e5307338d5b732a2dac3476defe',
  },
}

export const FOCUS = {
  chunk: 256,
  state: 126848,
  print: 192,
  lag: 256,
}

export const CUE = {
  chunk: 480,
  state: 5848,
  lagSteps: 1,
}

export const SPOT = {
  chunk: 480,
  state: 19672,
  classes: 59,
  lagSteps: 3,
  speechLagSteps: 1,
  frame: 0.03,
}

export const TIMBRE = {
  model: 'alkove-timbre-1-preview',
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

// what each model's stream takes and gives, the state looped back by the engine
export const STREAMS = {
  focus: {
    shapes: { audio: [1, FOCUS.chunk], print: [1, FOCUS.print], state: [FOCUS.state], audio_out: [1, FOCUS.chunk] },
    loops: [['state', 'next_state']] as [string, string][],
  },
  cue: {
    shapes: { audio: [1, CUE.chunk], state: [CUE.state], speech: [1, 1] },
    loops: [['state', 'next_state']] as [string, string][],
  },
  spot: {
    shapes: { audio: [1, SPOT.chunk], state: [SPOT.state], logp: [1, 1, SPOT.classes], speech: [1, 1] },
    loops: [['state', 'next_state']] as [string, string][],
  },
}
