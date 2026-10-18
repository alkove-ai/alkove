import type { Engine } from '../core/engine'
import { Lex } from '../core/lex'
import { compileWord } from '../core/spot/spot'
import { Timbre } from '../core/timbre'
import { type ProcessOptions, processPcm, Processor } from '../process'

// what the background worker (or Node, in its own thread) answers, without state of its own beyond the loaded models
export function makeCalls(engine: () => Engine) {
  let timbre: Timbre | null = null
  let lex: Lex | null = null
  const getTimbre = () => (timbre ??= new Timbre(engine().net('timbre')))
  const getLex = () => (lex ??= new Lex(engine().net('lex')))
  const streams = new Map<number, Processor>()
  let next = 0
  return {
    getTimbre,
    getLex,
    embed: (pcm: Float32Array, source?: 'raw' | 'focus') => getTimbre().embed(pcm, source).toJSON(),
    windows: (pcm: Float32Array) => getTimbre().windows(pcm).map((w) => ({ embedding: w.embedding, id: w.id, seconds: w.seconds })),
    phonemize: (texts: string[], lang: Parameters<Lex['phonemize']>[1]) => texts.map((t) => getLex().phonemize(t, lang)),
    compile: (text: string, opts: Parameters<typeof compileWord>[1]) => compileWord(text, opts, getLex).toJSON(),
    process: (pcm: Float32Array, opts: Omit<ProcessOptions, 'onProgress' | 'signal'>) => {
      const r = processPcm(engine(), pcm, opts)
      return { audio: r.audio, raw: r.raw, speech: r.speech, wakes: r.wakes, diag: r.diag, trace: r.trace }
    },
    streamOpen: (opts: Omit<ProcessOptions, 'onProgress' | 'signal'>) => {
      streams.set(++next, new Processor(engine(), opts))
      return next
    },
    streamPush: (id: number, pcm: Float32Array) => streams.get(id)!.push(pcm),
    streamEnd: (id: number) => {
      const part = streams.get(id)!.end()
      streams.delete(id)
      return part
    },
  }
}

export type Calls = ReturnType<typeof makeCalls>
export type CallName = Exclude<keyof Calls, 'getTimbre' | 'getLex'>
