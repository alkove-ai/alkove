import { execFileSync } from 'node:child_process'
import { join } from 'node:path'

// the core with its test exports (crates/wasm, feature harness), built once before the test files read it side by side
export default function setup(): void {
  const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: ['-C', 'target-feature=+simd128,+bulk-memory'].join('\x1f') }
  execFileSync('cargo', ['build', '-q', '--release', '-p', 'alkove-wasm', '--features', 'harness', '--target', 'wasm32-unknown-unknown', '--target-dir', 'target/harness'], { cwd: join(import.meta.dirname, '../../..'), env, stdio: 'inherit' })
}
