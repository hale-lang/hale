import { test, expect, errorBody } from './harness.mjs';

test.use({ organization: true });
const context = page => page.getByRole('region', { name: 'Working context', exact: true });
const query = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
const owners = 'org = acme\norg/support = partner\norg/support/équipe = partner\norg/finance = acme\nacme: alice\npartner: bob\nhost = acme\n';
async function choose(page, value) {
  await page.getByLabel('Working locus', { exact: true }).selectOption(value);
  await expect.poll(() => query(page).get('locus') || '').toBe(value);
  await expect(page.getByLabel('Working locus', { exact: true })).toBeFocused();
  await expect(page.locator('#content')).toHaveAttribute('aria-busy', 'false');
}
async function navigate(page, view) {
  await page.locator('#nav-' + view).click();
  await expect(page.locator('#nav-' + view)).toHaveAttribute('aria-current', 'page');
  await expect(context(page)).toBeVisible();
  await expect(page.locator('#content')).toHaveAttribute('aria-busy', 'false');
}

test('Declared scopes carry across DNA workspaces and mark only exact practice targets without changing identity', async ({ page, service }, testInfo) => {
  await service.changeOwnership(owners);
  const before = await service.projectState(), mutations = [];
  page.on('request', request => { if (request.method() !== 'GET') mutations.push(request.url()); });
  await page.goto(service.url('organization'));
  await expect(page.getByLabel('Working locus').locator('option')).toHaveCount(5);
  const principal = await page.locator('#principal').textContent();
  await page.getByRole('button', { name: 'Work from org/support/équipe', exact: true }).click();
  await expect(page.getByLabel('Working locus')).toBeFocused();
  await expect(context(page)).toContainText('Declared owner · partner');
  await expect(context(page)).toContainText('binding to these ownership scopes is unavailable');
  await navigate(page, 'practices');
  await expect(page.locator('.record-link')).toHaveCount(3);
  await expect(page.locator('.context-target-match')).toHaveCount(1);
  await expect(page.locator('.context-target-match')).toContainText(service.name);
  await expect(context(page)).toContainText('pages remain unfiltered');
  await choose(page, 'org');
  await expect(page.locator('.record-link')).toHaveCount(3);
  await expect(page.locator('.context-target-match')).toHaveCount(2);
  await navigate(page, 'reviews');
  expect(query(page).get('locus')).toBe('org');
  await expect(context(page)).toContainText('recorded scope and required authority');
  await expect(page.locator('#principal')).toHaveText(principal);
  await expect(page.locator('.read-only')).toContainText('Read only');
  expect(await service.projectState()).toEqual(before);
  expect(mutations).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('shared-context.png') });
});

test('Removed and unknown scopes do not silently broaden the workspace; clearing and refreshed ownership recover', async ({ page, service }) => {
  await page.goto(service.url('practices', { locus: 'org/support' }));
  await expect(context(page)).toContainText('Declared owner · partner');
  await service.changeOwnership('org = acme\norg/finance = acme\nacme: alice\nhost = acme\n');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Working context unavailable', exact: true })).toBeVisible();
  await expect(page.locator('.record-link')).toHaveCount(0);
  await expect(context(page)).not.toContainText('partner');
  await context(page).getByRole('button', { name: 'Clear working context', exact: true }).click();
  await expect(page.locator('.record-link')).toHaveCount(3);
  await choose(page, 'org/finance');
  await expect(context(page)).toContainText('Declared owner · acme');
  await page.goto(service.url('practices', { locus: 'Org.support.reviewer' }));
  await expect(page.getByRole('heading', { name: 'Working context unavailable', exact: true })).toBeVisible();
  await expect(page.locator('.record-link')).toHaveCount(0);
});

test('An optional context outage keeps unscoped reads usable but never ignores a selected scope', async ({ page, service }) => {
  await page.route('**/dna/organization?*', route => route.fulfill({ status: 503, json: errorBody('organization_unavailable', 'Organization is unavailable.') }));
  await page.goto(service.url('practices'));
  await expect(page.locator('.record-link')).toHaveCount(3);
  await expect(context(page)).toContainText('Declared locus contexts could not be read');
  await expect(page.getByLabel('Working locus')).toBeDisabled();
  await page.goto(service.url('practices', { locus: 'org' }));
  await expect(page.getByRole('heading', { name: 'Working context unavailable', exact: true })).toBeVisible();
  await expect(page.locator('.record-link')).toHaveCount(0);
});

test('Context authentication loss clears metadata, and malformed ownership never becomes a choice', async ({ page, service }) => {
  await page.goto(service.url('practices', { locus: 'org/support' }));
  await expect(context(page)).toContainText('Declared owner · partner');
  await page.route('**/dna/organization?*', route => route.fulfill({ status: 401, json: errorBody('unauthenticated', 'Sign in again.') }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(context(page)).toBeHidden();
  await expect(page.locator('#working-context')).toBeEmpty();
  await expect(page.locator('.record-link')).toHaveCount(0);
  await page.unroute('**/dna/organization?*');
  await page.route('**/dna/organization?*', async route => {
    const response = await route.fetch(), body = await response.json();
    body.data.ownership.positions.push({ ...body.data.ownership.positions[0] });
    await route.fulfill({ response, json: body });
  });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.locator('#content')).toContainText('ambiguous or invalid');
  await expect(context(page)).toBeHidden();
});

test('The context picker works on a narrow viewport and remains separate from generic application surfaces', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('practices'));
  await choose(page, 'org/support');
  await expect(context(page)).toContainText('Declared owner · partner');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await context(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('context-mobile.png') });
  await page.goto(service.url('application'));
  await expect(context(page)).toBeHidden();
  expect(query(page).has('locus')).toBe(false);
  await page.goBack();
  await expect(page.getByLabel('Working locus')).toHaveValue('org/support');
});

test.describe('Knowledge and Definition contexts', () => {
  test.use({ knowledge: true, definitions: true });
  test.skip(!process.env.HALE_COCKPIT_KNOWLEDGE_BIN || !process.env.HALE_COCKPIT_CATALOG_BIN, 'Requires the native Knowledge service and definition catalog fixtures.');

  test('Native Knowledge relevance follows the context across workspaces and browser history', async ({ page, service }) => {
    await service.changeOwnership(owners);
    const reads = [];
    page.on('request', request => { if (request.url().includes('/dna/knowledge/')) reads.push(new URL(request.url())); });
    await page.goto(service.url('knowledge', { locus: 'org' }));
    const register = page.getByRole('region', { name: 'Knowledge register', exact: true });
    await expect(register.locator('.record-link')).toHaveCount(1);
    await expect(register).toContainText('Earlier support goal');
    await choose(page, service.target);
    await expect(register.locator('.record-link')).toHaveCount(2);
    await expect(register).toContainText(service.name);
    await expect(page.getByLabel('Relevant to locus', { exact: true })).toHaveValue(service.target);
    await navigate(page, 'definitions');
    expect(query(page).get('locus')).toBe(service.target);
    await navigate(page, 'knowledge');
    await expect(register.locator('.record-link')).toHaveCount(2);
    await choose(page, 'org');
    await page.goBack();
    await expect(page.getByLabel('Working locus')).toHaveValue(service.target);
    await expect(register.locator('.record-link')).toHaveCount(2);
    expect(reads.some(url => url.searchParams.get('target') === service.target)).toBe(true);
    expect(reads.every(url => !url.searchParams.has('locus'))).toBe(true);
  });

  test('Definition matches come only from exact direct leaf targets on the current native catalog page', async ({ page, service }) => {
    await service.changeOwnership(owners);
    await page.goto(service.url('definitions', { locus: 'org/finance' }));
    await expect(page.locator('.record-link')).toHaveCount(25);
    await expect(page.locator('.context-target-match')).toHaveCount(2);
    const matched = await page.locator('.context-target-match').evaluateAll(links => links.map(link => new URLSearchParams(link.hash.split('?')[1]).get('id')));
    expect(matched.sort()).toEqual(['close@1', 'collect@9007199254740993']);
    await choose(page, 'org');
    await expect(page.locator('.record-link')).toHaveCount(25);
    await expect(page.locator('.context-target-match')).toHaveCount(0);
  });

  test('Changing context clears unsubmitted Knowledge edits; an independent relevance filter clears shared context', async ({ page, service }) => {
    await service.changeOwnership(owners);
    await page.goto(service.url('knowledge', { locus: service.target, id: service.knowledge }));
    const editor = page.getByRole('region', { name: 'Knowledge change editor', exact: true });
    await editor.getByRole('button', { name: 'Prepare knowledge change', exact: true }).click();
    await editor.getByLabel('Knowledge text').fill('Unsaved context-specific draft');
    await choose(page, 'org');
    await expect(editor.getByLabel('Knowledge text')).toHaveCount(0);
    await expect(page.locator('#content')).not.toContainText('Unsaved context-specific draft');
    await page.getByLabel('Relevant to locus', { exact: true }).fill(service.target);
    await page.getByRole('button', { name: 'Apply context', exact: true }).click();
    await expect(page.getByLabel('Working locus')).toHaveValue('');
    expect(query(page).has('locus')).toBe(false);
    await expect(page.getByRole('region', { name: 'Knowledge register', exact: true }).locator('.record-link')).toHaveCount(2);
  });
});
