// The admin UI in a browser (docs/DEEP_TESTING.md D7). Run by
// crates/fcp-admin/tests/ui.rs against an admin it starts; FCP_UI_URL
// names it. Chromium only.
import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [['list']],
  use: {
    baseURL: process.env.FCP_UI_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  outputDir: 'test-results',
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
