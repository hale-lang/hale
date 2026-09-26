// UI CONTRACT ONLY: the Review and candidate are retained actual native API
// responses. Discovery/capabilities/verdict transport are explicitly scripted;
// these checks do not prove native authority, publication, apply, or launch.
import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { UNGATED, commandReceipt, commandReply, describeLine, receiptLine, refusedReply, routeError } from './command-wire.mjs';

const evidence = process.env.HALE_ORGANIZATION_REVIEW_EVIDENCE;
const web = fileURLToPath(new URL('../web/', import.meta.url));
const API = '/api/hale/v1/applications';
const STORAGE = 'face.practice-recovery.v1:';
const NOTE = 'Approve the exact retained source — 第二版.\nKeep <img src=x onerror="window.__sourceInjected=true"> literal.';
test.skip(!evidence, 'Supply retained native candidate-response.json and review-response.json.');
let server, origin, native;

test.beforeAll(async () => {
  if (!evidence) return;
  const candidateText = await readFile(path.join(evidence, 'candidate-response.json'), 'utf8');
  const reviewsText = await readFile(path.join(evidence, 'review-response.json'), 'utf8');
  const candidate = JSON.parse(candidateText), reviews = JSON.parse(reviewsText);
  const review = reviews.data.items.find(row => row.id === candidate.data.review_id);
  expect(review?.organization_source).toBe(true);
  expect(review?.state).toBe('pending');
  expect(candidate.source).toEqual(reviews.source);
  native = { candidateText, reviewsText, candidate, reviews, review, application: candidate.source.record_id };
  const assets = new Set(['index.html', 'styles.css', 'app.js', 'application.js', 'definition-draft.js', 'organization-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js']);
  server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://localhost');
      const name = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
      if (!assets.has(name)) { response.writeHead(404); response.end(); return; }
      response.setHeader('content-type', name.endsWith('.js') ? 'text/javascript; charset=utf-8' : name.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/html; charset=utf-8');
      response.end(await readFile(path.join(web, name)));
    } catch { response.writeHead(500); response.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = 'http://127.0.0.1:' + server.address().port;
});
test.afterAll(async () => { if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } });

async function metadata(page) {
  return page.evaluate(prefix => Object.entries(localStorage).filter(([key]) => key.startsWith(prefix))
    .map(([key, value]) => ({ key, value: JSON.parse(value) })), STORAGE);
}
function envelope(data) { return { api_version: 'hale.v1', source: native.candidate.source, data }; }
function failure(code, message, retryable = false) { return { api_version: 'hale.v1', error: { code, message, retryable } }; }

async function fixture(page, options = {}) {
  const script = {
    principal: { mode: 'local', name: 'bob' }, sourceProfile: true, sourceAvailable: true, sourceAuthorized: true,
    candidateMissing: false, postMode: 'receipt', getMode: 'receipt', invalidActivation: false,
    posts: [], gets: [], candidateReads: [], savedBeforeSend: [], errors: [], ...options,
  };
  page.on('pageerror', error => script.errors.push(error.message));
  const capabilities = () => envelope({
    application_id: native.application, principal: script.principal, read_only: true,
    reads: { reviews: true, practices: false, organization: false, definitions: false, knowledge: false, workflows: false },
    api: { transport: 'unix', socket: '/run/scripted.sock', http: script.sourceProfile ? API + '/' + native.application + '/commands' : '' },
  });
  // The session's slice: one ReviewVerdict decides ordinary and source
  // Reviews alike; the head's provider checks the owner quorum.
  const slice = () => [...UNGATED, ...(script.sourceAvailable && script.sourceAuthorized ? ['ReviewVerdict'] : [])];
  const receipt = ({ payload: request }) => receiptLine(commandReply(commandReceipt({
    command_id: 'ui-contract/' + request.request_id, request_id: request.request_id,
    application_id: native.application, operation: 'dna.review.verdict', operation_version: '1',
    principal_mode: script.principal.mode, principal_name: script.principal.name,
    target_kind: 'dna.review', target_id: request.review_id, subject_digest: request.subject_digest,
    fingerprint: 'sha256:' + 'c'.repeat(64), state: 'recorded', proposal_state: '',
    verdict_value: request.verdict, verdict_state: 'pending',
    review_state: script.invalidActivation ? 'settled' : 'pending', review_outcome: script.invalidActivation ? 'approve' : '', review_subject_digest: request.subject_digest,
    activation_state: script.invalidActivation ? 'adopted' : 'unknown',
  }), native.candidate.source));
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url());
    const fulfill = (status, data) => route.fulfill({ status, contentType: 'application/json; charset=utf-8', body: JSON.stringify(data) });
    if (url.pathname === API) return fulfill(200, envelope({
      items: [{ id: native.application, kind: 'dna', name: 'Retained Organization source evidence', capabilities_url: API + '/' + native.application + '/capabilities' }],
      page: native.reviews.data.page,
    }));
    if (url.pathname === API + '/' + native.application + '/capabilities') return fulfill(200, capabilities());
    if (url.pathname === API + '/' + native.application + '/dna/reviews') {
      return route.fulfill({ status: 200, contentType: 'application/json; charset=utf-8', body: native.reviewsText });
    }
    if (url.pathname === API + '/' + native.application + '/dna/reviews/candidate') {
      script.candidateReads.push({ id: url.searchParams.get('id'), snapshot: url.searchParams.get('snapshot') });
      if (script.candidateMissing) return fulfill(404, failure('candidate_not_found', 'No visible canonical candidate was found.'));
      return route.fulfill({ status: 200, contentType: 'application/json; charset=utf-8', body: native.candidateText });
    }
    if (url.pathname !== API + '/' + native.application + '/commands') return fulfill(404, failure('not_found', 'Outside the UI contract fixture.'));
    if (request.method() === 'POST') {
      // the page's saved identity read first, then the POST counted: a test
      // waiting on `posts` then reads a complete `savedBeforeSend`, and its
      // next navigation cannot destroy this read
      const body = request.postDataJSON();
      if (body.describe) return fulfill(200, describeLine(slice()));
      script.savedBeforeSend = await metadata(page);
      script.posts.push({ body, headers: request.headers() });
      if (script.postMode === 'lost') return route.abort('failed');
      return fulfill(200, receipt(body));
    }
    const id = url.searchParams.get('request_id'); script.gets.push(id);
    if (script.getMode === 'unavailable') return fulfill(503, routeError('commands_unavailable', 'Scripted receipt source unavailable.'));
    const original = script.posts.find(post => post.body.payload.request_id === id);
    if (!original) return fulfill(200, receiptLine(refusedReply('command_not_found')));
    return fulfill(200, receipt(original.body));
  });
  return script;
}
async function open(page) {
  await page.goto(origin + '/#/reviews?' + new URLSearchParams({ app: native.application, id: native.review.id }));
  await expect(page.getByRole('region', { name: 'Review intervention', exact: true })).toBeVisible();
}
async function prepare(page) {
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: 'Approve', exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill(NOTE);
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  await expect(page.locator('#decision-confirmation')).toBeVisible();
}

test('Organization source UI contract: ReviewVerdict in the slice enables the exact candidate controls', async ({ page }, testInfo) => {
  const script = await fixture(page); await open(page);
  await expect(page.getByRole('region', { name: 'Organization candidate comparison', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeEnabled();
  expect(script.candidateReads).toContainEqual({ id: native.review.id, snapshot: native.candidate.source.record_head });
  await prepare(page);
  expect(script.posts).toHaveLength(0);
  await expect(page.locator('#decision-confirmation')).toContainText(native.review.subject_digest);
  await page.screenshot({ path: testInfo.outputPath('organization-source-decision-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('organization-source-decision-mobile.png'), fullPage: true });
  expect(script.errors).toEqual([]);
});

test('Organization source UI contract: a slice without ReviewVerdict cannot grant a source decision', async ({ page }) => {
  const script = await fixture(page, { sourceAuthorized: false }); await open(page);
  await expect(page.getByRole('region', { name: 'Organization candidate comparison', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Submit decision', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(0); expect(script.errors).toEqual([]);
});

test('Organization source UI contract: actual proposer remains unable to decide their source proposal', async ({ page }) => {
  const script = await fixture(page, { principal: { mode: 'local', name: native.review.author } }); await open(page);
  await expect(page.getByRole('region', { name: 'Organization candidate comparison', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.posts).toHaveLength(0); expect(script.errors).toEqual([]);
});

test('Organization source UI contract: missing exact candidate disables decisions despite source capability', async ({ page }) => {
  const script = await fixture(page, { candidateMissing: true }); await open(page);
  await expect(page.getByRole('region', { name: 'Organization candidate comparison', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.posts).toHaveLength(0); expect(script.errors).toEqual([]);
});

test('Organization source UI contract: exact ordinary verdict payload and source-profile GET-only reload recovery', async ({ page }) => {
  const script = await fixture(page, { postMode: 'lost', getMode: 'unavailable' }); await open(page); await prepare(page);
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
  await expect.poll(() => script.posts.length).toBe(1);
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  const sent = script.posts[0];
  expect(sent.body).toEqual({
    call: 'ReviewVerdict',
    payload: { request_id: expect.any(String), review_id: native.review.id, subject_digest: native.review.subject_digest, verdict: 'approve', comment: NOTE },
  });
  expect(sent.headers['x-hale-command']).toBe('1'); expect(sent.headers['content-type']).toContain('application/json');
  expect(script.savedBeforeSend).toHaveLength(1);
  expect(script.savedBeforeSend[0].value).toEqual({
    version: 3, application_id: native.application, principal: script.principal, request_id: sent.body.payload.request_id,
    operation: 'dna.review.verdict', operation_version: '1', position_id: 'org', target_kind: 'dna.review',
    target_id: native.review.id, subject_digest: native.review.subject_digest, source_review: true,
  });
  const saved = JSON.stringify(await metadata(page));
  expect(saved).not.toContain(NOTE); expect(saved).not.toContain(JSON.parse(native.candidate.data.document).module.text);
  expect(await page.evaluate(() => window.__sourceInjected)).toBeUndefined();
  script.sourceAuthorized = false; script.getMode = 'receipt';
  await page.reload();
  await expect.poll(() => script.gets.includes(sent.body.payload.request_id)).toBe(true);
  const recovery = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(recovery).toContainText(/recorded/i);
  await expect(recovery).toContainText('Organization result');
  await expect(recovery).toContainText('Follow change status');
  await expect(recovery.getByRole('link', { name: 'Open Organization', exact: true })).toHaveAttribute('href', '#/organization?app=' + native.application);
  await expect(recovery.getByRole('link', { name: 'Open candidate practice', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(1);
  expect((await metadata(page))[0].value).toEqual(script.savedBeforeSend[0].value);
  await expect(page.getByRole('textbox', { name: 'Decision note', exact: true })).toHaveCount(0);
  // This is deliberately invalid scripted transport, not a native adoption:
  // source verdict receipts cannot borrow the Knowledge activation field.
  script.invalidActivation = true;
  await recovery.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery).toContainText('The request outcome could not be verified.');
  await expect(recovery.getByRole('link', { name: 'Open Organization', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(1);
  expect((await metadata(page))[0].value).toEqual(script.savedBeforeSend[0].value);
  expect(script.errors).toEqual([]);
});
