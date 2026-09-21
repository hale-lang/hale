// UI CONTRACT ONLY: the New task form over a static host. Native admission,
// the organism's answer and durable recovery are separate gates.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './runtime-harness.mjs';

const positions = () => [{ position: 'org/support', owner: 'partner' }, { position: 'org', owner: 'acme' }, { position: 'org/finance', owner: 'acme' }];
const region = page => page.getByRole('region', { name: 'New task', exact: true });
const outcome = page => region(page).getByRole('textbox', { name: 'What should happen', exact: true });
const locus = page => region(page).getByRole('combobox', { name: 'For locus', exact: true });
const review = page => region(page).getByRole('button', { name: 'Review new task', exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['task-create.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const requests = [], host = await httpFixture((request, response) => {
      requests.push({ method: request.method, path: request.url }); response.setHeader('cache-control', 'no-store');
      if (request.url === '/') { response.setHeader('content-type', 'text/html; charset=utf-8'); response.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/task-create.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      const name = request.url.slice(1); if (!assets.has(name)) { response.writeHead(404).end(); return; }
      response.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); response.end(assets.get(name));
    });
    try { await use({ ...host, requests }); } finally { await host.close(); }
  }
});
async function mount(page, host, options = {}) {
  await page.goto(host.origin);
  await page.evaluate(({ options, positions }) => {
    window.calls = [];
    document.querySelector('main').replaceChildren(window.IrisTaskCreate.render({ canCreate: true, positions, onPrepare: ask => { window.calls.push(ask); return {}; }, ...options }));
  }, { options, positions: options.positions ?? positions() });
}

test('Task creation: exact asks validate and anything beyond an outcome for a locus fails closed', async ({ page, host }) => {
  await page.goto(host.origin);
  const result = await page.evaluate(() => {
    const ok = window.IrisTaskCreate.validate({ outcome: 'Confirm the supplier handover — équipe\r\nKeep the signed schedule.', to: 'org/support' });
    const rejected = [
      { outcome: '', to: 'org' }, { outcome: 'x', to: '' }, { outcome: 'x', to: 'a\nb' }, { outcome: 'x', to: 'x'.repeat(257) }, { outcome: 'é'.repeat(4097), to: 'org' },
      { outcome: 'x\u0000y', to: 'org' }, { outcome: 'x', to: 'org', from: 'riley' }, { outcome: 'x', to: 'org', intent_id: 'i1' }, { outcome: 'x' }, { to: 'org' }, null, 'ask', { outcome: 7, to: 'org' }
    ].map(value => { try { window.IrisTaskCreate.validate(value); return false; } catch { return true; } });
    const exact = window.IrisTaskCreate.validate({ outcome: 'é'.repeat(4096), to: 'x'.repeat(256) });
    return { ok, rejected, exactBytes: new TextEncoder().encode(exact.outcome).length, exactTo: exact.to.length };
  });
  expect(result.ok).toEqual({ outcome: 'Confirm the supplier handover — équipe\r\nKeep the signed schedule.', to: 'org/support' });
  expect(result.rejected).toHaveLength(13); expect(result.rejected.every(Boolean)).toBe(true);
  expect(result.exactBytes).toBe(8192); expect(result.exactTo).toBe(256);
});

test('Task creation: the whole organization comes first, declared loci follow, and only a typed outcome prepares an exact ask', async ({ page, host }, info) => {
  await mount(page, host, { defaultTo: 'org/finance' });
  expect(await locus(page).locator('option').allTextContents()).toEqual(['Whole organization · org', 'org/support · partner', 'org/finance · acme']);
  await expect(locus(page)).toHaveValue('org/finance');
  await expect(review(page)).toBeDisabled();
  const text = 'Confirm the supplier handover\nKeep the signed schedule.';
  await outcome(page).fill(text);
  await expect(region(page)).toContainText(new TextEncoder().encode(text).length + ' / 8192 bytes');
  await expect(review(page)).toBeEnabled();
  await locus(page).selectOption('org/support');
  await review(page).click();
  expect(await page.evaluate(() => window.calls)).toEqual([{ outcome: 'Confirm the supplier handover\nKeep the signed schedule.', to: 'org/support' }]);
  await expect(region(page)).toContainText('Nothing has been recorded yet');
  await expect(review(page)).toBeDisabled(); await expect(outcome(page)).toBeDisabled(); await expect(locus(page)).toBeDisabled();
  expect(await page.evaluate(() => ({ local: localStorage.length, session: sessionStorage.length }))).toEqual({ local: 0, session: 0 });
  expect(host.requests.every(request => request.method === 'GET' && !request.path.startsWith('/api/'))).toBe(true);
  await region(page).screenshot({ path: info.outputPath('task-create-desktop.png') });
  await page.setViewportSize({ width: 390, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await region(page).screenshot({ path: info.outputPath('task-create-mobile.png') });
});

test('Task creation: an over-long, empty or literal-markup outcome never prepares an ask, and markup stays text', async ({ page, host }) => {
  await mount(page, host);
  await outcome(page).fill('<img src=x onerror="window.injected=true">');
  await expect(review(page)).toBeEnabled();
  await page.evaluate(() => { document.querySelector('.task-create-form textarea').value = 'é'.repeat(4097); document.querySelector('.task-create-form textarea').dispatchEvent(new Event('input', { bubbles: true })); });
  await expect(region(page)).toContainText('8194 / 8192 bytes'); await expect(review(page)).toBeDisabled();
  await expect(outcome(page)).toHaveAttribute('aria-invalid', 'true');
  await outcome(page).fill('');
  await expect(review(page)).toBeDisabled();
  await page.evaluate(() => document.querySelector('.task-create-form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  expect(await page.evaluate(() => window.calls)).toEqual([]);
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
});

test('Task creation: denied sessions, absent callbacks and malformed loci cannot prepare an ask', async ({ page, host }) => {
  for (const options of [{ canCreate: false, reason: 'Raising work is unavailable for this connection.' }, { onPrepare: null }, { canCreate: 'true' }]) {
    await mount(page, host, options);
    await expect(outcome(page)).toBeDisabled(); await expect(locus(page)).toBeDisabled(); await expect(review(page)).toBeDisabled();
    await expect(region(page)).toHaveAttribute('data-state', 'unavailable');
    await page.evaluate(() => document.querySelector('.task-create-form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
    expect(await page.evaluate(() => window.calls)).toEqual([]);
  }
  await expect(region(page).getByRole('status')).toContainText('not available');
  for (const positions of [[{ position: 'org/support' }], [{ position: 'a\nb', owner: 'x' }], [{ position: 'org/support', owner: 'a' }, { position: 'org/support', owner: 'b' }], [42]]) {
    await mount(page, host, { positions });
    expect(await locus(page).locator('option').allTextContents()).toEqual(['Whole organization · org']);
  }
  await mount(page, host);
  await outcome(page).fill('Raise');
  await page.evaluate(() => { const select = document.querySelector('.task-create-form select'); const option = document.createElement('option'); option.value = 'invented'; select.append(option); select.value = 'invented'; select.dispatchEvent(new Event('change')); document.querySelector('.task-create-form').dispatchEvent(new Event('submit', { cancelable: true })); });
  expect(await page.evaluate(() => window.calls)).toEqual([]);
});

test('Task creation: a pending preparation rejects duplicate submission and a rejected preparation can be corrected', async ({ page, host }) => {
  await mount(page, host);
  await page.evaluate(positions => {
    window.calls = [];
    document.querySelector('main').replaceChildren(window.IrisTaskCreate.render({ canCreate: true, positions, onPrepare(ask) { window.calls.push(ask); return new Promise(resolve => { window.finish = resolve; }); } }));
  }, positions());
  await outcome(page).fill('Raise the schedule'); await review(page).click(); await expect(outcome(page)).toBeDisabled();
  await page.evaluate(() => document.querySelector('.task-create-form').dispatchEvent(new Event('submit', { cancelable: true })));
  expect(await page.evaluate(() => window.calls)).toEqual([{ outcome: 'Raise the schedule', to: 'org' }]);
  await page.evaluate(() => window.finish({ error: 'The Record moved. Refresh the handed Tasks.' }));
  await expect(region(page)).toContainText('Record moved'); await expect(outcome(page)).toBeEnabled();
  await locus(page).selectOption('org/finance'); await review(page).click();
  expect(await page.evaluate(() => window.calls)).toEqual([{ outcome: 'Raise the schedule', to: 'org' }, { outcome: 'Raise the schedule', to: 'org/finance' }]);
  await page.evaluate(() => window.finish({})); await expect(outcome(page)).toBeDisabled();
});
