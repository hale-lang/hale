// Browser contract: the baseline Review/status are unmodified native exports.
// Discovery, transport failures and explicitly marked scenario variants are
// scripted. These tests do not prove live Host/API composition or publication.
import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const evidence = process.env.HALE_ORGANIZATION_STATUS_EVIDENCE;
const API = '/api/hale/v1/applications';
const web = fileURLToPath(new URL('../web/', import.meta.url));
test.skip(!evidence, 'Supply genuine same-snapshot status-response.json and reviews-response.json.');
let server, origin, native;
test.beforeAll(async () => {
  if (!evidence) return;
  const statusText = await readFile(path.join(evidence, 'status-response.json'), 'utf8');
  const reviewsText = await readFile(path.join(evidence, 'reviews-response.json'), 'utf8');
  const status = JSON.parse(statusText), reviews = JSON.parse(reviewsText);
  expect(status.source).toEqual(reviews.source);
  const review = reviews.data.items.find(value => value.id === status.data.review_id);
  expect(review.organization_source).toBe(true);
  expect(status.data.current_running.available).toBe(false);
  expect(status.data.observation.state).toBe('healthy');
  native = { statusText, reviewsText, status, reviews, review, app: status.source.record_id };
  const assets = new Set(['index.html', 'app.js', 'styles.css', 'runtime.js', 'application.js', 'organization-draft.js', 'definition-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js']);
  server = createServer(async (request, response) => {
    const name = new URL(request.url, 'http://localhost').pathname.slice(1) || 'index.html';
    if (!assets.has(name)) { response.writeHead(404); response.end(); return; }
    try {
      response.setHeader('content-type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html');
      response.end(await readFile(path.join(web, name)));
    } catch { response.writeHead(500); response.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = 'http://127.0.0.1:' + server.address().port;
});
test.afterAll(async () => { if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } });
const region = page => page.getByRole('region', { name: 'Organization change status', exact: true });
const error = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: false } });
const wrap = data => ({ api_version: 'hale.v1', source: native.status.source, data });
async function fixture(page, options = {}) {
  const script = { mode: 'native', response: null, reads: [], commands: [], errors: [], ...options };
  page.on('pageerror', value => script.errors.push(value.message));
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url());
    const send = (status, body) => route.fulfill({ status, contentType: 'application/json', body: typeof body === 'string' ? body : JSON.stringify(body) });
    if (url.pathname === API) return send(200, wrap({ items: [{ id: native.app, kind: 'dna', name: 'Native Organization history', capabilities_url: API + '/' + native.app + '/capabilities' }], page: native.reviews.data.page }));
    if (url.pathname.endsWith('/capabilities')) return send(200, wrap({ application_id: native.app, principal: { mode: 'local', name: 'independent-reviewer' }, read_only: true, reads: { reviews: true, practices: false, organization: false, definitions: false, workflows: false, knowledge: false }, writes: { practice_propose: false, review_verdict: false } }));
    if (url.pathname.endsWith('/dna/reviews')) return send(200, native.reviewsText);
    if (url.pathname.endsWith('/dna/reviews/candidate')) return send(503, error('candidate_unavailable', 'Retained diff exceeds the source-comparison rendering bound.'));
    if (url.pathname.endsWith('/dna/organization/source-status')) {
      script.reads.push({ method: request.method(), query: [...url.searchParams] });
      if (script.mode === 'protected') return send(404, error('source_status_not_found', 'No visible source status.'));
      if (script.mode === 'forbidden') return send(403, error('forbidden', 'No source evidence read grant.'));
      if (script.mode === 'unauthenticated') return send(401, error('unauthenticated', 'Sign in required.'));
      if (script.mode === 'outage') return send(503, error('source_status_unavailable', 'Captured source unavailable.'));
      return send(200, script.response || native.statusText);
    }
    if (url.pathname.endsWith('/commands')) script.commands.push({ method: request.method(), query: url.search });
    return send(404, error('not_found', 'Outside this browser contract fixture.'));
  });
  return script;
}
async function open(page) {
  await page.goto(origin + '/#/reviews?' + new URLSearchParams({ app: native.app, id: native.review.id }));
}
function currentScenario() {
  const response = structuredClone(native.status), data = response.data;
  data.current_running = { available: true, profile: 'dna.organization-runtime/1', attempt_id: data.launch.attempt_id, launch_event_id: data.launch.event_id, candidate_commit: data.source.candidate_commit, binary_digest: data.launch.binary_digest, topology_digest: data.launch.topology_digest, topology_shape: data.launch.topology_shape, process_key: 'a'.repeat(64) };
  return response;
}

test('Organization status: real retained history stays useful without source text or proposer recovery identity', async ({ page }, info) => {
  const script = await fixture(page); await open(page);
  await expect(region(page)).toContainText('Healthy during window');
  await expect(region(page).getByRole('button', { name: 'Running', exact: true })).toContainText('Not established');
  await expect(region(page).getByRole('link', { name: 'Inspect exact process in Runtime' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.reads).toEqual([{ method: 'GET', query: [['id', native.review.id], ['snapshot', native.status.source.record_head]] }]);
  expect(await page.evaluate(() => localStorage.length)).toBe(0);
  await region(page).screenshot({ path: info.outputPath('change-journey-desktop.png') });
  await page.setViewportSize({ width: 390, height: 1100 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await region(page).screenshot({ path: info.outputPath('change-journey-mobile.png') });
  await region(page).getByRole('button', { name: 'Refresh change status' }).click();
  await expect(region(page)).toContainText('Healthy during window');
  await page.reload(); await expect(region(page)).toContainText('Healthy during window');
  expect(script.reads.length).toBe(3); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization status: access loss and outages clear prior history and process links', async ({ page }) => {
  const script = await fixture(page, { response: currentScenario() }); await open(page);
  await expect(region(page)).toContainText('Verified at this read');
  for (const mode of ['protected', 'forbidden', 'outage']) {
    script.mode = mode;
    await region(page).getByRole('button', { name: 'Refresh change status' }).click();
    await expect(region(page)).toContainText('Change status unavailable');
    await expect(region(page)).not.toContainText('Verified at this read');
    await expect(region(page)).not.toContainText('Healthy during window');
    await expect(region(page).getByRole('link')).toHaveCount(0);
  }
  script.mode = 'unauthenticated'; await region(page).getByRole('button', { name: 'Refresh change status' }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(region(page)).toHaveCount(0); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization status: a different candidate, snapshot or process artifact cannot provide running evidence', async ({ page }) => {
  const script = await fixture(page);
  for (const mutate of [value => { value.data.source.candidate_commit = '1'.repeat(40); }, value => { value.source.record_head = '2'.repeat(40); }, value => { value.data.current_running.binary_digest = 'sha256:' + '3'.repeat(64); }, value => { value.data.current_running.process_key = ''; }]) {
    script.response = currentScenario(); mutate(script.response); await open(page);
    await expect(page.getByRole('heading', { name: 'Response could not be verified' })).toBeVisible();
    await expect(region(page)).toHaveCount(0);
    await expect(page.getByRole('link', { name: 'Inspect exact process in Runtime' })).toHaveCount(0);
  }
  expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization status: exact process navigation keeps Review context and grants no application controls', async ({ page }) => {
  const script = await fixture(page, { response: currentScenario() }); await open(page);
  const jump = region(page).getByRole('link', { name: 'Inspect exact process in Runtime' });
  const href = await jump.getAttribute('href'), query = new URLSearchParams(href.split('?')[1]);
  expect(query.get('process')).toBe('a'.repeat(64)); expect(query.get('review')).toBe(native.review.id); expect(query.get('app')).toBe(native.app);
  await jump.click(); await expect(page.getByRole('region', { name: 'Runtime observer', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open application controls', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Back to Organization change', exact: true }).click();
  await expect(region(page)).toContainText('Verified at this read');
  expect(script.reads.length).toBe(2); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization status: unknown launch stays unresolved without inheriting healthy history', async ({ page }) => {
  const response = structuredClone(native.status), d = response.data;
  d.launch = { state: 'unknown', attempt_id: '', request_event_id: '', event_id: '', binary_digest: '', topology_digest: '', topology_shape: '' };
  d.observation = { state: 'unknown', event_id: '', window_started: '', window_ended: '' };
  const script = await fixture(page, { response }); await open(page);
  await expect(region(page).getByRole('button', { name: 'Launch', exact: true })).toContainText('unknown');
  await expect(region(page)).not.toContainText('Healthy during window');
  await expect(region(page).getByRole('button', { name: 'Running', exact: true })).toContainText('Not established');
  // A valid exit may still be known when its launch acknowledgment conflicts.
  response.data.exit = { state: 'exited', event_id: '4'.repeat(40), code: '2' };
  await region(page).getByRole('button', { name: 'Refresh change status' }).click();
  await expect(region(page)).toContainText('Process exited');
  await expect(region(page).getByRole('button', { name: 'Launch', exact: true })).toContainText('unknown');
  expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization status: rollback source and base launch are separate from current-running proof', async ({ page }, info) => {
  const response = structuredClone(native.status), d = response.data;
  d.exit = { state: 'exited', event_id: '5'.repeat(40), code: '2' };
  d.rollback = { state: 'applied', request_event_id: '6'.repeat(40), event_id: '7'.repeat(40), base_commit: d.source.base_commit, launch_state: 'launched', launch_request_event_id: '8'.repeat(40), launch_event_id: '9'.repeat(40), binary_digest: 'sha256:' + 'b'.repeat(64), topology_digest: 'sha256:' + 'c'.repeat(64) };
  const script = await fixture(page, { response }); await open(page);
  await expect(region(page)).toContainText('Base source restored');
  await expect(region(page).getByRole('button', { name: 'Running', exact: true })).toContainText('Not established');
  await region(page).getByText('Stage evidence', { exact: true }).click();
  await expect(region(page)).toContainText('Base process launch');
  await expect(region(page)).toContainText('does not establish that the base process is currently healthy or running');
  await page.setViewportSize({ width: 390, height: 1200 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await region(page).screenshot({ path: info.outputPath('rollback-journey-mobile.png') });
  expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});
