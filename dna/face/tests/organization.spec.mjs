import { test, expect, errorBody } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test.use({ organization: true });

const topology = page => page.getByRole('region', { name: 'Declared containment topology', exact: true });
const nodeDetail = page => page.getByRole('region', { name: 'Organization node', exact: true });
const shownRows = (data, scope = 'positions') => data.basis.position_group_declared && scope !== 'all' ? data.items.filter(item => item.in_position_outline) : data.items;
const routeQuery = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);

const organizationResponse = page => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith('/dna/organization') && !url.searchParams.has('id');
});
async function organizationData(response) {
  const result = await response;
  expect(result.status(), await result.text()).toBe(200);
  return (await result.json()).data;
}

// Verify the actual canvas against the real compiler response, independently
// of the optional outline and without supplying successful topology payloads.
async function expectTopology(page, rows) {
  const canvas = topology(page);
  if (!rows.length) {
    await expect(canvas).toHaveCount(0);
    return;
  }
  await expect(canvas).toBeVisible();
  await expect(canvas.locator('a[data-instance-id]')).toHaveCount(rows.length);
  const rendered = await canvas.locator('a[data-instance-id]').evaluateAll(links => links.map(link => ({
    id: link.dataset.instanceId,
    role: link.dataset.role,
    name: link.getAttribute('aria-label'),
    destination: new URLSearchParams(new URL(link.href).hash.split('?')[1]).get('id'),
    description: (link.getAttribute('aria-describedby') || '').split(' ').filter(Boolean).map(id => document.getElementById(id)?.textContent || '').join(' '),
  })));
  expect(rendered.map(item => item.id).sort()).toEqual(rows.map(item => item.id).sort());
  for (const item of rows) {
    const card = rendered.find(node => node.id === item.id);
    expect(card).toMatchObject({ name: item.id, destination: item.id, role: item.role });
    if (item.parent_id) expect(card.description).toBe(`Declared parent: ${item.parent_id}`);
    else expect(card.description).toBe('');
  }
  const ids = new Set(rows.map(item => item.id));
  const expected = rows.filter(item => ids.has(item.parent_id) && item.parent_id !== item.id).map(item => [item.parent_id, item.id]);
  const edges = await canvas.locator('li[data-edge-from][data-edge-to]').evaluateAll(items => items.map(item => [item.dataset.edgeFrom, item.dataset.edgeTo]));
  expect(edges.map(edge => JSON.stringify(edge)).sort()).toEqual(expected.map(edge => JSON.stringify(edge)).sort());
}

test('Organization uses real compiler instances, declaration groups and separate domain ownership', async ({ page, service }, testInfo) => {
  const refs = await service.refs();
  const mutations = [];
  const reads = [];
  page.on('request', request => {
    if (isWrite(request)) mutations.push(request.url());
    if (new URL(request.url()).pathname.endsWith('/dna/organization')) reads.push(request.url());
  });
  const response = organizationResponse(page);
  await page.goto(service.url('organization'));
  const data = await organizationData(response);
  expect(data.basis.source_head).toBe(service.organizationHead);
  expect(data.basis.coverage).toBe('static_instances');
  expect(data.basis.position_group_declared).toBe(true);
  expect(data.basis.declaration_count).toBe(5);
  expect(data.basis.uninstantiated_declaration_count).toBe(1);
  expect(data.items.map(item => item.id).sort()).toEqual([
    'Org', 'Org.assurance', 'Org.assurance.reviewer', 'Org.metrics',
    'Org.reviewer', 'Org.support', 'Org.support.reviewer',
  ]);
  expect(data.items.filter(item => item.role === 'position').map(item => item.id).sort()).toEqual([
    'Org.assurance.reviewer', 'Org.reviewer', 'Org.support.reviewer',
  ]);
  expect(data.items.map(item => item.source_file)).toEqual(Array(7).fill('dna/org/main.hl'));
  expect(data.ownership.instance_binding).toBe('unavailable');
  expect(data.ownership.positions).toContainEqual({ position: 'org/support', owner: 'partner' });
  const outline = page.getByRole('region', { name: 'Organization outline', exact: true });
  const rows = shownRows(data);
  await expectTopology(page, rows);
  await expect(outline.getByRole('button', { name: 'Topology', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(topology(page)).not.toContainText('partner');
  await expect(outline.getByRole('link', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(outline.getByRole('link', { name: 'Org.assurance.reviewer', exact: true })).toBeVisible();
  await expect(outline.getByRole('link', { name: 'Org.metrics', exact: true })).toHaveCount(0);
  await expect(outline.getByRole('link', { name: 'Reserve', exact: true })).toHaveCount(0);
  const principal = await page.locator('#principal').textContent();
  await topology(page).getByRole('link', { name: 'Org.support.reviewer', exact: true }).click();
  const detail = page.getByRole('region', { name: 'Organization node', exact: true });
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(detail).toContainText('Reviewer::on_notice');
  await expect(detail).toContainText('Notices');
  await expect(detail).toContainText('dna/org/main.hl');
  await expect(page.locator('#principal')).toHaveText(principal);
  await expect(page.locator('#source')).toContainText(service.organizationHead);
  await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveAttribute('data-instance-id', 'Org.support.reviewer');
  const selectedUrl = page.url();
  const readCount = reads.length;
  await outline.getByRole('button', { name: 'Outline', exact: true }).click();
  await expect(topology(page)).toHaveCount(0);
  await expect(outline.getByRole('button', { name: 'Outline', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const outlineIds = await outline.getByRole('link').evaluateAll(links => links.map(link => new URLSearchParams(new URL(link.href).hash.split('?')[1]).get('id')));
  expect(outlineIds.sort()).toEqual(rows.map(item => item.id).sort());
  await expect(outline.getByRole('link', { name: 'Org.support.reviewer', exact: true })).toHaveAttribute('aria-current', 'true');
  await expect(page).toHaveURL(selectedUrl);
  await outline.getByRole('button', { name: 'Topology', exact: true }).click();
  await expectTopology(page, rows);
  await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveAttribute('data-instance-id', 'Org.support.reviewer');
  await topology(page).getByRole('button', { name: 'Zoom out topology', exact: true }).click();
  await expect(topology(page).getByLabel('Topology zoom', { exact: true })).toHaveText('90%');
  await topology(page).getByRole('button', { name: 'Zoom in topology', exact: true }).click();
  await expect(topology(page).getByLabel('Topology zoom', { exact: true })).toHaveText('100%');
  await topology(page).getByRole('button', { name: 'Fit topology', exact: true }).click();
  const fitted = Number.parseInt(await topology(page).getByLabel('Topology zoom', { exact: true }).textContent(), 10);
  expect(fitted).toBeGreaterThanOrEqual(50);
  expect(fitted).toBeLessThanOrEqual(100);
  await expect(page).toHaveURL(selectedUrl);
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveAttribute('data-instance-id', 'Org.support.reviewer');
  expect(reads).toHaveLength(readCount);
  await topology(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('organization-topology-desktop.png') });
  await page.getByRole('link', { name: 'Back to organization', exact: true }).click();
  await page.getByRole('button', { name: 'All structure', exact: true }).click();
  await expect(outline.getByRole('link', { name: 'Org.metrics', exact: true })).toBeVisible();
  expect(routeQuery(page).get('scope')).toBe('all');
  await expectTopology(page, data.items);
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(refs);
});

test('Organization ignores dirty source and rejects stale committed source snapshots', async ({ page, service }) => {
  await page.goto(service.url('organization'));
  const row = topology(page).getByRole('link', { name: 'Org.support.reviewer', exact: true });
  await expect(row).toBeVisible();
  await service.changeOrganization({ valid: false, commit: false });
  const dirtyRead = organizationResponse(page);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  const dirtyData = await organizationData(dirtyRead);
  expect(dirtyData.basis.source_head).toBe(service.organizationHead);
  await expectTopology(page, shownRows(dirtyData));
  await expect(row).toBeVisible();
  const newHead = await service.changeOrganization();
  expect(newHead).not.toBe(service.organizationHead);
  const conflict = page.waitForResponse(response => response.url().includes('/dna/organization') && response.status() === 409);
  await row.click();
  await conflict;
  await expect(page.locator('#notice')).toContainText('changed');
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(page.locator('#source')).toContainText(newHead);
  await expect(page.locator('#source')).not.toContainText(service.organizationHead);
  await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveAttribute('data-instance-id', 'Org.support.reviewer');
});

test('Organization missing nodes and invalid committed source do not retain prior detail', async ({ page, service }) => {
  await page.goto(service.url('organization', { id: 'missing/position' }));
  await expect(page.getByRole('heading', { name: 'Organization node not found', exact: true })).toBeVisible();
  await expect(topology(page).locator('[aria-current="true"]')).toHaveCount(0);
  await page.goto(service.url('organization', { id: 'Org.support.reviewer' }));
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await service.changeOrganization({ valid: false });
  const refusal = page.waitForResponse(response => response.url().includes('/dna/organization') && response.status() === 503);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  expect((await (await refusal).json()).error.code).toBe('organization_check_failed');
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toHaveCount(0);
  await expect(topology(page)).toHaveCount(0);
  await expect(page.locator('[data-instance-id]')).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText('No declared instances');
  await expect(page.locator('body')).toContainText('could not be checked');
});

test('Organization session expiry clears structure and owner membership', async ({ page, service }) => {
  await page.goto(service.url('organization', { id: 'Org.support.reviewer' }));
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await page.route('**/api/hale/v1/**', route => route.fulfill({
    status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('unauthenticated', 'Sign in')),
  }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(topology(page)).toHaveCount(0);
  await expect(page.locator('[data-instance-id]')).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText('Org.support.reviewer');
  await expect(page.locator('body')).not.toContainText('partner');
  await expect(page.locator('body')).not.toContainText(service.organizationHead);
});

test('Organization mobile detail, back and browser history preserve the selected instance', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const response = organizationResponse(page);
  await page.goto(service.url('organization'));
  const rows = shownRows(await organizationData(response));
  const outline = page.getByRole('region', { name: 'Organization outline', exact: true });
  await expectTopology(page, rows);
  await topology(page).getByRole('button', { name: 'Fit topology', exact: true }).click();
  await topology(page).scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({ path: testInfo.outputPath('organization-topology-mobile.png') });
  const viewport = topology(page).getByLabel('Organization canvas. Scroll to explore declared containment.', { exact: true });
  await viewport.focus();
  const visited = [];
  for (let index = 0; index < rows.length; index += 1) {
    await page.keyboard.press('Tab');
    const id = await page.evaluate(() => document.activeElement?.getAttribute('data-instance-id'));
    visited.push(id);
    if (id === 'Org.support.reviewer') break;
  }
  expect(visited[0]).toBe('Org');
  expect(visited).not.toContain(null);
  await expect(topology(page).getByRole('link', { name: 'Org.support.reviewer', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  const detail = page.getByRole('region', { name: 'Organization node', exact: true });
  await expect(detail).toBeFocused();
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeInViewport();
  await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveAttribute('data-instance-id', 'Org.support.reviewer');
  expect(routeQuery(page).get('id')).toBe('Org.support.reviewer');
  await page.screenshot({ path: testInfo.outputPath('mobile-organization.png') });
  const selectedUrl = page.url();
  await page.getByRole('link', { name: 'Back to organization', exact: true }).click();
  await expect(outline).toBeFocused();
  await expect(outline.getByRole('heading', { name: 'Organization outline', exact: true })).toBeInViewport();
  await page.goBack();
  await expect(page).toHaveURL(selectedUrl);
  await expect(detail).toBeFocused();
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
});

test.describe('generated DNA project', () => {
  test.use({ organization: 'generated' });
  // The case checks the generated project's vendored DNA twice through the
  // API: under half a minute on a fast machine, a few on a loaded CI runner.
  // Budget the checks, not a guess about the runner.
  test.setTimeout(600_000);
  test('ignored vendored DNA is inspected without modification and dependency changes invalidate the cache', async ({ page, service }) => {
    const original = await service.projectState();
    const initial = organizationResponse(page);
    await page.goto(service.url('organization', { id: 'Org' }));
    const first = await organizationData(initial);
    expect(first.basis.source_head).toBe(service.organizationHead);
    expect(first.basis.dependency_source).toBe('local_vendor_snapshot');
    expect(first.basis.dependency_digest).toMatch(/^sha256:[0-9a-f]{64}$/);
    await expectTopology(page, shownRows(first));
    await expect(page.getByRole('region', { name: 'Organization node', exact: true }).getByRole('heading', { name: 'Org', exact: true })).toBeVisible();
    expect(await service.projectState()).toEqual(original);

    await service.changeDependency('comment');
    const changedState = await service.projectState();
    expect(changedState.refs).toBe(original.refs);
    expect(changedState.status).toBe(original.status);
    const changedRead = organizationResponse(page);
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    const changed = await organizationData(changedRead);
    expect(changed.basis.source_head).toBe(first.basis.source_head);
    expect(changed.basis.dependency_digest).not.toBe(first.basis.dependency_digest);
    expect(changed.page.snapshot).not.toBe(first.page.snapshot);
    await expectTopology(page, shownRows(changed));
    await expect(page.getByRole('region', { name: 'Organization node', exact: true }).getByRole('heading', { name: 'Org', exact: true })).toBeVisible();
    expect(await service.projectState()).toEqual(changedState);

    for (const mode of ['invalid', 'missing']) {
      await service.changeDependency(mode);
      const before = await service.projectState();
      const refusal = organizationResponse(page);
      await page.getByRole('button', { name: 'Refresh', exact: true }).click();
      const result = await refusal;
      expect(result.status()).toBe(503);
      expect((await result.json()).error.code).toMatch(/^organization_/);
      await expect(page.getByRole('heading', { name: 'Organization source unavailable', exact: true })).toBeVisible();
      await expect(page.getByRole('region', { name: 'Organization node', exact: true })).toHaveCount(0);
      await expect(topology(page)).toHaveCount(0);
      await expect(page.locator('#source')).not.toContainText(service.organizationHead);
      expect(await service.projectState()).toEqual(before);
      await service.changeDependency(mode === 'missing' ? 'restore-missing' : 'restore');
      const recovered = organizationResponse(page);
      await page.getByRole('button', { name: 'Refresh', exact: true }).click();
      const recoveredData = await organizationData(recovered);
      expect(recoveredData.basis.source_head).toBe(first.basis.source_head);
      await expectTopology(page, shownRows(recoveredData));
      await expect(page.getByRole('region', { name: 'Organization node', exact: true }).getByRole('heading', { name: 'Org', exact: true })).toBeVisible();
    }
    expect(await service.projectState()).toEqual(original);
  });
});

test.describe('paged Organization', () => {
  test.use({ organization: 'large' });
  test('a parent outside the current page preserves page and scope on return', async ({ page, service }) => {
    await page.goto(service.url('organization', { scope: 'all' }));
    await expect(page.getByRole('button', { name: 'Next page', exact: true })).toBeEnabled();
    const nextPage = organizationResponse(page);
    await page.getByRole('button', { name: 'Next page', exact: true }).click();
    const data = await organizationData(nextPage);
    expect(data.page.offset).toBe(25);
    const child = data.items.find(item => item.parent_id === 'Org');
    expect(child).toBeTruthy();
    expect(data.items.some(item => item.id === 'Org')).toBe(false);
    await expectTopology(page, data.items);
    await expect(topology(page).getByRole('link', { name: 'Org', exact: true })).toHaveCount(0);
    await expect(topology(page).locator('[data-edge-from="Org"]')).toHaveCount(0);
    const outline = page.getByRole('region', { name: 'Organization outline', exact: true });
    const childLink = topology(page).getByRole('link', { name: child.id, exact: true });
    await childLink.click();
    await expect(nodeDetail(page).getByRole('heading', { name: child.id, exact: true })).toBeVisible();
    const parentReference = topology(page).getByRole('link', { name: 'Parent outside page · Org', exact: true }).first();
    await expect(parentReference).toBeVisible();
    const parentQuery = new URLSearchParams(new URL(await parentReference.getAttribute('href'), page.url()).hash.split('?')[1]);
    expect(parentQuery.get('id')).toBe('Org');
    expect(parentQuery.get('offset')).toBe('25');
    expect(parentQuery.get('scope')).toBe('all');
    expect(parentQuery.get('snapshot')).toBe(data.page.snapshot);
    await parentReference.click();
    const detail = page.getByRole('region', { name: 'Organization node', exact: true });
    await expect(detail.getByRole('heading', { name: 'Org', exact: true })).toBeVisible();
    await expect(topology(page).getByRole('link', { name: 'Org', exact: true })).toHaveCount(0);
    await expect(topology(page).locator('[data-instance-id][aria-current="true"]')).toHaveCount(0);
    const query = () => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
    expect(query().get('offset')).toBe('25');
    expect(query().get('scope')).toBe('all');
    expect(query().get('snapshot')).toBe(data.page.snapshot);
    await page.getByRole('link', { name: 'Back to organization', exact: true }).click();
    await expect(outline).toBeFocused();
    await expect(outline.getByRole('link', { name: child.id, exact: true })).toBeVisible();
    await expectTopology(page, data.items);
    expect(query().get('offset')).toBe('25');
    expect(query().get('scope')).toBe('all');
    expect(query().get('snapshot')).toBe(data.page.snapshot);
  });
});
