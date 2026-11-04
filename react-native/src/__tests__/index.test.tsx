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

const { Spot, Word, useAgentInput, useWakeListener } = require('../index');

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

test('Spot.compile gives a Word with its warnings, its JSON as the native side gave it', async () => {
  const w = await Spot.compile('Hey Nova', { lang: 'en' });
  expect(w).toBeInstanceOf(Word);
  expect(w.text).toBe('Hey Nova');
  expect(w.toJSON()).toEqual({ alkove: 'word', text: 'Hey Nova', lang: ['en'] });
  expect(w.warnings).toEqual([{ code: 'word.too-short', message: 'short' }]);
});

test('useAgentInput opens with its options as JSON, gives status and mode, forwards events, decodes the audio', async () => {
  const got: any[] = [];
  const { out, r } = mount(() => useAgentInput({ near: true, on: { utterance: (u: any) => got.push(u), audio: (a: any) => got.push(a), turnend: (t: any) => got.push(t) } }));
  await flush();
  expect(mockCalls[0]).toMatch(/^start \d+ agent /);
  expect(JSON.parse(mockCalls[0]!.split(' agent ')[1]!)).toEqual({ near: true, audio: true, utterances: true });
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

test('strength and voiceprint go to the running input; another option opens a new one, after closing the first', async () => {
  let props: any = { strength: 1, near: false };
  const { rerender } = mount(() => useAgentInput(props));
  await flush();
  const first = Number(mockCalls[0]!.split(' ')[1]);
  props = { strength: 0.5, near: false, voiceprint: { alkove: 'voiceprint', model: 'm' } };
  rerender();
  await flush();
  expect(mockCalls.filter((c) => c.startsWith('start'))).toHaveLength(1);
  expect(mockCalls).toContain(`call ${first} setStrength 0.5`);
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
