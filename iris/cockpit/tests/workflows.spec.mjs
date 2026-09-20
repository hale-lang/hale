import { test, expect, errorBody } from './harness.mjs';
import { writeFile } from 'node:fs/promises';

test.skip(!process.env.HALE_COCKPIT_WORKFLOWS_BIN, 'Requires the native recorded workflow fixture.');
test.use({ workflows: true, organization: true });
const detail = page => page.getByRole('region', { name: 'Execution', exact: true });
const steps = page => page.getByRole('list', { name: 'Ordered execution Steps', exact: true });
const results = page => page.getByRole('region', { name: 'Work and attempt results', exact: true });
const params = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
async function open(page, service, extra = {}) {
  await page.goto(service.url('workflows', { id: service.execution, ...extra }));
  await expect(steps(page)).toBeVisible();
}
async function refresh(page) { await page.getByRole('button', { name: 'Refresh', exact: true }).click(); }

test('Work reads the native bound recipe, nested admission, failed attempt history and accepted result', async ({ page, service }, testInfo) => {
  const refs = await service.refs(), mutations = [];
  page.on('request', request => { if (request.method() !== 'GET') mutations.push(request.url()); });
  const response = page.waitForResponse(r => new URL(r.url()).pathname.endsWith('/dna/workflows') && new URL(r.url()).searchParams.get('id') === service.execution);
  await open(page, service);
  const payload = await (await response).json();
  await writeFile(testInfo.outputPath('native-workflow.json'), JSON.stringify(payload, null, 2));
  expect(payload.data.basis).toEqual({ projection: 'dna.workflow-projection/1', memory: 'record', routing: '0', runtime_association: false });
  expect(payload.data.items[0].nodes.filter(n => n.kind === 'task')).toHaveLength(2);
  await expect(page.getByRole('region', { name: 'Execution register', exact: true }).locator('.record-link')).toHaveCount(3);
  await expect(steps(page).locator(':scope > li')).toHaveCount(2);
  await expect(steps(page).locator(':scope > li').nth(1)).toHaveAttribute('data-state', 'bound');
  await expect(steps(page)).toContainText('Completion not recorded for · evidence');
  await detail(page).getByRole('link', { name: 'Inspect work ' + service.bank, exact: false }).click();
  await expect(results(page)).toContainText('Accepted Work result');
  await expect(results(page)).toContainText('Reconciled ✓');
  await expect(results(page).locator('.attempt-card')).toHaveCount(2);
  await expect(results(page).locator('.attempt-card').nth(0)).toContainText('failed');
  await expect(results(page).locator('.attempt-card').nth(0)).toContainText('Not an accepted Work result');
  await expect(results(page).locator('.attempt-card').nth(1)).toContainText('Accepted by the Work');
  await expect(results(page)).toContainText('exact historical binding v7');
  expect(await page.evaluate(() => window.bad)).toBeUndefined();
  await expect(results(page).locator('img')).toHaveCount(0);
  expect(await service.refs()).toBe(refs);
  expect(mutations).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('execution-desktop.png') });
});

test('Recursive child navigation, exact deep links and narrow-screen history preserve native identity', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, service, { locus: 'org/support' });
  await detail(page).getByRole('link', { name: 'Enter child ' + service.child, exact: false }).click();
  await expect(detail(page).getByRole('heading', { name: 'evidence@3', exact: true })).toBeVisible();
  await expect(steps(page).locator(':scope > li')).toHaveCount(1);
  await detail(page).getByRole('link', { name: 'Inspect work ' + service.human, exact: false }).click();
  await expect(results(page)).toContainText('No accepted Work result is recorded');
  await expect(results(page)).toContainText('Performer kind · human');
  expect(params(page).get('node')).toBe(service.human);
  expect(params(page).get('locus')).toBe('org/support');
  await page.reload();
  await expect(results(page)).toContainText('Attempt 0 · outstanding');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await results(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('execution-mobile.png') });
  await detail(page).getByRole('navigation', { name: 'Execution ancestry' }).getByRole('link', { name: 'monthly-close@7 · ' + service.execution, exact: true }).click();
  await expect(steps(page).locator(':scope > li')).toHaveCount(2);
  await page.goBack();
  await expect(results(page)).toContainText('Attempt 0 · outstanding');
});

test('Settled members do not advance the displayed barrier before native Step completion is recorded', async ({ page, service }) => {
  await open(page, service);
  await service.mutate('members');
  await refresh(page);
  await expect(steps(page)).toContainText('All registered members settled done; Step completion has not been recorded');
  await expect(steps(page).locator(':scope > li').nth(0)).toHaveAttribute('data-state', 'activated');
  await expect(steps(page).locator(':scope > li').nth(1)).toHaveAttribute('data-state', 'bound');
  await service.mutate('finish');
  await refresh(page);
  await expect(steps(page).locator(':scope > li').nth(0)).toHaveAttribute('data-state', 'completed');
  await expect(steps(page).locator(':scope > li').nth(1)).toHaveAttribute('data-state', 'completed');
  await expect(detail(page).locator('.detail-kicker')).toContainText('Done');
});

test('Cancellation keeps admitted child responsibilities visible; refusals and unclassified task births remain distinct', async ({ page, service }) => {
  await open(page, service);
  await service.mutate('cancel');
  await refresh(page);
  await expect(detail(page).locator('.detail-kicker')).toContainText('Cancelled');
  await detail(page).getByRole('link', { name: 'Enter child ' + service.child, exact: false }).click();
  await expect(detail(page)).toContainText('was cancelled');
  await detail(page).getByRole('link', { name: 'Inspect work ' + service.human, exact: false }).click();
  await expect(results(page)).toContainText('Attempt 0 · outstanding');
  await page.goto(service.url('workflows', { id: 'refused-close' }));
  await expect(detail(page)).toContainText('Required binding is unavailable');
  await expect(steps(page)).toHaveCount(0);
  await page.goto(service.url('workflows', { id: 'legacy-task' }));
  await expect(detail(page)).toContainText('Its execution engine and state are not established by this projection');
});

test('Changed evidence visibility clears an open execution and its inline results', async ({ page, service }) => {
  await open(page, service, { node: service.bank });
  await expect(results(page)).toContainText('Reconciled ✓');
  await service.mutate('redact');
  await refresh(page);
  await expect(results(page)).toHaveCount(0);
  await expect(page.locator('#content')).not.toContainText('Reconciled ✓');
  await expect(page.getByRole('region', { name: 'Execution register', exact: true }).locator('.record-link')).toHaveCount(2);
  const response = await page.request.get(service.origin + service.apiPath + '/dna/workflows?id=' + service.execution);
  expect(response.status()).toBe(404);
});

test('Split memory and invalid native facts report source failures rather than an empty or invented execution', async ({ page, service }) => {
  await open(page, service);
  await service.mutate('invalid');
  await refresh(page);
  await expect(page.locator('#content')).toContainText('refused by the native projection');
  await expect(steps(page)).toHaveCount(0);
});

test('A Ledger adoption and authentication loss both remove stale execution data', async ({ page, service }) => {
  await open(page, service);
  await service.mutate('adopt');
  await refresh(page);
  await expect(page.locator('#content')).toContainText('owning-service execution reader is not connected');
  await expect(steps(page)).toHaveCount(0);
  await page.route('**/dna/workflows?*', route => route.fulfill({ status: 401, json: errorBody('unauthenticated', 'Sign in again.') }));
  await refresh(page);
  await expect(page.getByRole('region', { name: 'Working context', exact: true })).toBeHidden();
  await expect(page.locator('#principal')).toContainText('Sign in required');
});
