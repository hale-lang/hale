// Connected Knowledge preparation over native reads. One explicitly identified
// transport overlay adds parallel/self edges for browser identity conformance;
// none of these cases claims graph publication or native write durability.
import { readFile } from 'node:fs/promises';
import { test, expect, errorBody } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test.skip(!process.env.HALE_FACE_KNOWLEDGE_BIN, 'Knowledge interaction requires the explicitly supplied native Knowledge fixture.');
test.use({ knowledge: true });

const map = page => page.getByRole('region', { name: 'Knowledge relationship map', exact: true });
const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const review = page => editor(page).getByRole('region', { name: 'Knowledge change review', exact: true });
const selectedEdge = page => map(page).getByRole('region', { name: 'Selected relationship', exact: true });
const graphDraft = page => map(page).getByRole('region', { name: 'Knowledge graph draft', exact: true });
const comparison = page => map(page).getByRole('group', { name: 'Knowledge comparison', exact: true });
const addedEdge = page => map(page).locator('[data-change="added"][data-from-id][data-to-id]');
const edgeButton = (page, id) => map(page).getByRole('button', { name: 'Inspect relationship ' + id, exact: true });
const edgeRow = (page, id) => map(page).locator('li[data-edge-id]').filter({
  has: page.getByRole('button', { name: 'Inspect relationship ' + id, exact: true }),
});
const UNCHECKED = 'Draft only · source check required.';
const CHECKED = 'Source checked · not submitted.';
const REASON = 'Make the relationship explicit while retaining its exact direction.';
const responseFor = (page, kind, predicate = () => true) => page.waitForResponse(response => {
  const url = new URL(response.url());
  return response.status() === 200 && url.pathname.endsWith('/dna/knowledge/' + kind) && predicate(url.searchParams);
});

async function open(page, service) {
  const nodes = responseFor(page, 'nodes', query => !query.has('id'));
  const edges = responseFor(page, 'edges');
  await page.goto(service.url('knowledge', { id: service.knowledge }));
  const [collection, relationships] = await Promise.all([nodes, edges].map(async response => (await (await response).json()).data));
  await expect(map(page)).toBeVisible();
  return { collection, relationships };
}

function mutations(page) {
  const requests = [];
  page.on('request', request => { if (isWrite(request)) requests.push(request.method() + ' ' + request.url()); });
  return requests;
}

async function prepareLink(page, service, label = 'clarifies', direction = 'incoming') {
  await map(page).getByRole('button', { name: 'Add relationship', exact: true }).click();
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('edge.link');
  await editor(page).getByLabel('Choose visible knowledge item', { exact: true }).selectOption(service.predecessor);
  await editor(page).getByLabel('Relationship direction', { exact: true }).selectOption(direction);
  await editor(page).getByLabel('Relationship label', { exact: true }).fill(label);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(REASON);
}

async function check(page) {
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
  await expect(graphDraft(page)).toContainText(CHECKED);
}

async function exported(page, testInfo, name) {
  const pending = page.waitForEvent('download');
  await review(page).getByRole('button', { name: 'Download knowledge draft', exact: true }).click();
  const destination = testInfo.outputPath(name);
  await (await pending).saveAs(destination);
  return JSON.parse(await readFile(destination, 'utf8'));
}

async function nativeMapEdges(page) {
  return map(page).locator('li[data-edge-id]').evaluateAll(rows => rows.map(row => ({
    id: row.dataset.edgeId, from_id: row.dataset.fromId, to_id: row.dataset.toId,
  })).sort((a, b) => a.id.localeCompare(b.id)));
}

test('knowledge interaction: exact native edge selection and removal stay distinct beside browser-only parallel and self-loop overlays', async ({ page, service }, testInfo) => {
  const writes = mutations(page);
  const project = await service.projectState();
  let nativeEdges, parallel, self;
  // These two additional edges are browser conformance data, not records
  // persisted by the native fixture. Existing edges retain their native IDs.
  await page.route('**/api/hale/v1/**/dna/knowledge/edges?*', async route => {
    const response = await route.fetch(), body = await response.json();
    nativeEdges = structuredClone(body.data.items);
    const original = nativeEdges.find(edge => edge.from_id === service.knowledge && edge.to_id === service.predecessor);
    parallel = { ...original, id: 'sha256:' + 'e'.repeat(64), rel: 'parallel browser-only relation' };
    self = { ...original, id: 'sha256:' + 'f'.repeat(64), to_id: service.knowledge, rel: 'self browser-only relation' };
    body.data.items.push(parallel, self);
    body.data.page.returned = body.data.items.length;
    await route.fulfill({ response, json: body });
  });
  const { relationships } = await open(page, service);
  const original = nativeEdges.find(edge => edge.from_id === service.knowledge && edge.to_id === service.predecessor);
  expect(await nativeMapEdges(page)).toEqual(relationships.items.map(({ id, from_id, to_id }) => ({ id, from_id, to_id })).sort((a, b) => a.id.localeCompare(b.id)));
  await map(page).getByRole('group', { name: 'Self references', exact: true }).getByRole('button', { name: 'Inspect relationship ' + self.id, exact: true }).click();
  await expect(selectedEdge(page)).toContainText(self.id);
  await expect(edgeButton(page, self.id)).toHaveAttribute('aria-pressed', 'true');
  await edgeButton(page, parallel.id).click();
  await expect(selectedEdge(page)).toContainText(parallel.id);
  await expect(edgeButton(page, self.id)).toHaveAttribute('aria-pressed', 'false');
  await edgeButton(page, original.id).click();
  await expect(selectedEdge(page)).toContainText(original.id);
  await expect(selectedEdge(page)).toContainText(original.rel);
  await selectedEdge(page).getByRole('button', { name: 'Remove this relationship', exact: true }).click();
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('edge.unlink');
  await expect(editor(page).getByLabel('Relationship to remove', { exact: true })).toHaveValue(original.id);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(REASON);
  await expect(edgeRow(page, original.id)).toHaveAttribute('data-change', 'removed');
  await expect(edgeRow(page, parallel.id)).toHaveAttribute('data-change', 'current');
  await expect(edgeRow(page, self.id)).toHaveAttribute('data-change', 'current');
  await check(page);
  const draft = await exported(page, testInfo, 'selected-native-edge-removal.json');
  expect(draft.operation).toBe('edge.unlink');
  expect(draft.arguments).toEqual({ ...original, rationale: REASON });
  expect(draft.source_checked).toBe(true);
  expect(draft.submitted).toBe(false);
  expect(writes).toEqual([]);
  expect(await service.projectState()).toEqual(project);
});

test('knowledge interaction: named visible-item selection preserves incoming direction and creates no relationship identity', async ({ page, service }, testInfo) => {
  const writes = mutations(page);
  const { collection, relationships } = await open(page, service);
  await prepareLink(page, service);
  const picker = editor(page).getByLabel('Choose visible knowledge item', { exact: true });
  const predecessor = collection.items.find(item => item.id === service.predecessor);
  await expect(picker.locator('option:checked')).toContainText(predecessor.name);
  await expect(editor(page).getByLabel('Other knowledge identity', { exact: true })).toHaveValue(service.predecessor);
  expect(await map(page).evaluate(element => Boolean(element.compareDocumentPosition(document.querySelector('.knowledge-draft')) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
  await expect(addedEdge(page)).toHaveCount(1);
  await expect(addedEdge(page)).toHaveAttribute('data-from-id', service.predecessor);
  await expect(addedEdge(page)).toHaveAttribute('data-to-id', service.knowledge);
  expect(await addedEdge(page).getAttribute('data-edge-id')).toBeNull();
  await expect(graphDraft(page)).toContainText(UNCHECKED);
  expect(await nativeMapEdges(page)).toEqual(relationships.items.map(({ id, from_id, to_id }) => ({ id, from_id, to_id })).sort((a, b) => a.id.localeCompare(b.id)));
  await check(page);
  const draft = await exported(page, testInfo, 'named-incoming-relationship.json');
  expect(draft.operation).toBe('edge.link');
  expect(draft.arguments).toEqual({ from_id: service.predecessor, to_id: service.knowledge, rel: 'clarifies', rationale: REASON });
  expect(draft.arguments).not.toHaveProperty('id');
  expect(draft.submitted).toBe(false);
  expect(writes).toEqual([]);
  await editor(page).getByRole('button', { name: 'Back to relationship map', exact: true }).click();
  await expect(map(page)).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('knowledge-incoming-draft-map.png') });
});

test('knowledge interaction: current/compare planes retain draft fields and native edges, and discard restores focused context without requests', async ({ page, service }) => {
  const project = await service.projectState();
  const { relationships } = await open(page, service);
  const requests = [];
  page.on('request', request => requests.push(request.method() + ' ' + request.url()));
  const url = page.url();
  await prepareLink(page, service, 'keeps context');
  await expect(comparison(page).getByRole('button', { name: 'Compare draft', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await comparison(page).getByRole('button', { name: 'Current graph', exact: true }).click();
  await expect(comparison(page).getByRole('button', { name: 'Current graph', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(addedEdge(page)).toBeHidden();
  await expect(editor(page).getByLabel('Other knowledge identity', { exact: true })).toHaveValue(service.predecessor);
  await expect(editor(page).getByLabel('Relationship direction', { exact: true })).toHaveValue('incoming');
  await expect(editor(page).getByLabel('Relationship label', { exact: true })).toHaveValue('keeps context');
  await expect(editor(page).getByLabel('Reason for knowledge change', { exact: true })).toHaveValue(REASON);
  await comparison(page).getByRole('button', { name: 'Compare draft', exact: true }).click();
  await expect(addedEdge(page)).toBeVisible();
  await editor(page).getByRole('button', { name: 'Discard knowledge draft', exact: true }).click();
  await expect(graphDraft(page)).toHaveCount(0);
  await expect(comparison(page)).toHaveCount(0);
  await expect(addedEdge(page)).toHaveCount(0);
  await expect(editor(page).getByRole('button', { name: 'Prepare knowledge change', exact: true })).toBeVisible();
  expect(await nativeMapEdges(page)).toEqual(relationships.items.map(({ id, from_id, to_id }) => ({ id, from_id, to_id })).sort((a, b) => a.id.localeCompare(b.id)));
  await expect(page).toHaveURL(url);
  await expect(map(page)).toContainText(service.name);
  expect(requests).toEqual([]);
  expect(await service.projectState()).toEqual(project);
});

test('knowledge interaction: editing clears source-checked graph status and a real Record race removes the candidate', async ({ page, service }) => {
  const writes = mutations(page);
  await open(page, service);
  await prepareLink(page, service);
  await check(page);
  await editor(page).getByLabel('Relationship label', { exact: true }).fill('revised after checking');
  await expect(graphDraft(page)).toContainText(UNCHECKED);
  await expect(graphDraft(page)).not.toContainText(CHECKED);
  await expect(review(page)).toBeEmpty();
  await expect(addedEdge(page)).toContainText('revised after checking');
  await service.mutate('advance');
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('button', { name: 'Prepare knowledge change', exact: true })).toBeVisible();
  await expect(graphDraft(page)).toHaveCount(0);
  await expect(addedEdge(page)).toHaveCount(0);
  await expect(editor(page).getByLabel('Relationship label', { exact: true })).toHaveCount(0);
  await expect(page.locator('#content')).not.toContainText('revised after checking');
  expect(writes).toEqual([]);
});

test.describe('Manual off-page endpoint', () => {
  test.use({ recordCount: 30 });
  test('knowledge interaction: an unverified manual endpoint resolves only through native review and access loss clears its preview', async ({ page, service }) => {
    const writes = mutations(page);
    const { collection } = await open(page, service);
    const visible = new Set(collection.items.map(item => item.id));
    const other = service.neighbors.find(item => !visible.has(item.id));
    expect(other).toBeTruthy();
    const exactReads = [];
    page.on('request', request => {
      const url = new URL(request.url());
      if (url.pathname.endsWith('/dna/knowledge/nodes') && url.searchParams.get('id') === other.id) exactReads.push(url);
    });
    await map(page).getByRole('button', { name: 'Add relationship', exact: true }).click();
    await editor(page).getByLabel('Other knowledge identity', { exact: true }).fill(other.id);
    await editor(page).getByLabel('Relationship label', { exact: true }).fill('off-page evidence');
    await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(REASON);
    await expect(editor(page).getByLabel('Choose visible knowledge item', { exact: true })).toHaveValue('');
    await expect(graphDraft(page)).toContainText('Unverified item');
    expect(exactReads).toHaveLength(0);
    const related = responseFor(page, 'nodes', query => query.get('id') === other.id);
    await check(page);
    const relatedData = (await (await related).json()).data;
    expect(relatedData.items).toHaveLength(1);
    expect(relatedData.items[0].id).toBe(other.id);
    expect(exactReads).toHaveLength(1);
    expect(exactReads[0].searchParams.get('snapshot')).toBe(collection.page.snapshot);
    await expect(graphDraft(page)).toContainText(relatedData.items[0].name);
    await expect(graphDraft(page)).not.toContainText('Unverified item');
    await page.route('**/api/hale/v1/applications/*/capabilities', route => route.fulfill({
      status: 401, json: errorBody('unauthenticated', 'Sign in again'),
    }));
    await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Sign in to read this application', exact: true })).toBeVisible();
    await expect(map(page)).toHaveCount(0);
    await expect(editor(page)).toHaveCount(0);
    await expect(page.locator('#content')).not.toContainText('off-page evidence');
    expect(writes).toEqual([]);
  });
});

test('knowledge interaction: contextual graph actions cannot overwrite an active revision draft', async ({ page, service }) => {
  const writes = mutations(page);
  const { relationships } = await open(page, service);
  await map(page).getByRole('button', { name: 'Revise focused item', exact: true }).click();
  const text = 'This unsent revision must survive other contextual actions — ✓';
  await editor(page).getByLabel('Knowledge text', { exact: true }).fill(text);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(REASON);
  await map(page).getByRole('button', { name: 'Add relationship', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Finish or discard the current knowledge draft before starting another change.');
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('node.revise');
  await expect(editor(page).getByLabel('Knowledge text', { exact: true })).toHaveValue(text);
  await edgeButton(page, relationships.items[0].id).click();
  await selectedEdge(page).getByRole('button', { name: 'Remove this relationship', exact: true }).click();
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('node.revise');
  await expect(editor(page).getByLabel('Knowledge text', { exact: true })).toHaveValue(text);
  await expect(editor(page).getByLabel('Reason for knowledge change', { exact: true })).toHaveValue(REASON);
  await expect(map(page).locator('li[data-edge-id][data-change="removed"]')).toHaveCount(0);
  expect(writes).toEqual([]);
});

test('knowledge interaction: a delayed review preserves the operator’s new graph focus and open relationship evidence', async ({ page, service }) => {
  const { collection, relationships } = await open(page, service);
  const nativeEdge = relationships.items.find(edge => edge.from_id === service.knowledge && edge.to_id === service.predecessor);
  await edgeButton(page, nativeEdge.id).click();
  await selectedEdge(page).getByText('Exact relationship identity', { exact: true }).click();
  await expect(selectedEdge(page).locator('details')).toHaveJSProperty('open', true);
  await prepareLink(page, service);
  await expect(selectedEdge(page).locator('details')).toHaveJSProperty('open', true);

  let release, entered;
  const held = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  const endpoint = '**/api/hale/v1/applications/*/capabilities';
  await page.route(endpoint, async route => {
    const response = await route.fetch();
    entered();
    await held;
    await route.fulfill({ response });
  });
  try {
    await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
    await started;
    const predecessor = collection.items.find(item => item.id === service.predecessor);
    const connection = map(page).getByRole('group', { name: 'Incoming', exact: true }).getByRole('link', {
      name: `Open connected knowledge item ${service.predecessor} · ${predecessor.name}`, exact: true,
    });
    await connection.focus();
    await expect(connection).toBeFocused();
    const focusedURL = page.url();
    release();
    await expect(graphDraft(page)).toContainText(CHECKED);
    await expect(connection).toBeFocused();
    await expect(selectedEdge(page).locator('details')).toHaveJSProperty('open', true);
    await expect(page).toHaveURL(focusedURL);
  } finally {
    release();
    await page.unroute(endpoint);
  }
});

test('knowledge interaction: narrow reduced-motion keyboard comparison preserves literal relationship text', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const writes = mutations(page);
  await open(page, service);
  await map(page).getByRole('button', { name: 'Add relationship', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(editor(page).getByLabel('Change kind', { exact: true })).toHaveValue('edge.link');
  await editor(page).getByLabel('Choose visible knowledge item', { exact: true }).selectOption(service.predecessor);
  const label = 'clarifies — 第二版 <img src=x onerror="window.__knowledgeInteractionInjected=true">';
  await editor(page).getByLabel('Relationship label', { exact: true }).fill(label);
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(REASON);
  await expect(addedEdge(page)).toContainText(label);
  await editor(page).getByRole('button', { name: 'Back to relationship map', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(map(page)).toBeFocused();
  await expect(editor(page).getByLabel('Relationship label', { exact: true })).toHaveValue(label);
  await comparison(page).getByRole('button', { name: 'Current graph', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect(comparison(page).getByRole('button', { name: 'Current graph', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(comparison(page).getByRole('button', { name: 'Current graph', exact: true })).toBeFocused();
  await expect(addedEdge(page)).toBeHidden();
  await comparison(page).getByRole('button', { name: 'Compare draft', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(comparison(page).getByRole('button', { name: 'Compare draft', exact: true })).toBeFocused();
  await expect(addedEdge(page)).toContainText(label);
  await expect(map(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__knowledgeInteractionInjected)).toBeUndefined();
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
  expect(writes).toEqual([]);
  await map(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('knowledge-draft-map-mobile.png') });
});
