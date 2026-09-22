// UI CONTRACT ONLY: scripted native DTOs exercise inspection and preparation.
// Native authority, reassignment admission and durable recovery are separate gates.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './runtime-harness.mjs';

const row = () => ({
  id: 'support:t-41', outcome: 'Confirm the supplier handover — équipe\r\nKeep the signed schedule.', state: 'handed', assignee: 'noor',
  obligation: 'supplier-handover', acceptance_digest: 'acceptance:original/v1', acceptance_bound: true, evidence_required: true, evidence_ref: '', waiting: '',
  assignment_digest: 'sha256:' + 'a'.repeat(64), reassignment_supported: true,
  history: [
    { event_id: '1'.repeat(40), sequence: '9007199254740993', kind: 'task.handed', from: '', to: 'mara', by: 'leader' },
    { event_id: '2'.repeat(40), sequence: '9007199254740995', kind: 'task.reassigned', from: 'mara', to: 'noor', by: 'riley' }
  ]
});
const region = page => page.getByRole('region', { name: 'Handed Task administration', exact: true });
const inspector = page => page.getByRole('group', { name: 'Selected assignment', exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['task-administration.js', 'projects.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const requests = [], host = await httpFixture((request, response) => {
      requests.push({ method: request.method, path: request.url }); response.setHeader('cache-control', 'no-store');
      if (request.url === '/') { response.setHeader('content-type', 'text/html; charset=utf-8'); response.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/task-administration.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      const name = request.url.slice(1); if (!assets.has(name)) { response.writeHead(404).end(); return; }
      response.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); response.end(assets.get(name));
    });
    try { await use({ ...host, requests }); } finally { await host.close(); }
  }
});
async function mount(page, host, input = row(), options = {}) {
  await page.goto(host.origin);
  await page.evaluate(({ input, options }) => {
    window.calls = []; window.refreshes = 0; window.input = input;
    const model = window.IrisTaskAdministration.validate(input);
    window.model = model;
    document.querySelector('main').replaceChildren(window.IrisTaskAdministration.render(model, {
      canReassign: true, recipients: ['noor', 'mara', 'dev'], inspectedAt: new Date('2026-09-19T22:00:00Z'),
      onPrepareReassignment: value => { window.calls.push(value); return {}; }, onRefresh: () => { window.refreshes++; }, ...options
    }));
  }, { input, options });
}

test('Task administration: exact supported and retained lifecycle rows validate without inventing receipt syntax', async ({ page, host }) => {
  await page.goto(host.origin);
  const valid = await page.evaluate(input => {
    const results = ['handed', 'done', 'failed', 'cancelled', 'refused', 'decided', 'declined', 'timeout', 'escalated', 'transfer_requested', 'transfer_accepted'].map(state => {
      const value = { ...input, state, reassignment_supported: state === 'handed' }; window.IrisTaskAdministration.validate(value); return state;
    });
    const empty = structuredClone(input); empty.assignee = ''; empty.history = [{ ...empty.history[0], to: '', by: '' }]; empty.acceptance_digest = ''; empty.acceptance_bound = true;
    window.IrisTaskAdministration.validate(empty);
    const repeated = structuredClone(input); repeated.history.push({ ...repeated.history[0], event_id: '3'.repeat(64), sequence: '9007199254740997', from: 'noor', to: 'dev', by: '' }); repeated.assignee = 'dev'; window.IrisTaskAdministration.validate(repeated);
    const copied = window.IrisTaskAdministration.validate(input); input.history[0].to = 'changed after validation';
    return { states: results, copied: copied.history[0].to, acceptance: copied.acceptance_digest };
  }, row());
  expect(valid.states).toHaveLength(11); expect(valid.copied).toBe('mara'); expect(valid.acceptance).toBe('acceptance:original/v1');
});

test('Task administration: malformed assignment chains, identity and unsupported write claims fail closed', async ({ page, host }) => {
  await page.goto(host.origin);
  const rejected = await page.evaluate(input => {
    const mutations = [
      value => { value.extra = true; }, value => { delete value.reassignment_supported; }, value => { value.reassignment_supported = 'true'; },
      value => { value.id = ''; }, value => { value.id = 'x'.repeat(257); }, value => { value.outcome = 'é'.repeat(4097); }, value => { value.waiting = 'x'.repeat(2049); },
      value => { value.assignment_digest = 'not-exact'; }, value => { value.state = 'running'; }, value => { value.state = 'done'; }, value => { value.evidence_required = 1; },
      value => { value.acceptance_bound = false; }, value => { value.evidence_ref = 'x'.repeat(257); }, value => { value.history = []; },
      value => { value.history[0].from = 'unknown'; }, value => { value.history[0].kind = 'task.reassigned'; }, value => { value.history[1].kind = 'task.done'; },
      value => { value.history[1].event_id = value.history[0].event_id; }, value => { value.history[1].sequence = value.history[0].sequence; },
      value => { value.history[1].sequence = '01'; }, value => { value.history[1].sequence = '9223372036854775808'; }, value => { value.history[1].sequence = 6; },
      value => { value.history[1].from = 'unrelated'; }, value => { value.history[1].to = ''; }, value => { value.history[1].by = ''; }, value => { value.assignee = 'different'; }
    ];
    return mutations.map(mutate => { const value = structuredClone(input); mutate(value); try { window.IrisTaskAdministration.validate(value); return false; } catch { return true; } });
  }, row());
  expect(rejected).toHaveLength(26); expect(rejected.every(Boolean)).toBe(true);
});

test('Task administration: inspect exact assignment trail and requirements with keyboard and narrow layout', async ({ page, host }, info) => {
  const input = row(); input.waiting = 'Approval from <img src=x onerror="window.injected=true"> is still required.';
  await mount(page, host, input);
  await expect(region(page).getByRole('heading', { level: 2 })).toHaveText(input.outcome);
  expect(await region(page).getByRole('heading', { level: 2 }).evaluate(node => node.textContent)).toBe(input.outcome);
  await expect(region(page)).toContainText('Handed · completion not recorded');
  await expect(inspector(page)).toContainText('mara → noor');
  const earlier = region(page).getByRole('button', { name: 'Handoff to mara · sequence 9007199254740993', exact: true });
  await earlier.focus(); await page.keyboard.press('Enter'); await expect(earlier).toBeFocused(); await expect(earlier).toHaveAttribute('aria-pressed', 'true');
  await expect(inspector(page)).toContainText('EARLIER ASSIGNMENT'); await expect(inspector(page)).toContainText('9007199254740993');
  await expect(region(page)).toContainText('acceptance:original/v1'); await expect(region(page)).toContainText('does not replace them');
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  await region(page).screenshot({ path: info.outputPath('handed-task-desktop.png') });
  await page.setViewportSize({ width: 390, height: 980 });
  const current = region(page).getByRole('button', { name: 'Reassignment from mara to noor · sequence 9007199254740995', exact: true });
  await current.focus(); await page.keyboard.press('Space'); await expect(current).toBeFocused(); await expect(inspector(page)).toContainText('CURRENT ASSIGNMENT');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await region(page).screenshot({ path: info.outputPath('handed-task-mobile.png') });
  expect(await page.evaluate(() => window.calls)).toEqual([]);
});

test('Task administration: an explicit eligible recipient prepares only exact intent without writing or changing the Task', async ({ page, host }) => {
  await mount(page, host);
  const people = region(page).getByRole('combobox', { name: 'New assignee', exact: true });
  expect(await people.locator('option').allTextContents()).toEqual(['Choose a person', 'mara', 'dev']);
  const prepare = region(page).getByRole('button', { name: 'Review reassignment', exact: true }); await expect(prepare).toBeDisabled();
  await people.selectOption('dev'); await expect(region(page)).toContainText('noor → dev · same Task, same requirements'); await prepare.click();
  expect(await page.evaluate(() => window.calls)).toEqual([{ to: 'dev' }]); await expect(prepare).toBeDisabled();
  await expect(region(page)).toContainText('recorded assignee has not changed');
  expect(await page.evaluate(() => window.input)).toEqual(row());
  await region(page).getByRole('button', { name: 'Refresh Task', exact: true }).click(); expect(await page.evaluate(() => window.refreshes)).toBe(1);
  expect(await page.evaluate(() => ({ local: localStorage.length, session: sessionStorage.length }))).toEqual({ local: 0, session: 0 });
  expect(host.requests.every(request => request.method === 'GET' && !request.path.startsWith('/api/'))).toBe(true);
});

test('Task administration: historical, unsupported, denied and absent recipients cannot prepare reassignment', async ({ page, host }) => {
  for (const [input, options] of [
    [row(), { historical: true }], [row(), { canReassign: false }], [{ ...row(), reassignment_supported: false }, {}],
    [{ ...row(), state: 'done', reassignment_supported: false }, {}], [row(), { recipients: [] }], [row(), { recipients: ['noor'] }], [row(), { recipients: ['dev', 'dev'] }], [row(), { recipients: [42] }]
  ]) {
    await mount(page, host, input, options);
    await expect(region(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled();
    await expect(region(page).getByRole('button', { name: 'Review reassignment', exact: true })).toBeDisabled();
    await page.evaluate(() => document.querySelector('.task-reassignment-form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
    expect(await page.evaluate(() => window.calls)).toEqual([]);
  }
  await mount(page, host);
  await page.evaluate(() => { const select = document.querySelector('.task-recipient-field select'); const option = document.createElement('option'); option.value = 'invented'; select.append(option); select.value = 'invented'; select.dispatchEvent(new Event('change')); document.querySelector('.task-reassignment-form').dispatchEvent(new Event('submit', { cancelable: true })); });
  expect(await page.evaluate(() => window.calls)).toEqual([]);
});

test('Task administration: pending preparation rejects duplicate submission and a rejected preparation can be corrected', async ({ page, host }) => {
  await mount(page, host);
  await page.evaluate(input => {
    window.calls = [];
    document.querySelector('main').replaceChildren(window.IrisTaskAdministration.render(input, { canReassign: true, recipients: ['dev', 'mara'], onPrepareReassignment(value) { window.calls.push(value); return new Promise(resolve => { window.finish = resolve; }); } }));
  }, row());
  const people = region(page).getByRole('combobox', { name: 'New assignee', exact: true });
  const prepare = region(page).getByRole('button', { name: 'Review reassignment', exact: true });
  await people.selectOption('dev'); await prepare.click(); await expect(people).toBeDisabled();
  await page.evaluate(() => document.querySelector('.task-reassignment-form').dispatchEvent(new Event('submit', { cancelable: true })));
  expect(await page.evaluate(() => window.calls)).toEqual([{ to: 'dev' }]);
  await page.evaluate(() => window.finish({ error: 'The captured assignment changed. Refresh the exact Task.' }));
  await expect(region(page)).toContainText('captured assignment changed'); await expect(people).toBeEnabled();
  await people.selectOption('mara'); await prepare.click();
  expect(await page.evaluate(() => window.calls)).toEqual([{ to: 'dev' }, { to: 'mara' }]);
  await page.evaluate(() => window.finish({})); await expect(people).toBeDisabled();
});
