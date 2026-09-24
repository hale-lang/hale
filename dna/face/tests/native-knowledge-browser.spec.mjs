// Existing browser -> public API -> native Record command -> real Knowledge
// projection in memory. Only lost transport / unavailable read transport are
// injected; command receipts and successful graph responses are always native.
//
// The spine projects an admitted command into memory on its tick (GH #985);
// the harness runs that tick after each admission, before the browser sees
// the reply. A test's own route on the commands path hands a POST on with
// `fallback()`, or ticks itself when it answers the POST with `fulfill`.
import { test as base, expect } from '@playwright/test';
import { startKnowledgeService, knowledgeEnvironmentPresent } from './native-knowledge-harness.mjs';

const test = base.extend({
  grants: [undefined, { option: true }],
  service: async ({ grants }, use, testInfo) => {
    const service = await startKnowledgeService({ grants });
    try { await use(service); }
    finally {
      await service.stop();
      const directory = testInfo.outputPath('native-service');
      await service.exportEvidence(directory);
      await testInfo.attach('native-knowledge-service', { path: directory + '/service.json', contentType: 'application/json' });
      expect(service.processes().filter(process => process.live)).toEqual([]);
    }
  },
  page: async ({ page, service }, use) => {
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.route('**/dna/knowledge/commands', async route => {
      if (route.request().method() !== 'POST') return route.fallback();
      const response = await route.fetch();
      await service.tick();
      await route.fulfill({ response });
    });
    await use(page); expect(errors, 'No unhandled face error').toEqual([]);
  },
});
test.skip(!knowledgeEnvironmentPresent(), 'Supply explicit native Knowledge API and seed binaries (HALE_KNOWLEDGE_API_BIN, HALE_KNOWLEDGE_SEED_BIN) and memory (HALE_DNA_MEMORY_DSN_OWNER).');
test.setTimeout(75_000);

const map = page => page.getByRole('region', { name: 'Knowledge relationship map', exact: true });
const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const recovery = page => page.getByRole('region', { name: 'Knowledge relationship request', exact: true });
const submit = page => editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true });
const metadata = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('face.knowledge-recovery.v1:')).map(([, value]) => JSON.parse(value)));
const responseFor = (page, service, method) => page.waitForResponse(response => response.request().method() === method && new URL(response.url()).pathname === service.apiPath + '/dna/knowledge/commands');
const trackPosts = page => {
  const posts = []; page.on('request', request => { if (request.method() === 'POST' && new URL(request.url()).pathname.endsWith('/dna/knowledge/commands')) posts.push(request.postDataJSON()); }); return posts;
};

async function prepare(page, service, { label = 'clarifies équipe <node>', direction = 'incoming' } = {}) {
  const refs = await service.refs();
  await page.goto(service.url());
  await map(page).getByRole('button', { name: 'Add relationship', exact: true }).click();
  await editor(page).getByLabel('Choose visible knowledge item', { exact: true }).selectOption(refs.predecessor);
  await editor(page).getByLabel('Relationship direction', { exact: true }).selectOption(direction);
  await editor(page).getByLabel('Relationship label', { exact: true }).fill(label);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill('Connect the exact evidence — keep its direction.');
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
  return refs;
}
async function send(page, service) {
  const pending = responseFor(page, service, 'POST'); await submit(page).click();
  const response = await pending;
  return { status: response.status(), body: await response.json(), command: response.request().postDataJSON() };
}
async function observed(page) {
  await expect(recovery(page)).toContainText('The relationship effect is committed in the Record.');
  await expect(recovery(page)).toContainText('Observed in graph');
}

test('native Knowledge: exact directed link is recorded once and observed through a fresh graph read', async ({ page, service }, testInfo) => {
  const posts = trackPosts(page), label = 'clarifies équipe <node>';
  const refs = await prepare(page, service, { label });
  await expect(submit(page)).toBeEnabled();
  const head = await service.recordHead();
  let savedBeforePost;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    savedBeforePost = await metadata(page);
    await route.fallback();
  });
  const graphReads = [];
  page.on('request', request => {
    const url = new URL(request.url());
    if (posts.length && url.pathname.endsWith('/dna/knowledge/nodes') && !url.searchParams.has('id')) graphReads.push(url.searchParams);
  });
  const sent = await send(page, service);
  expect(sent.status).toBe(202); expect(sent.body.data.state).toBe('recorded');
  expect(sent.command.arguments).toEqual({ from_id: refs.predecessor, to_id: refs.knowledge, rel: label, rationale: 'Connect the exact evidence — keep its direction.' });
  expect(sent.command.preconditions.record_head).toBe(head);
  expect(sent.command.preconditions.principal).toEqual(service.principal);
  expect(sent.command.preconditions).not.toHaveProperty('snapshot');
  expect(sent.command.preconditions).not.toHaveProperty('graph_generation');
  expect(savedBeforePost).toHaveLength(1); expect(savedBeforePost[0].request_id).toBe(sent.command.request_id);
  expect(Object.keys(savedBeforePost[0]).sort()).toEqual(['application_id', 'operation', 'principal', 'record_head', 'request_id', 'target_id', 'version']);
  expect(savedBeforePost[0]).toMatchObject({ version: 2, operation: 'dna.knowledge.edge.link' });
  await observed(page);
  expect(graphReads.length).toBeGreaterThan(0); expect(graphReads[0].has('snapshot')).toBe(false); expect(graphReads[0].has('cursor')).toBe(false);
  await expect(map(page).locator('li[data-edge-id="' + sent.body.data.edge_id + '"]')).toContainText(label);
  await expect(map(page).locator('li[data-change="added"]')).toHaveCount(0);
  await expect(map(page).locator('img')).toHaveCount(0);
  expect(posts).toHaveLength(1);
  expect(await service.lookup(sent.command.request_id)).toMatchObject({ status: 200, body: { data: { command_id: sent.body.data.command_id, fingerprint: sent.body.data.fingerprint } } });
  await recovery(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-observed.png') });
});

test('native Knowledge: lost POST reply and API restart recover by GET only after browser reload', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const posts = trackPosts(page); await prepare(page, service);
  let delivered;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); expect(response.status()).toBe(202);
    delivered = { body: await response.json(), command: route.request().postDataJSON() };
    await route.abort('failed');
  });
  await submit(page).click();
  await expect(recovery(page)).toContainText('The relationship outcome could not be confirmed');
  expect(delivered.body.data.state).toBe('recorded'); expect(posts).toHaveLength(1);
  const saved = await metadata(page); expect(saved).toHaveLength(1);
  expect(saved[0].request_id).toBe(delivered.command.request_id);
  // A pre-unlink browser saved v1 link metadata without an operation field.
  // It must recover the original link; migration never creates another POST.
  await page.evaluate(() => {
    const [key, raw] = Object.entries(localStorage).find(([key]) => key.startsWith('face.knowledge-recovery.v1:'));
    const value = JSON.parse(raw); value.version = 1; delete value.operation; localStorage.setItem(key, JSON.stringify(value));
  });
  await page.unroute('**/dna/knowledge/commands'); await service.restart();
  const recovered = responseFor(page, service, 'GET'); await page.reload();
  const response = await recovered; expect(response.status()).toBe(200);
  expect((await response.json()).data.command_id).toBe(delivered.body.data.command_id);
  await observed(page); expect(posts).toHaveLength(1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await recovery(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-recovered-mobile.png') });
});

async function nativeLink(service, requestId, { from, to, label = 'supports exact removal' } = {}) {
  const refs = await service.refs();
  const arguments_ = { from_id: from || refs.knowledge, to_id: to || refs.predecessor, rel: label, rationale: 'Create a real relationship for exact removal acceptance.' };
  const response = await service.post(await service.command(requestId, { arguments: arguments_ }));
  expect(response.status).toBe(202); expect(response.body.data.operation).toBe('dna.knowledge.edge.link');
  return { id: response.body.data.edge_id, from_id: arguments_.from_id, to_id: arguments_.to_id, rel: label };
}
async function prepareRemoval(page, service, edge, { navigate = true } = {}) {
  if (navigate) await page.goto(service.url());
  await map(page).getByRole('button', { name: 'Inspect relationship ' + edge.id, exact: true }).click();
  await map(page).getByRole('button', { name: 'Remove this relationship', exact: true }).click();
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('edge.unlink');
  await expect(editor(page).getByLabel('Relationship to remove', { exact: true })).toHaveValue(edge.id);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill('Remove this exact direction and label — retain other evidence.');
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
}
async function removalObserved(page) {
  await expect(recovery(page)).toContainText('The relationship removal is committed in the Record.');
  await expect(recovery(page)).toContainText('Removal observed');
}

test('native Knowledge unlink: the selected exact relationship is removed while its reverse and other label remain', async ({ page, service }, testInfo) => {
  const posts = trackPosts(page), refs = await service.refs();
  const reverse = await nativeLink(service, 'retained-reverse', { from: refs.knowledge, to: refs.predecessor, label: 'clarifies équipe <node>' });
  const other = await nativeLink(service, 'retained-label', { from: refs.predecessor, to: refs.knowledge, label: 'different evidence' });
  await prepare(page, service);
  const linked = await send(page, service); expect(linked.status).toBe(202); await observed(page);
  const edge = { id: linked.body.data.edge_id, ...linked.command.arguments };
  await recovery(page).getByRole('button', { name: 'Dismiss relationship request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  await prepareRemoval(page, service, edge, { navigate: false });
  await expect(submit(page)).toBeEnabled();
  await expect(map(page).locator('li[data-edge-id="' + edge.id + '"]')).toHaveAttribute('data-change', 'removed');
  await expect(map(page).locator('li[data-edge-id="' + reverse.id + '"]')).toHaveAttribute('data-change', 'current');
  await map(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-removal-preview.png') });
  const removed = await send(page, service);
  expect(removed.status).toBe(202); expect(removed.command.operation).toBe('dna.knowledge.edge.unlink');
  expect(removed.command.arguments).toEqual({ edge_id: edge.id, from_id: edge.from_id, to_id: edge.to_id, rel: edge.rel, rationale: 'Remove this exact direction and label — retain other evidence.' });
  expect(removed.command.arguments).not.toHaveProperty('id');
  expect((await metadata(page))[0]).toMatchObject({ version: 2, operation: 'dna.knowledge.edge.unlink', request_id: removed.command.request_id });
  await removalObserved(page);
  const remaining = await service.relationships();
  expect(remaining.map(row => row.id)).toEqual(expect.arrayContaining([reverse.id, other.id]));
  expect(remaining.map(row => row.id)).not.toContain(edge.id);
  await expect(map(page).locator('li[data-edge-id="' + edge.id + '"]')).toHaveCount(0);
  await expect(map(page).locator('li[data-edge-id="' + reverse.id + '"]')).toContainText(reverse.rel);
  expect(posts.map(command => command.operation)).toEqual(['dna.knowledge.edge.link', 'dna.knowledge.edge.unlink']);
  expect((await service.lookup(removed.command.request_id)).body.data.operation).toBe('dna.knowledge.edge.unlink');
  await recovery(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-removal-observed.png') });
});

test('native Knowledge unlink: lost removal reply recovers the saved operation by GET only after restart', async ({ page, service }, testInfo) => {
  const edge = await nativeLink(service, 'lost-removal-subject');
  await page.setViewportSize({ width: 390, height: 844 });
  const posts = trackPosts(page); await prepareRemoval(page, service, edge);
  await map(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-removal-preview-mobile.png') });
  let delivered;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); expect(response.status()).toBe(202);
    delivered = { body: await response.json(), command: route.request().postDataJSON() }; await route.abort('failed');
  });
  await submit(page).click();
  await expect(recovery(page)).toContainText('The relationship outcome could not be confirmed');
  expect((await metadata(page))[0].operation).toBe('dna.knowledge.edge.unlink');
  await page.unroute('**/dna/knowledge/commands'); await service.restart();
  await page.reload(); await removalObserved(page);
  expect(posts).toHaveLength(1);
  const recovered = await service.lookup(delivered.command.request_id);
  expect(recovered.body.data).toMatchObject({ command_id: delivered.body.data.command_id, operation: 'dna.knowledge.edge.unlink', edge_id: edge.id });
  expect((await service.relationships()).map(row => row.id)).not.toContain(edge.id);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await recovery(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('native-knowledge-removal-recovered-mobile.png') });
});

test('native Knowledge unlink: stale admission leaves the exact relationship intact', async ({ page, service }) => {
  const edge = await nativeLink(service, 'stale-removal-subject');
  const posts = trackPosts(page); await prepareRemoval(page, service, edge);
  await service.mutate('advance'); const head = await service.recordHead();
  const removed = await send(page, service);
  expect(removed.status).toBe(409); expect(removed.body.error.code).toBe('stale_subject');
  await expect(recovery(page)).toContainText('The service refused this relationship request');
  expect(await service.recordHead()).toBe(head);
  expect((await service.relationships()).map(row => row.id)).toContain(edge.id);
  expect((await service.lookup(removed.command.request_id)).status).toBe(404); expect(posts).toHaveLength(1);
});

test('native Knowledge unlink: removal authority is independent from link authority and defaults to denied', async ({ page, service }) => {
  const edge = await nativeLink(service, 'permission-removal-subject');
  const grant = { mode: 'local', name: 'alice', authority: 'knowledge-editor', edge_link: 'direct', recover: true };
  await service.setGrants([grant]); // Omitting edge_unlink does not grant it.
  const posts = trackPosts(page); await prepareRemoval(page, service, edge);
  await expect(submit(page)).toBeDisabled(); expect(posts).toEqual([]);
  expect((await service.capability()).body.data.authorized).toBe(true);
  expect((await service.capability('dna.knowledge.edge.unlink')).body.data.authorized).toBe(false);
  await service.setGrants([{ ...grant, edge_link: 'deny', edge_unlink: 'direct' }]);
  expect((await service.request('/capabilities')).body.data.read_only).toBe(false);
  await prepare(page, service); await expect(submit(page)).toBeDisabled();
  await prepareRemoval(page, service, edge); await expect(submit(page)).toBeEnabled();
  const removed = await send(page, service); expect(removed.status).toBe(202); await removalObserved(page);
  expect(posts).toHaveLength(1); expect(posts[0].operation).toBe('dna.knowledge.edge.unlink');
});

test('native Knowledge unlink: absence requires every native relationship page at one snapshot', async ({ page, service }) => {
  const edges = [];
  for (let i = 0; i < 27; i++) edges.push(await nativeLink(service, 'paged-link-' + i, { label: 'page-' + String(i).padStart(2, '0') }));
  const posts = trackPosts(page); await prepareRemoval(page, service, edges[0]);
  let committed = false, failedContinuation = false;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); committed = response.status() === 202; await service.tick(); await route.fulfill({ response });
  });
  await page.route('**/dna/knowledge/edges?*', route => {
    if (committed && new URL(route.request().url()).searchParams.has('cursor')) {
      failedContinuation = true;
      return route.fulfill({ status: 503, json: { api_version: 'hale.v1', error: { code: 'knowledge_unavailable', message: 'Continuation transport unavailable', retryable: true } } });
    }
    return route.continue();
  });
  const removed = await send(page, service); expect(removed.status).toBe(202);
  await expect(recovery(page)).toContainText('Graph readback is unavailable');
  await expect(recovery(page)).not.toContainText('Removal observed'); expect(failedContinuation).toBe(true);
  expect((await service.relationships()).map(row => row.id)).not.toContain(edges[0].id);
  await page.unroute('**/dna/knowledge/edges?*');
  const readbacks = [];
  page.on('response', async response => {
    if (response.status() === 200 && new URL(response.url()).pathname.endsWith('/dna/knowledge/edges')) readbacks.push((await response.json()).data);
  });
  await recovery(page).getByRole('button', { name: 'Check relationship request', exact: true }).click();
  await removalObserved(page); await expect(recovery(page)).toContainText('all 2 checked relationship pages');
  await expect.poll(() => readbacks.length).toBe(2); expect(readbacks[0].page.has_more).toBe(true); expect(readbacks[1].page.has_more).toBe(false);
  expect(new Set(readbacks.map(value => value.page.snapshot)).size).toBe(1); expect(posts).toHaveLength(1);
});

test('native Knowledge unlink: endpoint restriction between admission and graph read never proves absence', async ({ page, service }) => {
  const edge = await nativeLink(service, 'visibility-removal-subject');
  const posts = trackPosts(page); await prepareRemoval(page, service, edge);
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); expect(response.status()).toBe(202);
    await service.mutate('protect-predecessor'); await route.fulfill({ response });
  });
  const removed = await send(page, service); expect(removed.status).toBe(202);
  await expect(recovery(page)).toContainText('Relationship details are unavailable under current visibility');
  await expect(recovery(page)).not.toContainText('Removal observed'); await expect(map(page)).toHaveCount(0);
  expect((await service.lookup(removed.command.request_id)).body.data).toMatchObject({ state: 'recorded', details_visible: false, edge_id: '' });
  expect(posts).toHaveLength(1);
});

test('native Knowledge: stale Record head is refused without a graph effect or automatic resubmission', async ({ page, service }) => {
  const posts = trackPosts(page); await prepare(page, service);
  await service.mutate('advance'); const advanced = await service.recordHead();
  const sent = await send(page, service);
  expect(sent.status).toBe(409); expect(sent.body.error.code).toBe('stale_subject');
  await expect(recovery(page)).toContainText('The service refused this relationship request');
  await expect(recovery(page)).not.toContainText('effect is committed');
  expect(await service.recordHead()).toBe(advanced); expect(posts).toHaveLength(1);
  expect((await service.lookup(sent.command.request_id)).status).toBe(404);
  await recovery(page).getByRole('button', { name: 'Dismiss relationship request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0); expect(await metadata(page)).toEqual([]);
});

test('native Knowledge: unavailable graph readback preserves admission and later visibility restrictions clear old content', async ({ page, service }) => {
  const posts = trackPosts(page), refs = await prepare(page, service);
  let committed = false;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); committed = response.status() === 202; await service.tick(); await route.fulfill({ response });
  });
  await page.route('**/dna/knowledge/nodes?*', route => committed ? route.fulfill({ status: 503, json: { api_version: 'hale.v1', error: { code: 'knowledge_unavailable', message: 'Read transport unavailable', retryable: true } } }) : route.continue());
  const sent = await send(page, service); expect(sent.status).toBe(202);
  await expect(recovery(page)).toContainText('The relationship effect is committed in the Record.');
  await expect(recovery(page)).toContainText('Graph readback is unavailable');
  await expect(map(page)).toHaveCount(0); expect(posts).toHaveLength(1);
  await page.unroute('**/dna/knowledge/nodes?*');
  await recovery(page).getByRole('button', { name: 'Check relationship request', exact: true }).click(); await observed(page);
  await service.mutate('protect');
  await recovery(page).getByRole('button', { name: 'Check relationship request', exact: true }).click();
  await expect(recovery(page)).toContainText('Relationship details are unavailable under current visibility');
  await expect(map(page)).toHaveCount(0);
  await expect(page.locator('#content')).not.toContainText(refs.name);
  await expect(page.locator('#content')).not.toContainText(refs.text);
  expect(posts).toHaveLength(1);
});

test.describe('Review-required Knowledge policy', () => {
  test.use({ grants: [{ mode: 'local', name: 'alice', authority: 'knowledge-editor', edge_link: 'review', recover: true }] });
  test('native Knowledge: read access and Review-required authority never enable a direct fallback', async ({ page, service }) => {
    const posts = trackPosts(page); await prepare(page, service);
    await expect(submit(page)).toBeEnabled();
    await expect(editor(page)).toContainText('required native Review');
    const before = await service.relationships(), sent = await send(page, service);
    expect(sent.status).toBe(202); expect(sent.body.data).toMatchObject({ state: 'recorded', relationship: { proposal_state: 'pending', candidate_digest: '', review_id: '', effect_state: 'unknown' } });
    await expect(recovery(page).getByRole('button', { name: 'Relationship effect', exact: true })).toContainText('unknown');
    await expect(recovery(page)).not.toContainText('The relationship effect is committed in the Record.');
    expect(await service.relationships()).toEqual(before); expect(posts).toHaveLength(1);
    const recovered = await service.lookup(sent.command.request_id);
    expect(recovered.body.data.relationship.proposal_state).toBe('pending');
    // This standalone composition has no Body/relay. Admission is durable;
    // candidate/Review creation and effect are deliberately not claimed here.
  });
});
