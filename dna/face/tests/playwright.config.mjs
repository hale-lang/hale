import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '**/*.spec.mjs',
  timeout: 45_000,
  expect: { timeout: 7_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  outputDir: '../test-results',
  use: {
    browserName: 'chromium',
    headless: true,
    viewport: { width: 1440, height: 980 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
