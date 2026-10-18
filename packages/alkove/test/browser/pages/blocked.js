// an input made at page load, before any user gesture; resumed in a click
import * as lib from '/browser/lib.js'

const send = (tag, value) => console.log(`${tag} ${JSON.stringify(value)}`)

try {
  lib.setup()
  const player = new lib.Player()
  const input = await lib.sdk.createCallInput({ stream: player.stream })
  const statuses = [input.status]
  input.on('status', (s) => statuses.push(s))
  document.getElementById('resume').addEventListener('click', async () => {
    await input.resume()
    send('ALKOVE_RESUMED', { status: input.status, statuses })
    input.dispose()
  })
  send('ALKOVE_BLOCKED', { status: input.status, ctx: player.ctx.state })
} catch (e) {
  send('ALKOVE_BLOCKED_ERROR', `${e?.name}: ${e?.message}`)
}
