import type { Engine } from '../core/engine'
import type { Lang } from '../manifest'
import { Timbre } from '../core/timbre'
import { type ProcessOptions, processPcm, Processor } from '../process'

// what the background worker (or Node, in its own thread) answers, without state of its own beyond the loaded models
export function makeCalls(engine: () => Engine) {
  let timbre: Timbre | null = null
  const getTimbre = () => (timbre ??= new Timbre(engine().net('timbre')))
  const streams = new Map<number, Processor>()
  let next = 0
  return {
    getTimbre,
    embed: (pcm: Float32Array, source?: 'raw' | 'focus') => getTimbre().embed(pcm, source).toJSON(),
    windows: (pcm: Float32Array) => getTimbre().windows(pcm).map((w) => ({ embedding: w.embedding, id: w.id, seconds: w.seconds })),
    phonemize: (texts: string[], lang: Lang) => engine().lex().phonemize(texts, lang),
    // the warnings stay in the worker, as they did
    compile: (text: string, opts: { lang?: Lang | Lang[]; chain?: string }) => engine().compile(text, opts.lang === undefined ? [] : [opts.lang].flat(), opts.chain).word,
    process: (pcm: Float32Array, opts: Omit<ProcessOptions, 'onProgress' | 'signal'>, onProgress?: ProcessOptions['onProgress']) => {
      const r = processPcm(engine(), pcm, { ...opts, onProgress })
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
export type CallName = Exclude<keyof Calls, 'getTimbre'>
