// UI CONTRACT ONLY: Organization, Task and command responses are scripted.
// This checks browser routing/validation/recovery, not native authority or effects.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './runtime-harness.mjs';

const API = '/api/hale/v1/applications', APP = 'a'.repeat(40), OTHER = '2'.repeat(40);
const HEAD = 'b'.repeat(40), NEXT = 'c'.repeat(40), EVENT = 'd'.repeat(40), DIGEST = 'sha256:' + 'e'.repeat(64);
const PERSON = 'mara / équipe ? &', PRINCIPAL = { mode: 'local', name: 'riley' };
const task = (index = 0) => ({ id: 'task/' + index, outcome: 'Confirm handover ' + index, state: 'handed', assignee: PERSON, obligation: 'handover', acceptance_digest: 'acceptance:original/v1', acceptance_bound: true, evidence_required: true, evidence_ref: '', waiting: '', assignment_digest: DIGEST, reassignment_supported: true, history: [{ event_id: '1'.repeat(40), sequence: '3', kind: 'task.handed', from: '', to: PERSON, by: 'leader' }] });
const query = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
const tasks = page => page.getByRole('region', { name: 'Handed Task administration', exact: true });
const recovery = page => page.getByRole('region', { name: 'Task reassignment request', exact: true });
const people = page => page.getByRole('region', { name: 'Declared members', exact: true });
const error = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: false } });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', value => errors.push(value.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const names = ['index.html', 'app.js', 'styles.css', 'runtime.js', 'application.js', 'organization-draft.js', 'definition-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js'];
    const assets = new Map(await Promise.all(names.map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const host = await httpFixture((request, response) => { const name = request.url === '/' ? 'index.html' : request.url.slice(1);
      if (!assets.has(name)) { response.writeHead(404).end(); return; }
      response.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html') + '; charset=utf-8'); response.setHeader('cache-control', 'no-store'); response.end(assets.get(name));
    });
    try { await use(host); } finally { await host.close(); }
  }
});
async function fixture(page, options = {}) {
  const script = { rows: [task()], authorized: true, applied: false, bad: '', lost: false, lookupUnavailable: false, reads: [], posts: [], gets: [], ...options };
  const source = (app = APP) => ({ record_id: app, record_head: script.applied ? NEXT : HEAD, record_revision: script.applied ? '51' : '50' });
  const wrap = (data, app = APP) => ({ api_version: 'hale.v1', source: source(app), data });
  const pageInfo = (total, offset = 0, limit = 25, snapshot = source().record_head) => ({ total, offset, limit, next_offset: offset + limit < total ? offset + limit : -1, snapshot });
  const receipt = command => wrap({ command_id: 'scripted/' + command.request_id, request_id: command.request_id, application_id: APP, operation: 'dna.task.reassign', operation_version: '1', principal: PRINCIPAL, context: command.context, target: command.target, subject_digest: command.preconditions.subject_digest, fingerprint: 'sha256:' + 'f'.repeat(64), state: 'succeeded', reason: '', task: { state: 'applied', from: command.preconditions.assignee, to: command.arguments.to, event_id: EVENT }, review: { state: 'unavailable', outcome: '', subject_digest: '' }, activation: { state: 'unknown', reason: '' } });
  const org = (url, app) => {
    const id = 'Org', item = { id, declaration: 'Org', parent_id: '', thread_domain: 'main', role: 'position', in_position_outline: true, source_file: 'dna/org/main.hl', sealed: false, parameters: [], methods: [], publishes: [], subscribes: [], supervises: [] };
    return wrap({ items: [item], page: pageInfo(1, Number(url.searchParams.get('offset') || 0)), basis: { source_head: '9'.repeat(40), seed: 'dna/org', artifact_digest: 'scripted-source', dependency_digest: DIGEST, dependency_source: 'none', shape_hash: 'scripted-shape', schema: '1.19', semantics: 2, position_group_declared: true, exact_ownership: true, coverage: 'static_instances', declaration_count: 1, uninstantiated_declaration_count: 0 }, ownership: { mode: 'shared', host_owner: 'support', positions: [{ position: 'org/support', owner: 'support' }], memberships: [{ owner: 'support', members: [PERSON] }, { owner: 'assurance', members: [PERSON, 'dev'] }], instance_binding: 'unavailable' } }, app);
  };
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url()), app = url.pathname.includes('/' + OTHER + '/') ? OTHER : APP;
    const send = (status, data) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) });
    if (url.pathname === API) return send(200, wrap({ items: [APP, OTHER].map((id, index) => ({ id, kind: 'dna', name: 'Scripted Organization ' + index, capabilities_url: API + '/' + id + '/capabilities' })), page: pageInfo(2) }));
    if (url.pathname.endsWith('/capabilities')) return send(200, wrap({ application_id: app, principal: PRINCIPAL, read_only: !script.authorized, reads: { tasks: true, organization: true, workflows: false, practices: false, reviews: false, definitions: false, knowledge: false }, writes: { practice_propose: false, review_verdict: false, task_reassign: script.authorized }, task_commands: { profile: 'dna.task.reassign.v1', available: true, authorized: script.authorized, position_id: 'org', recovery: 'record_lifetime', max_identity_bytes: '256', max_request_bytes: '32768', recipients: script.authorized ? [PERSON, 'dev'] : [], reason: '' } }, app));
    if (url.pathname.endsWith('/dna/organization')) return send(200, org(url, app));
    if (url.pathname.endsWith('/dna/tasks')) {
      script.reads.push({ app, query: [...url.searchParams], source: source(app) });
      if (url.searchParams.has('snapshot') && url.searchParams.get('snapshot') !== source(app).record_head) return send(409, error('snapshot_changed', 'Scripted Record changed.'));
      const id = url.searchParams.get('id'), assignee = url.searchParams.get('assignee'), offset = Number(url.searchParams.get('offset') || 0), limit = Number(url.searchParams.get('limit') || 25);
      const selected = script.rows.filter(row => (!id || row.id === id) && (!assignee || row.assignee === assignee));
      const items = structuredClone(selected.slice(offset, offset + limit));
      if (!id && script.bad === 'item' && items[0]) { items[0].assignee = 'dev'; items[0].history[0].to = 'dev'; }
      const data = { profile: 'dna.task-administration.v1', items, page: pageInfo(selected.length, offset, limit), basis: { projection: 'dna.task-administration/1', memory: 'record', routing: '0', record_head: source(app).record_head, record_revision: source(app).record_revision } };
      if (assignee) data.assignee = script.bad === 'echo' ? 'dev' : assignee;
      if (!id && script.bad === 'missing') delete data.assignee;
      return send(200, wrap(data, app));
    }
    if (url.pathname.endsWith('/commands')) {
      if (request.method() === 'POST') {
        const command = request.postDataJSON(); script.posts.push(command); script.applied = true;
        const row = script.rows.find(row => row.id === command.target.id); row.assignee = command.arguments.to; row.assignment_digest = 'sha256:' + '8'.repeat(64);
        row.history.push({ event_id: EVENT, sequence: '50', kind: 'task.reassigned', from: command.preconditions.assignee, to: command.arguments.to, by: PRINCIPAL.name });
        if (script.lost) return route.abort('failed'); return send(200, receipt(command));
      }
      const id = url.searchParams.get('request_id'); script.gets.push(id);
      if (script.lookupUnavailable) return send(503, error('commands_unavailable', 'Scripted recovery unavailable.'));
      const command = script.posts.find(value => value.request_id === id); return command ? send(200, receipt(command)) : send(404, error('command_not_found', 'No scripted receipt.'));
    }
    return send(404, error('not_found', 'Outside this scripted browser contract.'));
  });
  return script;
}
async function open(page, host, extra = {}) { await page.goto(host.origin + '/#/tasks?' + new URLSearchParams({ app: APP, assignee: PERSON, ...extra })); }
async function prepare(page) { await expect(tasks(page)).toBeVisible(); await tasks(page).getByRole('combobox', { name: 'New assignee', exact: true }).selectOption('dev'); await tasks(page).getByRole('button', { name: 'Review reassignment', exact: true }).click(); await expect(page.getByRole('group', { name: 'Confirm Task reassignment', exact: true })).toContainText(PERSON + ' → dev'); }

test('Scripted people routes: declared member keyboard navigation preserves exact identity without owner/locus authority inference', async ({ page, host }, info) => {
  const script = await fixture(page, { authorized: false }); await page.setViewportSize({ width: 390, height: 844 }); await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(host.origin + '/#/organization?' + new URLSearchParams({ app: APP, locus: 'org/support' }));
  const member = people(page).getByRole('button', { name: 'Inspect recorded assignments for ' + PERSON + ' · declared owner assurance', exact: true });
  await expect(member).toBeVisible(); await member.focus(); await page.keyboard.press('Enter');
  await expect.poll(() => query(page).get('assignee')).toBe(PERSON); expect(query(page).get('locus')).toBeNull(); expect(query(page).get('owner')).toBeNull();
  await expect(page.getByRole('region', { name: 'Assignments for ' + PERSON, exact: true })).toBeVisible();
  expect(script.reads.at(-1).query).toContainEqual(['assignee', PERSON]); expect(script.posts).toEqual([]);
  await page.getByRole('link', { name: 'Confirm handover 0', exact: false }).click(); await expect(tasks(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); await page.screenshot({ path: info.outputPath('declared-person-tasks-mobile.png') });
});

test('Scripted people routes: filter survives pagination/reload and application switching clears it', async ({ page, host }) => {
  const script = await fixture(page, { rows: Array.from({ length: 26 }, (_, index) => task(index)) }); await open(page, host);
  await page.getByRole('button', { name: 'Next page', exact: true }).click(); await expect.poll(() => query(page).get('offset')).toBe('25');
  expect(query(page).get('assignee')).toBe(PERSON); expect(query(page).get('snapshot')).toBe(HEAD); await page.reload();
  await expect(page.getByRole('link', { name: 'Confirm handover 25', exact: false })).toBeVisible(); expect(script.reads.at(-1).query).toEqual([['limit', '25'], ['offset', '25'], ['assignee', PERSON], ['snapshot', HEAD]]);
  await page.getByLabel('Application', { exact: true }).selectOption(OTHER); await expect.poll(() => query(page).get('app')).toBe(OTHER);
  await expect.poll(() => script.reads.at(-1)?.app).toBe(OTHER); expect(query(page).get('assignee')).toBeNull(); expect(script.reads.at(-1).query.some(([key]) => key === 'assignee')).toBe(false); expect(script.posts).toEqual([]);
});

for (const mode of ['echo', 'item', 'missing']) test('Scripted people routes: ' + mode + ' filter response mismatch clears all Task details', async ({ page, host }) => {
  const script = await fixture(page); await open(page, host, { id: task().id }); await expect(tasks(page)).toBeVisible();
  script.bad = mode; await page.reload(); await expect(page.getByRole('heading', { name: 'Response could not be verified', exact: true })).toBeVisible();
  await expect(tasks(page)).toHaveCount(0); await expect(page.getByText(task().outcome, { exact: true })).toHaveCount(0); expect(script.posts).toEqual([]);
});

test('Scripted people routes: confirmed reassignment keeps exact selected Task after it leaves the filtered list', async ({ page, host }, info) => {
  const script = await fixture(page); await open(page, host, { id: task().id }); await prepare(page); expect(script.posts).toEqual([]);
  await page.getByRole('button', { name: 'Confirm reassignment', exact: true }).click(); await expect(recovery(page)).toHaveAttribute('data-observation', 'observed');
  await expect(tasks(page).locator('.task-current-assignment strong')).toHaveText('dev'); expect(query(page).get('assignee')).toBe(PERSON); expect(query(page).get('id')).toBe(task().id);
  await expect(page.getByText('No visible Task has this recorded assignee in the inspected snapshot. This does not establish that the person has no other responsibilities.', { exact: true })).toBeVisible();
  await expect(page.getByText('This Task is now recorded under dev. The list still shows assignments for ' + PERSON + '.', { exact: true })).toBeVisible();
  const current = script.reads.filter(read => read.source.record_head === NEXT); expect(current.some(read => read.query.some(([key, value]) => key === 'assignee' && value === PERSON))).toBe(true);
  expect(current.some(read => read.query.some(([key, value]) => key === 'id' && value === task().id) && !read.query.some(([key]) => key === 'assignee'))).toBe(true);
  expect(script.posts).toHaveLength(1); expect(script.posts[0].preconditions.assignee).toBe(PERSON); expect(script.posts[0].arguments).toEqual({ to: 'dev' });
  await page.screenshot({ path: info.outputPath('reassigned-task-retained-selection.png') });
  await page.getByRole('link', { name: 'View assignments for dev', exact: true }).click(); await expect.poll(() => query(page).get('assignee')).toBe('dev');
});

test('Scripted people routes: lost response reload recovers by GET when the old assignee list is empty', async ({ page, host }) => {
  const script = await fixture(page, { lost: true, lookupUnavailable: true }); await open(page, host, { id: task().id }); await prepare(page); await page.getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  await expect(recovery(page)).toContainText('could not be verified'); expect(script.posts).toHaveLength(1); const id = script.posts[0].request_id;
  script.lookupUnavailable = false; script.authorized = false; await page.reload(); await expect(recovery(page)).toHaveAttribute('data-observation', 'observed');
  expect(script.posts).toHaveLength(1); expect(script.gets).toContain(id); expect(query(page).get('assignee')).toBe(PERSON); expect(query(page).get('id')).toBe(task().id);
  await expect(tasks(page).locator('.task-current-assignment strong')).toHaveText('dev'); await expect(tasks(page).getByRole('combobox', { name: 'New assignee', exact: true })).toBeDisabled();
});

test('Scripted people routes: duplicate or empty assignee route refuses before Task data read', async ({ page, host }) => {
  const script = await fixture(page);
  for (const suffix of ['assignee=', 'assignee=mara&assignee=dev']) {
    await page.goto(host.origin + '/#/tasks?app=' + APP + '&' + suffix); await expect(page.getByRole('heading', { name: 'Unable to read the application', exact: true })).toBeVisible();
    await expect(page.getByText('Choose one exact person from the ownership map, or return to all handed Tasks.', { exact: true })).toBeVisible();
  }
  expect(script.reads).toEqual([]); expect(script.posts).toEqual([]);
});
