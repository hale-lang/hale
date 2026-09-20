// Browser contract only: the existing scripted observer supplies observations.
// This proves exact read-only process-key selection, not an Organization status
// grant, native process liveness or an application-control association.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture, scriptedObserver } from './runtime-harness.mjs';

const KEY = 'ab'.repeat(32), OTHER = 'cd'.repeat(32);
const linked = page => page.getByRole('region', { name: 'Linked process inspection', exact: true });
const canvas = page => page.getByRole('region', { name: 'Observed processes', exact: true });
const inspector = page => page.getByRole('complementary', { name: 'Runtime inspector', exact: true });
const connect = page => page.getByRole('button', { name: 'Connect observer', exact: true }).click();
const refresh = page => page.getByRole('button', { name: 'Refresh observation', exact: true }).click();
const focus = page => linked(page).getByRole('button', { name: 'Focus linked process', exact: true });

const test = base.extend({
  page: async ({ page }, use) => {
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await use(page); expect(errors).toEqual([]);
  },
  observer: async ({}, use) => {
    const observer = await scriptedObserver();
    observer.snapshot.processes[0].process_key = KEY;
    observer.snapshot.processes[1].process_key = OTHER;
    try { await use(observer); } finally { await observer.close(); }
  },
  host: async ({ observer }, use) => {
    const requests = [], assets = new Map(await Promise.all(['runtime.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const host = Object.assign(await httpFixture((req, res) => {
      requests.push(req.url); res.setHeader('cache-control', 'no-store');
      if (req.url === '/') { res.setHeader('content-type', 'text/html; charset=utf-8'); res.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/runtime.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      if (req.url === '/iris/observer.json') { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ profile: 'hale.iris.observer.v0', origin: observer.origin })); return; }
      const name = req.url.slice(1); if (!assets.has(name)) { res.writeHead(404).end(); return; }
      res.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); res.end(assets.get(name));
    }), { requests });
    try { await use(host); } finally { await host.close(); }
  },
});
async function mount(page, host, expectedProcessKey = KEY) {
  // Mount the production Runtime module directly. App route parsing and the
  // source-status grant/link are intentionally outside this contract fixture.
  await page.goto(host.origin);
  await page.evaluate(key => { window.testRuntime = window.IrisRuntime.mount(document.querySelector('main'), { expectedProcessKey: key }); }, expectedProcessKey);
  await expect(page.getByRole('button', { name: 'Connect observer', exact: true })).toBeEnabled();
}
async function expectUnfocused(page, state) {
  await expect(linked(page)).toHaveAttribute('data-state', state);
  await expect(focus(page)).toHaveCount(0);
  await expect(canvas(page)).toHaveAttribute('data-scope-kind', 'fleet');
  await expect(inspector(page)).toContainText('Select a process');
}

test('Organization Runtime link: exact fresh identity selects once and preserves locus exploration', async ({ page, observer, host }, testInfo) => {
  await mount(page, host); expect(observer.requests).toHaveLength(0);
  await connect(page);
  await expect(linked(page)).toHaveAttribute('data-state', 'matched');
  await expect(linked(page)).toContainText('Exact linked process observed · PID 41');
  await expect(canvas(page)).toHaveAttribute('data-scope-pid', '41');
  await expect(inspector(page)).toContainText('0011223344556677');
  await expect(linked(page)).toContainText('does not establish');
  await expect(page.getByRole('button', { name: 'Open application controls', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Enter locus 1 in process 41', exact: true }).click();
  await page.getByRole('button', { name: 'Inspect locus 2', exact: true }).click();
  await refresh(page);
  await expect(canvas(page)).toHaveAttribute('data-scope-locus', '1');
  await expect(inspector(page)).toContainText('Worker / équipe');
  await focus(page).click();
  await expect(canvas(page)).toHaveAttribute('data-scope-kind', 'process');
  await expect(inspector(page)).toContainText('0011223344556677');
  await linked(page).screenshot({ path: testInfo.outputPath('linked-process-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await focus(page).focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await canvas(page).screenshot({ path: testInfo.outputPath('linked-process-mobile.png') });
  expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
  expect(observer.requests.every(request => request.method === 'GET' && request.url === '/snapshot')).toBe(true);
  expect(await page.evaluate(() => window.__runtimeInjected)).toBeUndefined();
});

test('Organization Runtime link: reused PID and equal names cannot replace the expected process', async ({ page, observer, host }) => {
  await mount(page, host); await connect(page); await expect(focus(page)).toBeVisible();
  await page.getByRole('button', { name: 'Enter locus 1 in process 41', exact: true }).click();
  await page.getByRole('button', { name: 'Inspect locus 2', exact: true }).click();
  observer.snapshot.processes[1].name = observer.snapshot.processes[0].name;
  observer.snapshot.processes[0].process_key = 'ef'.repeat(32);
  await refresh(page); await expectUnfocused(page, 'missing');
  await expect(linked(page)).toContainText('reused PID does not replace it');
  await expect(page.getByRole('button', { name: 'Open application controls', exact: true })).toHaveCount(0);
});

test('Organization Runtime link: ended, missing and ambiguous identities clear focus without automatic substitution', async ({ page, observer, host }) => {
  await mount(page, host); await connect(page); await expect(focus(page)).toBeVisible();
  const original = structuredClone(observer.snapshot.processes[0]);
  observer.snapshot.processes[0].state = 'dead';
  await refresh(page); await expectUnfocused(page, 'ended');
  observer.snapshot.processes.shift(); observer.snapshot.edges = [];
  await refresh(page); await expectUnfocused(page, 'missing');
  observer.snapshot.processes.push(original, { ...structuredClone(original), pid: 43 });
  await refresh(page); await expectUnfocused(page, 'ambiguous');
  observer.snapshot.processes.pop();
  await refresh(page); await expect(focus(page)).toBeVisible();
  await expect(canvas(page)).toHaveAttribute('data-scope-kind', 'fleet');
  await focus(page).click(); await expect(canvas(page)).toHaveAttribute('data-scope-pid', '41');
});

test('Organization Runtime link: unchanged timestamp clears matched focus before the observer stalls', async ({ page, observer, host }) => {
  await mount(page, host); await connect(page); await expect(focus(page)).toBeVisible();
  observer.advance = false;
  await refresh(page); await expectUnfocused(page, 'stale');
  await expect(linked(page)).toContainText('not fresh');
  await expect(linked(page)).toHaveAttribute('data-state', 'unavailable');
  await expect(canvas(page)).toHaveCount(0);
  await expect(focus(page)).toHaveCount(0);
});

test('Organization Runtime link: unverifiable timestamp and transport loss never retain a process match', async ({ page, observer, host }) => {
  await mount(page, host); await connect(page); await expect(focus(page)).toBeVisible();
  observer.snapshot.ts = Number.MAX_SAFE_INTEGER + 1;
  await refresh(page); await expectUnfocused(page, 'stale');
  observer.status = 503;
  await refresh(page); await expect(linked(page)).toHaveAttribute('data-state', 'unavailable');
  await expect(canvas(page)).toHaveCount(0); await expect(focus(page)).toHaveCount(0);
});

test('Organization Runtime link: malformed hint does not select a process or grant controls', async ({ page, observer, host }) => {
  await mount(page, host, KEY.toUpperCase()); await connect(page);
  await expectUnfocused(page, 'invalid');
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open application controls', exact: true })).toHaveCount(0);
  expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
});

test('Organization Runtime link: absent query hint preserves ordinary fleet inspection', async ({ page, host }) => {
  await mount(page, host, ''); await connect(page);
  await expect(linked(page)).toHaveCount(0);
  await expect(canvas(page)).toHaveAttribute('data-scope-kind', 'fleet');
  await page.getByRole('button', { name: 'Inspect process 41', exact: true }).click();
  await expect(inspector(page)).toContainText('0011223344556677');
  await expect(page.getByRole('button', { name: 'Open application controls', exact: true })).toHaveCount(0);
});
