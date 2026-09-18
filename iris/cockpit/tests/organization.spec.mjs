import { test, expect, errorBody } from './harness.mjs';

test.use({ organization: true });

const organizationResponse = page => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith('/dna/organization') && !url.searchParams.has('id');
});
async function organizationData(response) {
  const result = await response;
  expect(result.status(), await result.text()).toBe(200);
  return (await result.json()).data;
}

test('Organization uses real compiler instances, declaration groups and separate domain ownership', async ({ page, service }) => {
  const refs = await service.refs();
  const mutations = [];
  page.on('request', request => { if (request.method() !== 'GET') mutations.push(request.url()); });
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
  await expect(outline.getByRole('link', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(outline.getByRole('link', { name: 'Org.assurance.reviewer', exact: true })).toBeVisible();
  await expect(outline.getByRole('link', { name: 'Org.metrics', exact: true })).toHaveCount(0);
  await expect(outline.getByRole('link', { name: 'Reserve', exact: true })).toHaveCount(0);
  const principal = await page.locator('#principal').textContent();
  await outline.getByRole('link', { name: 'Org.support.reviewer', exact: true }).click();
  const detail = page.getByRole('region', { name: 'Organization node', exact: true });
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await expect(detail).toContainText('Reviewer::on_notice');
  await expect(detail).toContainText('Notices');
  await expect(detail).toContainText('dna/org/main.hl');
  await expect(page.locator('#principal')).toHaveText(principal);
  await expect(page.locator('#source')).toContainText(service.organizationHead);
  await page.getByRole('link', { name: 'Back to organization', exact: true }).click();
  await page.getByRole('button', { name: 'All structure', exact: true }).click();
  await expect(outline.getByRole('link', { name: 'Org.metrics', exact: true })).toBeVisible();
  expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('scope')).toBe('all');
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(refs);
});

test('Organization ignores dirty source and rejects stale committed source snapshots', async ({ page, service }) => {
  await page.goto(service.url('organization'));
  const row = page.getByRole('link', { name: 'Org.support.reviewer', exact: true });
  await expect(row).toBeVisible();
  await service.changeOrganization({ valid: false, commit: false });
  const dirtyRead = organizationResponse(page);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  expect((await organizationData(dirtyRead)).basis.source_head).toBe(service.organizationHead);
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
});

test('Organization missing nodes and invalid committed source do not retain prior detail', async ({ page, service }) => {
  await page.goto(service.url('organization', { id: 'missing/position' }));
  await expect(page.getByRole('heading', { name: 'Organization node not found', exact: true })).toBeVisible();
  await page.goto(service.url('organization', { id: 'Org.support.reviewer' }));
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeVisible();
  await service.changeOrganization({ valid: false });
  const refusal = page.waitForResponse(response => response.url().includes('/dna/organization') && response.status() === 503);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  expect((await (await refusal).json()).error.code).toBe('organization_check_failed');
  await expect(page.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toHaveCount(0);
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
  await expect(page.locator('body')).not.toContainText('Org.support.reviewer');
  await expect(page.locator('body')).not.toContainText('partner');
  await expect(page.locator('body')).not.toContainText(service.organizationHead);
});

test('Organization mobile detail, back and browser history preserve the selected instance', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('organization'));
  const outline = page.getByRole('region', { name: 'Organization outline', exact: true });
  await outline.getByRole('link', { name: 'Org.support.reviewer', exact: true }).click();
  const detail = page.getByRole('region', { name: 'Organization node', exact: true });
  await expect(detail).toBeFocused();
  await expect(detail.getByRole('heading', { name: 'Org.support.reviewer', exact: true })).toBeInViewport();
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
  test.setTimeout(120_000);
  test('ignored vendored DNA is inspected without modification and dependency changes invalidate the cache', async ({ page, service }) => {
    const original = await service.projectState();
    const initial = organizationResponse(page);
    await page.goto(service.url('organization', { id: 'Org' }));
    const first = await organizationData(initial);
    expect(first.basis.source_head).toBe(service.organizationHead);
    expect(first.basis.dependency_source).toBe('local_vendor_snapshot');
    expect(first.basis.dependency_digest).toMatch(/^sha256:[0-9a-f]{64}$/);
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
      await expect(page.locator('#source')).not.toContainText(service.organizationHead);
      expect(await service.projectState()).toEqual(before);
      await service.changeDependency(mode === 'missing' ? 'restore-missing' : 'restore');
      const recovered = organizationResponse(page);
      await page.getByRole('button', { name: 'Refresh', exact: true }).click();
      expect((await organizationData(recovered)).basis.source_head).toBe(first.basis.source_head);
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
    const outline = page.getByRole('region', { name: 'Organization outline', exact: true });
    await outline.getByRole('link', { name: child.id, exact: true }).click();
    await page.getByRole('link', { name: 'Open parent', exact: true }).click();
    const detail = page.getByRole('region', { name: 'Organization node', exact: true });
    await expect(detail.getByRole('heading', { name: 'Org', exact: true })).toBeVisible();
    const query = () => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
    expect(query().get('offset')).toBe('25');
    expect(query().get('scope')).toBe('all');
    expect(query().get('snapshot')).toBe(data.page.snapshot);
    await page.getByRole('link', { name: 'Back to organization', exact: true }).click();
    await expect(outline).toBeFocused();
    await expect(outline.getByRole('link', { name: child.id, exact: true })).toBeVisible();
    expect(query().get('offset')).toBe('25');
    expect(query().get('scope')).toBe('all');
    expect(query().get('snapshot')).toBe(data.page.snapshot);
  });
});
