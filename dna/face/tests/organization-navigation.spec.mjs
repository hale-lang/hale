// Branch navigation is checked against native source projections. A displayed
// declaration does not establish a running occupant or permission to act.
import { test, expect, errorBody } from './harness.mjs';

test.use({ organization: true });

const outline = page => page.getByRole('region', { name: 'Organization outline', exact: true });
const topology = page => page.getByRole('region', { name: 'Declared containment topology', exact: true });
const detail = page => page.getByRole('region', { name: 'Organization node', exact: true });
const branchNavigation = page => page.getByRole('navigation', { name: 'Organization branch', exact: true });
const branchContext = page => page.getByRole('region', { name: 'Branch context', exact: true });
const COMPLETE_LEAF = 'No immediate children are declared in this complete source snapshot.';
const query = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
const collectionResponse = page => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith('/dna/organization') && !url.searchParams.has('id');
});
const exactResponse = (page, id) => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith('/dna/organization') && url.searchParams.get('id') === id;
});
async function payload(responsePromise) {
  const response = await responsePromise;
  expect(response.status(), await response.text()).toBe(200);
  return response.json();
}
async function open(page, service, route = {}) {
  const response = collectionResponse(page);
  await page.goto(service.url('organization', route));
  return payload(response);
}
const visibleRows = (data, scope = 'positions') => data.basis.position_group_declared && scope !== 'all'
  ? data.items.filter(row => row.in_position_outline) : data.items;
const branchRows = (root, rows) => [root, ...rows.filter(row => row.parent_id === root.id && row.id !== root.id)];

async function expectTopology(page, rows) {
  await expect(topology(page)).toBeVisible();
  await expect(topology(page).locator('a[data-instance-id]')).toHaveCount(rows.length);
  const rendered = await topology(page).locator('a[data-instance-id]').evaluateAll(links => links.map(link => ({
    id: link.dataset.instanceId,
    role: link.dataset.role,
    name: link.getAttribute('aria-label'),
    destination: new URLSearchParams(new URL(link.href).hash.split('?')[1]).get('id'),
  })));
  expect(rendered.map(row => row.id).sort()).toEqual(rows.map(row => row.id).sort());
  for (const row of rows) expect(rendered.find(item => item.id === row.id)).toEqual({
    id: row.id, role: row.role, name: row.id, destination: row.id,
  });
  const ids = new Set(rows.map(row => row.id));
  const expectedEdges = rows.filter(row => ids.has(row.parent_id) && row.parent_id !== row.id)
    .map(row => JSON.stringify([row.parent_id, row.id])).sort();
  const edges = await topology(page).locator('li[data-edge-from][data-edge-to]')
    .evaluateAll(items => items.map(item => JSON.stringify([item.dataset.edgeFrom, item.dataset.edgeTo])).sort());
  expect(edges).toEqual(expectedEdges);
}

async function enterSelectedBranch(page, id) {
  await detail(page).getByRole('link', { name: 'Enter branch', exact: true }).click();
  await expect.poll(() => query(page).get('branch')).toBe(id);
  await expect(outline(page)).toBeFocused();
  await expect(branchNavigation(page).locator('[aria-current="page"]')).toHaveText(id);
}

test('organization navigation: a branch shows the native root and immediate children with exact edges, preserving working context', async ({ page, service }, testInfo) => {
  const project = await service.projectState();
  const writes = [];
  page.on('request', request => { if (request.method() !== 'GET') writes.push(request.url()); });
  const { data } = await open(page, service, { id: 'Org', scope: 'all', locus: 'org/support' });
  await expectTopology(page, data.items);
  const principal = await page.locator('#principal').textContent();
  await enterSelectedBranch(page, 'Org');
  const root = data.items.find(row => row.id === 'Org');
  await expectTopology(page, branchRows(root, data.items));
  await expect(topology(page).getByRole('link', { name: 'Org.support.reviewer', exact: true })).toHaveCount(0);
  await expect(topology(page).getByRole('link', { name: 'Org.assurance.reviewer', exact: true })).toHaveCount(0);
  await expect(branchContext(page)).toContainText(/immediate child(?:ren)?.*page/i);
  await expect(branchContext(page)).toContainText(/source|declared/i);
  expect(query(page).get('id')).toBe('Org');
  expect(query(page).get('scope')).toBe('all');
  expect(query(page).get('locus')).toBe('org/support');
  await expect(page.locator('#principal')).toHaveText(principal);
  expect(writes).toEqual([]);
  expect(await service.projectState()).toEqual(project);
  await page.screenshot({ path: testInfo.outputPath('organization-branch-overview.png') });
});

test('organization navigation: inspect, descend to a leaf, switch presentation and return through reload and browser history', async ({ page, service }) => {
  const { data } = await open(page, service, { id: 'Org', branch: 'Org', scope: 'all' });
  const support = data.items.find(row => row.id === 'Org.support');
  const reviewer = data.items.find(row => row.id === 'Org.support.reviewer');
  await topology(page).getByRole('link', { name: support.id, exact: true }).click();
  await expect(detail(page).getByRole('heading', { name: support.id, exact: true })).toBeVisible();
  expect(query(page).get('branch')).toBe('Org');
  await enterSelectedBranch(page, support.id);
  await expectTopology(page, branchRows(support, data.items));
  await expect(branchNavigation(page).getByRole('link', { name: 'Org', exact: true })).toBeVisible();
  await expect(topology(page).getByRole('link', { name: /Parent outside page/ })).toHaveCount(0);

  await topology(page).getByRole('link', { name: reviewer.id, exact: true }).click();
  await expect(detail(page).getByRole('heading', { name: reviewer.id, exact: true })).toBeVisible();
  expect(query(page).get('branch')).toBe(support.id);
  await enterSelectedBranch(page, reviewer.id);
  await expectTopology(page, [reviewer]);
  await expect(branchContext(page)).toContainText(COMPLETE_LEAF);
  const leafURL = page.url();
  await outline(page).getByRole('button', { name: 'Outline', exact: true }).click();
  await expect(topology(page)).toHaveCount(0);
  await expect(outline(page).locator('.structure-list').getByRole('link', { name: reviewer.id, exact: true })).toBeVisible();
  await expect(page).toHaveURL(leafURL);
  await outline(page).getByRole('button', { name: 'Topology', exact: true }).click();
  await expectTopology(page, [reviewer]);
  await expect(page).toHaveURL(leafURL);
  await page.reload();
  await expectTopology(page, [reviewer]);
  await expect(branchNavigation(page).locator('[aria-current="page"]')).toHaveText(reviewer.id);
  expect(query(page).get('id')).toBe(reviewer.id);

  await branchNavigation(page).getByRole('link', { name: 'Whole organization', exact: true }).click();
  await expectTopology(page, data.items);
  expect(query(page).get('branch')).toBeNull();
  expect(query(page).get('id')).toBe(reviewer.id);
  expect(query(page).get('scope')).toBe('all');
  const wholeURL = page.url();
  await page.goBack();
  await expect(page).toHaveURL(leafURL);
  await expectTopology(page, [reviewer]);
  await page.goForward();
  await expect(page).toHaveURL(wholeURL);
  await expectTopology(page, data.items);
});

test('organization navigation: positions filter keeps the exact branch root while filtering its immediate children', async ({ page, service }) => {
  const { data } = await open(page, service, { id: 'Org.metrics', branch: 'Org.metrics' });
  const metrics = data.items.find(row => row.id === 'Org.metrics');
  expect(metrics.in_position_outline).toBe(false);
  await expectTopology(page, [metrics]);
  await expect(detail(page).getByRole('heading', { name: metrics.id, exact: true })).toBeVisible();
  await expect(outline(page).getByRole('button', { name: 'Declared positions', exact: true })).toHaveAttribute('aria-pressed', 'true');

  await branchNavigation(page).getByRole('link', { name: 'Org', exact: true }).click();
  const root = data.items.find(row => row.id === 'Org');
  await expectTopology(page, branchRows(root, visibleRows(data)));
  await expect(topology(page).getByRole('link', { name: 'Org.metrics', exact: true })).toHaveCount(0);
  await expect(branchContext(page)).toContainText(/outside the positions outline/i);
  await outline(page).getByRole('button', { name: 'All structure', exact: true }).click();
  await expectTopology(page, branchRows(root, data.items));
  expect(query(page).get('branch')).toBe('Org');
  expect(query(page).get('scope')).toBe('all');
  await outline(page).getByRole('button', { name: 'Declared positions', exact: true }).click();
  await expectTopology(page, branchRows(root, visibleRows(data)));
  expect(query(page).get('branch')).toBe('Org');
});

test('organization navigation: a missing hostile branch is literal, has no fallback topology and can return to the whole organization', async ({ page, service }) => {
  const missing = 'Org.<img src=x onerror="window.__branchInjected=true">/é?part=one#fragment';
  const { data } = await open(page, service, { branch: missing, id: 'Org.support', scope: 'all', locus: 'org/support' });
  await expect(outline(page).getByRole('heading', { name: 'Organization branch not found', exact: true })).toBeVisible();
  await expect(outline(page)).toContainText(missing);
  await expect(topology(page)).toHaveCount(0);
  await expect(outline(page).locator('[data-instance-id]')).toHaveCount(0);
  await expect(outline(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__branchInjected)).toBeUndefined();
  expect(query(page).get('branch')).toBe(missing);
  await branchNavigation(page).getByRole('link', { name: 'Whole organization', exact: true }).click();
  await expectTopology(page, data.items);
  expect(query(page).get('branch')).toBeNull();
  expect(query(page).get('id')).toBe('Org.support');
  expect(query(page).get('scope')).toBe('all');
  expect(query(page).get('locus')).toBe('org/support');
});

test('organization navigation: narrow reduced-motion keyboard entry focuses the outline and retains orientation', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { data } = await open(page, service, { id: 'Org.support', scope: 'all' });
  const root = data.items.find(row => row.id === 'Org.support');
  await detail(page).getByRole('link', { name: 'Enter branch', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(outline(page)).toBeFocused();
  await expect(outline(page).getByRole('heading', { name: 'Organization outline', exact: true })).toBeInViewport();
  await expectTopology(page, branchRows(root, data.items));
  expect(query(page).get('branch')).toBe(root.id);
  await expect(branchNavigation(page).locator('[aria-current="page"]')).toHaveText(root.id);
  await topology(page).getByRole('button', { name: 'Fit topology', exact: true }).click();
  const child = topology(page).getByRole('link', { name: 'Org.support.reviewer', exact: true });
  await child.focus();
  await page.keyboard.press('Enter');
  await expect(detail(page)).toBeFocused();
  expect(query(page).get('branch')).toBe(root.id);
  await detail(page).getByRole('link', { name: 'Enter branch', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(outline(page)).toBeFocused();
  await expect(branchContext(page)).toContainText(COMPLETE_LEAF);
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({ path: testInfo.outputPath('organization-branch-mobile-keyboard.png') });
});

test.describe('paged Organization branch', () => {
  test.use({ organization: 'large' });
  test('organization navigation: an off-page root uses its exact native snapshot while children stay paged and access loss clears it', async ({ page, service }) => {
    await open(page, service, { scope: 'all' });
    const nextPage = collectionResponse(page);
    await page.getByRole('button', { name: 'Next page', exact: true }).click();
    const secondPage = await payload(nextPage);
    const rootRead = exactResponse(page, 'Org');
    const collection = await open(page, service, { branch: 'Org', offset: String(secondPage.data.page.offset), snapshot: secondPage.data.page.snapshot, scope: 'all' });
    const rootResponse = await rootRead;
    const exact = await payload(rootResponse);
    const root = exact.data.items.find(row => row.id === 'Org');
    expect(root).toBeTruthy();
    expect(collection.data.page.offset).toBe(25);
    expect(collection.data.items.some(row => row.id === 'Org')).toBe(false);
    expect(new URL(rootResponse.url()).searchParams.get('snapshot')).toBe(collection.data.page.snapshot);
    expect(exact.source).toEqual(collection.source);
    expect(exact.data.basis).toEqual(collection.data.basis);
    await expectTopology(page, branchRows(root, collection.data.items));
    await expect(branchContext(page)).toContainText('Branch root read by identity; children are limited to this page.');
    await expect(branchContext(page)).not.toContainText(COMPLETE_LEAF);
    await expect(branchNavigation(page).locator('[aria-current="page"]')).toHaveText('Org');
    const child = collection.data.items.find(row => row.parent_id === root.id);
    await topology(page).getByRole('link', { name: child.id, exact: true }).click();
    await expect(detail(page).getByRole('heading', { name: child.id, exact: true })).toBeVisible();
    await enterSelectedBranch(page, child.id);
    await expectTopology(page, branchRows(child, collection.data.items));
    await expect(branchContext(page)).toContainText('No immediate children on this page. Other pages may contain children of this branch.');
    await expect(branchContext(page)).not.toContainText(COMPLETE_LEAF);
    await branchNavigation(page).getByRole('link', { name: 'Whole organization', exact: true }).click();
    await expectTopology(page, collection.data.items);
    expect(query(page).get('branch')).toBeNull();
    expect(query(page).get('id')).toBe(child.id);
    expect(query(page).get('offset')).toBe('25');
    expect(query(page).get('scope')).toBe('all');

    await page.goBack();
    await expectTopology(page, branchRows(child, collection.data.items));
    await page.route('**/api/hale/v1/**', route => route.fulfill({
      status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('unauthenticated', 'Sign in')),
    }));
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Sign in to read this application', exact: true })).toBeVisible();
    await expect(topology(page)).toHaveCount(0);
    await expect(branchContext(page)).toHaveCount(0);
    await expect(page.locator('[data-instance-id]')).toHaveCount(0);
  });

  test('organization navigation: a separately read branch with another basis or identity cannot restore a mixed chart', async ({ page, service }) => {
    await open(page, service, { scope: 'all' });
    const nextPage = collectionResponse(page);
    await page.getByRole('button', { name: 'Next page', exact: true }).click();
    const collection = await payload(nextPage);
    const route = { branch: 'Org', offset: String(collection.data.page.offset), snapshot: collection.data.page.snapshot, scope: 'all' };
    let fault = '';
    await page.route('**/api/hale/v1/**/dna/organization?*', async interception => {
      if (new URL(interception.request().url()).searchParams.get('id') !== 'Org') return interception.continue();
      const response = await interception.fetch();
      const body = await response.json();
      if (fault === 'basis') body.data.basis.source_head = '0'.repeat(40);
      if (fault === 'identity') body.data.items[0].id = 'Org.unexpected';
      await interception.fulfill({ response, json: body });
    });
    const initialRootRead = exactResponse(page, 'Org');
    await open(page, service, route);
    const root = (await payload(initialRootRead)).data.items[0];
    await expectTopology(page, branchRows(root, collection.data.items));
    for (const mode of ['basis', 'identity']) {
      fault = mode;
      // Reload preserves the captured second page. Refresh intentionally resets
      // pagination and would put Org back on-page, bypassing this separate read.
      await page.reload();
      await expect(page.getByRole('heading', { name: 'Response could not be verified', exact: true })).toBeVisible();
      await expect(topology(page)).toHaveCount(0);
      await expect(branchContext(page)).toHaveCount(0);
      await expect(page.locator('[data-instance-id]')).toHaveCount(0);
      await expect(page.locator('body')).not.toContainText('Org.unexpected');
      fault = '';
      await page.reload();
      await expectTopology(page, branchRows(root, collection.data.items));
    }
  });
});
