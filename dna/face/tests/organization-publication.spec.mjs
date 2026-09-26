// UI CONTRACT ONLY. Chart/source/changed-validation bodies are actual retained
// native API responses. Publication capabilities and command receipts are
// scripted; passing these checks does not establish native publication effects.
import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { UNGATED, commandReceipt, commandReply, describeLine, receiptLine, refusedReply } from './command-wire.mjs';

const evidence = process.env.HALE_ORGANIZATION_PUBLICATION_EVIDENCE;
const web = fileURLToPath(new URL('../web/', import.meta.url));
const API = '/api/hale/v1/applications', STORAGE = 'face.practice-recovery.v1:';
const RATIONALE = 'Place billing in this exact Organization source — équipe.\nKeep <img src=x onerror="window.__publicationInjected=true"> literal.';
test.skip(!evidence, 'Supply genuine same-app Organization and draft GET/changed-validation response exports.');
let native, server, origin;
test.beforeAll(async () => {
  if (!evidence) return;
  const files = ['capabilities-response.json', 'organization-response.json', 'draft-response.json', 'draft-validation-request.json', 'draft-validation-response.json'];
  const bytes = Object.fromEntries(await Promise.all(files.map(async file => [file, await readFile(path.join(evidence, file), 'utf8')])));
  const [capabilities, organization, original, request, checked] = files.map(file => JSON.parse(bytes[file]));
  expect(original.data.validation).toBe('checked_source'); expect(checked.data.validation).toBe('valid_draft');
  expect(checked.data.module.text).toBe(request.source_text); expect(checked.data.module.digest).not.toBe(original.data.module.digest);
  expect(checked.data.base).toEqual(original.data.base); expect(organization.source).toEqual(original.source); expect(checked.source).toEqual(original.source);
  native = { bytes, capabilities, organization, original, request, checked, application: original.source.record_id, principal: original.data.principal };
  const assets = new Set(['index.html', 'styles.css', 'app.js', 'application.js', 'definition-draft.js', 'organization-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js']);
  server = createServer(async (req, res) => {
    try {
      const pathname = new URL(req.url, 'http://localhost').pathname, file = pathname === '/' ? 'index.html' : pathname.slice(1);
      if (!assets.has(file)) { res.writeHead(404); res.end(); return; }
      res.setHeader('content-type', file.endsWith('.js') ? 'text/javascript; charset=utf-8' : file.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8');
      res.end(await readFile(path.join(web, file)));
    } catch { res.writeHead(500); res.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = 'http://127.0.0.1:' + server.address().port;
});
test.afterAll(async () => { if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } });
const editor = page => page.getByRole('region', { name: 'Organization editing', exact: true });
const source = page => editor(page).getByLabel('Organization Hale source', { exact: true });
const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
async function metadata(page) { return page.evaluate(prefix => Object.entries(localStorage).filter(([key]) => key.startsWith(prefix)).map(([key, text]) => ({ key, value: JSON.parse(text) })), STORAGE); }
const envelope = data => ({ api_version: 'hale.v1', source: native.original.source, data });
const failure = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: code === 'commands_unavailable' } });

async function fixture(page, options = {}) {
  const script = { authorized: true, available: true, postMode: 'receipt', stage: 'created', invalidSource: false, posts: [], gets: [], validations: [], savedBeforeSend: [], errors: [], ...options };
  page.on('pageerror', error => script.errors.push(error.message));
  // The retained capabilities with the forwarding route the head names.
  const capabilities = () => {
    const data = structuredClone(native.capabilities.data);
    data.api = { transport: 'unix', socket: '/run/scripted.sock', http: API + '/' + native.application + '/commands' };
    return envelope(data);
  };
  // The session's slice: the board's OrganizationPropose while the script seats it.
  const slice = () => [...UNGATED, ...(script.authorized && script.available ? ['OrganizationPropose'] : [])];
  const receipt = ({ payload: request }) => {
    const unknown = script.stage === 'unknown', created = ['created', 'applied'].includes(script.stage), applied = script.stage === 'applied';
    const commit = 'e'.repeat(40), reviewId = 'source-' + 'd'.repeat(64);
    return receiptLine(commandReply(commandReceipt({
      command_id: 'ui-contract/' + request.request_id, request_id: request.request_id, application_id: native.application,
      operation: 'dna.organization.propose', operation_version: '1', principal_mode: native.principal.mode, principal_name: native.principal.name,
      target_kind: 'dna.organization.module', target_id: 'dna/org/main.hl',
      subject_digest: request.module_digest, fingerprint: 'sha256:' + 'c'.repeat(64),
      state: created ? 'succeeded' : unknown ? 'outcome_unknown' : 'recorded', proposal_state: '',
      organization: {
        proposal_state: created ? 'created' : unknown ? 'unknown' : 'pending', source_head: request.source_head,
        source_digest: script.invalidSource ? 'sha256:' + 'f'.repeat(64) : native.checked.data.module.digest,
        mutation_id: created ? reviewId : '', candidate_commit: created ? commit : '',
        application_state: applied ? 'applied' : unknown ? 'unknown' : 'pending', application_reason_code: '',
        restart_handoff_state: applied ? 'requested' : unknown ? 'unknown' : 'pending',
      },
      review_state: applied ? 'settled' : created ? 'pending' : 'unavailable', review_outcome: applied ? 'approve' : '', review_subject_digest: created ? commit : '',
    }), native.original.source));
  };
  await page.route('**/api/hale/v1{,/**}', async route => {
    const req = route.request(), url = new URL(req.url());
    const fulfill = (status, value) => route.fulfill({ status, contentType: 'application/json; charset=utf-8', body: JSON.stringify(value) });
    const retained = file => route.fulfill({ status: 200, contentType: 'application/json; charset=utf-8', body: native.bytes[file] });
    const base = API + '/' + native.application;
    if (url.pathname === API) return fulfill(200, envelope({ items: [{ id: native.application, kind: 'dna', name: 'Retained native Organization draft', capabilities_url: base + '/capabilities' }], page: { limit: 25, offset: 0, total: 1, next_offset: -1, snapshot: native.original.source.record_head } }));
    if (url.pathname === base + '/capabilities') return fulfill(200, capabilities());
    if (url.pathname === base + '/dna/organization') return retained('organization-response.json');
    if (url.pathname === base + '/dna/organization/draft') {
      if (req.method() === 'GET') return retained('draft-response.json');
      const body = req.postDataJSON(); script.validations.push(body);
      // A native result is returned only for the exact retained request. This
      // fixture cannot validate arbitrary edited source by inventing a digest.
      expect(body).toEqual(native.request);
      return retained('draft-validation-response.json');
    }
    if (url.pathname === base + '/dna/reviews') return fulfill(503, failure('commands_unavailable', 'Review read intentionally unavailable in this UI contract fixture.'));
    if (url.pathname !== base + '/commands') return fulfill(404, failure('not_found', 'Outside this UI contract fixture.'));
    if (req.method() === 'POST') {
      const body = req.postDataJSON();
      if (body.describe) return fulfill(200, describeLine(slice()));
      script.savedBeforeSend = await metadata(page); script.posts.push({ body, headers: req.headers() }); // counted once the page read is done, so a test's next navigation cannot destroy it
      if (script.postMode === 'lost') return route.abort('failed');
      return fulfill(200, receipt(body));
    }
    const id = url.searchParams.get('request_id'); script.gets.push(id);
    const original = script.posts.find(post => post.body.payload.request_id === id);
    if (!original) return fulfill(200, receiptLine(refusedReply('command_not_found')));
    return fulfill(200, receipt(original.body));
  });
  return script;
}
async function checked(page) {
  await page.goto(origin + '/#/organization?' + new URLSearchParams({ app: native.application }));
  await editor(page).getByRole('button', { name: 'Edit organization source', exact: true }).click();
  await expect(source(page)).toHaveValue(native.original.data.module.text);
  await source(page).fill(native.checked.data.module.text);
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Native organization validation passed');
}
async function confirm(page) {
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill(RATIONALE);
  await page.getByRole('button', { name: 'Review publication', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Submit Organization proposal', exact: true })).toBeEnabled();
}
async function send(page) { await confirm(page); await page.getByRole('button', { name: 'Submit Organization proposal', exact: true }).click(); await expect(recovery(page)).toBeVisible(); }

test('Organization publication UI contract: exact native checked bytes and five-field base are submitted with v4 metadata', async ({ page }, testInfo) => {
  const script = await fixture(page); await checked(page); await confirm(page);
  expect(script.posts).toHaveLength(0); expect(await metadata(page)).toHaveLength(0);
  const chart = page.getByLabel('Checked candidate chart');
  await chart.getByRole('button', { name: 'Org.billing', exact: true }).click();
  await expect(chart.getByRole('group', { name: 'Selected instance impact', exact: true })).toContainText('Org.billing');
  await page.screenshot({ path: testInfo.outputPath('organization-publication-desktop.png'), fullPage: true });
  await page.getByRole('group', { name: 'Organization publication confirmation', exact: true }).screenshot({ path: testInfo.outputPath('publication-confirmation-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('organization-publication-mobile.png'), fullPage: true });
  await page.getByRole('group', { name: 'Organization publication confirmation', exact: true }).screenshot({ path: testInfo.outputPath('publication-confirmation-mobile.png') });
  await page.getByRole('button', { name: 'Submit Organization proposal', exact: true }).click();
  await expect.poll(() => script.posts.length).toBe(1); await expect(recovery(page)).toContainText('Source application');
  const sent = script.posts[0];
  expect(sent.body).toEqual({
    call: 'OrganizationPropose',
    payload: { request_id: expect.any(String), ...native.original.data.base, source_text: native.checked.data.module.text, rationale: RATIONALE },
  });
  expect(sent.headers['x-hale-command']).toBe('1'); expect(script.savedBeforeSend).toHaveLength(1);
  expect(script.savedBeforeSend[0].value).toEqual({
    version: 4, application_id: native.application, principal: native.principal, request_id: sent.body.payload.request_id,
    operation: 'dna.organization.propose', operation_version: '1', position_id: 'org', target_kind: 'dna.organization.module', target_id: 'dna/org/main.hl',
    subject_digest: native.original.data.base.module_digest, base: native.original.data.base, source_digest: native.checked.data.module.digest,
  });
  const storage = JSON.stringify(await metadata(page)); expect(storage).not.toContain(RATIONALE); expect(storage).not.toContain(native.checked.data.module.text);
  await expect(recovery(page).getByRole('list', { name: 'Request and outcome' }).getByRole('button')).toHaveCount(6);
  await expect(recovery(page)).toContainText('Not established by this receipt');
  await expect(recovery(page).getByRole('link', { name: 'Open source Review', exact: true })).toHaveAttribute('href', '#/reviews?app=' + native.application + '&id=source-' + 'd'.repeat(64));
  // Keep the narrow width, but fit this tall region below the sticky navigation.
  await page.setViewportSize({ width: 390, height: 1400 });
  await recovery(page).screenshot({ path: testInfo.outputPath('publication-recovery-mobile.png') });
  await page.setViewportSize({ width: 1440, height: 1100 });
  await recovery(page).screenshot({ path: testInfo.outputPath('publication-recovery-desktop.png') });
  expect(await page.evaluate(() => window.__publicationInjected)).toBeUndefined(); expect(script.errors).toEqual([]);
});

test('Organization publication UI contract: native validation and export do not confer source publication permission', async ({ page }) => {
  const script = await fixture(page, { authorized: false }); await checked(page);
  await expect(editor(page).getByRole('button', { name: 'Download validated Hale', exact: true })).toBeEnabled();
  const review = page.getByRole('button', { name: 'Review publication', exact: true });
  if (await review.count()) await expect(review).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Submit Organization proposal', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(0); expect(await metadata(page)).toHaveLength(0); expect(script.errors).toEqual([]);
});

test('Organization publication UI contract: editing checked source invalidates the pending publication confirmation', async ({ page }) => {
  const script = await fixture(page); await checked(page); await confirm(page);
  await source(page).fill(native.checked.data.module.text + '\n// Later edit has not been validated.\n');
  await expect(page.getByRole('button', { name: 'Submit Organization proposal', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(0); expect(await metadata(page)).toHaveLength(0); expect(script.errors).toEqual([]);
});

test('Organization publication UI contract: lost reply reload uses GET only and separates applied source from a running version', async ({ page }) => {
  const script = await fixture(page, { postMode: 'lost' }); await checked(page); await send(page);
  await expect.poll(() => script.posts.length).toBe(1); const id = script.posts[0].body.payload.request_id;
  script.authorized = false; script.stage = 'applied'; await page.reload();
  await expect.poll(() => script.gets.includes(id)).toBe(true);
  await expect(recovery(page)).toContainText('applied'); await expect(recovery(page)).toContainText('requested');
  await expect(recovery(page)).toContainText('Running version'); await expect(recovery(page)).toContainText('Not established by this receipt');
  expect(script.posts).toHaveLength(1); expect((await metadata(page))[0].value).toEqual(script.savedBeforeSend[0].value);
  await recovery(page).getByRole('link', { name: 'Open source Review', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Reviews', exact: true })).toBeVisible();
  await expect(recovery(page)).toBeVisible(); expect(script.posts).toHaveLength(1);
  expect((await metadata(page))[0].value.request_id).toBe(id); expect(script.errors).toEqual([]);
});

test('Organization publication UI contract: unknown stages remain unknown and changed candidate identity fails closed', async ({ page }) => {
  const script = await fixture(page, { stage: 'unknown' }); await checked(page); await send(page);
  await expect(recovery(page)).toContainText('outcome_unknown');
  const trail = recovery(page).getByRole('list', { name: 'Request and outcome' });
  await expect(trail).toContainText('unknown'); await expect(trail).not.toContainText('pending');
  script.stage = 'created'; script.invalidSource = true;
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toContainText('The request outcome could not be verified.');
  await expect(recovery(page).getByRole('link', { name: 'Open source Review', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(1); expect(await metadata(page)).toHaveLength(1); expect(script.errors).toEqual([]);
});
