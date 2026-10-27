import { expect, test } from 'claude-code/testing'

import { defaults, match, upgrade } from '../hooks/commands'

test('a command allows what Whistle gets slightly wrong', async () => {
  const en = defaults('en')
  expect(match('stop.', en)?.action).toBe('abort')
  expect(match('Stopp.', en)?.action).toBe('abort')
  expect(match('sort', en)).toBeUndefined()
  expect(match('and', en)).toBeUndefined()
  expect(match('comment', en)).toBeUndefined()
  expect(match('Run the tests!', en)?.text).toBe('Run the tests.')
  expect(match('refactor the auth middleware', en)).toBeUndefined()
  expect(match('stop using useEffect in this component', en)).toBeUndefined()
})

test('the defaults of each language', async () => {
  expect(match('compacte', defaults('fr'))?.text).toBe('/compact')
  expect(match('lanza los tests', defaults('es'))?.action).toBe('run')
  expect(match('compatta', defaults('it'))?.text).toBe('/compact')
  expect(match('contacts', defaults('fr'))).toBeUndefined()
  expect(match('est face', defaults('fr'))?.action).toBe('clear')
})

test('commands an earlier version stored still work', async () => {
  expect(upgrade({ id: 'c', action: 'compact', phrases: ['compact'], text: null })?.text).toBe('/compact')
  expect(upgrade({ id: 'r', action: 'read', phrases: ['read it'], text: null })).toBeUndefined()
})
