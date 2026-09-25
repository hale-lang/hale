// UI CONTRACT ONLY: the head behind /api/hale/v1/head* is scripted by this
// fixture's HTTP server. Real verbs, durable receipts and re-adoption belong
// to the native head lane; nothing here proves an effect on any machine.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { httpFixture } from './http-fixture.mjs';

const APP = 'a'.repeat(40), PRINCIPAL = { mode: 'local', name: 'riley' }, RECORD = 'b'.repeat(40), NEXT = 'c'.repeat(40);
const STORAGE = 'face.projects-recovery.v1:';
const KEY = STORAGE + encodeURIComponent(JSON.stringify([PRINCIPAL.mode, PRINCIPAL.name]));
const OPERATIONS = ['dna.project.create', 'dna.project.init', 'dna.project.attach', 'dna.project.detach', 'dna.project.forget', 'dna.project.sync', 'dna.project.publish', 'dna.forge.configure', 'dna.forge.sync', 'dna.body.local.start', 'dna.body.local.stop', 'dna.body.provision', 'dna.body.start', 'dna.body.stop', 'dna.body.logs', 'dna.secret.set', 'dna.secret.rotate', 'dna.models.probe', 'dna.connection.propose', 'dna.connection.close', 'dna.handoff.publish', 'dna.handoff.accept', 'dna.handoff.sync'];
const HEAD_SCOPED = new Set(['dna.project.create', 'dna.project.init', 'dna.project.attach', 'dna.project.forget']);
const commandId = requestId => 'command-' + createHash('sha256').update(requestId).digest('hex');
const child = state => ({ state, pid: state === 'running' ? 4242 : -1, since: state === 'running' ? 1758470400 : 0, command_id: '', organism_alive: state === 'running', mode: state === 'running' ? 'run' : '', exit_code: state === 'exited' ? 3 : -2 });
const workspace = page => page.getByRole('region', { name: 'Projects workspace', exact: true });
const request = page => page.getByRole('region', { name: 'Project request', exact: true });
const form = (page, title) => page.getByRole('form', { name: title, exact: true });
const saved = page => page.evaluate(prefix => Object.entries(localStorage).filter(([key]) => key.startsWith(prefix)).map(([key, value]) => ({ key, value: JSON.parse(value) })), STORAGE);

function script(options = {}) {
  const s = { active: false, apiState: 'ready', body: 'stopped', busy: '', github: '', board: [], secrets: [], connections: [], recent: [], mode: 'succeeded', lookupsUntilSettled: 0, extraKey: false, unavailable: [], receipts: new Map(), posts: [], gets: [], requests: [], registered: options.active === true, ...options };
  s.operations = OPERATIONS.map(name => ({ name, version: '1', available: !s.unavailable.includes(name) && (s.active || HEAD_SCOPED.has(name)), reason_code: s.unavailable.includes(name) ? 'body_running' : s.active || HEAD_SCOPED.has(name) ? '' : 'detached' }));
  return s;
}
const envelope = (s, data) => ({ api_version: 'hale.v1', head: { profile: 'dna.head.v1', principal: PRINCIPAL, active: s.active ? APP : '' }, data });
const headData = s => ({
  state: s.active ? 'attached' : 'detached',
  active: s.active ? { application_id: APP, root: '/home/operator/dna/demo', name: 'demo', api: { port: 8793, state: s.apiState, pid: 555 } } : null,
  state_dir: '/home/operator/.local/state/hale/dna/head', sources_dir: '/home/operator/.config/hale-dna/sources', projects_dir: '/home/operator/dna',
  children: { body: child(s.body) },
  credentials: { needed: ['MODEL_API_KEY', 'SEARCH_API_KEY'], file_sources: ['MODEL_API_KEY'], env_present: ['SEARCH_API_KEY'] },
  busy: s.busy, operations: s.operations.map(operation => ({ ...operation, available: !s.unavailable.includes(operation.name) && (s.active || HEAD_SCOPED.has(operation.name)), reason_code: s.unavailable.includes(operation.name) ? 'body_running' : s.active || HEAD_SCOPED.has(operation.name) ? '' : 'detached' }))
});
const project = (s, detail) => ({
  application_id: APP, name: 'demo', root: '/home/operator/dna/demo', attached_at: 1758470400, active: s.active,
  record: { head: s.recordHead || RECORD, revision: '12' }, body: child(s.body), forge: { github: s.github, board: s.board }, recent: s.recent,
  ...(detail ? { authority: { command: true, task: true, organization: false }, connections: s.connections, handoffs: [], secrets: s.secrets, receipts: [] } : {})
});
function outcomeFor(s, body) {
  switch (body.operation) {
    case 'dna.project.attach': case 'dna.project.create': case 'dna.project.init': return { application_id: APP, root: '/home/operator/dna/demo', name: 'demo', api_port: 8793, authority_written: true };
    case 'dna.project.sync': return { summary: 'no remote: the record is local', record: { head_before: s.recordHead || RECORD, head_after: s.recordHead || RECORD, rows: [] } };
    case 'dna.forge.configure': return { github: body.arguments.github, board: body.arguments.board };
    case 'dna.secret.set': case 'dna.secret.rotate': return { name: body.arguments.name, where: 'local', source_consumed: body.arguments.source.kind === 'file', record: { head_before: RECORD, head_after: NEXT, rows: [{ seq: 13, kind: 'secret.rotated', entity: body.arguments.name, author: 'riley' }] } };
    case 'dna.body.provision': return body.arguments.dry_run ? { target: body.arguments.target, dry_run: true, preview: '#!/bin/sh\n# provisioning preview for ' + body.arguments.target + '\n' } : { target: body.arguments.target, dry_run: false, text: 'provisioned', record: { head_before: RECORD, head_after: NEXT, rows: [{ seq: 13, kind: 'body.provisioned', entity: body.arguments.target, author: 'riley' }] } };
    case 'dna.connection.propose': return { name: body.arguments.name, text: 'proposed', record: { head_before: RECORD, head_after: NEXT, rows: [{ seq: 13, kind: 'connection.proposed', entity: body.arguments.name, author: 'riley' }] } };
    case 'dna.body.local.start': return { pid: 4242, mode: body.arguments.mode, organism_alive_at: 1758470401 };
    case 'dna.models.probe': return { table: 'model  latency\nalpha  12ms\n' };
    default: return {};
  }
}
function applyEffect(s, body) {
  switch (body.operation) {
    case 'dna.project.attach': case 'dna.project.create': case 'dna.project.init': s.active = true; break;
    case 'dna.project.detach': s.active = false; break;
    case 'dna.forge.configure': s.github = body.arguments.github; s.board = body.arguments.board; break;
    case 'dna.secret.set': case 'dna.secret.rotate': s.secrets = [...s.secrets, body.arguments.name]; s.recordHead = NEXT; break;
    case 'dna.body.provision': if (!body.arguments.dry_run) s.recordHead = NEXT; break;
    case 'dna.connection.propose': s.connections = [...s.connections, { name: body.arguments.name, url: body.arguments.record_url, position: body.arguments.position, purpose: body.arguments.purpose, classes: body.arguments.classes, state: 'proposed' }]; s.recordHead = NEXT; break;
    case 'dna.body.local.start': s.body = 'running'; break;
    case 'dna.body.local.stop': s.body = 'stopped'; break;
    default: break;
  }
}
function receipt(s, body, state, extra = {}) {
  const id = commandId(body.request_id);
  const terminal = ['succeeded', 'failed', 'outcome_unknown', 'refused'].includes(state);
  const run = state === 'refused' || state === 'recorded' ? null : { kind: 'run', pid: 777, deadline: 1758470600, external: extra.external === true, exit_code: state === 'succeeded' ? 0 : state === 'failed' ? 3 : -1, log: '/head/logs?run=' + id };
  return { command_id: id, request_id: body.request_id, operation: body.operation, operation_version: '1', principal: PRINCIPAL, context: body.context, target: body.target, fingerprint: 'sha256:' + createHash('sha256').update(JSON.stringify(body)).digest('hex'), state, reason_code: state === 'refused' ? 'no_record' : state === 'failed' ? '' : '', reason: state === 'refused' ? 'the project has no DNA Record' : state === 'failed' ? 'hale dna sync: fatal: could not read from remote\n' : state === 'outcome_unknown' ? 'the run ended without recording an exit' : '', submitted_at: 1758470400, updated_at: 1758470400 + (terminal ? 3 : 1), run, outcome: state === 'succeeded' ? outcomeFor(s, body) : {} };
}
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['projects.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    let s = script();
    const host = await httpFixture(async (req, res) => {
      const url = new URL(req.url, 'http://127.0.0.1');
      s.requests.push({ method: req.method, path: url.pathname + url.search });
      res.setHeader('cache-control', 'no-store');
      const send = (status, value) => { res.writeHead(status, { 'content-type': 'application/json; charset=utf-8' }); res.end(JSON.stringify(value)); };
      const error = (status, code, message) => send(status, { api_version: 'hale.v1', error: { code, message, retryable: status >= 500 } });
      if (url.pathname === '/') { res.setHeader('content-type', 'text/html; charset=utf-8'); res.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/projects.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      if (assets.has(url.pathname.slice(1))) { res.setHeader('content-type', (url.pathname.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); res.end(assets.get(url.pathname.slice(1))); return; }
      if (url.pathname === '/api/hale/v1/head') { const value = envelope(s, headData(s)); if (s.extraKey) value.data.extra = true; return send(200, value); }
      if (url.pathname === '/api/hale/v1/head/projects') {
        const id = url.searchParams.get('id');
        if (id && id !== APP) return error(404, 'project_not_found', 'unknown project');
        return send(200, envelope(s, { items: s.registered || s.active ? [project(s, Boolean(id))] : [] }));
      }
      // the head's push (GH #986): one `changed`, then the stream ends and
      // the browser reconnects after `retry` — a head whose state moves
      // all the time, which is what following a receipt needs of it
      if (url.pathname === '/api/hale/v1/head/events') { res.writeHead(200, { 'content-type': 'text/event-stream' }); res.end('retry: 100\n\nevent: changed\ndata: 1\n\n'); return; }
      if (url.pathname === '/api/hale/v1/head/logs') return send(200, envelope(s, { name: url.searchParams.get('run') ? 'run ' + url.searchParams.get('run') : 'child ' + url.searchParams.get('child'), offset: Number(url.searchParams.get('offset') || 0), next_offset: 46, text: 'hale dna sync: no remote: the record is local\n', complete: true }));
      if (url.pathname === '/api/hale/v1/head/commands') {
        if (req.method === 'POST') {
          let raw = ''; for await (const chunk of req) raw += chunk;
          const body = JSON.parse(raw);
          s.posts.push({ body, headers: req.headers });
          const first = s.lookupsUntilSettled > 0 ? 'running' : s.mode;
          const stored = { body, remaining: s.lookupsUntilSettled, state: first };
          s.receipts.set(body.request_id, stored);
          if (first === 'succeeded') applyEffect(s, body);
          return send(first === 'running' ? 202 : 200, envelope(s, receipt(s, body, first, { external: s.mode === 'outcome_unknown' })));
        }
        const id = url.searchParams.get('request_id'); s.gets.push(id);
        const stored = s.receipts.get(id);
        if (!stored) return error(404, 'command_not_found', 'no receipt');
        if (stored.state === 'running') { stored.remaining -= 1; if (stored.remaining <= 0) { stored.state = s.mode; if (s.mode === 'succeeded') applyEffect(s, stored.body); } }
        return send(['succeeded', 'failed', 'outcome_unknown', 'refused'].includes(stored.state) ? 200 : 202, envelope(s, receipt(s, stored.body, stored.state, { external: s.mode === 'outcome_unknown' })));
      }
      return error(404, 'not_found', 'outside this scripted head');
    });
    try { await use({ ...host, script: options => { s = script(options); return s; }, state: () => s }); } finally { await host.close(); }
  }
});
async function mount(page, host, options = {}) {
  host.script(options);
  await page.goto(host.origin);
  await page.evaluate(() => {
    window.heads = []; window.attached = []; window.invalidated = [];
    window.controller = window.FaceProjects.mount(document.querySelector('main'), { onHead: head => window.heads.push(head), onAttached: id => window.attached.push(id), onInvalidate: error => window.invalidated.push(error.message) });
  });
  await expect(workspace(page)).toBeVisible();
  await expect.poll(() => host.state().requests.filter(r => r.path === '/api/hale/v1/head').length).toBeGreaterThan(0);
}
async function submitSync(page, host) {
  const posted = page.waitForRequest(r => r.method() === 'POST' && new URL(r.url()).pathname === '/api/hale/v1/head/commands');
  await form(page, 'Sync record').getByRole('button', { name: 'Sync record', exact: true }).click();
  await posted;
  await expect(request(page)).toBeVisible();
}

test('Projects: the head envelope validates closed at every level and never as a Record envelope', async ({ page, host }) => {
  await page.goto(host.origin);
  const s = host.script({ active: true });
  const results = await page.evaluate(({ head, projects }) => {
    const attempt = (value, kind) => { try { window.FaceProjects.validate(value, kind); return 'ok'; } catch (error) { return error.message; } };
    const clone = () => structuredClone(head);
    const out = { valid: attempt(clone(), 'head'), projects: attempt(projects, 'projects') };
    let value = clone(); value.extra = 1; out.topLevel = attempt(value, 'head');
    value = clone(); value.source = { record_id: 'x' }; delete value.head; out.recordEnvelope = attempt(value, 'head');
    value = clone(); value.data.extra = true; out.data = attempt(value, 'head');
    value = clone(); value.head.profile = 'dna.head.v2'; out.profile = attempt(value, 'head');
    value = clone(); value.head.active = ''; out.activeMismatch = attempt(value, 'head');
    value = clone(); value.data.children.body.extra = 1; out.child = attempt(value, 'head');
    value = clone(); value.data.operations[0].scope = 'x'; out.operation = attempt(value, 'head');
    value = clone(); value.head.principal = { mode: 'local', name: 'riley', extra: true }; out.principal = attempt(value, 'head');
    const p = structuredClone(projects); p.data.items[0].note = 'x'; out.projectItem = attempt(p, 'projects');
    return out;
  }, { head: envelope(s, headData(s)), projects: envelope(s, { items: [project(s, true)] }) });
  expect(results.valid).toBe('ok'); expect(results.projects).toBe('ok');
  for (const key of ['topLevel', 'recordEnvelope', 'data', 'profile', 'activeMismatch', 'child', 'operation', 'principal', 'projectItem']) expect(results[key], key).not.toBe('ok');
  const receipts = await page.evaluate(({ good }) => {
    const attempt = value => { try { window.FaceProjects.validate(value, 'command'); return 'ok'; } catch (error) { return error.message; } };
    const out = { valid: attempt(structuredClone(good)) };
    let value = structuredClone(good); value.data.state = 'queued'; out.state = attempt(value);
    value = structuredClone(good); value.data.run.log = '/head/logs?run=other'; out.log = attempt(value);
    value = structuredClone(good); value.data.value = 'leak'; out.extra = attempt(value);
    value = structuredClone(good); value.data.target = { kind: 'dna.task', id: 'x' }; out.target = attempt(value);
    return out;
  }, { good: envelope(s, receipt(s, { request_id: 'head-1', operation: 'dna.project.sync', context: { head: 'local', application_id: APP }, target: { kind: 'dna.project', id: APP }, preconditions: { principal: PRINCIPAL }, arguments: {} }, 'succeeded')) });
  expect(receipts.valid).toBe('ok');
  for (const key of ['state', 'log', 'extra', 'target']) expect(receipts[key], key).not.toBe('ok');
});

test('Projects: a detached head offers create, init and attach with a pre-filled parent and refuses invalid values in the browser', async ({ page, host }, testInfo) => {
  await mount(page, host);
  await expect(workspace(page)).toContainText('No project attached');
  await expect(workspace(page)).toContainText('No project is registered on this machine yet');
  const create = form(page, 'Create project');
  await expect(create.getByLabel(/Parent directory/)).toHaveValue('/home/operator/dna');
  await expect(create.getByLabel('Discover the toolchain while creating')).not.toBeChecked();
  await expect(create.getByLabel('Profile')).toHaveValue('local');
  for (const title of ['Initialize project', 'Attach project']) await expect(form(page, title).getByRole('button', { name: title, exact: true })).toBeEnabled();
  await expect(form(page, 'Sync record').getByRole('button', { name: 'Sync record', exact: true })).toBeDisabled();
  await expect(form(page, 'Sync record')).toContainText('Unavailable: detached');
  await create.getByLabel(/Project name/).fill('Bad Name');
  await create.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(create.getByRole('alert')).toContainText('lowercase letters, digits');
  await expect(create.getByLabel(/Project name/)).toHaveAttribute('aria-invalid', 'true');
  await create.getByLabel(/Project name/).fill('demo');
  await create.getByLabel(/Parent directory/).fill('relative/dir');
  await create.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(create.getByRole('alert')).toContainText('absolute directory');
  const init = form(page, 'Initialize project');
  await init.getByLabel(/Project root/).fill('-rf');
  await init.getByRole('button', { name: 'Initialize project', exact: true }).click();
  await expect(init.getByRole('alert')).toContainText('absolute path');
  const attach = form(page, 'Attach project');
  await attach.getByLabel(/Project root/).fill('/tmp/with\u0007bell');
  await attach.getByRole('button', { name: 'Attach project', exact: true }).click();
  await expect(attach.getByRole('alert')).toBeVisible();
  expect(host.state().posts).toHaveLength(0);
  expect(await saved(page)).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('projects-detached.png'), fullPage: true });
});

test('Projects: an attached head renders every operation form; secrets name a source and never a value; the probe spend is gated', async ({ page, host }, testInfo) => {
  await mount(page, host, { active: true, github: 'octo/demo', board: ['alice'], secrets: ['MODEL_API_KEY'], recent: [{ command_id: commandId('head-old'), request_id: 'head-old', operation: 'dna.project.sync', state: 'succeeded', updated_at: 1758470000 }] });
  await expect(workspace(page)).toContainText('Attached to demo · API ready');
  const registry = page.getByRole('region', { name: 'Project registry', exact: true });
  await expect(registry).toContainText('demo'); await expect(registry).toContainText('active'); await expect(registry).toContainText('forge octo/demo');
  await expect(registry).toContainText('Secrets set');
  await expect(registry).toContainText('MODEL_API_KEY');
  for (const title of ['Sync record', 'Publish genome', 'Configure forge', 'Sync forge', 'Start local body', 'Stop local body', 'Provision body', 'Start remote body', 'Stop remote body', 'Remote body logs', 'Set secret', 'Rotate secret', 'Probe models', 'Propose connection', 'Close connection', 'Publish handoff', 'Accept handoff', 'Sync handoffs']) await expect(form(page, title), title).toBeVisible();
  await expect(form(page, 'Provision body').getByRole('button', { name: 'Preview provisioning', exact: true })).toBeEnabled();
  for (const title of ['Set secret', 'Rotate secret']) {
    const secret = form(page, title);
    expect(await secret.locator('input, textarea, select').evaluateAll(nodes => nodes.map(node => node.name))).toEqual(['name', 'name_custom', 'source', 'body']);
    await expect(secret.locator('[name="value"], input[type="password"]')).toHaveCount(0);
    expect(await secret.getByLabel('Secret name', { exact: true }).locator('option').evaluateAll(nodes => nodes.map(node => node.value))).toEqual(['', 'MODEL_API_KEY', 'SEARCH_API_KEY']);
    expect(await secret.getByLabel('Source', { exact: true }).locator('option').evaluateAll(nodes => nodes.map(node => node.value))).toEqual(['file:MODEL_API_KEY', 'env:SEARCH_API_KEY']);
  }
  const probe = form(page, 'Probe models');
  await expect(probe.getByRole('button', { name: 'Probe models', exact: true })).toBeDisabled();
  await probe.getByLabel('I confirm this probe may spend model credits').check();
  await expect(probe.getByRole('button', { name: 'Probe models', exact: true })).toBeEnabled();
  await probe.getByLabel('I confirm this probe may spend model credits').uncheck();
  await expect(probe.getByRole('button', { name: 'Probe models', exact: true })).toBeDisabled();
  const secret = form(page, 'Set secret');
  await secret.getByLabel(/Custom secret name/).fill('lower');
  await secret.getByRole('button', { name: 'Set secret', exact: true }).click();
  await expect(secret.getByRole('alert')).toBeVisible();
  expect(host.state().posts).toHaveLength(0);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('projects-attached-mobile.png'), fullPage: true });
});

test('Projects: an attach reserves the identity before the POST, follows the receipt and reports the attached project only after a fresh read', async ({ page, host }) => {
  await mount(page, host, { lookupsUntilSettled: 2 });
  // What the browser had saved when its POST left, read while the POST is
  // held: the reservation comes first, or a lost response has no identity
  // to look up.
  let reservedAtPost = null;
  await page.route('**/api/hale/v1/head/commands', async route => {
    if (route.request().method() === 'POST' && reservedAtPost === null) reservedAtPost = await saved(page);
    await route.continue();
  });
  const attach = form(page, 'Attach project');
  await attach.getByLabel(/Project root/).fill('/home/operator/dna/demo');
  const posted = page.waitForRequest(r => r.method() === 'POST' && new URL(r.url()).pathname === '/api/hale/v1/head/commands');
  await attach.getByRole('button', { name: 'Attach project', exact: true }).click();
  const post = await posted;
  expect(post.headers()['x-hale-command']).toBe('1');
  const body = post.postDataJSON();
  expect(body).toMatchObject({ operation: 'dna.project.attach', operation_version: '1', context: { head: 'local', application_id: '' }, target: { kind: 'dna.head', id: 'local' }, preconditions: { principal: PRINCIPAL }, arguments: { root: '/home/operator/dna/demo' } });
  expect(Object.keys(body).sort()).toEqual(['arguments', 'context', 'operation', 'operation_version', 'preconditions', 'request_id', 'target']);
  expect(body.request_id).toMatch(/^head-[0-9a-f-]{36}$/);
  // The browser reports the POST as it leaves; the scripted head records it
  // only once it has read the body. Wait on the head's own record.
  await expect.poll(() => host.state().posts.length).toBe(1);
  expect(host.state().posts[0].headers['x-hale-command']).toBe('1');
  expect(reservedAtPost).toHaveLength(1);
  expect(reservedAtPost[0].value).toEqual({ version: 1, request_id: body.request_id, operation: 'dna.project.attach', target: { kind: 'dna.head', id: 'local' } });
  // The receipt is followed to its end. Whether the browser ever shows it
  // running depends on how soon the head's push makes it look again, so a
  // transient state is not asserted; the end and what follows it are.
  await expect(request(page)).toHaveAttribute('data-state', 'succeeded', { timeout: 10_000 });
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  expect(host.state().gets.filter(id => id === body.request_id).length).toBeGreaterThanOrEqual(2);
  expect(host.state().posts).toHaveLength(1);
  expect(await page.evaluate(() => window.attached)).toEqual([APP]);
  // Reported only after a fresh read: the scripted head settles the attach
  // on the second lookup, and a read of the head follows that lookup.
  const log = host.state().requests;
  const lookups = log.flatMap((r, i) => r.method === 'GET' && r.path === '/api/hale/v1/head/commands?request_id=' + encodeURIComponent(body.request_id) ? [i] : []);
  expect(lookups.length).toBeGreaterThanOrEqual(2);
  expect(log.slice(lookups[1] + 1).some(r => r.method === 'GET' && r.path === '/api/hale/v1/head')).toBe(true);
  await expect(workspace(page)).toContainText('Attached to demo');
  expect(await saved(page)).toEqual([]);
  await expect(request(page).getByRole('button', { name: 'Dismiss', exact: true })).toBeVisible();
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  await expect(request(page)).toBeHidden();
});

test('Projects: refused, failed and unknown outcomes render as themselves, and an unknown outcome names the effect, links the log and offers no retry', async ({ page, host }) => {
  await mount(page, host, { active: true, mode: 'refused' });
  await submitSync(page, host);
  await expect(request(page)).toHaveAttribute('data-state', 'refused');
  await expect(request(page)).toContainText('Refused · no_record');
  await expect(request(page)).toContainText('the project has no DNA Record');
  await expect(request(page)).toHaveAttribute('data-observation', 'none');
  expect(await saved(page)).toEqual([]);
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();

  host.state().mode = 'failed';
  await submitSync(page, host);
  await expect(request(page)).toHaveAttribute('data-state', 'failed');
  await expect(request(page)).toContainText('Failed · exit 3');
  await expect(request(page)).toContainText('could not read from remote');
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();

  host.state().mode = 'outcome_unknown';
  await submitSync(page, host);
  await expect(request(page)).toHaveAttribute('data-state', 'outcome_unknown');
  await expect(request(page)).toHaveAttribute('data-observation', 'none');
  const id = commandId(host.state().posts.at(-1).body.request_id);
  await expect(request(page)).toContainText(id);
  await expect(request(page)).toContainText('dna.project.sync');
  await expect(request(page)).toContainText('dna.project · ' + APP);
  await expect(request(page)).toContainText('Inspect the run log and the project\'s Record before submitting again');
  await expect(request(page).getByRole('link', { name: 'Run log', exact: true })).toHaveAttribute('href', '/api/hale/v1/head/logs?run=' + id);
  await expect(request(page).getByRole('button', { name: /retry|resubmit|try again|submit again/i })).toHaveCount(0);
  await request(page).getByRole('button', { name: 'Show run log', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Head logs', exact: true })).toContainText('no remote: the record is local');
  expect(host.state().posts).toHaveLength(3);
  expect(await saved(page)).toEqual([]);
});

test('Projects: a succeeded verb is observed through the head, the preview submits dry_run and the forge form sends a closed argument set', async ({ page, host }) => {
  await mount(page, host, { active: true });
  await submitSync(page, host);
  await expect(request(page)).toHaveAttribute('data-state', 'succeeded');
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  await expect(request(page)).toContainText('no remote: the record is local');
  await expect(request(page)).toContainText('Rows appended');
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  const provision = form(page, 'Provision body');
  await provision.getByLabel(/Target/).fill('operator@body.example');
  await provision.getByRole('button', { name: 'Preview provisioning', exact: true }).click();
  await expect(request(page)).toHaveAttribute('data-state', 'succeeded');
  expect(host.state().posts.at(-1).body.arguments).toEqual({ target: 'operator@body.example', dsn: '', dir: '', dry_run: true });
  await expect(request(page).locator('pre')).toContainText('provisioning preview for operator@body.example');
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  const forge = form(page, 'Configure forge');
  await forge.getByLabel(/GitHub repository/).fill('octo/demo');
  await forge.getByLabel(/Board logins/).fill('alice, bob');
  await forge.getByRole('button', { name: 'Configure forge', exact: true }).click();
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  expect(host.state().posts.at(-1).body).toMatchObject({ operation: 'dna.forge.configure', target: { kind: 'dna.project', id: APP }, context: { head: 'local', application_id: APP }, arguments: { github: 'octo/demo', board: ['alice', 'bob'] } });
  await expect(page.getByRole('region', { name: 'Project registry', exact: true })).toContainText('forge octo/demo');
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  const secret = form(page, 'Set secret');
  await secret.getByLabel('Secret name', { exact: true }).selectOption('MODEL_API_KEY');
  await secret.getByLabel('Source', { exact: true }).selectOption('file:MODEL_API_KEY');
  await secret.getByRole('button', { name: 'Set secret', exact: true }).click();
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  const posted = host.state().posts.at(-1).body;
  expect(posted.arguments).toEqual({ name: 'MODEL_API_KEY', source: { kind: 'file', name: 'MODEL_API_KEY' }, body: '' });
  expect(JSON.stringify(posted)).not.toContain('value');
  await expect(request(page)).toContainText('source_consumed');
  expect(host.state().posts).toHaveLength(4);
});

test('Projects: a saved identity is restored as a lookup, never a POST; a lost response keeps its identity and is looked up until it settles', async ({ page, host }) => {
  await page.goto(host.origin);
  const s = host.script({ active: true });
  const body = { request_id: 'head-restored', operation: 'dna.project.sync', operation_version: '1', context: { head: 'local', application_id: APP }, target: { kind: 'dna.project', id: APP }, preconditions: { principal: PRINCIPAL }, arguments: {} };
  s.receipts.set('head-restored', { body, remaining: 0, state: 'succeeded' });
  await page.evaluate(([key, value]) => localStorage.setItem(key, value), [KEY, JSON.stringify({ version: 1, request_id: 'head-restored', operation: 'dna.project.sync', target: body.target })]);
  await page.evaluate(() => { window.controller = window.FaceProjects.mount(document.querySelector('main'), {}); });
  await expect(request(page)).toContainText('Saved request');
  await expect(request(page)).toHaveAttribute('data-state', 'succeeded');
  // The receipt renders first; the saved identity is cleared only once the
  // head has been read again, which is when the observation settles.
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  expect(host.state().posts).toHaveLength(0);
  expect(host.state().gets).toEqual(['head-restored']);
  expect(await saved(page)).toEqual([]);
  await request(page).getByRole('button', { name: 'Dismiss', exact: true }).click();

  await page.evaluate(([key, value]) => localStorage.setItem(key, value), [KEY, JSON.stringify({ version: 1, request_id: 'head-missing', operation: 'dna.project.sync', target: body.target })]);
  await page.evaluate(() => { window.controller.destroy(); window.controller = window.FaceProjects.mount(document.querySelector('main'), {}); });
  await expect(request(page)).toContainText('No receipt was found for the saved request');
  await expect(request(page).getByRole('button', { name: 'Check status', exact: true })).toBeVisible();
  await expect(form(page, 'Sync record').getByRole('button', { name: 'Sync record', exact: true })).toBeDisabled();
  await expect(form(page, 'Sync record')).toContainText('Resolve the request above');
  expect(await saved(page)).toHaveLength(1);
  await request(page).getByRole('button', { name: 'Discard unconfirmed identity', exact: true }).click();
  await expect(request(page)).toBeHidden();
  expect(await saved(page)).toEqual([]);
  expect(host.state().posts).toHaveLength(0);

  // The head records the POST; only its response is lost on the way back.
  // The uncertain moment in between is not asserted: the head's push makes
  // the browser look the identity up at once, so it may never be seen.
  let reservedAtPost = null;
  await page.route('**/api/hale/v1/head/commands', async route => { if (route.request().method() === 'POST') { reservedAtPost = await saved(page); await route.fetch(); await route.abort('failed'); } else await route.continue(); });
  await form(page, 'Sync record').getByRole('button', { name: 'Sync record', exact: true }).click();
  await expect.poll(() => host.state().posts.length).toBe(1);
  await page.unroute('**/api/hale/v1/head/commands');
  const lost = host.state().posts[0].body.request_id;
  expect(reservedAtPost).toHaveLength(1);
  expect(reservedAtPost[0].value.request_id).toBe(lost);
  await expect(request(page)).toHaveAttribute('data-state', 'succeeded', { timeout: 10_000 });
  await expect(request(page)).toHaveAttribute('data-observation', 'observed');
  // learned by looking the saved identity up, never by posting again
  expect(host.state().posts).toHaveLength(1);
  expect(host.state().gets).toContain(lost);
  expect(host.state().gets.at(-1)).toBe(lost);
  expect(await saved(page)).toEqual([]);
});

test('Projects: a busy head and an unavailable operation disable their forms with the reason; an unsupported head answer is not a head', async ({ page, host }) => {
  await mount(page, host, { active: true, busy: commandId('head-busy'), unavailable: ['dna.body.local.start'] });
  await expect(workspace(page)).toContainText('running ' + commandId('head-busy'));
  await expect(form(page, 'Sync record').getByRole('button', { name: 'Sync record', exact: true })).toBeDisabled();
  await expect(form(page, 'Sync record')).toContainText('The head is running');
  await expect(form(page, 'Start local body')).toContainText('Unavailable: body_running');
  const heads = await page.evaluate(() => window.heads.length);
  expect(heads).toBeGreaterThan(0);
  host.script({ active: true, extraKey: true });
  await workspace(page).getByRole('button', { name: 'Refresh projects', exact: true }).click();
  await expect(workspace(page)).toContainText('last read failed: The project service returned an unsupported or incomplete response.');
  expect(await page.evaluate(() => window.heads.length)).toBe(heads);
  expect(host.state().posts).toHaveLength(0);
});
