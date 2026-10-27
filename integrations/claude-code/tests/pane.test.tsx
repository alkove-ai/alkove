import { expect, test } from 'claude-code/testing'

const PANE = {
  plugin: 'voice-commands',
  component: 'Pane',
  requestId: 'voice-commands',
  props: { title: 'Voice commands', isFocused: false, bodyColumns: 90, placement: 'dock', scroll: { offset: 0, bodyRows: 40 }, view: {} },
} as const

test('the pane picks a language with a press and says the number is optional', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  for (const surface of ['terminal', 'desktop'] as const) {
    const ui = await $.ui.mount({ ...PANE, surface })
    expect((await ui.find({ type: 'Text', text: /optional/ }))?.text).toContain('session 2')
    await ui.press({ key: 'lang-fr' })
    expect((await ui.find({ key: surface === 'terminal' ? 'phrases-abort-0' : 'edit-phrases-abort-0' }))?.text).toBe('stop, arrête, annule')
    await ui.unmount()
  }
})

test('a narrow pane puts each field under its label', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, props: { ...PANE.props, bodyColumns: 40 }, surface: 'terminal' })
  expect((await ui.find({ key: 'phrases-abort-0' }))?.text).toBe('stop, cancel')
  await ui.unmount()
})

test('a command is removed with its cross and added at the end', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  await ui.press({ key: 'remove-abort-0' })
  expect(await ui.find({ key: 'phrases-abort-0' })).toBeUndefined()
  const before = (await ui.findAll({ type: 'Input' })).length
  await ui.press({ key: 'add' })
  expect(await ui.findAll({ type: 'Input' })).toHaveLength(before + 2)
  await ui.unmount()
})

test('a field saves as it is typed and keeps what was typed', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  await ui.input({ key: 'phrases-abort-0', text: 'stop, ', kind: 'change' })
  expect((await ui.find({ key: 'phrases-abort-0' }))?.text).toBe('stop, ')
  await ui.unmount()
})

test('Enter in a field keeps its text', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  await ui.input({ key: 'phrases-abort-0', text: 'stop, halt', kind: 'submit' })
  expect((await ui.find({ key: 'phrases-abort-0~1' }))?.text).toBe('stop, halt')
  await ui.unmount()
})

test('reading aloud shows the voices and the quality, 4 steps by default', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  expect(await ui.find({ key: 'voice-0' })).toBeUndefined()
  await ui.press({ key: 'read-summary' })
  expect((await ui.find({ key: 'voice-5' }))?.text).toBe('1')
  expect((await ui.find({ key: 'steps-4' }))?.props.variant).toBe('primary')
  await ui.unmount()
})

test('a desktop cell shows its text and becomes a field once pressed', async ($, on) => {
  on('store.set', () => ({ value: undefined }))
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  expect(await ui.find({ key: 'phrases-abort-0' })).toBeUndefined()
  await ui.press({ key: 'edit-phrases-abort-0' })
  expect((await ui.find({ key: 'phrases-abort-0' }))?.text).toBe('stop, cancel')
  expect(await ui.find({ key: 'does-run-3' })).toBeUndefined()
  await ui.unmount()
})
