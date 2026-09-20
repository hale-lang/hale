import { test as base, expect } from '@playwright/test';
import { nativeObserver } from './runtime-harness.mjs';

const observer = process.env.HALE_COCKPIT_OBSERVER_BIN;
const application = process.env.HALE_COCKPIT_APPLICATION_BIN;
const test = base.extend({
  joined: async ({}, use) => {
    const fixture = await nativeObserver(observer, application, { application: true });
    try { await use(fixture); } finally { await fixture.close(); }
  },
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await use(page);
    expect(errors).toEqual([]);
  },
});
test.skip(!observer || !application, 'Supply native observer and intake-control binaries for the same-application lane.');
const association = page => page.getByRole('region', { name: 'Application runtime association', exact: true });
async function enterRuntime(page, app) {
  await page.goto(app.applicationOrigin);
  await page.getByRole('button', { name: 'Inspect this running application', exact: true }).click();
  await page.getByRole('button', { name: 'Connect observer', exact: true }).click();
  await expect(association(page)).toContainText('matched to process');
}

test('Same application: observed process leads to controls and actual effective change', async ({ page, joined: app }, testInfo) => {
  const state = await app.applicationState();
  expect(state.runtime.process_key).toMatch(/^[0-9a-f]{64}$/);
  await expect.poll(async () => (await app.snapshot()).processes.find(p => p.pid === app.appPID)?.process_key).toBe(state.runtime.process_key);
  await enterRuntime(page, app);
  await association(page).getByRole('button', { name: 'Focus application process', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('Verified application process · Intake control');
  await page.screenshot({ path: testInfo.outputPath('application-runtime.png'), fullPage: true });
  await association(page).getByRole('button', { name: 'Open application controls', exact: true }).click();
  await page.getByRole('button', { name: 'Prepare change', exact: true }).click();
  await page.getByLabel('Intake mode', { exact: true }).selectOption({ label: 'Paused' });
  await page.getByRole('button', { name: 'Review change', exact: true }).click();
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText('Command succeeded');
  await expect.poll(async () => (await app.applicationState()).control.effective_value).toBe('paused');
  const paused = await app.applicationState();
  await new Promise(resolve => setTimeout(resolve, 550));
  expect((await app.applicationState()).activity.count).toBe(paused.activity.count);
});

test('Same application: process replacement clears association until explicitly revisited', async ({ page, joined: app }) => {
  await enterRuntime(page, app);
  const before = await app.applicationState();
  await app.restartApp();
  await expect.poll(async () => (await app.applicationState()).application.incarnation_id).not.toBe(before.application.incarnation_id);
  await expect(association(page)).toContainText('application or access changed');
  await expect(association(page).getByRole('button', { name: 'Open application controls' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Back to application', exact: true }).click();
  await page.getByRole('button', { name: 'Inspect this running application', exact: true }).click();
  await page.getByRole('button', { name: 'Connect observer', exact: true }).click();
  await expect(association(page)).toContainText('matched to process ' + app.appPID);
  expect((await app.applicationState()).runtime.process_key).not.toBe(before.runtime.process_key);
});

test('Same application: missing or mismatched process key never links a PID alone', async ({ page, joined: app }) => {
  let omit = true;
  await page.route(app.origin + '/snapshot', async route => {
    const response = await route.fetch(); const body = await response.json();
    for (const process of body.processes) { if (omit) delete process.process_key; else process.process_key = '0'.repeat(64); }
    await route.fulfill({ response, json: body });
  });
  await page.goto(app.applicationOrigin + '/#/runtime');
  await page.getByRole('button', { name: 'Connect observer', exact: true }).click();
  await expect(association(page)).toContainText('not verified in this observation');
  await expect(page.getByRole('button', { name: 'Open application controls' })).toHaveCount(0);
  omit = false;
  await page.getByRole('button', { name: 'Refresh observation', exact: true }).click();
  await expect(association(page)).toContainText('not verified in this observation');
  await page.unroute(app.origin + '/snapshot');
  await expect(association(page)).toContainText('matched to process');
});

test('Same application: state access loss removes links without stopping independent observation', async ({ page, joined: app }) => {
  await enterRuntime(page, app);
  await page.route('**/api/hale/v1/applications/**/state', route => route.fulfill({ status: 403, contentType: 'application/json', body: '{}' }));
  await expect(association(page)).toContainText('could not be verified');
  await expect(page.getByRole('button', { name: 'Open application controls' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Runtime observer status' })).toContainText('Observer live');
  await page.getByRole('button', { name: 'Disconnect observer', exact: true }).click();
  await expect(association(page)).toContainText('No application process is verified');
});
