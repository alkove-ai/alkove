import { beforeEach, expect, jest, test } from '@jest/globals';

const mockCalls: string[] = [];

jest.mock('../NativeAlkoveLiveKit', () => ({
  __esModule: true,
  default: {
    attach: jest.fn(async (options: string) => void mockCalls.push(`attach ${options}`)),
    detach: jest.fn(() => mockCalls.push('detach')),
    setEnabled: jest.fn((on: boolean) => mockCalls.push(`enabled ${on}`)),
    setStrength: jest.fn((s: number) => mockCalls.push(`strength ${s}`)),
    setVoiceprint: jest.fn((v: string) => mockCalls.push(`voiceprint ${v}`)),
    processed: jest.fn(() => 0),
  },
}));

const { AlkoveFocus } = require('../index');

beforeEach(() => {
  mockCalls.length = 0;
});

test('settings given before the attach, and before a rejoin, go with every attach', async () => {
  const f = AlkoveFocus({ normalize: true });
  f.setStrength(0.5);
  f.setVoiceprint({ alkove: 'voiceprint', model: 'm' });
  f.setEnabled(false);
  await f.init({ track: {} as never, kind: 'audio' as never });
  const attached = mockCalls.find((c) => c.startsWith('attach'))!;
  expect(JSON.parse(attached.slice('attach '.length))).toEqual({ normalize: true, strength: 0.5, voiceprint: { alkove: 'voiceprint', model: 'm' } });
  expect(mockCalls[mockCalls.length - 1]).toBe('enabled false');
  await f.destroy();
  f.setStrength(0.25);
  mockCalls.length = 0;
  await f.init({ track: {} as never, kind: 'audio' as never });
  expect(JSON.parse(mockCalls[0]!.slice('attach '.length))).toMatchObject({ strength: 0.25 });
});
