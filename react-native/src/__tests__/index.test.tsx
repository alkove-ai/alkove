import { beforeEach, expect, jest, test } from '@jest/globals';
import { act, create } from 'react-test-renderer';
import { createElement } from 'react';

type Ev = { id: number; type: string; json: string };
const mockCalls: string[] = [];
let mockEmit: (e: Ev) => void = () => {};
let mockGate: Promise<void> | null = null;

jest.mock('../NativeAlkove', () => ({
  __esModule: true,
  default: {
    configure: jest.fn((config: string) => mockCalls.push(`configure ${config}`)),
    preload: jest.fn(async (id: number, usage: string, voiceprint: boolean, typedWords: boolean) => void mockCalls.push(`preload ${id} ${usage} ${voiceprint} ${typedWords}`)),
    cached: jest.fn(async (usage: string, voiceprint: boolean, typedWords: boolean) => (mockCalls.push(`cached ${usage} ${voiceprint} ${typedWords}`), usage === 'wake')),
    recent: jest.fn(async () => ''),
    record: jest.fn(async () => ''),
    stats: jest.fn(async () => 'null'),
    compile: jest.fn(async (text: string, langs: string[]) => JSON.stringify({ word: { alkove: 'word', text, lang: langs }, warnings: [{ code: 'word.too-short', message: 'short' }] })),
    start: jest.fn(async (id: number, kind: string, options: string) => {
      mockCalls.push(`start ${id} ${kind} ${options}`);
      if (mockGate) await mockGate;
      return JSON.stringify({ mode: 'raw', status: 'running' });
    }),
    call: jest.fn((id: number, method: string, arg: string) => mockCalls.push(`call ${id} ${method} ${arg}`)),
    close: jest.fn((id: number) => mockCalls.push(`close ${id}`)),
    onEvent: jest.fn((fn: (e: Ev) => void) => {
      mockEmit = fn;
      return { remove() {} };
    }),
  },
}));

jest.mock('react-native', () => ({ Platform: { OS: 'ios' }, PermissionsAndroid: {} }));

const { Alkove, Spot, Word, useAgentInput, useWakeListener } = require('../index');

const flush = () => act(async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
});

function mount(hook: () => unknown) {
  const out: { current: any } = { current: null };
  function C() {
    out.current = hook();
    return null;
  }
  let r: any;
  act(() => {
    r = create(createElement(C));
  });
  const rerender = () => act(() => r.update(createElement(C)));
  return { out, r, rerender };
}

beforeEach(() => {
  mockCalls.length = 0;
  mockGate = null;
});

const b64 = (bytes: number[]) => Buffer.from(bytes).toString('base64');

test('Alkove.configure sends a URL, a folder on the device or a folder shipped in the app; a later call keeps what it does not name', () => {
  const sent = (c: object) => {
    mockCalls.length = 0;
    Alkove.configure(c);
    return JSON.parse(mockCalls[0]!.slice('configure '.length));
  };
  expect(sent({ modelBaseUrl: 'https://example.com/alkove/' })).toEqual({ url: 'https://example.com/alkove/' });
  expect(sent({})).toEqual({ url: 'https://example.com/alkove/' });
  expect(sent({ modelBaseUrl: '/data/user/0/app/files/alkove' })).toEqual({ dir: '/data/user/0/app/files/alkove' });
  expect(sent({ modelBaseUrl: 'file:///var/mobile/My%20Models/' })).toEqual({ dir: '/var/mobile/My Models/' });
  expect(sent({ modelBaseUrl: './alkove/' })).toEqual({ bundled: 'alkove' });
  expect(sent({ modelBaseUrl: undefined })).toEqual({});
  expect(() => Alkove.configure({ modelBaseUrl: 'content://x/alkove' })).toThrow(TypeError);
  expect(sent({})).toEqual({});
});

test('Spot.compile gives a Word with its warnings, its JSON as the native side gave it', async () => {
  const w = await Spot.compile('Hey Nova', { lang: 'en' });
  expect(w).toBeInstanceOf(Word);
  expect(w.text).toBe('Hey Nova');
  expect(w.toJSON()).toEqual({ alkove: 'word', text: 'Hey Nova', lang: ['en'] });
  expect(w.warnings).toEqual([{ code: 'word.too-short', message: 'short' }]);
});

test('useAgentInput opens with its options as JSON, gives status and mode, forwards events, decodes the audio', async () => {
  const got: any[] = [];
  const { out, r } = mount(() => useAgentInput({ near: true, turn: { maxPause: 2 }, on: { utterance: (u: any) => got.push(u), audio: (a: any) => got.push(a), turnend: (t: any) => got.push(t) } }));
  await flush();
  expect(mockCalls[0]).toMatch(/^start \d+ agent /);
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!)).toEqual({ near: true, turn: { maxPause: 2 }, audio: true, utterances: true });
  expect(out.current.status).toBe('running');
  const id = Number(mockCalls[0]!.split(' ')[1]);
  for (const bytes of [[1], [1, 2], [1, 2, 3], [255, 0, 128, 7]]) {
    act(() => mockEmit({ id, type: 'audio', json: JSON.stringify(b64(bytes)) }));
    expect(Array.from(new Uint8Array(got.pop()))).toEqual(bytes);
  }
  act(() => mockEmit({ id, type: 'utterance', json: JSON.stringify({ pcm: b64([9, 8, 7, 6]), start: 1, end: 2 }) }));
  expect(Array.from(new Uint8Array(got[0].pcm))).toEqual([9, 8, 7, 6]);
  act(() => mockEmit({ id, type: 'mode', json: JSON.stringify({ mode: 'focus' }) }));
  expect(out.current.mode).toBe('focus');
  act(() => mockEmit({ id: id + 99, type: 'turnend', json: '{}' }));
  expect(got).toHaveLength(1);
  act(() => out.current.input.farEndActive(true));
  expect(mockCalls).toContain(`call ${id} farEndActive true`);
  act(() => r.unmount());
  await flush();
  expect(mockCalls).toContain(`close ${id}`);
});

test("the continuous audio and the phrases' PCM cross only for an app that listens to them", async () => {
  mount(() => useAgentInput({}));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!)).toEqual({ audio: false, utterances: false });
});

test("an agent's wake words go to the native side as JSON texts, typed ones as typed, with their lang and pause by default", async () => {
  const word = new Word({ alkove: 'word', text: 'Hey Nova' });
  mount(() => useAgentInput({ wake: { words: ['ok luciole', word, { alkove: 'word', text: 'x' }], lang: 'fr' } }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!).wake).toEqual({ words: ['ok luciole', '{"alkove":"word","text":"Hey Nova"}', '{"alkove":"word","text":"x"}'], lang: ['fr'], context: 'pause' });
});

test("an agent's wake words take several languages, or none for compiled words, and any context", async () => {
  mount(() => useAgentInput({ wake: { words: ['ok luciole', 'hey nova'], lang: ['fr', 'en'], context: 'both' } }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!).wake).toEqual({ words: ['ok luciole', 'hey nova'], lang: ['fr', 'en'], context: 'both' });
  mockCalls.length = 0;
  mount(() => useAgentInput({ wake: { words: [{ alkove: 'word', text: 'x' }] } }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!).wake).toEqual({ words: ['{"alkove":"word","text":"x"}'], lang: [], context: 'pause' });
});

test('a wake listener sends its voiceprint, a print or a recording, its context and trace', async () => {
  const words = [{ alkove: 'word', text: 'x' }];
  mount(() => useWakeListener({ words, voiceprint: { alkove: 'voiceprint', model: 'm' }, context: 'both', trace: true }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' wake ')[1]!)).toEqual({ words: ['{"alkove":"word","text":"x"}'], voiceprint: { alkove: 'voiceprint', model: 'm' }, context: 'both', trace: true });
  mockCalls.length = 0;
  mount(() => useWakeListener({ words, voiceprint: [0.25, -0.5] }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' wake ')[1]!)).toEqual({ words: ['{"alkove":"word","text":"x"}'], voiceprint: [0.25, -0.5], context: 'pause' });
});

test("a wake listener's voiceprint reopens it, as on the web", async () => {
  let props: any = { words: [{ alkove: 'word', text: 'x' }], voiceprint: [0.25] };
  const { rerender } = mount(() => useWakeListener(props));
  await flush();
  const first = Number(mockCalls[0]!.split(' ')[1]);
  props = { ...props, voiceprint: [0.5] };
  rerender();
  await flush();
  expect(mockCalls).toContain(`close ${first}`);
  expect(mockCalls.filter((c) => c.startsWith('start'))).toHaveLength(2);
  expect(mockCalls.some((c) => c.includes('setVoiceprint'))).toBe(false);
});

test('keepRecent and trace go to the native side; recent() gives the WAV as bytes, null without it', async () => {
  const got: any[] = [];
  const { out } = mount(() => useAgentInput({ keepRecent: 20, trace: true, on: { trace: (t: any) => got.push(t), heard: (h: any) => got.push(h) } }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!)).toEqual({ keepRecent: 20, trace: true, audio: false, utterances: false });
  const id = Number(mockCalls[0]!.split(' ')[1]);
  act(() => mockEmit({ id, type: 'trace', json: JSON.stringify([{ type: 'trace', step: 'focus', at: 1 }]) }));
  act(() => mockEmit({ id, type: 'heard', json: JSON.stringify({ speech: 0.5, scores: [{ word: 'x', score: 0.25 }] }) }));
  expect(got).toEqual([[{ type: 'trace', step: 'focus', at: 1 }], { speech: 0.5, scores: [{ word: 'x', score: 0.25 }] }]);
  const Native = require('../NativeAlkove').default;
  Native.recent.mockImplementationOnce(async () => b64([82, 73, 70, 70]));
  expect(Array.from(new Uint8Array((await out.current.input.recent())!))).toEqual([82, 73, 70, 70]);
  expect(Native.recent).toHaveBeenLastCalledWith(id);
  expect(await out.current.input.recent()).toBeNull();
});

test('record() gives the next seconds as WAV bytes, its rejection as an AlkoveError; stats() the web shape, null before the models', async () => {
  const { out } = mount(() => useAgentInput({}));
  await flush();
  const id = Number(mockCalls[0]!.split(' ')[1]);
  const Native = require('../NativeAlkove').default;
  Native.record.mockImplementationOnce(async () => b64([82, 73, 70, 70]));
  expect(Array.from(new Uint8Array(await out.current.input.record(10)))).toEqual([82, 73, 70, 70]);
  expect(Native.record).toHaveBeenLastCalledWith(id, 10);
  Native.record.mockImplementationOnce(async () => {
    throw Object.assign(new Error('The models are not running.'), { code: 'engine.failed' });
  });
  await expect(out.current.input.record(1)).rejects.toMatchObject({ name: 'AlkoveError', code: 'engine.failed', message: 'The models are not running.' });
  Native.stats.mockImplementationOnce(async () => JSON.stringify({ speechSeconds: 1.5, seconds: 4, noiseRemovedDb: null }));
  expect(await out.current.input.stats()).toEqual({ speechSeconds: 1.5, seconds: 4, noiseRemovedDb: null });
  expect(Native.stats).toHaveBeenLastCalledWith(id);
  expect(await out.current.input.stats()).toBeNull();
});

test("a wake's audio comes as a Float32Array of the word, little-endian as the native sides write it, on a wake listener and on an agent", async () => {
  const samples = [0.25, -0.5, 1, -1, 0.125];
  const d = new DataView(new ArrayBuffer(4 * samples.length));
  samples.forEach((x, i) => d.setFloat32(4 * i, x, true));
  const audio = Buffer.from(d.buffer).toString('base64');
  const wake = { word: 'x', score: 0.9, threshold: 0.5, context: 'pause', start: 1, end: 1.5 };
  for (const kind of ['wake', 'agent']) {
    const got: any[] = [];
    mockCalls.length = 0;
    const on = { wake: (w: any) => got.push(w) };
    mount(() => (kind === 'wake' ? useWakeListener({ words: [{ alkove: 'word', text: 'x' }], on }) : useAgentInput({ wake: { words: ['x'], lang: 'en' }, on })));
    await flush();
    const id = Number(mockCalls[0]!.split(' ')[1]);
    act(() => mockEmit({ id, type: 'wake', json: JSON.stringify({ ...wake, audio }) }));
    act(() => mockEmit({ id, type: 'wake', json: JSON.stringify(wake) }));
    expect(got[0].audio).toBeInstanceOf(Float32Array);
    expect(Array.from(got[0].audio)).toEqual(samples);
    expect(got[0]).toMatchObject(wake);
    expect(got[1]).toEqual(wake);
  }
});

test("Alkove.preload sends the usage and what it adds, gives its own download's progress until it ends", async () => {
  const Native = require('../NativeAlkove').default;
  const seen: unknown[] = [];
  let id = 0;
  Native.preload.mockImplementationOnce(async (i: number, ...rest: unknown[]) => {
    id = i;
    mockCalls.push(`preload ${i} ${rest.join(' ')}`);
    mockEmit({ id: i, type: 'progress', json: JSON.stringify({ loaded: 1, total: 4 }) });
    mockEmit({ id: i + 1000, type: 'progress', json: JSON.stringify({ loaded: 9, total: 9 }) });
    mockEmit({ id: i, type: 'progress', json: JSON.stringify({ loaded: 4, total: 4 }) });
  });
  await Alkove.preload('wake', { voiceprint: true, typedWords: true, onProgress: (p: unknown) => seen.push(p) });
  expect(mockCalls).toEqual([`preload ${id} wake true true`]);
  expect(seen).toEqual([{ loaded: 1, total: 4 }, { loaded: 4, total: 4 }]);
  mockEmit({ id, type: 'progress', json: JSON.stringify({ loaded: 5, total: 4 }) });
  expect(seen).toHaveLength(2);
  await Alkove.preload('call');
  expect(mockCalls[1]).toMatch(/^preload \d+ call false false$/);
});

test('Alkove.preload: a native rejection gives an AlkoveError, an unknown usage a TypeError', async () => {
  const Native = require('../NativeAlkove').default;
  Native.preload.mockImplementationOnce(async () => {
    throw Object.assign(new Error('HTTP 404'), { code: 'models.fetch' });
  });
  await expect(Alkove.preload('agent')).rejects.toMatchObject({ name: 'AlkoveError', code: 'models.fetch' });
  await expect(Alkove.preload('focus')).rejects.toBeInstanceOf(TypeError);
  await expect(Alkove.cached('focus')).rejects.toBeInstanceOf(TypeError);
  expect(mockCalls).toEqual([]);
});

test("Alkove.cached sends the usage and what it adds, and gives the native side's answer", async () => {
  expect(await Alkove.cached('wake', { typedWords: true })).toBe(true);
  expect(await Alkove.cached('agent')).toBe(false);
  expect(mockCalls).toEqual(['cached wake false true', 'cached agent false false']);
});

test('a recording as the voiceprint goes to the native side as numbers, at the start and live', async () => {
  let props: any = { voiceprint: [0.25, -0.5, 0.125] };
  const { rerender } = mount(() => useAgentInput(props));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!).voiceprint).toEqual([0.25, -0.5, 0.125]);
  const id = Number(mockCalls[0]!.split(' ')[1]);
  props = { voiceprint: [0.5, 0.25] };
  rerender();
  await flush();
  expect(mockCalls.filter((c) => c.startsWith('start'))).toHaveLength(1);
  expect(mockCalls).toContain(`call ${id} setVoiceprint [0.5,0.25]`);
});

test('onlyMyVoice goes to the native side as it is', async () => {
  mount(() => useAgentInput({ onlyMyVoice: true }));
  await flush();
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!)).toEqual({ onlyMyVoice: true, audio: false, utterances: false });
});

test('an event the input sends after the cleanup changes nothing: switched off while opening, it stays off', async () => {
  let open!: () => void;
  mockGate = new Promise((res) => (open = res));
  let props: any = { enabled: true };
  const { out, rerender } = mount(() => useAgentInput(props));
  await flush();
  const id = Number(mockCalls[0]!.split(' ')[1]);
  props = { enabled: false };
  rerender();
  act(() => mockEmit({ id, type: 'status', json: JSON.stringify('running') }));
  act(() => mockEmit({ id, type: 'mode', json: JSON.stringify({ mode: 'focus' }) }));
  open();
  await flush();
  expect(out.current).toMatchObject({ status: 'off', mode: null, input: null });
  expect(mockCalls).toContain(`close ${id}`);
});

test('unmounted after a failed opening: no unhandled rejection', async () => {
  const rejected: unknown[] = [];
  const onRejection = (e: unknown) => rejected.push(e);
  process.on('unhandledRejection', onRejection);
  const Native = require('../NativeAlkove').default;
  Native.start.mockImplementationOnce(async () => {
    throw Object.assign(new Error('denied'), { code: 'mic.denied' });
  });
  const { r } = mount(() => useAgentInput({}));
  await flush();
  act(() => r.unmount());
  await flush();
  await new Promise((res) => setTimeout(res, 10));
  process.off('unhandledRejection', onRejection);
  expect(rejected).toEqual([]);
});

test('the voiceprint goes to the running input; another option opens a new one, after closing the first', async () => {
  let props: any = { near: false };
  const { rerender } = mount(() => useAgentInput(props));
  await flush();
  const first = Number(mockCalls[0]!.split(' ')[1]);
  props = { near: false, voiceprint: { alkove: 'voiceprint', model: 'm' } };
  rerender();
  await flush();
  expect(mockCalls.filter((c) => c.startsWith('start'))).toHaveLength(1);
  expect(mockCalls).toContain(`call ${first} setVoiceprint {"alkove":"voiceprint","model":"m"}`);
  props = { ...props, voiceprint: { alkove: 'voiceprint', model: 'm' } };
  rerender();
  await flush();
  expect(mockCalls.filter((c) => c.includes('setVoiceprint'))).toHaveLength(1);
  props = { ...props, near: true };
  rerender();
  await flush();
  const starts = mockCalls.filter((c) => c.startsWith('start'));
  expect(starts.length).toBeGreaterThanOrEqual(2);
  expect(mockCalls.indexOf(`close ${first}`)).toBeLessThan(mockCalls.indexOf(starts[starts.length - 1]!));
});

test('unmounted while opening: the input opened late is closed', async () => {
  let open!: () => void;
  mockGate = new Promise((res) => (open = res));
  const { r } = mount(() => useWakeListener({ words: [{ alkove: 'word', text: 'x' }] }));
  await flush();
  act(() => r.unmount());
  open();
  await flush();
  const id = Number(mockCalls[0]!.split(' ')[1]);
  expect(mockCalls).toEqual([expect.stringMatching(/^start \d+ wake /), `close ${id}`]);
});

test('a native rejection with a code gives an AlkoveError and status off', async () => {
  const Native = require('../NativeAlkove').default;
  Native.start.mockImplementationOnce(async () => {
    throw Object.assign(new Error('The microphone is not allowed.'), { code: 'mic.denied' });
  });
  const { out } = mount(() => useAgentInput({}));
  await flush();
  expect(out.current.status).toBe('off');
  expect(out.current.error).toMatchObject({ name: 'AlkoveError', code: 'mic.denied' });
});
