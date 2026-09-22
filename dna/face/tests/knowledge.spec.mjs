import { test, expect, errorBody } from './harness.mjs';

test.skip(!process.env.HALE_COCKPIT_KNOWLEDGE_BIN, 'Knowledge browser integration requires an explicitly supplied native provider fixture (HALE_COCKPIT_KNOWLEDGE_BIN).');
test.use({ knowledge: true });
const detail = page => page.getByRole('region', { name: 'Knowledge item', exact: true });
const register = page => page.getByRole('region', { name: 'Knowledge register', exact: true });
const relationshipMap = page => page.getByRole('region', { name: 'Knowledge relationship map', exact: true });
const connectedName = (id, visibleName = 'Knowledge item') => `Open connected knowledge item ${id} · ${visibleName}`;
const endpointOf = (edge, id) => edge.from_id === id ? edge.to_id : edge.from_id;
const routeParams = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
const responseFor = (page, kind, predicate = () => true, status = 200) => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith(`/dna/knowledge/${kind}`) && response.status() === status && predicate(url.searchParams);
});
const bodyOf = async response => (await (await response).json()).data;

// Expectations come from the real native-service response for this exact page.
// No successful graph payload is supplied by the browser test.
async function expectMapPage(page, id, edges, collection) {
  const names = new Map(collection.map(item => [item.id, item.name || 'Unnamed knowledge item']));
  const labelFor = endpoint => connectedName(endpoint, names.get(endpoint) || 'Knowledge item');
  const map = relationshipMap(page);
  await expect(map).toBeVisible();
  await expect(map).toHaveAttribute('id', 'relationship-map-panel');
  await expect(map).toHaveAttribute('tabindex', '-1');
  await expect(map.getByRole('heading', { name: 'Relationship map', exact: true })).toBeVisible();
  await expect(map).toContainText('Only relationships on this page are shown.');
  const directions = {
    Incoming: edges.filter(edge => edge.to_id === id && edge.from_id !== id),
    Outgoing: edges.filter(edge => edge.from_id === id && edge.to_id !== id),
    'Self references': edges.filter(edge => edge.from_id === id && edge.to_id === id),
  };
  for (const [direction, items] of Object.entries(directions)) {
    const group = map.getByRole('group', { name: direction, exact: true });
    if (direction === 'Self references' && !items.length) {
      await expect(group).toHaveCount(0);
      continue;
    }
    await expect(group.getByRole('heading', { name: direction, exact: true })).toBeVisible();
    const labels = group.getByRole('listitem').filter({ hasNot: page.getByRole('link') });
    await expect(labels).toHaveCount(items.length);
    expect((await labels.allTextContents()).sort()).toEqual(items.map(edge => edge.rel).sort());
    if (direction === 'Self references') continue;
    const endpoints = [...new Set(items.map(edge => endpointOf(edge, id)))];
    await expect(group.getByRole('link')).toHaveCount(endpoints.length);
    for (const endpoint of endpoints) {
      const link = page.getByRole('link', { name: labelFor(endpoint), exact: true });
      const destination = group.getByRole('link', { name: labelFor(endpoint), exact: true });
      await expect(destination).toContainText(names.get(endpoint) || 'Knowledge item');
      const href = new URL(await destination.getAttribute('href'), page.url());
      expect(new URLSearchParams(href.hash.split('?')[1]).get('id')).toBe(endpoint);
      const card = group.getByRole('listitem').filter({ has: link });
      await expect(card).toHaveCount(1);
      expect((await card.getByRole('listitem').allTextContents()).sort()).toEqual(items.filter(edge => endpointOf(edge, id) === endpoint).map(edge => edge.rel).sort());
    }
  }
  const labels = await map.getByRole('link', { name: /^Open connected knowledge item / }).evaluateAll(links => links.map(link => link.getAttribute('aria-label')));
  const expectedLinks = [directions.Incoming, directions.Outgoing].flatMap(items => [...new Set(items.map(edge => labelFor(endpointOf(edge, id))))]);
  expect(labels.sort()).toEqual(expectedLinks.sort());
  await expect(map.locator('img')).toHaveCount(0);
}

test('Knowledge reads real receipt projection, directed stored relationships and locus bindings', async ({ page, service }, testInfo) => {
  const refs = await service.refs();
  const mutations = [];
  const graphReads = [];
  page.on('request', request => {
    if (request.method() !== 'GET') mutations.push(request.url());
    const pathname = new URL(request.url()).pathname;
    if (/\/dna\/knowledge\/(nodes|edges|bindings|dependents)$/.test(pathname)) graphReads.push(pathname.split('/').pop());
  });
  const nodes = responseFor(page, 'nodes', query => query.get('id') === service.knowledge);
  const collection = responseFor(page, 'nodes', query => !query.has('id'));
  const edges = responseFor(page, 'edges');
  const bindings = responseFor(page, 'bindings');
  await page.goto(service.url('knowledge', { id: service.knowledge }));
  const [nodeData, edgeData, bindingData, collectionData] = await Promise.all([nodes, edges, bindings, collection].map(bodyOf));
  await expect(detail(page).getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await expect(page.locator('#nav-knowledge')).not.toHaveAttribute('aria-disabled');
  expect(nodeData.items).toHaveLength(1);
  expect(nodeData.items[0]).toMatchObject({
    id: service.knowledge, name: service.name, text: service.text, kind: 'goal',
    author: 'org', projection_state: 'ratified', accepted: true,
    supersedes: service.predecessor, source_provenance: null,
  });
  expect(nodeData.items[0].revision).toMatch(/^-?(0|[1-9][0-9]*)$/);
  expect(nodeData.items[0].ratified_seq).toBe(nodeData.items[0].revision);
  for (const data of [edgeData, bindingData]) {
    expect(data.basis).toEqual(nodeData.basis);
    expect(data.page.snapshot).toBe(nodeData.page.snapshot);
    expect(data.page).not.toHaveProperty('total');
    expect(data.page).not.toHaveProperty('offset');
  }
  expect(edgeData.items).toEqual(expect.arrayContaining([
    expect.objectContaining({ from_id: service.knowledge, to_id: service.predecessor, rel: 'supports <stored relationship>' }),
    expect.objectContaining({ from_id: service.predecessor, to_id: service.knowledge, rel: 'informs' }),
  ]));
  expect(edgeData.items.some(edge => edge.from_id === service.hidden || edge.to_id === service.hidden)).toBe(false);
  expect(bindingData.items).toEqual([expect.objectContaining({ idea_id: service.knowledge, target: service.target, author: 'org', class: 'goal', applicability: 'unfiltered' })]);
  await expectMapPage(page, service.knowledge, edgeData.items, collectionData.items);
  expect(graphReads.sort()).toEqual(['bindings', 'edges', 'nodes', 'nodes']);
  expect(await relationshipMap(page).evaluate(map => Boolean(map.compareDocumentPosition(document.querySelector('#record-list-panel')) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
  for (const protectedValue of [service.hidden, 'Protected knowledge', 'protected-endpoint relationship']) {
    expect(await relationshipMap(page).evaluate(map => map.outerHTML)).not.toContain(protectedValue);
  }
  await expect(detail(page)).toContainText(service.text);
  await expect(detail(page)).toContainText(service.knowledge);
  await expect(detail(page)).toContainText('Original provenance unavailable');
  await expect(detail(page)).toContainText('supports <stored relationship>');
  await expect(detail(page)).toContainText('This item →');
  await expect(detail(page)).toContainText('→ This item');
  await expect(detail(page)).toContainText(service.target);
  await expect(detail(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(page.locator('#content')).toContainText('Run, Definition, and Practice dependencies are unavailable.');
  await expect(page.locator('body')).not.toContainText('Protected knowledge');
  await expect(page.locator('body')).not.toContainText('protected-endpoint relationship');
  await page.screenshot({ path: testInfo.outputPath('knowledge-desktop.png') });
  await detail(page).getByRole('link', { name: 'Open predecessor', exact: true }).click();
  await expect(detail(page).getByRole('heading', { name: 'Earlier support goal', exact: true })).toBeVisible();
  await page.goBack();
  await expect(detail(page).getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await page.goto(service.url('knowledge', { id: service.proposed }));
  await expect(detail(page)).toContainText('Still proposed, not accepted');
  await expect(detail(page)).toContainText('proposed');
  await expect(detail(page).locator('dd').filter({ hasText: /^No$/ })).toHaveCount(1);
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(refs);
});

test('Knowledge locus context and relationship-map navigation retain target, snapshot and mobile history', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('knowledge'));
  await page.getByLabel('Relevant to locus', { exact: true }).fill(service.target);
  const filtered = responseFor(page, 'nodes', query => query.get('target') === service.target && !query.has('id'));
  await page.getByRole('button', { name: 'Apply context', exact: true }).click();
  const data = await bodyOf(filtered);
  expect(data.items.map(item => item.id).sort()).toEqual([service.knowledge, service.predecessor].sort());
  expect(data.basis.target).toBe(service.target);
  expect(routeParams(page).get('target')).toBe(service.target);
  await expect(register(page).getByRole('link')).toHaveCount(2);
  await register(page).getByRole('link', { name: `${service.name} · ${service.knowledge}`, exact: true }).click();
  await expect(detail(page)).toContainText('Exact locus');
  const selectedUrl = page.url();
  const selectedParams = routeParams(page);
  await relationshipMap(page).getByRole('group', { name: 'Outgoing', exact: true }).getByRole('link', { name: connectedName(service.predecessor, data.items.find(item => item.id === service.predecessor).name), exact: true }).click();
  await expect(detail(page)).toContainText('Ancestor locus');
  await expect(detail(page)).toContainText(service.predecessor);
  await expect(relationshipMap(page)).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('knowledge-map-mobile.png') });
  // The focused item is first in mobile reading/tab order, before connections.
  await page.keyboard.press('Tab');
  await expect(relationshipMap(page).getByRole('button', { name: 'Inspect this item', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(relationshipMap(page).getByRole('group', { name: 'Incoming', exact: true }).getByRole('link', { name: connectedName(service.knowledge, service.name), exact: true })).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Enter');
  await expect(detail(page)).toBeFocused();
  expect(routeParams(page).get('id')).toBe(service.predecessor);
  for (const key of ['target', 'cursor', 'snapshot']) expect(routeParams(page).get(key)).toBe(selectedParams.get(key));
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.goBack();
  await expect(page).toHaveURL(selectedUrl);
  await expect(detail(page)).toContainText('Exact locus');
  await expect(relationshipMap(page).getByRole('group', { name: 'Outgoing', exact: true }).getByRole('link', { name: connectedName(service.predecessor, data.items.find(item => item.id === service.predecessor).name), exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', { name: 'All loci', exact: true }).click();
  await expect(register(page).getByRole('link', { name: `Proposed knowledge · ${service.proposed}`, exact: true })).toBeVisible();
  expect(routeParams(page).has('target')).toBe(false);
});

test.describe('Native keyset pages', () => {
  test.use({ recordCount: 30 });
  test('Knowledge retains opaque list cursors through detail/history and restarts all pages after real Record movement', async ({ page, service }) => {
    const firstResponse = responseFor(page, 'nodes', query => !query.has('id'));
    await page.goto(service.url('knowledge'));
    const first = await bodyOf(firstResponse);
    expect(first.items).toHaveLength(25);
    expect(first.page.has_more).toBe(true);
    const secondResponse = responseFor(page, 'nodes', query => query.get('cursor') === first.page.next_cursor);
    await page.getByRole('button', { name: 'Next page of knowledge items', exact: true }).click();
    const second = await bodyOf(secondResponse);
    expect(second.items).toHaveLength(8);
    expect(new Set([...first.items, ...second.items].map(item => item.id)).size).toBe(33);
    const listUrl = page.url();
    // The later focus change must navigate even when digest ordering puts the
    // fixture's main item first on this page. Same-URL navigation fetches nothing.
    const selected = second.items.find(item => item.id !== service.knowledge);
    await register(page).getByRole('link', { name: `${selected.name} · ${selected.id}`, exact: true }).click();
    await expect(detail(page).getByRole('heading', { name: selected.name, exact: true })).toBeVisible();
    expect(routeParams(page).get('cursor')).toBe(first.page.next_cursor);
    const detailUrl = page.url();
    await detail(page).getByRole('link', { name: 'Back to knowledge', exact: true }).click();
    await expect(page).toHaveURL(listUrl);
    await page.goBack();
    await expect(page).toHaveURL(detailUrl);
    await expect(detail(page).getByRole('heading', { name: selected.name, exact: true })).toBeVisible();
    // Use the same list continuation while focusing a different item.
    const retained = Object.fromEntries(routeParams(page));
    const initialEdges = responseFor(page, 'edges', query => !query.has('cursor'));
    await page.goto(service.url('knowledge', { ...retained, id: service.knowledge }));
    await expect(detail(page).getByRole('heading', { name: service.name, exact: true })).toBeVisible();
    expect(routeParams(page).get('cursor')).toBe(first.page.next_cursor);
    const firstEdges = await bodyOf(initialEdges);
    await expectMapPage(page, service.knowledge, firstEdges.items, second.items);
    // Names from a previously viewed collection page cannot label endpoints
    // absent from the currently returned collection. No endpoint fetch fills them.
    for (const omitted of first.items) {
      const links = relationshipMap(page).getByRole('link', { name: connectedName(omitted.id), exact: true });
      for (const link of await links.all()) await expect(link).not.toContainText(omitted.name);
    }
    const edgePage = responseFor(page, 'edges', query => query.has('cursor'));
    await page.getByRole('button', { name: 'Next page of relationships', exact: true }).click();
    const nextEdges = await bodyOf(edgePage);
    expect(nextEdges.items).toHaveLength(7);
    await expectMapPage(page, service.knowledge, nextEdges.items, second.items);
    expect(routeParams(page).get('edges_cursor')).toBeTruthy();
    const pagedUrl = page.url();
    const pagedParams = routeParams(page);
    const connectedId = endpointOf(nextEdges.items[0], service.knowledge);
    const connectedTitle = second.items.find(item => item.id === connectedId)?.name || 'Knowledge item';
    await relationshipMap(page).getByRole('link', { name: connectedName(connectedId, connectedTitle), exact: true }).first().click();
    await expect(relationshipMap(page)).toBeFocused();
    expect(routeParams(page).get('id')).toBe(connectedId);
    for (const key of ['target', 'cursor', 'snapshot']) expect(routeParams(page).get(key)).toBe(pagedParams.get(key));
    expect(routeParams(page).has('edges_cursor')).toBe(false);
    await page.goBack();
    await expect(page).toHaveURL(pagedUrl);
    await expectMapPage(page, service.knowledge, nextEdges.items, second.items);
    await service.mutate('advance');
    const conflict = responseFor(page, 'nodes', () => true, 409);
    await detail(page).getByRole('link', { name: 'Open predecessor', exact: true }).click();
    await conflict;
    await expect(page.locator('#notice')).toContainText('All pages restarted from the current snapshot.');
    await expect(detail(page).getByRole('heading', { name: 'Earlier support goal', exact: true })).toBeVisible();
    for (const key of ['cursor', 'edges_cursor', 'bindings_cursor']) expect(routeParams(page).has(key)).toBe(false);
  });
});

test('Knowledge protected and missing identities share 404, and new restrictions clear an open detail', async ({ page, service }) => {
  const missing = `sha256:${'0'.repeat(64)}`;
  const errors = [];
  for (const id of [service.hidden, missing]) {
    const response = responseFor(page, 'nodes', query => query.get('id') === id, 404);
    await page.goto(service.url('knowledge', { id }));
    errors.push((await (await response).json()).error);
    await expect(page.getByRole('heading', { name: 'Knowledge item not found', exact: true })).toBeVisible();
    await expect(relationshipMap(page)).toHaveCount(0);
    await expect(page.locator('#content')).toContainText('This item is not available in the inspected knowledge view.');
    await expect(page.locator('body')).not.toContainText('Protected knowledge');
  }
  expect(errors[0]).toEqual(errors[1]);
  await page.goto(service.url('knowledge', { id: service.knowledge }));
  await expect(detail(page)).toContainText(service.text);
  await service.mutate('protect-predecessor');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(detail(page)).toContainText(service.text);
  await expect(detail(page)).toContainText('Predecessor not available in this view.');
  await expect(detail(page).getByRole('link', { name: 'Open predecessor', exact: true })).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText(service.predecessor);
  expect(await relationshipMap(page).evaluate(map => map.outerHTML)).not.toContain(service.predecessor);
  await service.mutate('protect');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Knowledge item not found', exact: true })).toBeVisible();
  await expect(relationshipMap(page)).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText(service.name);
  await expect(page.locator('body')).not.toContainText(service.text);
  await expect(page.locator('body')).not.toContainText('supports <stored relationship>');
});

test('A stopped Knowledge service reports unavailable, clears prior data, and recovers on retry', async ({ page, service }) => {
  await page.goto(service.url('knowledge', { id: service.knowledge }));
  await expect(detail(page)).toContainText(service.text);
  await expect(relationshipMap(page)).toBeVisible();
  await service.stopKnowledge();
  const unavailable = responseFor(page, 'nodes', () => true, 503);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await unavailable;
  await expect(page.getByRole('heading', { name: 'Knowledge unavailable', exact: true })).toBeVisible();
  await expect(relationshipMap(page)).toHaveCount(0);
  await expect(page.locator('#content')).not.toContainText(service.text);
  await expect(page.locator('#content')).not.toContainText(service.name);
  await expect(page.locator('#content')).not.toContainText('No knowledge items');
  await service.startKnowledge();
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(detail(page)).toContainText(service.text);
  await expect(relationshipMap(page)).toBeVisible();
});

test('Knowledge session expiry clears receipt identities, graph text, relationships and source basis', async ({ page, service }) => {
  await page.goto(service.url('knowledge', { id: service.knowledge }));
  await expect(detail(page)).toContainText(service.text);
  await expect(relationshipMap(page)).toBeVisible();
  // Only the authentication error is injected. All successful graph reads
  // above came through the real API and native service.
  await page.route('**/api/hale/v1/**', route => route.fulfill({
    status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('unauthenticated', 'Sign in')),
  }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application', exact: true })).toBeVisible();
  await expect(relationshipMap(page)).toHaveCount(0);
  for (const value of [service.name, service.text, service.knowledge, service.predecessor, 'supports <stored relationship>']) {
    await expect(page.locator('body')).not.toContainText(value);
  }
  await expect(page.locator('#source')).not.toContainText('Projection watermark');
});

test('Knowledge mobile detail, context and browser history retain focus without horizontal overflow', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('knowledge'));
  await register(page).getByRole('link', { name: `${service.name} · ${service.knowledge}`, exact: true }).click();
  await expect(detail(page)).toBeFocused();
  await expect(detail(page).getByRole('heading', { name: service.name, exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('knowledge-mobile.png') });
  const selected = page.url();
  await detail(page).getByRole('link', { name: 'Back to knowledge', exact: true }).click();
  await expect(register(page)).toBeFocused();
  await page.goBack();
  await expect(page).toHaveURL(selected);
  await expect(detail(page)).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test.describe('Knowledge without configured private service', () => {
  test.use({ knowledge: false });
  test('disables unsupported navigation and reports unavailable on a direct link', async ({ page, service }) => {
    await page.goto(service.url('knowledge'));
    await expect(page.getByRole('heading', { name: 'Knowledge unavailable', exact: true })).toBeVisible();
    await expect(page.locator('#nav-knowledge')).toHaveAttribute('aria-disabled', 'true');
    await expect(page.locator('#nav-knowledge')).not.toHaveAttribute('href');
    await expect(page.locator('#content')).not.toContainText('No knowledge items');
  });
});
