// UI CONTRACT ONLY: source membership names are scripted. The helper neither
// reads Tasks nor proves native assignment, policy authority or reassignment.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './http-fixture.mjs';

const ownership = () => ({ mode: 'shared', host_owner: 'operations', instance_binding: 'unavailable', positions: [{ position: 'org/support', owner: 'operations' }], memberships: [{ owner: 'operations', members: ['alex', 'noor'] }, { owner: 'research', members: ['alex', 'Zoë 第二版'] }] });
const region = page => page.getByRole('region', { name: 'Declared members', exact: true });
const person = (page, name, owner) => region(page).getByRole('button', { name: `Inspect recorded assignments for ${name} · declared owner ${owner}`, exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['organization-draft.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const requests = [], host = await httpFixture((request, response) => {
      requests.push({ method: request.method, path: request.url }); response.setHeader('cache-control', 'no-store');
      if (request.url === '/') { response.setHeader('content-type', 'text/html; charset=utf-8'); response.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/organization-draft.js" defer></script></head><body><main class="workspace"><section class="panel" style="padding:24px"></section></main></body></html>'); return; }
      const name = request.url.slice(1); if (!assets.has(name)) { response.writeHead(404).end(); return; }
      response.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); response.end(assets.get(name));
    });
    try { await use({ ...host, requests }); } finally { await host.close(); }
  }
});
async function mount(page, host, value = ownership(), available = true) {
  await page.goto(host.origin);
  await page.evaluate(({ value, available }) => {
    window.calls = []; window.input = value;
    document.querySelector('main > section').replaceChildren(window.IrisOwnershipPeople.render(value, { available, onInspectPerson: selection => window.calls.push(selection) }));
  }, { value, available });
}

test('declared members retain separate owner context and emit exact read-only selections', async ({ page, host }) => {
  await mount(page, host);
  await expect(region(page).getByRole('button')).toHaveCount(4);
  await page.evaluate(() => { window.input.memberships[0].owner = 'mutated'; window.input.memberships[0].members[0] = 'changed'; });
  await person(page, 'alex', 'operations').click(); await person(page, 'alex', 'research').click();
  expect(await page.evaluate(() => window.calls)).toEqual([{ owner: 'operations', name: 'alex' }, { owner: 'research', name: 'alex' }]);
  await expect(region(page)).toContainText('does not establish a position occupant, Task owner or reassignment authority');
  expect(host.requests.every(request => request.method === 'GET' && ['/', '/organization-draft.js', '/styles.css', '/favicon.ico'].includes(request.path))).toBe(true);
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
});

test('declared members preserve literal identities and reject unsafe or incomplete membership without actions', async ({ page, host }) => {
  const value = ownership(); value.memberships = [{ owner: '<owner & équipe>', members: ['<img src=x onerror="window.injected=true">', 'Zoë 第二版'] }];
  await mount(page, host, value);
  await expect(person(page, value.memberships[0].members[0], value.memberships[0].owner)).toBeVisible();
  expect(await region(page).locator('h4').textContent()).toBe(value.memberships[0].owner);
  await expect(region(page).locator('img, script')).toHaveCount(0);
  const rejected = await page.evaluate(input => {
    const mutations = [v => { v.memberships = null; }, v => { v.instance_binding = 'inferred'; }, v => { v.memberships[0].members = ['bad\nidentity']; }, v => { v.memberships[0].owner = ''; }, v => { v.memberships[0].members = ['\uD800']; }, v => { v.memberships[0].members = ['é'.repeat(129)]; }, v => { v.memberships[0].extra = true; }];
    return mutations.map(mutate => { const v = structuredClone(input); mutate(v); const root = window.IrisOwnershipPeople.render(v, { available: true, onInspectPerson: () => window.calls.push('unexpected') }); return root.querySelectorAll('button').length === 0 && root.textContent.includes('unavailable'); });
  }, ownership());
  expect(rejected.every(Boolean)).toBe(true); expect(await page.evaluate(() => window.injected)).toBeUndefined();
});

test('unavailable Task reads and missing callback keep source membership visible without actions', async ({ page, host }) => {
  await mount(page, host, ownership(), false);
  for (const button of await region(page).getByRole('button').all()) await expect(button).toBeDisabled();
  await expect(region(page)).toContainText('Recorded assignments are unavailable on this connection');
  expect(await page.evaluate(() => { document.querySelector('.ownership-person').click(); return window.calls; })).toEqual([]);
  expect(await page.evaluate(input => {
    const noCallback = window.IrisOwnershipPeople.render(input, { available: true });
    const defaultClosed = window.IrisOwnershipPeople.render(input, { onInspectPerson: () => {} });
    return [noCallback, defaultClosed].every(root => [...root.querySelectorAll('button')].every(button => button.disabled));
  }, ownership())).toBe(true);
  const empty = ownership(); empty.memberships = [{ owner: 'unfilled', members: [] }]; await mount(page, host, empty);
  await expect(region(page)).toContainText('No members declared for this owner'); await expect(region(page).getByRole('button')).toHaveCount(0);
});

test('declared member inspection supports native keyboard navigation and narrow layout', async ({ page, host }, info) => {
  await mount(page, host);
  await page.keyboard.press('Tab'); await expect(person(page, 'alex', 'operations')).toBeFocused(); await page.keyboard.press('Enter');
  await page.keyboard.press('Tab'); await expect(person(page, 'noor', 'operations')).toBeFocused(); await page.keyboard.press('Space');
  expect(await page.evaluate(() => window.calls)).toEqual([{ owner: 'operations', name: 'alex' }, { owner: 'operations', name: 'noor' }]);
  await page.screenshot({ path: info.outputPath('declared-members-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await person(page, 'Zoë 第二版', 'research').focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.calls.at(-1))).toEqual({ owner: 'research', name: 'Zoë 第二版' });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('declared-members-mobile.png'), fullPage: true });
});
