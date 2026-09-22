// UI CONTRACT ONLY: explicitly scripted read envelopes exercise Practice route
// joins, historical action gating and graph navigation. No native outcomes,
// authority, publication, persistence or recovery are established by this file.
import { test as base, expect } from '@playwright/test';
import { cockpitHost } from './http-fixture.mjs';

const API = '/api/hale/v1/applications', APP = 'a'.repeat(40), HEAD = 'b'.repeat(40);
const ITEM = 'sha256:' + 'c'.repeat(64), OTHER = 'sha256:' + 'd'.repeat(64), EDGE = 'sha256:' + 'e'.repeat(64);
const SOURCE = { record_id: APP, record_head: HEAD, record_revision: '9' };
const envelope = (data, source = SOURCE) => ({ api_version: 'hale.v1', source, data });
const pageOf = (items, source = SOURCE) => ({ items, page: { limit: 25, offset: 0, total: items.length, next_offset: -1, snapshot: source.record_head } });
const principal = { mode: 'local', name: 'alice' };
const row = { id: ITEM, digest: ITEM, name: 'Read-contract Practice', kind: 'practice', text: 'Retained exact text — équipe <practice>', text_status: 'available', author: 'org', target: 'org/canonical', binding_class: 'goal', provenance: 'proposed', supersedes: '', request_id: 'ui-contract/request', review_id: '', review_state: 'settled', review_settled: 'approved', review_outcome: 'approve', state: 'ratified', requester: 'alice', rationale: 'Scripted route contract only', text_available: true, ratified: true, retired: false, declined: false };
const basis = { record_id: APP, projection_record_head: HEAD, projection_record_revision: '9', projection_watermark: '9', store_generation: '1', routing_version: '0', visibility_record_head: HEAD, visibility_record_revision: '9', visibility_ledger_head: null, visibility_ledger_revision: null, reader_scope: 'contract-public', target: '', coverage: 'native_ideas_edges_bindings' };
const graphPage = items => ({ items, page: { limit: 25, returned: items.length, has_more: false, next_cursor: null, snapshot: 'contract-graph-snapshot' }, basis, coverage: { bindings: 'complete', runs: 'unavailable', definitions: 'unavailable', practices: 'unavailable' } });
const editor = page => page.getByRole('region', { name: 'Practice change editor', exact: true });
const map = page => page.getByRole('region', { name: 'Knowledge relationship map', exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => { const host = await cockpitHost(); try { await use(host); } finally { await host.close(); } }
});

async function fixture(page, host, options = {}) {
  const state = { mismatch: '', historical: '', requests: [], ...options };
  const practice = () => ({ ...row, ...(state.historical === 'retired' ? { retired: true, state: 'retired' } : state.historical === 'pending' ? { ratified: false, state: 'pending', review_state: 'pending', review_outcome: '', review_settled: '' } : {}) });
  const node = id => ({ id, kind: id === ITEM ? 'practice' : 'idea', text: id === ITEM ? row.text : 'Related read-contract item', author: 'org', projection_state: state.historical === 'retired' ? 'retired' : state.historical === 'pending' ? 'proposed' : 'ratified', revision: '4', name: id === ITEM ? row.name : 'Related item', supersedes: '', ratified_seq: '3', accepted: !state.historical, source_provenance: null });
  await page.route('**/api/hale/v1{,/**}', async route => {
    const req = route.request(), url = new URL(req.url()); state.requests.push({ method: req.method(), path: url.pathname, query: Object.fromEntries(url.searchParams) });
    const fulfill = data => route.fulfill({ status: 200, contentType: 'application/json; charset=utf-8', body: JSON.stringify(data) });
    if (req.method() !== 'GET') return route.fulfill({ status: 405, contentType: 'application/json', body: JSON.stringify({ api_version: 'hale.v1', error: { code: 'ui_contract_read_only', message: 'No command endpoint exists in this fixture.', retryable: false } }) });
    if (url.pathname === API) return fulfill(envelope(pageOf([{ id: APP, kind: 'dna', name: 'Scripted Practice route contract' }])));
    if (url.pathname === API + '/' + APP + '/capabilities') return fulfill(envelope({ application_id: APP, principal, read_only: true, reads: { practices: true, knowledge: true, organization: false, reviews: false, definitions: false, workflows: false }, writes: { practice_propose: false, review_verdict: false } }));
    if (url.pathname.endsWith('/dna/practices')) {
      let p = practice(), source = SOURCE;
      if (state.mismatch === 'snapshot') source = { ...SOURCE, record_head: 'f'.repeat(40), record_revision: '10' };
      if (state.mismatch === 'item') p = { ...p, id: OTHER, digest: OTHER };
      if (state.mismatch === 'text') p = { ...p, text: p.text + ' changed' };
      return fulfill(envelope(pageOf([p], source), source));
    }
    if (url.pathname.endsWith('/dna/knowledge/nodes')) return fulfill(envelope(graphPage(url.searchParams.has('id') ? [node(url.searchParams.get('id'))] : [node(ITEM), node(OTHER)])));
    if (url.pathname.endsWith('/dna/knowledge/edges')) return fulfill(envelope(graphPage([{ id: EDGE, from_id: ITEM, to_id: OTHER, rel: 'informs — exact stored tuple' }])));
    if (url.pathname.endsWith('/dna/knowledge/bindings')) return fulfill(envelope(graphPage([{ id: 'sha256:' + '1'.repeat(64), idea_id: ITEM, target: 'org/canonical', author: 'org', class: 'goal', applicability: 'unfiltered' }])));
    return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ api_version: 'hale.v1', error: { code: 'outside_contract', message: 'Outside the scripted read contract.', retryable: false } }) });
  });
  state.open = async (action = 'applicability') => {
    const url = host.origin + '/#/knowledge?' + new URLSearchParams({ app: APP, id: ITEM, practice_action: action });
    if (page.url() === url) await page.reload(); else await page.goto(url);
  };
  return state;
}

test('Practice routes: exact snapshot, item and canonical text mismatches never open an editor', async ({ page, host }) => {
  const state = await fixture(page, host);
  for (const mismatch of ['snapshot', 'item', 'text']) {
    state.mismatch = mismatch; await state.open('revise');
    await expect(page.locator('#content')).toContainText(mismatch === 'text' ? 'does not match the exact Practice document' : 'do not share one exact source snapshot');
    await expect(editor(page)).toHaveCount(0); await expect(map(page)).toHaveCount(0);
  }
  expect(state.requests.filter(request => request.path.endsWith('/dna/practices')).every(request => request.query.snapshot === HEAD && request.query.id === ITEM)).toBe(true);
  expect(state.requests.every(request => request.method === 'GET')).toBe(true);
});

test('Practice routes: retired and pending Practices retain readable history without enabled edit entry points', async ({ page, host }) => {
  const state = await fixture(page, host);
  for (const historical of ['retired', 'pending']) {
    state.historical = historical; await state.open();
    await expect(map(page)).toBeVisible(); await expect(editor(page)).toHaveCount(0);
    await expect(page.locator('#content')).toContainText('not currently in force');
    await expect(map(page).getByRole('button', { name: 'Continue practice draft', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Remove this binding', exact: true })).toBeDisabled();
    await page.getByRole('link', { name: 'Back to practice', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Practice detail', exact: true })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Inspect applicability', exact: true })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Edit practice & scope', exact: true })).toHaveCount(0);
    await expect(page.getByRole('link', { name: 'Retire practice', exact: true })).toHaveCount(0);
  }
  expect(state.requests.every(request => request.method === 'GET')).toBe(true);
});

test('Practice routes: stored relationship selection and both return paths preserve the Practice boundary', async ({ page, host }) => {
  const state = await fixture(page, host); await state.open();
  await expect(editor(page)).toBeVisible();
  await map(page).getByRole('button', { name: 'Inspect relationship ' + EDGE, exact: true }).click();
  await expect(map(page).getByRole('region', { name: 'Selected relationship', exact: true })).toContainText('informs — exact stored tuple');
  await expect(map(page).getByRole('button', { name: 'Remove this relationship', exact: true })).toHaveCount(0);
  await page.getByRole('link', { name: 'Back to practice', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Practice detail', exact: true })).toBeVisible();
  await expect(page).toHaveURL(new RegExp('#/practices\\?app=' + APP + '&id=' + encodeURIComponent(ITEM) + '(?:&|$)'));
  await page.getByRole('link', { name: 'Manage applicability', exact: true }).click();
  await expect(editor(page)).toBeVisible();
  await page.locator('#record-detail-panel').getByRole('link', { name: 'Back to Practices', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Practice register', exact: true })).toBeVisible();
  const query = new URLSearchParams(page.url().split('?')[1]); expect(query.has('practice_action')).toBe(false); expect(query.has('id')).toBe(false);
  expect(state.requests.every(request => request.method === 'GET')).toBe(true);
});
