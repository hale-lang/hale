// UI CONTRACT ONLY: every capability, Task and receipt response below is
// scripted. The real ask, the organism's answer and durable recovery have a
// separate native gate (native-task-create-browser.spec.mjs).
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './http-fixture.mjs';

const API = '/api/hale/v1/applications', APP = 'a'.repeat(40), PRINCIPAL = { mode: 'local', name: 'riley' };
const HEAD = 'b'.repeat(40), NEXT = 'c'.repeat(40), EVENT = 'd'.repeat(40), INTENT = 'i1f4';
const STORAGE = 'iris.practice-recovery.v1:';
const KEY = STORAGE + encodeURIComponent(JSON.stringify([APP, PRINCIPAL.mode, PRINCIPAL.name]));
const OUTCOME = 'Confirm the supplier handover — équipe\nKeep the signed schedule.';
const region = page => page.getByRole('region', { name: 'New task', exact: true });
const recovery = page => page.getByRole('region', { name: 'New task request', exact: true });
const confirmation = page => page.getByRole('group', { name: 'Confirm new task', exact: true });
const error = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: false } });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const names = ['index.html', 'app.js', 'styles.css', 'application.js', 'organization-draft.js', 'definition-draft.js', 'knowledge-draft.js', 'task-administration.js', 'task-create.js'];
    const assets = new Map(await Promise.all(names.map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const host = await httpFixture((req, res) => {
      const name = req.url === '/' ? 'index.html' : req.url.slice(1); if (!assets.has(name)) { res.writeHead(404).end(); return; }
      res.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html') + '; charset=utf-8'); res.setHeader('cache-control', 'no-store'); res.end(assets.get(name));
    });
    try { await use(host); } finally { await host.close(); }
  }
});
async function saved(page) { return page.evaluate(prefix => Object.entries(localStorage).filter(([key]) => key.startsWith(prefix)).map(([key, value]) => ({ key, value: JSON.parse(value) })), STORAGE); }
async function fixture(page, options = {}) {
  const script = { authorized: true, available: true, profile: true, inconsistent: false, applied: false, answer: 'requested', postMode: 'receipt', getMode: 'receipt', badReceipt: '', posts: [], gets: [], reads: [], savedBefore: [], ...options };
  const source = () => ({ record_id: APP, record_head: script.applied ? NEXT : HEAD, record_revision: script.applied ? '11' : '10' });
  const wrap = data => ({ api_version: 'hale.v1', source: source(), data });
  const pageInfo = (items, limit = 25, offset = 0) => ({ limit, offset, total: items.length, next_offset: -1, snapshot: source().record_head });
  const receipt = command => {
    const born = script.answer === 'born';
    const data = { command_id: 'ui-create/' + command.request_id, request_id: command.request_id, application_id: APP, operation: 'dna.task.create', operation_version: '1', principal: PRINCIPAL, context: command.context, target: command.target, subject_digest: command.preconditions.record_head, fingerprint: 'sha256:' + 'f'.repeat(64), state: 'succeeded', reason: '', task_create: { intent_id: INTENT, intent_state: script.answer, task_id: born ? 'org:t9' : '', event_id: EVENT }, review: { state: 'unavailable', outcome: '', subject_digest: '' }, activation: { state: 'unknown', reason: '' } };
    if (script.badReceipt === 'task') data.task_create.task_id = 'org:t9';
    if (script.badReceipt === 'target') data.target = { ...data.target, id: 'different-record' };
    if (script.badReceipt === 'activation') data.activation.state = 'adopted';
    return wrap(data);
  };
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url());
    const send = (status, data) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) });
    if (url.pathname === API) return send(200, wrap({ items: [{ id: APP, kind: 'dna', name: 'Task creation UI contract', capabilities_url: API + '/' + APP + '/capabilities' }], page: pageInfo([{}]) }));
    if (url.pathname.endsWith('/capabilities')) {
      const write = script.profile && script.available && script.authorized;
      const data = { application_id: APP, principal: PRINCIPAL, read_only: script.inconsistent ? true : !write, reads: { tasks: true, workflows: false, reviews: false, practices: false, organization: false, definitions: false, knowledge: false }, writes: { practice_propose: false, review_verdict: false, task_create: write } };
      if (script.profile) data.task_create_commands = { profile: 'dna.task.create.v1', available: script.available, authorized: script.authorized, position_id: 'org', recovery: 'record_lifetime', max_outcome_bytes: '8192', max_identity_bytes: '256', max_request_bytes: '32768', reason: '' };
      return send(200, wrap(data));
    }
    if (url.pathname.endsWith('/dna/tasks')) {
      script.reads.push({ query: [...url.searchParams], source: source() });
      if (url.searchParams.has('snapshot') && url.searchParams.get('snapshot') !== source().record_head) return send(409, error('snapshot_changed', 'Captured Record changed.'));
      return send(200, wrap({ profile: 'dna.task-administration.v1', items: [], page: pageInfo([], Number(url.searchParams.get('limit') || 25), Number(url.searchParams.get('offset') || 0)), basis: { projection: 'dna.task-administration/1', memory: 'record', routing: '0', record_head: source().record_head, record_revision: source().record_revision } }));
    }
    if (url.pathname.endsWith('/commands')) {
      if (request.method() === 'POST') {
        const command = request.postDataJSON(); script.posts.push({ body: command, headers: request.headers() }); script.savedBefore = await saved(page);
        script.applied = true;
        if (script.postMode === 'lost') return route.abort('failed');
        return send(200, receipt(command));
      }
      const requestId = url.searchParams.get('request_id'); script.gets.push(requestId);
      if (script.getMode === 'unavailable') return send(503, error('commands_unavailable', 'Receipt temporarily unavailable.'));
      const command = script.posts.find(post => post.body.request_id === requestId)?.body;
      return command ? send(200, receipt(command)) : send(404, error('command_not_found', 'No fixture receipt.'));
    }
    return send(404, error('not_found', 'Outside this scripted UI contract.'));
  });
  return script;
}
async function open(page, host) { const url = host.origin + '/#/tasks?' + new URLSearchParams({ app: APP }); if (page.url() === url) await page.reload(); else await page.goto(url); }
async function prepare(page) {
  await expect(region(page)).toBeVisible();
  await region(page).getByRole('textbox', { name: 'What should happen', exact: true }).fill(OUTCOME);
  await region(page).getByRole('button', { name: 'Review new task', exact: true }).click();
  await expect(confirmation(page)).toBeVisible();
}

test('New task app contract: exact confirmation stores identity before POST and follows the organism through lookup', async ({ page, host }, info) => {
  const script = await fixture(page); await open(page, host);
  await expect(page.locator('.read-only')).toContainText('New tasks enabled');
  expect(await region(page).getByRole('combobox', { name: 'For locus', exact: true }).locator('option').allTextContents()).toEqual(['Whole organization · org']);
  await prepare(page);
  expect(script.posts).toEqual([]); await expect(confirmation(page)).toContainText('Keep the signed schedule.'); await expect(confirmation(page)).toContainText('the whole organization');
  await expect(region(page).getByRole('textbox', { name: 'What should happen', exact: true })).toBeDisabled();
  await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested');
  expect(script.posts).toHaveLength(1); const post = script.posts[0];
  expect(post.body).toEqual({ request_id: expect.any(String), operation: 'dna.task.create', operation_version: '1', context: { application_id: APP, position_id: 'org' }, target: { application_id: APP, kind: 'dna.record', id: APP }, preconditions: { record_head: HEAD, principal: PRINCIPAL }, arguments: { outcome: OUTCOME, to: 'org' } });
  expect(post.headers['x-hale-command']).toBe('1'); expect(post.headers['content-type']).toContain('application/json');
  expect(script.savedBefore).toEqual([{ key: KEY, value: { version: 7, application_id: APP, principal: PRINCIPAL, request_id: post.body.request_id, operation: 'dna.task.create', operation_version: '1', position_id: 'org', target_kind: 'dna.record', target_id: APP, subject_digest: HEAD } }]);
  await expect(recovery(page)).toContainText(INTENT + ' · requested'); await expect(recovery(page)).toContainText('Not yet born');
  await expect(region(page)).toContainText('Check the saved request');
  script.answer = 'born';
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'born');
  await expect(recovery(page)).toContainText('org:t9'); expect(script.posts).toHaveLength(1); expect(script.gets).toEqual([post.body.request_id]);
  await recovery(page).getByRole('button', { name: 'Intent', exact: true }).click(); await expect(recovery(page)).toContainText('minted its Task');
  await recovery(page).getByRole('button', { name: 'Task', exact: true }).click(); await expect(recovery(page)).toContainText('The Task exists');
  await recovery(page).screenshot({ path: info.outputPath('task-create-readback-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 }); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await recovery(page).screenshot({ path: info.outputPath('task-create-readback-mobile.png') });
  await recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0); expect(await saved(page)).toEqual([]);
  await expect(region(page).getByRole('textbox', { name: 'What should happen', exact: true })).toBeEnabled();
});

test('New task app contract: a refused ask is a completed request, not a failed command', async ({ page, host }) => {
  const script = await fixture(page, { answer: 'refused' }); await open(page, host); await prepare(page);
  await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'refused');
  await expect(recovery(page)).toContainText('refused this ask'); await expect(recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true })).toBeVisible();
  expect(script.posts).toHaveLength(1);
});

test('New task app contract: lost POST reply reload recovers with GET only even after write authority is revoked', async ({ page, host }) => {
  const script = await fixture(page, { postMode: 'lost', getMode: 'unavailable' }); await open(page, host); await prepare(page);
  await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); const snapshot = await saved(page); expect(snapshot).toHaveLength(1);
  script.authorized = false; script.getMode = 'receipt'; await page.reload();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested'); expect(script.posts).toHaveLength(1);
  expect(script.gets).toContain(snapshot[0].value.request_id); expect(await saved(page)).toEqual(snapshot);
  await expect(region(page).getByRole('textbox', { name: 'What should happen', exact: true })).toBeDisabled();
});

test('New task app contract: missing or inconsistent capability cannot raise work', async ({ page, host }) => {
  const script = await fixture(page);
  await open(page, host); await expect(page.locator('#record-list-panel')).toBeVisible();
  // No profile or a profile that contradicts read_only is not a supported
  // surface at all; a denied grant keeps the form visible but disabled.
  for (const options of [{ profile: false }, { profile: true, inconsistent: true }, { inconsistent: false, authorized: false }]) {
    Object.assign(script, options); await open(page, host); await expect(page.locator('#record-list-panel')).toBeVisible();
    if (options.authorized === false) { await expect(region(page).getByRole('textbox', { name: 'What should happen', exact: true })).toBeDisabled(); await expect(region(page)).toHaveAttribute('data-state', 'unavailable'); }
    else await expect(region(page)).toHaveCount(0);
    await expect(page.locator('.read-only')).toContainText('Read only');
  }
  expect(script.posts).toEqual([]); expect(await saved(page)).toEqual([]);
});

test('New task app contract: a receipt that invents a Task or moves the target never becomes observed and triggers no second POST', async ({ page, host }) => {
  const script = await fixture(page, { badReceipt: 'task' }); await open(page, host); await prepare(page);
  await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); await expect(recovery(page)).not.toHaveAttribute('data-intent-state', 'requested'); expect(script.posts).toHaveLength(1); expect(await saved(page)).toHaveLength(1);
  for (const bad of ['target', 'activation']) {
    script.badReceipt = bad; await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
    await expect(recovery(page)).toContainText('could not be verified'); expect(script.posts).toHaveLength(1);
  }
  script.badReceipt = ''; await page.reload(); await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested'); expect(script.posts).toHaveLength(1);
});

test('New task app contract: the existing unresolved shared recovery slot blocks a new ask', async ({ page, host }) => {
  const metadata = { version: 5, application_id: APP, principal: PRINCIPAL, request_id: 'older-task-request', operation: 'dna.task.reassign', operation_version: '1', position_id: 'org', target_kind: 'dna.task', target_id: 'support:t-41', subject_digest: 'sha256:' + '7'.repeat(64) };
  await page.addInitScript(({ key, value }) => localStorage.setItem(key, JSON.stringify(value)), { key: KEY, value: metadata });
  const script = await fixture(page); await open(page, host); await expect(region(page)).toBeVisible();
  await expect(region(page).getByRole('textbox', { name: 'What should happen', exact: true })).toBeDisabled();
  expect(await saved(page)).toEqual([{ key: KEY, value: metadata }]); expect(script.posts).toEqual([]);
});
