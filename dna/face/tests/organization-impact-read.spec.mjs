// Full-app read contract. Baseline Review/impact responses are retained native
// exports. Discovery, transport and explicitly named count variants are scripted;
// no browser fixture claims to authorize or apply a source change.
import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const evidence = process.env.HALE_ORGANIZATION_IMPACT_EVIDENCE;
const API = '/api/hale/v1/applications';
const web = fileURLToPath(new URL('../web/', import.meta.url));
test.skip(!evidence, 'Supply matching native impact-response.json and reviews-response.json.');
let native, server, origin;
test.beforeAll(async () => {
  if (!evidence) return;
  const impactText = await readFile(path.join(evidence, 'impact-response.json'), 'utf8');
  const reviewsText = await readFile(path.join(evidence, 'reviews-response.json'), 'utf8');
  const impact = JSON.parse(impactText), reviews = JSON.parse(reviewsText);
  expect(impact.source).toEqual(reviews.source);
  const review = reviews.data.items.find(row => row.id === impact.data.review_id);
  expect(review.organization_source).toBe(true);
  expect(review.subject_digest).toBe(impact.data.source.candidate_commit);
  native = { impactText, reviewsText, impact, reviews, review, app: impact.source.record_id };
  const assets = new Set(['index.html', 'app.js', 'styles.css', 'application.js', 'organization-draft.js', 'definition-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js']);
  server = createServer(async (request, response) => {
    const name = new URL(request.url, 'http://localhost').pathname.slice(1) || 'index.html';
    if (!assets.has(name)) { response.writeHead(404); response.end(); return; }
    try { response.setHeader('content-type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html'); response.end(await readFile(path.join(web, name))); }
    catch { response.writeHead(500); response.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = 'http://127.0.0.1:' + server.address().port;
});
test.afterAll(async () => { if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } });
const region = page => page.getByRole('region', { name: 'Current responsibility check', exact: true });
const error = (code, message) => ({ api_version: 'hale.v1', error: { code, message, retryable: false } });
const wrap = data => ({ api_version: 'hale.v1', source: native.impact.source, data });
async function fixture(page, options = {}) {
  const script = { mode: 'native', response: null, reads: [], commands: [], errors: [], ...options };
  page.on('pageerror', value => script.errors.push(value.message));
  await page.route('**/api/hale/v1{,/**}', async route => {
    const request = route.request(), url = new URL(request.url());
    const send = (status, body) => route.fulfill({ status, contentType: 'application/json', body: typeof body === 'string' ? body : JSON.stringify(body) });
    if (url.pathname === API) return send(200, wrap({ items: [{ id: native.app, kind: 'dna', name: 'Native source responsibility evidence', capabilities_url: API + '/' + native.app + '/capabilities' }], page: native.reviews.data.page }));
    if (url.pathname.endsWith('/capabilities')) return send(200, wrap({ application_id: native.app, principal: { mode: 'local', name: 'independent-reviewer' }, read_only: true, reads: { reviews: true, practices: false, organization: false, definitions: false, workflows: false, knowledge: false }, writes: { practice_propose: false, review_verdict: false } }));
    if (url.pathname.endsWith('/dna/reviews')) return send(200, native.reviewsText);
    if (url.pathname.endsWith('/dna/reviews/candidate')) return send(503, error('candidate_unavailable', 'Comparison unavailable in this read contract.'));
    if (url.pathname.endsWith('/dna/organization/source-impact')) {
      script.reads.push({ method: request.method(), query: [...url.searchParams] });
      if (script.mode === 'protected') return send(404, error('source_impact_not_found', 'No visible source impact.'));
      if (script.mode === 'forbidden') return send(403, error('forbidden', 'No source evidence read grant.'));
      if (script.mode === 'unauthenticated') return send(401, error('unauthenticated', 'Sign in required.'));
      if (script.mode === 'outage') return send(503, error('source_impact_unavailable', 'Operational reader unavailable.'));
      if (script.mode === 'race') { script.mode = 'native'; return send(409, error('snapshot_changed', 'Record moved while reading.')); }
      return send(200, script.response || native.impactText);
    }
    if (request.method() !== 'GET') script.commands.push({ method: request.method(), path: url.pathname });
    return send(404, error('not_found', 'Outside this read contract.'));
  });
  return script;
}
async function open(page) {
  const url = origin + '/#/reviews?' + new URLSearchParams({ app: native.app, id: native.review.id });
  if (page.url() === url) await page.reload(); else await page.goto(url);
}
function observedScenario() {
  const response = structuredClone(native.impact);
  response.data.state = 'obligations_observed'; response.data.reason_code = 'impact_obligations_observed';
  response.data.counts = { workflow_tasks: '4', open_workflow_tasks: '1', open_works: '2', outstanding_attempts: '1', legacy_tasks: '5', open_legacy_tasks: '2', handed_tasks: '1', unresolved_intents: '3', schedules: '1', pending_effects: '1', unknown_effects: '0' };
  return response;
}

test('Organization impact read: native captured evidence is useful without source comparison or request identity', async ({ page }, info) => {
  const script = await fixture(page); await open(page);
  await expect(region(page)).toHaveAttribute('data-state', native.impact.data.state);
  await expect(region(page)).toContainText('Application-wide');
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.reads).toEqual([{ method: 'GET', query: [['id', native.review.id], ['snapshot', native.impact.source.record_head]] }]);
  await region(page).screenshot({ path: info.outputPath('native-responsibility-read.png') });
  await region(page).getByRole('button', { name: 'Refresh responsibility check', exact: true }).click();
  await expect(region(page)).toHaveAttribute('data-state', native.impact.data.state);
  expect(script.reads).toHaveLength(2); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
  expect(await page.evaluate(() => localStorage.length)).toBe(0);
});

test('Organization impact read: incomplete or inaccessible evidence clears previously displayed counts', async ({ page }) => {
  const script = await fixture(page, { response: observedScenario() }); await open(page);
  await expect(region(page)).toHaveAttribute('data-state', 'obligations_observed');
  script.response = structuredClone(native.impact); Object.assign(script.response.data, { state: 'unavailable', reason_code: 'impact_unsupported_history', counts: null });
  await region(page).getByRole('button', { name: 'Refresh responsibility check', exact: true }).click();
  await expect(region(page)).toHaveAttribute('data-state', 'unavailable');
  await expect(region(page).getByRole('group', { name: 'Selected responsibility category', exact: true })).toHaveCount(0);
  for (const mode of ['protected', 'forbidden', 'outage']) {
    script.mode = mode; await region(page).getByRole('button', { name: 'Refresh responsibility check', exact: true }).click();
    await expect(region(page)).toHaveAttribute('data-state', 'unavailable');
    await expect(region(page).getByRole('group', { name: 'Selected responsibility category', exact: true })).toHaveCount(0);
  }
  script.mode = 'unauthenticated'; await region(page).getByRole('button', { name: 'Refresh responsibility check', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(region(page)).toHaveCount(0); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization impact read: mismatched candidate or Record evidence cannot attach to the selected Review', async ({ page }) => {
  const script = await fixture(page);
  for (const mutate of [value => { value.data.review_id = 'other-review'; }, value => { value.data.source.candidate_commit = '1'.repeat(40); }, value => { value.data.basis.record_head = '2'.repeat(40); }, value => { value.source.record_revision = '999'; }]) {
    script.response = observedScenario(); mutate(script.response); await open(page);
    await expect(page.getByRole('heading', { name: 'Response could not be verified' })).toBeVisible();
    await expect(region(page)).toHaveCount(0);
  }
  expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});

test('Organization impact read: a capture conflict reloads the Review and current evidence without mutation', async ({ page }) => {
  const script = await fixture(page, { mode: 'race' }); await open(page);
  await expect(region(page)).toHaveAttribute('data-state', native.impact.data.state);
  expect(script.reads).toHaveLength(2); expect(script.commands).toEqual([]); expect(script.errors).toEqual([]);
});
