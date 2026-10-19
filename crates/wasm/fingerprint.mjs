// node fingerprint.mjs engine.wasm model.alk [runs]: for each plan, the fingerprint of 200 runs on fixed inputs (as examples/bench.rs computes it), then time per run
import { readFileSync } from 'node:fs'

const [wasmPath, alkPath, runsArg] = process.argv.slice(2)
const runs = Number(runsArg ?? 200)
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {})
const e = instance.exports
const bytes = () => new Uint8Array(e.memory.buffer)
const put = (data) => {
  const p = e.alk_alloc(data.length)
  bytes().set(data, p)
  return p
}
const text = (s) => new TextEncoder().encode(s)

const file = readFileSync(alkPath)
const loadStart = performance.now()
const model = e.alk_model(put(file), file.length)
const loadMs = performance.now() - loadStart
if (!model) {
  const p = e.alk_alloc(512)
  throw new Error(new TextDecoder().decode(bytes().slice(p, p + e.alk_error(p, 512))))
}
const readName = (fn) => {
  const nb = e.alk_alloc(256)
  return new TextDecoder().decode(bytes().slice(nb, nb + fn(nb)))
}
const plans = e.alk_plans(model)
for (let plan = 0; plan < plans; plan++) {
  const session = e.alk_session(model, plan)
  const ios = (kind) => {
    const out = []
    for (let i = 0; i < e.alk_io_count(model, plan, kind); i++) {
      const name = readName((nb) => e.alk_io_name(model, plan, kind, i, nb, 256))
      const db = e.alk_alloc(32)
      const rank = e.alk_io_shape(model, plan, kind, i, db, 8)
      const len = Array.from(new Uint32Array(e.memory.buffer, db, rank)).reduce((a, b) => a * b, 1)
      const nm = text(name)
      const ptr = (kind === 0 ? e.alk_input : e.alk_output)(session, put(nm), nm.length)
      out.push({ name, ptr, len })
    }
    return out
  }
  const inputs = ios(0)
  const outputs = ios(1)

  let seed = 0x2545f4914f6cdd1dn
  const mask = (1n << 64n) - 1n
  const next = () => {
    seed ^= (seed << 13n) & mask
    seed ^= seed >> 7n
    seed ^= (seed << 17n) & mask
    return Number(seed >> 40n)
  }
  const noise = () => Math.fround(Math.fround(Math.fround(next() / 16777216) - 0.5) * Math.fround(0.2))
  const fill = () => {
    for (const io of inputs) {
      const v = new Float32Array(e.memory.buffer, io.ptr, io.len)
      for (let i = 0; i < io.len; i++) v[i] = io.name === 'audio' || io.name === 'print' ? noise() : 1 + (next() % 6)
    }
  }

  let hash = 0xcbf29ce484222325n
  for (let r = 0; r < 200; r++) {
    fill()
    e.alk_run(session)
    for (const io of outputs) {
      const bits = new Uint32Array(e.memory.buffer, io.ptr, io.len)
      const vals = new Float32Array(e.memory.buffer, io.ptr, io.len)
      for (let i = 0; i < bits.length; i++) hash = ((hash ^ BigInt(Number.isNaN(vals[i]) ? 0x7fc00000 : bits[i])) * 0x100000001b3n) & mask
    }
  }
  e.alk_reset(session)
  fill()
  const t0 = performance.now()
  for (let r = 0; r < runs; r++) e.alk_run(session)
  const us = ((performance.now() - t0) / runs) * 1000
  const label = alkPath.split('/').pop() + (plans > 1 ? ` ${readName((nb) => e.alk_plan_name(model, plan, nb, 256))}` : '')
  console.log(`${label}: ${us.toFixed(1)} us per run, loaded in ${loadMs.toFixed(1)} ms, fingerprint ${hash.toString(16).padStart(16, '0')}`)
}
