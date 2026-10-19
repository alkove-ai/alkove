import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: { testTimeout: 60_000, globalSetup: ['test/harness.ts'], exclude: ['test/browser/**', 'node_modules/**'] },
})
