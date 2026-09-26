import { readFile } from 'node:fs/promises';
import { test, expect } from './harness.mjs';
import { isDescribe } from './command-wire.mjs';

test.use({ organization: true, organizationDrafts: true });
const editor = page => page.getByRole('region', { name: 'Organization editing', exact: true });
const form = page => page.getByRole('region', { name: 'Structured organization editor', exact: true });
const validation = page => page.getByRole('region', { name: 'Organization validation', exact: true });
const source = page => editor(page).getByLabel('Organization Hale source', { exact: true });
const endpoint = '**/api/hale/v1/applications/*/dna/organization/draft';
async function open(page, service, id = 'Org') {
  await page.goto(service.url('organization', { id }));
  await page.getByRole('button', { name: 'Edit this instance', exact: true }).click();
  await expect(form(page)).toBeVisible();
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue(id);
  return source(page).inputValue();
}
async function apply(page, name) {
  const response = page.waitForResponse(r => r.request().method() === 'POST' && r.url().endsWith('/dna/organization/draft'));
  await form(page).getByRole('button', { name, exact: true }).click();
  const result = await response;
  expect(result.status(), await result.text()).toBe(200);
  await expect(editor(page).getByRole('status')).toContainText('Native organization validation passed');
  return (await result.json()).data;
}

test('Structured organization adds a position, validates the real chart and exports preserved Hale', async ({ page, service }, testInfo) => {
  const before = await service.projectState(), baseline = await open(page, service);
  await form(page).getByLabel('New child name', { exact: true }).fill('billing');
  await form(page).getByLabel('Child declaration', { exact: true }).selectOption('Reviewer');
  const data = await apply(page, 'Validate new child');
  expect(data.projection.items.find(row => row.id === 'Org.billing')).toMatchObject({ parent_id: 'Org', declaration: 'Reviewer', role: 'position' });
  expect(data.module.text).toContain('billing: Reviewer = Reviewer { };');
  // Existing declarations, bodies and comments remain byte-for-byte intact.
  expect(data.module.text.slice(0, data.module.text.indexOf('main locus Org'))).toBe(baseline.slice(0, baseline.indexOf('main locus Org')));
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue('Org.billing');
  await expect(validation(page).getByLabel('Checked candidate chart').getByRole('button', { name: 'Org.billing', exact: true })).toBeVisible();
  const download = page.waitForEvent('download');
  await validation(page).getByRole('button', { name: 'Download validated Hale', exact: true }).click();
  const output = testInfo.outputPath('structured-organization.hl'); await (await download).saveAs(output);
  expect(await readFile(output, 'utf8')).toBe(data.module.text);
  expect(await service.projectState()).toEqual(before);
  await editor(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('structured-organization-desktop.png'), fullPage: true });
});

test('Shared fields show every affected instance before rename and preserve unrelated source', async ({ page, service }) => {
  const baseline = await open(page, service, 'Org.support.reviewer');
  const scope = form(page).getByRole('group', { name: 'Declared instance', exact: true });
  await expect(scope).toContainText('Org.support.reviewer'); await expect(scope).toContainText('Org.assurance.reviewer');
  await form(page).getByLabel('Instance name', { exact: true }).fill('auditor');
  const data = await apply(page, 'Validate instance name');
  const ids = data.projection.items.map(row => row.id);
  expect(ids).toContain('Org.support.auditor'); expect(ids).toContain('Org.assurance.auditor');
  expect(ids).toContain('Org.reviewer'); expect(ids).not.toContain('Org.support.reviewer');
  expect(data.module.text).toBe(baseline.replace('params { reviewer: Reviewer = Reviewer { }; }', 'params { auditor: Reviewer = Reviewer { }; }'));
});

test('Shared declaration defaults and position role affect the native source without claiming authority', async ({ page, service }) => {
  await open(page, service, 'Org.support.reviewer');
  const role = form(page).getByRole('group', { name: 'Position declaration', exact: true });
  for (const id of ['Org.reviewer', 'Org.support.reviewer', 'Org.assurance.reviewer']) await expect(role).toContainText(id);
  await form(page).getByLabel('seen default (Int)', { exact: true }).fill('9007199254740993');
  const config = await apply(page, 'Validate declaration defaults');
  expect(config.module.text).toContain('seen: Int = 9007199254740993');
  await form(page).getByLabel('Include this declaration in positions', { exact: true }).uncheck();
  await form(page).getByRole('button', { name: 'Validate position membership', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('explicitly allowing an empty positions group');
  await form(page).getByLabel('Allow an empty positions group', { exact: true }).check();
  const membership = await apply(page, 'Validate position membership');
  expect(membership.module.text).toContain('may_be_empty');
  expect(membership.projection.items.filter(row => row.declaration === 'Reviewer').every(row => row.role === 'structure')).toBe(true);
  await expect(form(page)).toContainText('It establishes no occupant, ownership assignment or command permission');
});

test('Source removal previews the entire subtree and requires explicit draft confirmation', async ({ page, service }) => {
  const before = await service.projectState(); await open(page, service, 'Org.support');
  await form(page).getByText('Remove this source field', { exact: true }).click();
  const removal = form(page).locator('details');
  await expect(removal).toContainText('Org.support.reviewer');
  await expect(form(page).getByRole('button', { name: 'Validate source removal', exact: true })).toBeDisabled();
  await form(page).getByLabel('I reviewed every affected instance', { exact: true }).check();
  const data = await apply(page, 'Validate source removal');
  expect(data.projection.items.some(row => row.id === 'Org.support' || row.id.startsWith('Org.support.'))).toBe(false);
  expect(data.projection.items.some(row => row.id === 'Org.assurance.reviewer')).toBe(true);
  expect(await service.projectState()).toEqual(before);
});

test('Unsafe identifiers, duplicate fields, recursion and Int64 overflow do not submit', async ({ page, service }) => {
  await open(page, service, 'Org.support');
  const posts = []; page.on('request', r => { if (r.method() === 'POST' && !isDescribe(r)) posts.push(r.url()); });
  await form(page).getByLabel('New child name', { exact: true }).fill('reviewer');
  await form(page).getByRole('button', { name: 'Validate new child', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('unique Hale identifier');
  await form(page).getByLabel('New child name', { exact: true }).fill('injected; bad');
  await form(page).getByRole('button', { name: 'Validate new child', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('unique Hale identifier');
  await form(page).getByLabel('Instance to edit', { exact: true }).selectOption('Org.support.reviewer');
  await form(page).getByLabel('New child name', { exact: true }).fill('loop');
  await form(page).getByLabel('Child declaration', { exact: true }).selectOption('Support');
  await form(page).getByRole('button', { name: 'Validate new child', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('recursive static containment');
  await form(page).getByLabel('seen default (Int)', { exact: true }).fill('9223372036854775808');
  await form(page).getByRole('button', { name: 'Validate declaration defaults', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('whole Int64 value');
  expect(posts).toEqual([]);
});

test('Handwritten source refreshes structured bindings only after native validation', async ({ page, service }) => {
  await open(page, service, 'Org.support');
  await editor(page).getByText('Hale source', { exact: true }).click();
  const baseline = await source(page).inputValue();
  const candidate = baseline.replace('params { reviewer: Reviewer = Reviewer { }; }', 'params { /* braces { } and params { fake: X; } */ reviewer: Reviewer = Reviewer { }; }') + '\n// locus Fake { params { x: Int = 9; } }\n';
  await source(page).fill(candidate);
  await expect(form(page)).toHaveCount(0);
  await expect(editor(page)).toContainText('Previous instance bindings are no longer used');
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  await expect(form(page)).toBeVisible();
  await form(page).getByLabel('New child name', { exact: true }).fill('helper');
  await form(page).getByLabel('Child declaration', { exact: true }).selectOption('Reviewer');
  const data = await apply(page, 'Validate new child');
  expect(data.module.text).toContain('/* braces { } and params { fake: X; } */');
  expect(data.module.text).toContain('// locus Fake { params { x: Int = 9; } }');
  expect(data.projection.items.some(row => row.id === 'Org.assurance.helper')).toBe(true);
});

test('Native refusal retains the candidate source; a race clears structured drafts', async ({ page, service }) => {
  await open(page, service, 'Org.support');
  await page.route(endpoint, async route => {
    if (route.request().method() === 'GET') return route.continue();
    await route.fulfill({ status: 422, json: { error: { message: 'The candidate could not be checked.' } } });
  });
  await form(page).getByLabel('Instance name', { exact: true }).fill('support_team');
  await form(page).getByRole('button', { name: 'Validate instance name', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('could not be checked');
  expect(await source(page).inputValue()).toContain('support_team: Support');
  await expect(validation(page)).toBeEmpty();
  await page.unroute(endpoint);
  await service.changeOrganization();
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  await expect(form(page)).toHaveCount(0);
  await expect(source(page)).toHaveCount(0);
});

test('Structured edits support narrow keyboard navigation without horizontal overflow', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, service, 'Org.support.reviewer');
  const name = form(page).getByLabel('Instance name', { exact: true });
  await name.focus(); await name.fill('auditor');
  const submit = form(page).getByRole('button', { name: 'Validate instance name', exact: true });
  await submit.focus(); await page.keyboard.press('Enter');
  await expect(editor(page).getByRole('status')).toContainText('Native organization validation passed');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await editor(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('structured-organization-mobile.png'), fullPage: true });
});

test('String and Boolean defaults preserve literal text through the native compiler', async ({ page, service }) => {
  await open(page, service, 'Org.reviewer');
  await editor(page).getByText('Hale source', { exact: true }).click();
  const original = await source(page).inputValue();
  await source(page).fill(original.replace('seen: Int = 0;', 'seen: Int = 0; purpose: String = "Review"; active: Bool = true;'));
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  await expect(form(page).getByLabel('purpose default (String)', { exact: true })).toBeVisible();
  const purpose = 'Équipe "review" } // text\n<img src=x onerror="window.injected=true">';
  await form(page).getByLabel('purpose default (String)', { exact: true }).fill(purpose);
  await form(page).getByLabel('active default (Bool)', { exact: true }).selectOption('false');
  const data = await apply(page, 'Validate declaration defaults');
  expect(data.module.text).toContain('purpose: String = ' + JSON.stringify(purpose));
  expect(data.module.text).toContain('active: Bool = false');
  await expect(form(page).getByLabel('purpose default (String)', { exact: true })).toHaveValue(purpose);
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  await expect(editor(page).locator('img')).toHaveCount(0);
});

test('Position membership additions and non-final removals preserve the remaining group', async ({ page, service }) => {
  await open(page, service, 'Org.support');
  await form(page).getByLabel('Include this declaration in positions', { exact: true }).check();
  const added = await apply(page, 'Validate position membership');
  expect(added.projection.items.filter(row => row.declaration === 'Support').every(row => row.role === 'position')).toBe(true);
  await form(page).getByLabel('Instance to edit', { exact: true }).selectOption('Org.reviewer');
  await form(page).getByLabel('Include this declaration in positions', { exact: true }).uncheck();
  const removed = await apply(page, 'Validate position membership');
  expect(removed.projection.items.filter(row => row.role === 'position').map(row => row.id).sort()).toEqual(['Org.assurance', 'Org.support']);
  expect(removed.module.text).not.toContain('may_be_empty');
});

test('Moving a shared child preserves its constructor and exports the native checked chart', async ({ page, service }, testInfo) => {
  const before = await service.projectState();
  const original = await open(page, service, 'Org.support.reviewer');
  await editor(page).getByText('Hale source', { exact: true }).click();
  const constructor = 'Reviewer { /* retain this override */ seen: 7 }';
  await source(page).fill(original.replace('params { reviewer: Reviewer = Reviewer { }; }', 'params { reviewer: Reviewer = ' + constructor + '; }'));
  await editor(page).getByRole('button', { name: 'Validate organization', exact: true }).click();
  const move = form(page).getByRole('group', { name: 'Move to another parent', exact: true });
  await expect(move).toBeVisible();
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org');
  await move.getByLabel('Child name after move', { exact: true }).fill('audit');
  await expect(move).toContainText('Org.support.reviewer');
  await expect(move).toContainText('Org.assurance.reviewer');
  await expect(move).toContainText('Org.audit');
  await expect(move.getByRole('button', { name: 'Validate move', exact: true })).toBeDisabled();
  await move.getByLabel('I reviewed both parent declarations', { exact: true }).check();
  const data = await apply(page, 'Validate move');
  expect(data.module.text).toContain('audit: Reviewer = ' + constructor + ';');
  expect(data.projection.items.map(row => row.id).sort()).toEqual(['Org', 'Org.assurance', 'Org.audit', 'Org.metrics', 'Org.reviewer', 'Org.support']);
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue('Org.audit');
  await expect(validation(page).getByLabel('Checked candidate chart').getByRole('button', { name: 'Org.audit', exact: true })).toBeVisible();
  const download = page.waitForEvent('download');
  await validation(page).getByRole('button', { name: 'Download validated Hale', exact: true }).click();
  const file = testInfo.outputPath('moved-organization.hl'); await (await download).saveAs(file);
  expect(await readFile(file, 'utf8')).toBe(data.module.text);
  expect(await service.projectState()).toEqual(before);
});

test('Moving into a shared parent previews every destination and refreshes acknowledgement', async ({ page, service }, testInfo) => {
  const before = await service.projectState(); await open(page, service, 'Org.metrics');
  const move = form(page).getByRole('group', { name: 'Move to another parent', exact: true });
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org.support');
  await expect(move).toContainText('Org.support.metrics'); await expect(move).toContainText('Org.assurance.metrics');
  const reviewed = move.getByLabel('I reviewed both parent declarations', { exact: true });
  await reviewed.check();
  await move.getByLabel('Child name after move', { exact: true }).fill('telemetry');
  await expect(reviewed).not.toBeChecked();
  await expect(move.getByRole('button', { name: 'Validate move', exact: true })).toBeDisabled();
  await reviewed.check();
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org.assurance');
  await expect(reviewed).not.toBeChecked(); await reviewed.check();
  await move.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('organization-move-impact.png') });
  const data = await apply(page, 'Validate move');
  const ids = data.projection.items.map(row => row.id);
  expect(ids).toContain('Org.support.telemetry'); expect(ids).toContain('Org.assurance.telemetry');
  expect(ids).not.toContain('Org.metrics'); expect(ids).toHaveLength(8);
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue('Org.assurance.telemetry');
  expect(await service.projectState()).toEqual(before);
});

test('Moves reject duplicate names and recursive containment without submitting', async ({ page, service }) => {
  await open(page, service, 'Org.metrics');
  const move = form(page).getByRole('group', { name: 'Move to another parent', exact: true });
  const posts = []; page.on('request', r => { if (r.method() === 'POST' && !isDescribe(r)) posts.push(r.url()); });
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org.support');
  await move.getByLabel('Child name after move', { exact: true }).fill('reviewer');
  await move.getByLabel('I reviewed both parent declarations', { exact: true }).check();
  await move.getByRole('button', { name: 'Validate move', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('unique Hale identifier');
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org.metrics');
  await move.getByLabel('Child name after move', { exact: true }).fill('nested');
  await move.getByLabel('I reviewed both parent declarations', { exact: true }).check();
  await move.getByRole('button', { name: 'Validate move', exact: true }).click();
  await expect(form(page).getByRole('alert')).toContainText('recursive static containment');
  expect(posts).toEqual([]);
});
