import { readFile } from 'node:fs/promises';
import { test, expect, errorBody } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test.skip(!process.env.HALE_FACE_KNOWLEDGE_BIN, 'Knowledge editing reads require the real native Knowledge provider fixture.');
test.use({ knowledge: true });
const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const review = page => editor(page).getByRole('region', { name: 'Knowledge change review', exact: true });
async function open(page, service, { selected = true, action } = {}) {
  await page.goto(service.url('knowledge', selected ? { id: service.knowledge } : {}));
  await editor(page).getByRole('button', { name: 'Prepare knowledge change', exact: true }).click();
  if (action) await editor(page).getByLabel('Change kind', { exact: true }).selectOption(action);
}
async function check(page) {
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
}
async function exported(page, testInfo, name = 'knowledge-change.draft.json') {
  const download = page.waitForEvent('download');
  await review(page).getByRole('button', { name: 'Download knowledge draft', exact: true }).click();
  const file = await download, path = testInfo.outputPath(name);
  await file.saveAs(path);
  return JSON.parse(await readFile(path, 'utf8'));
}

test('Knowledge revision preserves exact identity and historical text, checks real service reads and exports a clearly unsubmitted draft', async ({ page, service }, testInfo) => {
  const before = await service.projectState(), mutations = [];
  page.on('request', request => { if (isWrite(request)) mutations.push(request.method() + ' ' + request.url()); });
  await open(page, service);
  await expect(editor(page).getByLabel('Knowledge text')).toHaveValue(service.text);
  const changed = 'Revised équipe 🧬\n<img src=x onerror="window.changed=true">';
  await editor(page).getByLabel('Knowledge text').fill(changed);
  await editor(page).getByLabel('Target locus', { exact: true }).fill(service.target);
  await editor(page).getByLabel('Reason for knowledge change').fill('Clarify the assertion while preserving its predecessor.');
  await expect(editor(page).getByLabel('Knowledge draft preview')).toContainText(service.text);
  await expect(editor(page).getByLabel('Knowledge draft preview')).toContainText(changed);
  await check(page);
  await expect(review(page)).toContainText(service.target);
  await expect(review(page)).toContainText('supports <stored relationship>');
  await expect(review(page).getByRole('button', { name: 'Submit knowledge change', exact: true })).toBeDisabled();
  await expect(review(page)).toContainText('No proposal, Review, adoption or graph mutation has been recorded');
  const artifact = await exported(page, testInfo);
  expect(artifact.operation).toBe('node.revise');
  expect(artifact.arguments.supersedes).toBe(service.knowledge);
  expect(artifact.arguments.text).toBe(changed);
  expect(artifact.arguments.author).toBe('org');
  expect(artifact.base.basis.record_id).toBe(service.application);
  expect(artifact.base.snapshot).toMatch(/^sha256:/);
  expect(artifact.source_checked).toBe(true);
  expect(artifact.submitted).toBe(false);
  expect(artifact.prepared_by.mode).toBe('local');
  expect(await service.projectState()).toEqual(before);
  expect(mutations).toEqual([]);
  expect(await page.evaluate(() => window.changed)).toBeUndefined();
  await expect(editor(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => Object.values(localStorage).some(value => value.includes('Clarify the assertion')))).toBe(false);
  await editor(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('knowledge-editor.png') });
});

test('New-item preparation requires meaningful fields and offers only creation without a selected identity', async ({ page, service }, testInfo) => {
  await open(page, service, { selected: false });
  await expect(editor(page).getByLabel('Change kind').locator('option')).toHaveCount(1);
  await expect(editor(page).getByRole('button', { name: 'Review knowledge draft' })).toBeDisabled();
  await editor(page).getByLabel('Knowledge name').fill('Response quality');
  await editor(page).getByLabel('Knowledge kind').selectOption('concept');
  await editor(page).getByLabel('Knowledge text').fill('A new assertion for independent review.');
  await editor(page).getByLabel('Requested authoring locus').fill('org');
  await editor(page).getByLabel('Target locus', { exact: true }).fill(service.target);
  await editor(page).getByLabel('Reason for knowledge change').fill('Capture evidence as a new concept.');
  await check(page);
  await expect(review(page)).toContainText('New item: the service must establish its identity');
  const artifact = await exported(page, testInfo);
  expect(artifact.arguments.kind).toBe('concept');
  expect(artifact.arguments).not.toHaveProperty('id');
  expect(artifact.arguments).not.toHaveProperty('accepted');
  expect(artifact.operation).toBe('node.propose');
});

test('Relationship drafts preserve direction and verify the other exact visible endpoint', async ({ page, service }, testInfo) => {
  await open(page, service, { action: 'edge.link' });
  await editor(page).getByLabel('Other knowledge identity').fill(service.predecessor);
  await editor(page).getByLabel('Relationship direction').selectOption('incoming');
  await editor(page).getByLabel('Relationship label').fill('clarifies');
  await editor(page).getByLabel('Reason for knowledge change').fill('Retain the direction of the evidence.');
  await check(page);
  await expect(review(page)).toContainText('Related item verified · Earlier support goal');
  const artifact = await exported(page, testInfo);
  expect(artifact.arguments.from_id).toBe(service.predecessor);
  expect(artifact.arguments.to_id).toBe(service.knowledge);
  expect(artifact.arguments.rel).toBe('clarifies');
  await editor(page).getByLabel('Relationship direction').selectOption('outgoing');
  await expect(review(page)).toBeEmpty();
});

test('Retirement, exact relationship removal and binding operations remain distinct proposals', async ({ page, service }, testInfo) => {
  await open(page, service, { action: 'node.retire' });
  await editor(page).getByLabel('Reason for knowledge change').fill('Retire after a governed review.');
  await check(page);
  const retirement = await exported(page, testInfo, 'retirement.json');
  expect(retirement.arguments.id).toBe(service.knowledge);
  expect(retirement.arguments).not.toHaveProperty('text');
  await editor(page).getByLabel('Change kind').selectOption('edge.unlink');
  const edge = await editor(page).getByLabel('Relationship to remove').inputValue();
  await editor(page).getByLabel('Reason for knowledge change').fill('Remove the exact stale relationship.');
  await check(page);
  expect((await exported(page, testInfo, 'unlink.json')).arguments.id).toBe(edge);
  await editor(page).getByLabel('Change kind').selectOption('binding.bind');
  await editor(page).getByLabel('Target locus', { exact: true }).fill('org/support');
  await editor(page).getByLabel('Reason for knowledge change').fill('Request wider applicability.');
  await expect(editor(page).getByLabel('Knowledge draft preview')).toContainText('Binding class: determined by the service');
  await check(page);
  const binding = await exported(page, testInfo, 'bind.json');
  expect(binding.arguments.target).toBe('org/support');
  expect(binding.arguments).not.toHaveProperty('class');
  await editor(page).getByLabel('Change kind').selectOption('binding.unbind');
  const id = await editor(page).getByLabel('Binding to remove').inputValue();
  await editor(page).getByLabel('Reason for knowledge change').fill('Remove applicability after service review.');
  await check(page);
  expect((await exported(page, testInfo, 'unbind.json')).arguments.id).toBe(id);
});

test('A protected endpoint cannot be confirmed, and stale Record snapshots clear candidate text', async ({ page, service }) => {
  await open(page, service, { action: 'edge.link' });
  await editor(page).getByLabel('Other knowledge identity').fill(service.hidden);
  await editor(page).getByLabel('Relationship label').fill('unknown');
  await editor(page).getByLabel('Reason for knowledge change').fill('This target must remain unavailable.');
  await editor(page).getByRole('button', { name: 'Review knowledge draft' }).click();
  await expect(editor(page).getByRole('button', { name: 'Prepare knowledge change' })).toBeVisible();
  await expect(page.locator('#content')).not.toContainText('Protected knowledge title');
  await expect(page.locator('#content')).not.toContainText('Protected knowledge body');
  await editor(page).getByRole('button', { name: 'Prepare knowledge change' }).click();
  await editor(page).getByLabel('Target locus', { exact: true }).fill(service.target);
  await editor(page).getByLabel('Reason for knowledge change').fill('Draft that should be cleared.');
  await service.mutate('advance');
  await editor(page).getByRole('button', { name: 'Review knowledge draft' }).click();
  await expect(editor(page).getByRole('button', { name: 'Prepare knowledge change' })).toBeVisible();
  await expect(editor(page).getByLabel('Knowledge text')).toHaveCount(0);
});

test('Review failures retain editable text without exports; session expiry clears it', async ({ page, service }) => {
  await open(page, service, { action: 'node.retire' });
  await editor(page).getByLabel('Reason for knowledge change').fill('Retain this explanation while disconnected.');
  const endpoint = '**/api/hale/v1/applications/*/capabilities';
  await page.route(endpoint, route => route.fulfill({ status: 503, json: errorBody('record_unavailable', 'Service is offline') }));
  await editor(page).getByRole('button', { name: 'Review knowledge draft' }).click();
  await expect(editor(page).getByRole('status')).toContainText('Service is offline');
  await expect(editor(page).getByLabel('Reason for knowledge change')).toHaveValue('Retain this explanation while disconnected.');
  await expect(review(page)).toBeEmpty();
  await page.unroute(endpoint);
  await check(page);
  await page.route(endpoint, route => route.fulfill({ status: 401, json: errorBody('unauthenticated', 'Sign in again') }));
  await editor(page).getByRole('button', { name: 'Review knowledge draft' }).click();
  await expect(editor(page)).toHaveCount(0);
  await expect(page.locator('#content')).toContainText('Sign in');
});

test('Draft changes invalidate an in-flight review, and mobile navigation clears the draft', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, service, { action: 'node.retire' });
  await editor(page).getByLabel('Reason for knowledge change').fill('First explanation.');
  let release, entered;
  const held = new Promise(resolve => { release = resolve; }), started = new Promise(resolve => { entered = resolve; });
  await page.route('**/api/hale/v1/applications/*/capabilities', async route => {
    const response = await route.fetch(); entered(); await held; await route.fulfill({ response });
  });
  await editor(page).getByRole('button', { name: 'Review knowledge draft' }).click(); await started;
  await editor(page).getByLabel('Reason for knowledge change').fill('New explanation.'); release();
  await expect(review(page)).toBeEmpty();
  await page.unroute('**/api/hale/v1/applications/*/capabilities');
  await check(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await editor(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('knowledge-editor-mobile.png') });
  await page.getByRole('link', { name: 'Practices', exact: true }).click();
  await page.getByRole('link', { name: 'Knowledge', exact: true }).click();
  await expect(editor(page).getByLabel('Reason for knowledge change')).toHaveCount(0);
});
