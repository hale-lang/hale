import { test, expect, errorBody } from './harness.mjs';

test.skip(!process.env.HALE_FACE_CATALOG_BIN, 'Definitions browser integration requires an explicitly supplied application catalog fixture (HALE_FACE_CATALOG_BIN).');
test.use({ definitions: true });
const detail = page => page.getByRole('region', { name: 'Definition revision', exact: true });
const catalogResponse = page => page.waitForResponse(response => {
  const url = new URL(response.url());
  return url.pathname.endsWith('/dna/definitions') && !url.searchParams.has('id') && response.status() === 200;
});

test('Definitions inspect native ordered Steps, exact child revisions and leaf specifications', async ({ page, service }, testInfo) => {
  const refs = await service.refs();
  const response = catalogResponse(page);
  await page.goto(service.url('definitions', { id: 'close@1' }));
  const data = (await (await response).json()).data;
  expect(data.page.total).toBe(29);
  expect(data.basis.source_revision).toBe('fixture-catalog-v1');
  await expect(detail(page).getByRole('heading', { name: 'Close the month — équipe <workflow>', exact: true })).toBeVisible();
  await expect(detail(page).getByRole('heading', { level: 4 })).toHaveText(['Step 1', 'Step 2']);
  await detail(page).getByRole('button', { name: 'Open child collect@9007199254740993', exact: true }).click();
  await expect(detail(page).getByRole('heading', { name: 'Collect evidence', exact: true })).toBeVisible();
  await expect(detail(page)).toContainText('9223372036854775807');
  await expect(detail(page)).toContainText('opaque exact binding specification');
  await expect(detail(page)).toContainText('Ask a human\nfor evidence — ✓');
  await expect(detail(page)).toContainText('<img src=x onerror="window.__injected=true">');
  await expect(detail(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(detail(page)).toContainText('org/finance');
  await expect(detail(page)).toContainText('Evidence');
  await expect(detail(page)).toContainText('judgment');
  await expect(page.locator('#source')).toContainText('fixture-catalog-v1');
  await expect(page.locator('#source')).toContainText('claims');
  await page.screenshot({ path: testInfo.outputPath('definitions-desktop.png') });
  await detail(page).getByText('Direct catalog references · 1', { exact: true }).click();
  await detail(page).getByRole('link', { name: 'Open close@1', exact: true }).click();
  await expect(detail(page).getByRole('heading', { name: 'Close the month — équipe <workflow>', exact: true })).toBeVisible();
  expect(await service.refs()).toBe(refs);
});

test('Definitions retain page context across exact links and reject changed catalog snapshots', async ({ page, service }) => {
  await page.goto(service.url('definitions'));
  await page.getByRole('button', { name: 'Next page', exact: true }).click();
  await expect(page.locator('#content')).toContainText('Procedure 25');
  const oldUrl = page.url();
  await page.goto(service.url('definitions', {
    id: 'collect@9007199254740993', offset: '25',
    snapshot: new URLSearchParams(new URL(oldUrl).hash.split('?')[1]).get('snapshot'),
  }));
  await expect(detail(page)).toContainText('This revision is on another page');
  const returned = catalogResponse(page);
  await detail(page).getByRole('link', { name: 'Back to definitions', exact: true }).click();
  await returned;
  const last = page.getByRole('link', { name: 'Procedure 25 · procedure-25@1', exact: true });
  await expect(last).toBeVisible();
  expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('offset')).toBe('25');
  await service.changeCatalog('updated');
  const conflict = page.waitForResponse(response => response.url().includes('/dna/definitions') && response.status() === 409);
  await last.click();
  await conflict;
  await expect(page.locator('#notice')).toContainText('changed');
  await expect(page.locator('#source')).toContainText('fixture-catalog-v2');
  await expect(page.locator('#source')).not.toContainText('fixture-catalog-v1');
});

test('Definitions unavailable and invalid sources clear previously displayed catalog data', async ({ page, service }) => {
  await page.goto(service.url('definitions', { id: 'collect@9007199254740993' }));
  await expect(detail(page)).toContainText('Collect evidence');
  await service.changeCatalog('unavailable');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Definition catalog unavailable', exact: true })).toBeVisible();
  await expect(page.locator('#content')).not.toContainText('Collect evidence');
  await expect(page.locator('#content')).not.toContainText('No definitions in this catalog');
  await service.changeCatalog('invalid');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Definition catalog could not be verified', exact: true })).toBeVisible();
  await service.changeCatalog('original');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(detail(page)).toContainText('Collect evidence');
  await page.goto(service.url('definitions', { id: 'collect@1' }));
  await expect(page.getByRole('heading', { name: 'Definition revision not found', exact: true })).toBeVisible();
});

test('Definitions session expiry clears source claims, leaf text and exact identities', async ({ page, service }) => {
  await page.goto(service.url('definitions', { id: 'collect@9007199254740993' }));
  await expect(detail(page)).toContainText('Collect evidence');
  await page.route('**/api/hale/v1/**', route => route.fulfill({
    status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('unauthenticated', 'Sign in')),
  }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(page.locator('body')).not.toContainText('Collect evidence');
  await expect(page.locator('body')).not.toContainText('fixture-catalog-v1');
  await expect(page.locator('body')).not.toContainText('opaque exact binding specification');
});

test('Definitions mobile detail and history preserve exact revision selection', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('definitions'));
  await page.getByRole('link', { name: 'Collect evidence · collect@9007199254740993', exact: true }).click();
  await expect(detail(page)).toBeFocused();
  await expect(detail(page).getByRole('heading', { name: 'Collect evidence', exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('definitions-mobile.png') });
  const selected = page.url();
  await detail(page).getByRole('link', { name: 'Back to definitions', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Definition catalog', exact: true })).toBeInViewport();
  await page.goBack();
  await expect(page).toHaveURL(selected);
  await expect(detail(page)).toBeFocused();
});

test.describe('A standalone head without an application catalog', () => {
  test.use({ definitions: false });
  test('reports unsupported instead of an empty catalog', async ({ page, service }) => {
    await page.goto(service.url('definitions'));
    await expect(page.getByRole('heading', { name: 'Definition catalog unavailable', exact: true })).toBeVisible();
    await expect(page.locator('#content')).not.toContainText('No definitions in this catalog');
  });
});
