import { join } from 'node:path'

import { defineConfig, devices } from '@playwright/test'

const speech = join(import.meta.dirname, 'test/fixtures/speech.wav')

export default defineConfig({
  testDir: './test/browser',
  testMatch: '*.spec.ts',
  outputDir: './.cache/playwright-results',
  globalSetup: './test/browser/global-setup.ts',
  // a shared Mac: real-time audio needs the CPU it is promised
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  reporter: [['list']],
  use: { serviceWorkers: 'block', trace: 'off' },
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        permissions: ['microphone'],
        launchOptions: {
          args: ['--autoplay-policy=no-user-gesture-required', '--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', `--use-file-for-fake-audio-capture=${speech}`],
        },
      },
    },
    {
      name: 'firefox',
      use: {
        ...devices['Desktop Firefox'],
        launchOptions: {
          firefoxUserPrefs: {
            'media.autoplay.default': 0,
            'media.autoplay.block-webaudio': false,
            'media.navigator.streams.fake': true,
            'media.navigator.permission.disabled': true,
          },
        },
      },
    },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
  ],
})
