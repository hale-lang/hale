import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { test, expect, errorBody } from './harness.mjs';

test.use({ organization: true, organizationDrafts: true });
const editor = page => page.getByRole('region', { name: 'Organization editing', exact: true });
const source = page => editor(page).getByLabel('Organization Hale source', { exact: true });
const validation = page => editor(page).getByRole('region', { name: 'Organization validation', exact: true });
const endpoint = '**/api/hale/v1/applications/*/dna/organization/draft';
const hash = value => 'sha256:' + createHash('sha256').update(value).digest('hex');
async function open(page, service) {
  await page.goto(service.url('organization'));
  await editor(page).getByRole('button', { name: 'Edit organization source', exact: true }).click();
  await expect(source(page)).toBeFocused();
  return source(page).inputValue();
}
async function submit(page) {
  const response = page.waitForResponse(r => r.request().method() === 'POST' && r.url().endsWith('/dna/organization/draft'));
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  return response;
}
const addPosition = text => text.replace('        metrics: Metrics', '        billing: Support = Support { };\n        metrics: Metrics');

test('Organization editor validates real Hale, previews static impact and exports exact preserved source', async ({ page, service }, testInfo) => {
  const state = await service.projectState();
  const baseline = await open(page, service);
  const candidate = addPosition(baseline) + '\n// Handwritten comment: café 🧬 <img src=x onerror="window.injected=true">\n';
  await source(page).fill(candidate);
  await expect(editor(page).getByLabel('Proposed source changes')).toContainText('+         billing: Support');
  const response = await submit(page);
  expect(response.status()).toBe(200);
  const data = (await response.json()).data;
  expect(data.validation).toBe('valid_draft');
  expect(data.module.text).toBe(candidate);
  expect(data.module.digest).toBe(hash(candidate));
  expect(data.projection.items).toHaveLength(9);
  await expect(editor(page).getByRole('status')).toContainText('Native organization validation passed');
  await expect(validation(page)).toContainText('2 added · 0 removed · 1 changed');
  await expect(validation(page).getByLabel('Checked candidate chart').getByRole('button', { name: 'Org.billing.reviewer', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  await expect(editor(page).locator('img')).toHaveCount(0);
  const download = page.waitForEvent('download');
  await validation(page).getByRole('button', { name: 'Download validated Hale', exact: true }).click();
  const file = await download, target = testInfo.outputPath('main.hl');
  await file.saveAs(target);
  expect(await readFile(target, 'utf8')).toBe(candidate);
  expect(file.suggestedFilename()).toBe('main.hl');
  expect(await service.projectState()).toEqual(state);
  const saved = await page.request.get(service.origin + service.apiPath + '/dna/organization/draft');
  expect((await saved.json()).data.module.text).toBe(baseline);
  expect(await page.evaluate(() => Object.values(localStorage).some(value => value.includes('Handwritten comment')))).toBe(false);
  await editor(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('organization-draft.png'), fullPage: true });
});

test('Unchanged source roundtrips; any edit invalidates the export and reset preserves handwritten code', async ({ page, service }) => {
  const baseline = await open(page, service);
  expect((await submit(page)).status()).toBe(200);
  await expect(validation(page)).toContainText('0 added · 0 removed · 0 changed');
  await source(page).fill(baseline + '\n// draft\n');
  await expect(validation(page).getByRole('button', { name: 'Download validated Hale' })).toHaveCount(0);
  await editor(page).getByRole('button', { name: 'Discard draft', exact: true }).click();
  await expect(source(page)).toHaveValue(baseline);
  await expect(validation(page)).toBeEmpty();
});

test('Invalid Hale is refused and corrected source can be checked without changing the project', async ({ page, service }) => {
  const before = await service.projectState(), baseline = await open(page, service);
  await source(page).fill('main locus Broken { invalid source\n');
  expect((await submit(page)).status()).toBe(422);
  await expect(editor(page).getByRole('status')).toContainText('could not be checked');
  await expect(validation(page)).toBeEmpty();
  await source(page).fill(addPosition(baseline));
  expect((await submit(page)).status()).toBe(200);
  await expect(validation(page)).toContainText('Org.billing');
  expect(await service.projectState()).toEqual(before);
});

test('Source and Record races clear the editor instead of exporting a stale proposal', async ({ page, service }) => {
  const baseline = await open(page, service);
  await source(page).fill(addPosition(baseline));
  await service.changeOrganization();
  expect((await submit(page)).status()).toBe(409);
  await expect(source(page)).toHaveCount(0);
  await expect(editor(page).getByRole('button', { name: 'Edit organization source', exact: true })).toBeEnabled();
  await editor(page).getByRole('button', { name: 'Edit organization source', exact: true }).click();
  await expect(source(page)).toBeVisible();
  await service.mutate('append');
  expect((await submit(page)).status()).toBe(409);
  await expect(source(page)).toHaveCount(0);
});

test('Editing during a check discards its late result; navigation discards drafts', async ({ page, service }) => {
  const baseline = await open(page, service);
  let release, entered;
  const held = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  await page.route(endpoint, async route => {
    if (route.request().method() !== 'POST') return route.continue();
    const response = await route.fetch(); entered(); await held; await route.fulfill({ response });
  });
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  await started;
  await source(page).fill(baseline + '\n// newer edit\n');
  release();
  await expect(validation(page)).toBeEmpty();
  await page.getByRole('link', { name: 'Practices', exact: true }).click();
  await page.getByRole('link', { name: 'Organization', exact: true }).click();
  await expect(source(page)).toHaveCount(0);
});

test('Forged digests and changed authentication cannot enable export', async ({ page, service }) => {
  await open(page, service);
  await page.route(endpoint, async route => {
    const response = await route.fetch(), value = await response.json();
    if (route.request().method() === 'POST') value.data.module.digest = 'sha256:' + '0'.repeat(64);
    await route.fulfill({ response, json: value });
  });
  await submit(page);
  await expect(editor(page).getByRole('status')).toContainText('could not be verified');
  await expect(validation(page)).toBeEmpty();
  await page.unroute(endpoint);
  await page.route(endpoint, route => route.fulfill({ status: 401, json: errorBody('unauthenticated', 'Sign in again') }));
  await submit(page);
  await expect(source(page)).toHaveCount(0);
  await expect(page.locator('#content')).toContainText('Sign in');
});

test('Native draft boundary requires exact identity, source basis, framing and module allowlist', async ({ page, service }) => {
  const before = await service.projectState();
  const url = service.origin + service.apiPath + '/dna/organization/draft';
  const read = await page.request.get(url), data = (await read.json()).data;
  const request = { profile: data.profile, application_id: service.application, principal: data.principal, base: data.base, source_text: data.module.text };
  const headers = { Origin: service.origin, 'X-Hale-Command': '1', 'Content-Type': 'application/json' };
  async function post(body = request, overrides = {}, query = '') { return page.request.post(url + query, { headers: { ...headers, ...overrides }, data: JSON.stringify(body) }); }
  expect((await post(request, { Origin: 'http://example.invalid' })).status()).toBe(403);
  expect((await post(request, { 'X-Hale-Command': '0' })).status()).toBe(400);
  expect((await post(request, { 'Content-Type': 'text/plain' })).status()).toBe(415);
  expect((await post({ ...request, principal: { ...data.principal, name: 'someone-else' } })).status()).toBe(409);
  expect((await post({ ...request, path: '/tmp/other.hl' })).status()).toBe(400);
  expect((await post({ ...request, base: { ...data.base, module_digest: hash('another') } })).status()).toBe(409);
  expect((await post({ ...request, source_text: 'x'.repeat(16385) })).status()).toBe(413);
  expect((await post(request, {}, '?snapshot=x')).status()).toBe(400);
  expect((await post({ ...request, source_text: 'import "/tmp/iris-outside-not-captured" as outside;\n' + data.module.text })).status()).not.toBe(200);
  expect(await service.projectState()).toEqual(before);
});

test.describe('Host opt-in', () => {
  test.use({ organizationDrafts: false });
  test('Reading the chart does not opt the host into source disclosure or drafting', async ({ page, service }) => {
    await page.goto(service.url('organization'));
    await expect(editor(page).getByRole('button', { name: 'Edit organization source' })).toBeDisabled();
    const response = await page.request.get(service.origin + service.apiPath + '/dna/organization/draft');
    expect(response.status()).toBe(503);
    expect((await response.json()).error.code).toBe('organization_drafts_unsupported');
  });
});
