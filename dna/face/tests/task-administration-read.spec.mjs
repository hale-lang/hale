// UI CONTRACT ONLY: all Task/capability/receipt responses below are scripted.
// Real native reassignment, authority and durable recovery have a separate gate.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './http-fixture.mjs';

const API = '/api/hale/v1/applications', APP = 'a'.repeat(40), PRINCIPAL = { mode: 'local', name: 'riley' };
const HEAD = 'b'.repeat(40), NEXT = 'c'.repeat(40), EVENT = 'd'.repeat(40), DIGEST = 'sha256:' + 'e'.repeat(64);
const STORAGE = 'face.practice-recovery.v1:';
const KEY = STORAGE + encodeURIComponent(JSON.stringify([APP, PRINCIPAL.mode, PRINCIPAL.name]));
const task = () => ({ id: 'support:t-41', outcome: 'Confirm the supplier handover — équipe', state: 'handed', assignee: 'mara', obligation: 'supplier-handover', acceptance_digest: 'acceptance:original/v1', acceptance_bound: true, evidence_required: true, evidence_ref: '', waiting: '', assignment_digest: DIGEST, reassignment_supported: true, history: [{ event_id: '1'.repeat(40), sequence: '3', kind: 'task.handed', from: '', to: 'mara', by: 'leader' }] });
const region = page => page.getByRole('region', { name: 'Handed Task administration', exact: true });
const recovery = page => page.getByRole('region', { name: 'Task reassignment request', exact: true });
const error = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: false } });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const names = ['index.html', 'app.js', 'styles.css', 'application.js', 'organization-draft.js', 'definition-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js'];
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
  const script = { authorized: true, available: true, profile: true, inconsistent: false, recipients: ['mara', 'dev'], row: task(), applied: false, postMode: 'receipt', getMode: 'receipt', readMode: 'ok', badReceipt: '', posts: [], gets: [], reads: [], savedBefore: [], ...options };
  const source = () => ({ record_id: APP, record_head: script.applied ? NEXT : HEAD, record_revision: script.applied ? '11' : '10' });
  const wrap = data => ({ api_version: 'hale.v1', source: source(), data });
  const pageInfo = (items, limit = 25, offset = 0) => ({ limit, offset, total: items.length, next_offset: -1, snapshot: source().record_head });
  const receipt = command => {
    const data = { command_id: 'ui-task/' + command.request_id, request_id: command.request_id, application_id: APP, operation: 'dna.task.reassign', operation_version: '1', principal: PRINCIPAL, context: command.context, target: command.target, subject_digest: command.preconditions.subject_digest, fingerprint: 'sha256:' + 'f'.repeat(64), state: 'succeeded', reason: '', task: { state: 'applied', from: command.preconditions.assignee, to: command.arguments.to, event_id: EVENT }, review: { state: 'unavailable', outcome: '', subject_digest: '' }, activation: { state: 'unknown', reason: '' } };
    if (script.badReceipt === 'to') data.task.to = 'someone-else';
    if (script.badReceipt === 'target') data.target = { ...data.target, id: 'different-task' };
    if (script.badReceipt === 'activation') data.activation.state = 'adopted';
    return wrap(data);
  };
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url());
    const send = (status, data) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) });
    if (url.pathname === API) return send(200, wrap({ items: [{ id: APP, kind: 'dna', name: 'Task UI contract', capabilities_url: API + '/' + APP + '/capabilities' }], page: pageInfo([{}]) }));
    if (url.pathname.endsWith('/capabilities')) {
      const write = script.profile && script.available && script.authorized;
      const data = { application_id: APP, principal: PRINCIPAL, read_only: script.inconsistent ? true : !write, reads: { tasks: true, workflows: false, reviews: false, practices: false, organization: false, definitions: false, knowledge: false }, writes: { practice_propose: false, review_verdict: false, task_reassign: write } };
      if (script.profile) data.task_commands = { profile: 'dna.task.reassign.v1', available: script.available, authorized: script.authorized, position_id: 'org', recovery: 'record_lifetime', max_identity_bytes: '256', max_request_bytes: '32768', recipients: script.available && script.authorized ? script.recipients : [], reason: '' };
      return send(200, wrap(data));
    }
    if (url.pathname.endsWith('/dna/tasks')) {
      script.reads.push({ query: [...url.searchParams], source: source() });
      if (script.readMode === 'protected') return send(403, error('forbidden', 'Task details withheld.'));
      if (script.readMode === 'unauthenticated') return send(401, error('unauthenticated', 'Sign in required.'));
      if (url.searchParams.has('snapshot') && url.searchParams.get('snapshot') !== source().record_head) return send(409, error('snapshot_changed', 'Captured Record changed.'));
      const row = structuredClone(script.row); if (script.readMode === 'malformed') row.assignee = 'mismatched';
      const items = [row];
      return send(200, wrap({ profile: 'dna.task-administration.v1', items, page: pageInfo(items, Number(url.searchParams.get('limit') || 25), Number(url.searchParams.get('offset') || 0)), basis: { projection: 'dna.task-administration/1', memory: 'record', routing: '0', record_head: source().record_head, record_revision: source().record_revision } }));
    }
    if (url.pathname.endsWith('/commands')) {
      if (request.method() === 'POST') {
        const command = request.postDataJSON(); script.posts.push({ body: command, headers: request.headers() }); script.savedBefore = await saved(page);
        script.applied = true; script.row.assignee = command.arguments.to; script.row.assignment_digest = 'sha256:' + '9'.repeat(64);
        script.row.history.push({ event_id: EVENT, sequence: '10', kind: 'task.reassigned', from: command.preconditions.assignee, to: command.arguments.to, by: PRINCIPAL.name });
        if (script.postMode === 'lost') return route.abort('failed');
        return send(200, receipt(command));
      }
      const requestId = url.searchParams.get('request_id'); script.gets.push(requestId);
      if (script.getMode === 'unavailable') return send(503, error('commands_unavailable', 'Receipt temporarily unavailable.'));
      if (script.getMode === 'unauthenticated') return send(401, error('unauthenticated', 'Sign in required.'));
      const command = script.posts.find(post => post.body.request_id === requestId)?.body;
      return command ? send(200, receipt(command)) : send(404, error('command_not_found', 'No fixture receipt.'));
    }
    return send(404, error('not_found', 'Outside this scripted UI contract.'));
  });
  return script;
}
async function open(page, host) { const url = host.origin + '/#/tasks?' + new URLSearchParams({ app: APP, id: task().id }); if (page.url() === url) await page.reload(); else await page.goto(url); }
async function prepare(page) { await expect(region(page)).toBeVisible(); await region(page).getByRole('combobox', { name: 'New assignee', exact: true }).selectOption('dev'); await region(page).getByRole('button', { name: 'Review reassignment', exact: true }).click(); await expect(page.getByRole('group', { name: 'Confirm Task reassignment', exact: true })).toBeVisible(); }

test('Task app contract: exact confirmation stores identity before POST and follows a fresh Task read', async ({ page, host }, info) => {
  const script = await fixture(page); await open(page, host); await prepare(page);
  expect(script.posts).toEqual([]); await expect(page.getByRole('group', { name: 'Confirm Task reassignment', exact: true })).toContainText('mara → dev');
  await page.getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-observation', 'observed');
  expect(script.posts).toHaveLength(1); const post = script.posts[0];
  expect(post.body).toEqual({ request_id: expect.any(String), operation: 'dna.task.reassign', operation_version: '1', context: { application_id: APP, position_id: 'org' }, target: { application_id: APP, kind: 'dna.task', id: task().id }, preconditions: { subject_digest: DIGEST, principal: PRINCIPAL, assignee: 'mara' }, arguments: { to: 'dev' } });
  expect(post.headers['x-hale-command']).toBe('1'); expect(post.headers['content-type']).toContain('application/json');
  expect(script.savedBefore).toEqual([{ key: KEY, value: { version: 5, application_id: APP, principal: PRINCIPAL, request_id: post.body.request_id, operation: 'dna.task.reassign', operation_version: '1', position_id: 'org', target_kind: 'dna.task', target_id: task().id, subject_digest: DIGEST } }]);
  expect(script.gets).toContain(post.body.request_id); expect(script.reads.some(read => read.source.record_head === NEXT)).toBe(true);
  await expect(region(page).locator('.task-current-assignment strong')).toHaveText('dev'); await expect(region(page)).toContainText('acceptance:original/v1'); await expect(region(page)).toContainText('Handed · completion not recorded');
  await recovery(page).getByRole('button', { name: 'Current Task', exact: true }).click(); await expect(recovery(page)).toContainText('Open · dev');
  await recovery(page).screenshot({ path: info.outputPath('task-readback-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 }); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await recovery(page).screenshot({ path: info.outputPath('task-readback-mobile.png') });
});

test('Task app contract: lost POST reply reload recovers with GET only even after write authority is revoked', async ({ page, host }) => {
  const script = await fixture(page, { postMode: 'lost', getMode: 'unavailable' }); await open(page, host); await prepare(page); await page.getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); const snapshot = await saved(page); expect(snapshot).toHaveLength(1);
  script.authorized = false; script.getMode = 'receipt'; await page.reload();
  await expect(recovery(page)).toHaveAttribute('data-observation', 'observed'); expect(script.posts).toHaveLength(1);
  expect(script.gets).toContain(snapshot[0].value.request_id); expect(await saved(page)).toEqual(snapshot);
  await expect(region(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled();
});

test('Task app contract: inaccessible, signed-out or malformed Task evidence clears prior responsibility details', async ({ page, host }) => {
  const script = await fixture(page); await open(page, host); await expect(region(page)).toBeVisible();
  for (const [mode, title] of [['protected', 'Unable to read the application'], ['unauthenticated', 'Sign in to read this application'], ['malformed', 'Response could not be verified']]) {
    script.readMode = mode; await page.reload(); await expect(region(page)).toHaveCount(0); await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(page.getByText(task().outcome, { exact: true })).toHaveCount(0);
  }
  expect(script.posts).toEqual([]);
});

test('Task app contract: missing or inconsistent capability and unsupported Task cannot grant reassignment', async ({ page, host }) => {
  const script = await fixture(page);
  for (const options of [{ profile: false }, { profile: true, inconsistent: true }, { inconsistent: false, recipients: ['dev', 'dev'] }, { recipients: ['mara'] }, { recipients: ['mara', 'dev'], row: { ...task(), reassignment_supported: false } }]) {
    Object.assign(script, options); await open(page, host); await expect(region(page)).toBeVisible();
    await expect(region(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'Confirm reassignment', exact: true })).toHaveCount(0);
  }
  expect(script.posts).toEqual([]); expect(await saved(page)).toEqual([]);
});

test('Task app contract: receipt intent mismatch never becomes observed reassignment or triggers another POST', async ({ page, host }) => {
  const script = await fixture(page, { badReceipt: 'to' }); await open(page, host); await prepare(page); await page.getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); await expect(recovery(page)).not.toHaveAttribute('data-observation', 'observed'); expect(script.posts).toHaveLength(1); expect(await saved(page)).toHaveLength(1);
  script.badReceipt = 'activation'; await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); expect(script.posts).toHaveLength(1);
  script.badReceipt = ''; await page.reload(); await expect(recovery(page)).toHaveAttribute('data-observation', 'observed'); expect(script.posts).toHaveLength(1);
});

test('Task app contract: the existing unresolved shared recovery slot blocks a new Task request', async ({ page, host }) => {
  const metadata = { version: 2, application_id: APP, principal: PRINCIPAL, request_id: 'older-review-request', operation: 'dna.review.verdict', operation_version: '1', position_id: 'org', target_kind: 'dna.review', target_id: 'review:old', subject_digest: 'sha256:' + '7'.repeat(64) };
  await page.addInitScript(({ key, value }) => localStorage.setItem(key, JSON.stringify(value)), { key: KEY, value: metadata });
  const script = await fixture(page); await open(page, host); await expect(region(page)).toBeVisible();
  await expect(region(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled();
  expect(await saved(page)).toEqual([{ key: KEY, value: metadata }]); expect(script.posts).toEqual([]);
});
