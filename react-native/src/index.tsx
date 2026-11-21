import { useEffect, useRef, useState } from 'react';
import { PermissionsAndroid, Platform } from 'react-native';

import Native, { type NativeEvent } from './NativeAlkove';
import { same } from './same';

export type Lang = 'fr' | 'en' | 'es' | 'pt' | 'it' | 'ca' | 'ro';
export type Mode = 'raw' | 'focus' | 'focus+print' | 'spot' | 'spot+print';
export type Status = 'starting' | 'running' | 'interrupted' | 'disposed';
export type Context = 'pause' | 'anywhere' | 'both';
export type Usage = 'call' | 'agent' | 'wake';

export interface Warning {
  code: string;
  message: string;
}

export interface WakeEvent {
  word: string;
  score: number;
  threshold: number;
  context: 'pause' | 'anywhere';
  start: number;
  end: number;
  // the word's raw audio at 16 kHz
  audio?: Float32Array;
  // with a voiceprint: the voice's likeness to it
  voice?: number;
}

export interface Diag {
  code: string;
  level: 'info' | 'warn';
  message?: string;
  [key: string]: unknown;
}

export interface PhraseLevel {
  voiceDb: number | null;
  noiseDb: number;
  refDb: number | null;
}

export type VoiceprintJSON = Record<string, unknown>;

export interface InputEvents {
  speechstart: { start: number };
  speechend: { start: number; end: number; level?: PhraseLevel };
  speechcancel: { start: number };
  turn: { pause: number; probability: number; hesitation: number; at: number };
  turnend: { start: number; end: number; probability: number; hesitation: number; at: number };
  interruptstart: { start: number };
  interrupt: { start: number; at: number };
  interruptcancel: { start: number; end: number };
  utterance: { pcm: ArrayBuffer; start: number; end: number; level?: PhraseLevel };
  audio: ArrayBuffer;
  voiceprint: VoiceprintJSON;
  learn: { progress: number };
  wake: WakeEvent;
  heard: { speech: number; scores: { word: string; score: number }[] };
  trace: Record<string, unknown>[];
  mode: { mode: Mode; reason?: string };
  status: Status;
  diag: Diag;
  mic: { state: 'muted' | 'unmuted' | 'lost' | 'reopened' | 'denied' };
}

export class AlkoveError extends Error {
  override readonly name = 'AlkoveError';

  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

// a word compiled by Spot, as the web SDK's: store toJSON(), give it back as is
export class Word {
  readonly text: string;

  constructor(
    private readonly json: Record<string, unknown>,
    readonly warnings: Warning[] = [],
  ) {
    this.text = String(json.text);
  }

  toJSON(): Record<string, unknown> {
    return this.json;
  }

  static fromJSON(json: Record<string, unknown> | string): Word {
    return new Word(typeof json === 'string' ? JSON.parse(json) : json);
  }
}

const errorOf = (e: unknown): Error => {
  const n = e as { code?: string; message?: string };
  return n && typeof n.code === 'string' && n.code.includes('.') ? new AlkoveError(n.code, n.message ?? n.code) : e instanceof Error ? e : new Error(String(e));
};

export interface Config {
  // a relative path is a folder shipped in the app (Android's assets, the iOS bundle); an absolute path or a file: URL, a folder on the device
  modelBaseUrl?: string;
}

let config: Config = {};

export interface UsageOptions {
  voiceprint?: boolean;
  typedWords?: boolean;
}

export interface Progress {
  loaded: number;
  total: number;
}

let ids = 0;
const listeners = new Map<number, (e: NativeEvent) => void>();
let subscribed = false;

function listen(id: number, fn: (e: NativeEvent) => void): () => void {
  if (!subscribed) {
    subscribed = true;
    Native.onEvent((e) => listeners.get(e.id)?.(e));
  }
  listeners.set(id, fn);
  return () => void listeners.delete(id);
}

const langsOf = (lang: Lang | readonly Lang[] | undefined): Lang[] => (lang === undefined ? [] : typeof lang === 'string' ? [lang] : [...lang]);

function usageOf(usage: Usage): Usage {
  if (usage !== 'call' && usage !== 'agent' && usage !== 'wake') throw new TypeError(`usage: 'call', 'agent' or 'wake', not ${String(usage)}`);
  return usage;
}

function where(base: string | undefined): { url?: string; dir?: string; bundled?: string } {
  if (!base) return {};
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(base)?.[1]?.toLowerCase();
  if (scheme === 'http' || scheme === 'https') return { url: base };
  if (scheme === 'file') return { dir: decodeURIComponent(base.replace(/^file:(\/\/[^/]*)?/i, '')) };
  if (scheme) throw new TypeError(`modelBaseUrl: an http(s) URL, a path or a file: URL, not ${scheme}:`);
  if (base.startsWith('/')) return { dir: base };
  return { bundled: base.replace(/^(\.\/)+/, '').replace(/\/+$/, '') };
}

export const Alkove = {
  // before the first input: a model already loaded keeps its source
  configure(c: Config): void {
    const next = { ...config, ...c };
    const json = JSON.stringify(where(next.modelBaseUrl));
    config = next;
    Native.configure(json);
  },

  // a settings or onboarding screen: everything a usage needs, downloaded ahead
  async preload(usage: Usage, opts: UsageOptions & { onProgress?: (p: Progress) => void } = {}): Promise<void> {
    const id = ++ids;
    const onProgress = opts.onProgress;
    const off = onProgress ? listen(id, (e) => e.type === 'progress' && onProgress(JSON.parse(e.json))) : null;
    try {
      await Native.preload(id, usageOf(usage), !!opts.voiceprint, !!opts.typedWords);
    } catch (e) {
      throw errorOf(e);
    } finally {
      off?.();
    }
  },

  // true when everything is already on the device: an input can start without network
  async cached(usage: Usage, opts: UsageOptions = {}): Promise<boolean> {
    try {
      return await Native.cached(usageOf(usage), !!opts.voiceprint, !!opts.typedWords);
    } catch (e) {
      throw errorOf(e);
    }
  },
};

export const Spot = {
  // any typed word, no training: Lex turns it into phonemes for Spot
  async compile(text: string, opts: { lang?: Lang | Lang[] } = {}): Promise<Word> {
    try {
      const r = JSON.parse(await Native.compile(text, langsOf(opts.lang)));
      return new Word(r.word, r.warnings);
    } catch (e) {
      throw errorOf(e);
    }
  },
};

export type Handlers = { [K in keyof InputEvents]?: (value: InputEvents[K]) => void };

export interface HookOptions {
  enabled?: boolean;
  on?: Handlers;
}

export interface WakeOptions {
  words: readonly (Word | Record<string, unknown> | string)[];
  context?: Context;
  // only this voice wakes it: a print's JSON, or a few seconds of the user alone at 16 kHz
  voiceprint?: VoiceprintJSON | number[] | null;
  trace?: boolean;
}

export interface AgentOptions {
  voiceprint?: VoiceprintJSON | number[] | null;
  output?: Partial<{ format: 'f32' | 's16'; rate: number; chunkMs: number }>;
  near?: boolean;
  turn?: Partial<{ threshold: number; minPause: number; maxPause: number }>;
  wake?: { words: readonly (Word | Record<string, unknown> | string)[]; lang?: Lang | Lang[]; context?: Context };
  onlyMyVoice?: boolean;
  keepRecent?: number;
  trace?: boolean;
}

// what a running input takes from the app, as the web input's methods
export interface InputHandle {
  listen(on: boolean): void;
  hold(): void;
  release(): void;
  farEndActive(on: boolean): void;
  // the last keepRecent seconds as a stereo WAV, null without keepRecent
  recent(): Promise<ArrayBuffer | null>;
  // the next seconds (up to 600) as recent() gives them; engine.failed before the models run
  record(seconds: number): Promise<ArrayBuffer>;
  // since the models started, null before
  stats(): Promise<{ speechSeconds: number; seconds: number; noiseRemovedDb: number | null } | null>;
}

export interface InputState {
  status: Status | 'off';
  mode: Mode | null;
  error: Error | null;
  input: InputHandle | null;
}

// every hook's inputs open one after the other: a remount opening while the previous input still loads would hold the mic twice
let queue: Promise<unknown> = Promise.resolve();

async function micAllowed(): Promise<void> {
  if (Platform.OS !== 'android') return;
  const r = await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.RECORD_AUDIO);
  if (r !== PermissionsAndroid.RESULTS.GRANTED) throw new AlkoveError('mic.denied', 'The microphone is not allowed.');
}

const wordJSON = (w: Word | Record<string, unknown> | string) => (typeof w === 'string' ? w : JSON.stringify(w instanceof Word ? w.toJSON() : w));

function bytes(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

function decode(type: string, json: string): unknown {
  const v = JSON.parse(json);
  if (type === 'audio') return bytes(v);
  if (type === 'utterance') return { ...v, pcm: bytes(v.pcm) };
  if (type === 'wake' && typeof v.audio === 'string') return { ...v, audio: new Float32Array(bytes(v.audio)) };
  return v;
}

const LIVE = ['voiceprint'] as const;
type Live = { voiceprint?: VoiceprintJSON | number[] | null };

function useInput(kind: 'wake' | 'agent', options: object & HookOptions, live: readonly (keyof Live)[]): InputState {
  const { enabled = true, on, ...opts } = options;
  // the continuous audio and the phrases' PCM only cross to JS for an app that listens to them
  const rest = (kind === 'agent' ? { ...opts, audio: !!on?.audio, utterances: !!on?.utterance } : opts) as Record<string, unknown> & Live;
  const handlers = useRef(on);
  const latest = useRef(rest);
  const structural = Object.fromEntries(Object.entries(rest).filter(([k]) => !(live as readonly string[]).includes(k)));
  const shape = useRef({ value: structural as object, version: 0 });
  const version = same(shape.current.value, structural) ? shape.current.version : shape.current.version + 1;

  // refs written once React commits: a render it throws away changes nothing
  useEffect(() => {
    handlers.current = on;
    latest.current = rest;
    if (version !== shape.current.version) shape.current = { value: structural, version };
  });

  const [status, setStatus] = useState<Status | 'off'>('off');
  const [mode, setMode] = useState<Mode | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [input, setInput] = useState<InputHandle | null>(null);
  const current = useRef(0);
  const applied = useRef<Live>({});

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let id = 0;
    let off: (() => void) | null = null;
    setStatus('starting');
    setError(null);
    const opening = queue.then(async () => {
      if (cancelled) return;
      await micAllowed();
      if (cancelled) return;
      const o = latest.current;
      id = ++ids;
      off = listen(id, (e) => {
        // an input closing after the cleanup: its last events no longer belong to this render
        if (cancelled) return;
        const v = decode(e.type, e.json);
        if (e.type === 'status') setStatus(v as Status);
        if (e.type === 'mode') setMode((v as { mode: Mode }).mode);
        (handlers.current?.[e.type as keyof InputEvents] as ((v: unknown) => void) | undefined)?.(v);
      });
      const wake = o.wake as AgentOptions['wake'];
      const payload =
        kind === 'wake'
          ? { ...o, words: (o.words as WakeOptions['words']).map(wordJSON), context: o.context ?? 'pause' }
          : wake
            ? { ...o, wake: { words: wake.words.map(wordJSON), lang: langsOf(wake.lang), context: wake.context ?? 'pause' } }
            : o;
      applied.current = { voiceprint: o.voiceprint ?? null };
      const r = JSON.parse(await Native.start(id, kind, JSON.stringify(payload)));
      if (cancelled) return;
      current.current = id;
      const call = (method: string, arg: unknown = null) => Native.call(id, method, JSON.stringify(arg));
      const recent = async () => {
        const wav = await Native.recent(id);
        return wav ? bytes(wav) : null;
      };
      const record = async (seconds: number) => {
        try {
          return bytes(await Native.record(id, seconds));
        } catch (e) {
          throw errorOf(e);
        }
      };
      const stats = async () => JSON.parse(await Native.stats(id));
      setInput({ listen: (on) => call('listen', on), hold: () => call('hold'), release: () => call('release'), farEndActive: (on) => call('farEndActive', on), recent, record, stats });
      setStatus(r.status);
      setMode(r.mode);
    });
    queue = opening.catch(() => {});
    opening.catch((e) => {
      if (cancelled) return;
      off?.();
      setError(errorOf(e));
      setStatus('off');
    });
    return () => {
      cancelled = true;
      current.current = 0;
      // after its opening, failed or not: a close sent before the start would leave the input open
      void opening
        .catch(() => {})
        .then(() => {
          off?.();
          if (id) Native.close(id);
        });
      setInput(null);
      setStatus('off');
      setMode(null);
      setError(null);
    };
  }, [enabled, version, kind]);

  const voiceprint = live.includes('voiceprint') ? rest.voiceprint : undefined;
  useEffect(() => {
    const id = current.current;
    if (!id) return;
    if (live.includes('voiceprint') && !same(voiceprint ?? null, applied.current.voiceprint ?? null)) {
      applied.current.voiceprint = voiceprint ?? null;
      Native.call(id, 'setVoiceprint', JSON.stringify(applied.current.voiceprint));
    }
  }, [input, voiceprint, live]);

  return { status, mode, error, input };
}

export function useWakeListener(options: WakeOptions & HookOptions): InputState {
  return useInput('wake', options, []);
}

export function useAgentInput(options: AgentOptions & HookOptions = {}): InputState {
  return useInput('agent', options, LIVE);
}
