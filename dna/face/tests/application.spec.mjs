import { test as base, expect } from '@playwright/test';
import { applicationFixture, binary, pause } from './application-harness.mjs';

const test = base.extend({
  application: async ({}, use) => {
    const application = await applicationFixture();
    try { await use(application); } finally { await application.close(); }
  },
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await use(page);
    expect(errors, 'application raised no unhandled JavaScript errors').toEqual([]);
  },
});
test.skip(!binary, 'Supply HALE_FACE_APPLICATION_BIN for the real plain-Hale administration lane.');

test('Native application: configuration, actual work, no-op and competing revisions', async ({ application: app }) => {
  const initial = await app.state();
  await expect.poll(async () => BigInt((await app.state()).activity.count) > BigInt(initial.activity.count)).toBe(true);
  const command = app.command(initial);
  const paused = await app.submit(command);
  expect(paused.status).toBe(200);
  expect(paused.body.data.receipt.result).toEqual({ changed: true, revision: '1', value: 'paused' });
  await expect.poll(async () => (await app.state()).control.effective_value).toBe('paused');
  const stopped = await app.state();
  await pause(600);
  expect((await app.state()).activity.count).toBe(stopped.activity.count);
  const noop = await app.submit(app.command(stopped));
  expect(noop.status).toBe(200);
  expect(noop.body.data.receipt.result).toEqual({ changed: false, revision: '1', value: 'paused' });
  const contenders = [app.command(stopped, 'running'), app.command(stopped, 'running')];
  const decisions = await Promise.all(contenders.map(command => app.submit(command)));
  expect(decisions.map(result => result.status).sort()).toEqual([200, 409]);
  const refusal = decisions.find(result => result.status === 409).body.data.receipt;
  expect(refusal.state).toBe('refused'); expect(refusal.reason).toBe('stale_revision');
  expect((await app.lookup(refusal.request_id)).body.data.receipt).toEqual(refusal);
  await expect.poll(async () => (await app.state()).control.effective_value).toBe('running');
  await expect.poll(async () => BigInt((await app.state()).activity.count) > BigInt(stopped.activity.count)).toBe(true);
  expect((await app.state()).control.revision).toBe('2');
});

test('Native application: exact recovery survives API and target restarts', async ({ application: app }) => {
  const before = await app.state();
  const command = app.command(before);
  const submitted = await app.submit(command);
  expect(submitted.status).toBe(200);
  const receipt = submitted.body.data.receipt;
  expect((await app.submit(command)).body.data.receipt).toEqual(receipt);
  const conflict = await app.submit({ ...command, arguments: { value: 'running' } });
  expect(conflict.status).toBe(409); expect(conflict.body.error.code).toBe('request_conflict');
  expect(conflict.body.data).toBeUndefined();
  await app.stopApi(); await app.startApi();
  expect((await app.lookup(command.request_id)).body.data.receipt).toEqual(receipt);
  await app.stopApp(); await app.startApp();
  expect(app.identity.incarnation_id).not.toBe(before.application.incarnation_id);
  expect((await app.submit(command)).body.data.receipt).toEqual(receipt);
  const stale = await app.submit({ ...command, request_id: crypto.randomUUID() });
  expect(stale.status).toBe(409); expect(stale.body.data.receipt.reason).toBe('stale_incarnation');
  expect((await app.state()).control.revision).toBe('1');
  const oldWorker = await app.startApp();
  await app.startApp();
  await expect.poll(() => oldWorker.exitCode).toBe(2);
});

test('Native application: transport, operation and principal restrictions precede effects', async ({ application: app }) => {
  const command = app.command(await app.state());
  for (const headers of [{ Origin: 'http://foreign.invalid' }, { 'X-Face-Command': '0' }, { 'Content-Type': 'text/plain' }]) {
    const refused = await app.request('/commands', { method: 'POST', body: command, headers });
    expect(refused.status).toBeGreaterThanOrEqual(400);
  }
  const duplicate = JSON.stringify(command).replace('"request_id":', '"request_id":"duplicate","request_id":');
  expect((await app.request('/commands', { method: 'POST', body: duplicate })).status).toBe(400);
  const unknown = await app.submit({ ...command, operation: 'unregistered.control' });
  expect(unknown.status).toBe(400); expect(unknown.body.error.code).toBe('unsupported_operation');
  expect((await app.lookup(command.request_id)).status).toBe(404);
  expect((await app.state()).control.revision).toBe('0');
  await app.stopApi(); await app.startApi('viewer');
  const capabilities = await app.request('/capabilities');
  expect(capabilities.body.data.operations[0].authorized).toBe(false);
  expect((await app.submit(app.command(await app.state()))).status).toBe(403);
  expect((await app.lookup(command.request_id)).status).toBe(403);
  expect((await app.state()).control.revision).toBe('0');
});

const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
async function prepare(page, value = 'Paused') {
  await page.getByRole('button', { name: 'Prepare change', exact: true }).click();
  await page.getByLabel('Intake mode', { exact: true }).selectOption({ label: value });
  await page.getByRole('button', { name: 'Review change', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Application change confirmation', exact: true })).toBeFocused();
}
const savedRequests = page => page.evaluate(() => Object.keys(localStorage).filter(key => key.startsWith('face.application-recovery.v1:')).map(key => JSON.parse(localStorage.getItem(key))));

test('Application workspace: actual control and effect, literal evidence', async ({ page, application: app }, testInfo) => {
  const paths = [];
  page.on('request', request => { if (new URL(request.url()).pathname.startsWith('/api/')) paths.push(new URL(request.url()).pathname); });
  await page.goto(app.origin);
  await expect(page).toHaveURL(app.origin + '/#/application');
  await expect(page.getByRole('heading', { name: 'Application', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Organization', exact: true })).toBeHidden();
  await expect(page.getByRole('region', { name: 'App-effective configuration', exact: true })).toContainText('running');
  await prepare(page);
  await page.screenshot({ path: testInfo.outputPath('application-confirmation.png'), fullPage: true });
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(recovery(page)).toContainText('Command succeeded');
  await expect(page.getByRole('region', { name: 'Desired configuration', exact: true })).toContainText('paused');
  await expect(page.getByRole('region', { name: 'App-effective configuration', exact: true })).toContainText('paused');
  const saved = await savedRequests(page);
  expect(saved).toHaveLength(1);
  expect(saved[0].request_binding_sha256).toMatch(/^sha256:[a-f0-9]{64}$/);
  expect(saved[0]).not.toHaveProperty('arguments'); expect(saved[0]).not.toHaveProperty('value');
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('application-result.png'), fullPage: true });
  await expect(recovery(page)).toContainText('Command succeeded');
  expect(paths.every(path => !path.includes('/dna/'))).toBe(true);
});

test('Application workspace: lost reply recovers after both processes restart without a second POST', async ({ page, application: app }) => {
  let posts = 0, reservedBeforeSend;
  await page.route('**/commands', async route => {
    posts += 1;
    reservedBeforeSend = await savedRequests(page);
    const response = await route.fetch();
    expect(response.status()).toBe(200);
    await route.abort('failed');
  });
  await page.goto(app.origin);
  await prepare(page);
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(recovery(page)).toContainText('Request outcome unconfirmed');
  expect(posts).toBe(1); expect(reservedBeforeSend).toHaveLength(1);
  const retained = (await app.lookup(reservedBeforeSend[0].request_id)).body.data.receipt;
  await app.stopApi(); await app.stopApp(); await app.startApp(); await app.startApi();
  await page.reload();
  await expect(recovery(page)).toContainText('Command succeeded');
  await expect(recovery(page)).toContainText(retained.command_id);
  expect((await app.state()).control.revision).toBe('1'); expect(posts).toBe(1);
});

test('Application workspace: refused exact request remains visible and cannot overwrite a competing change', async ({ page, application: app }) => {
  await page.goto(app.origin); await prepare(page);
  await page.route('**/commands', async route => {
    const competitor = await app.submit(app.command(await app.state()));
    expect(competitor.status).toBe(200);
    await route.continue();
  });
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(recovery(page)).toContainText('Command refused');
  await expect(recovery(page)).toContainText('stale_revision');
  expect((await app.state()).control.revision).toBe('1');
  await page.reload();
  await expect(recovery(page)).toContainText('Command refused');
});

test('Application workspace: receipt recovery remains available while state reads fail', async ({ page, application: app }) => {
  await page.goto(app.origin); await prepare(page);
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(recovery(page)).toContainText('Command succeeded');
  await page.route('**/state', route => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ api_version: 'hale.v1', profile: 'hale.application.v1', error: { code: 'provider_unavailable', message: 'Unavailable' } }) }));
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Current state unavailable', exact: true })).toBeVisible();
  await expect(recovery(page)).toContainText('Command succeeded');
  await expect(page.getByRole('region', { name: 'Desired configuration', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Prepare change', exact: true })).toHaveCount(0);
});

test('Application workspace: unverifiable receipt is withheld and principal change clears private context', async ({ page, application: app }) => {
  await page.goto(app.origin); await prepare(page);
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(recovery(page)).toContainText('Command succeeded');
  await page.route('**/commands?request_id=*', async route => {
    const response = await route.fetch(); const body = await response.json();
    body.data.receipt.fingerprint = 'sha256:' + '0'.repeat(64);
    await route.fulfill({ response, json: body });
  });
  await page.reload();
  await expect(recovery(page)).toContainText('Request outcome unconfirmed');
  await expect(recovery(page)).not.toContainText('Command succeeded');
  await expect(page.getByRole('button', { name: 'Release request reservation', exact: true })).toHaveCount(0);
  await app.stopApi(); await app.startApi('viewer');
  await expect(page.getByRole('region', { name: 'Application administration', exact: true })).toContainText('Application identity or access changed');
  await expect(recovery(page)).toBeHidden();
  await expect(page.getByRole('region', { name: 'Desired configuration', exact: true })).toHaveCount(0);
  expect(await savedRequests(page)).toHaveLength(1);
  await page.getByRole('button', { name: 'Refresh application', exact: true }).click();
  await expect(page.locator('#principal')).toContainText('viewer');
  await expect(page.getByRole('button', { name: 'Prepare change', exact: true })).toBeDisabled();
  await expect(recovery(page)).toBeHidden();
});

test('Application workspace: storage failure prevents transmission; mobile keyboard review stays usable', async ({ page, application: app }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(app.origin);
  await page.getByRole('button', { name: 'Prepare change', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('Intake mode', { exact: true })).toBeFocused();
  await page.keyboard.press('ArrowDown'); await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Tab'); await page.keyboard.press('Enter');
  await expect(page.getByRole('group', { name: 'Application change confirmation', exact: true })).toBeFocused();
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('application-mobile.png'), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  let posts = 0;
  page.on('request', request => { if (request.method() === 'POST') posts += 1; });
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('storage unavailable'); }; });
  await page.getByRole('button', { name: 'Submit change', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Application change', exact: true })).toContainText('Nothing was submitted');
  expect(posts).toBe(0); expect((await app.state()).control.revision).toBe('0');
});
