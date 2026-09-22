import { readFile, writeFile } from 'node:fs/promises';
import { test, expect, errorBody } from './harness.mjs';

test.use({ organization: true, organizationDrafts: true });
const editor = page => page.getByRole('region', { name: 'Ownership editing', exact: true });
const form = page => editor(page).getByRole('region', { name: 'Ownership map editor', exact: true });
const source = page => editor(page).getByLabel('Ownership source', { exact: true });
const evidence = page => editor(page).getByRole('region', { name: 'Ownership validation', exact: true });
const endpoint = '**/api/hale/v1/applications/*/dna/organization/ownership/draft';
const path = service => service.origin + service.apiPath + '/dna/organization/ownership/draft';
async function open(page, service) {
  await page.goto(service.url('organization'));
  await editor(page).getByRole('button', { name: 'Edit ownership', exact: true }).click();
  await expect(form(page)).toBeVisible();
  return source(page).inputValue();
}
async function apply(page, name) {
  const response = page.waitForResponse(r => r.request().method() === 'POST' && r.url().endsWith('/dna/organization/ownership/draft'));
  await editor(page).getByRole('button', { name, exact: true }).click();
  const result = await response;
  expect(result.status(), await result.text()).toBe(200);
  await expect(editor(page).getByRole('status')).toContainText('Native ownership validation passed');
  return result.json();
}
function payload(service, data, source_text) {
  return { profile: data.profile, application_id: service.application, principal: data.principal, base: data.base, source_text };
}

test('Ownership assignment preserves exact source and exports native review impact', async ({ page, service }, testInfo) => {
  const original = '# Ownership for équipe\r\norg = acme\r\norg/support = partner\r\nacme: alice\r\npartner: bob\r\nhost = acme\r\n';
  await service.changeOwnership(original);
  const before = await service.projectState(); expect(await open(page, service)).toBe(original.replace(/\r\n/g, '\n'));
  const assignment = form(page).getByRole('group', { name: 'Scope assignment', exact: true });
  await assignment.getByLabel('Assignment to edit').selectOption({ label: 'org/support' });
  await assignment.getByLabel('Assigned owner').fill('acme');
  const response = await apply(page, 'Validate assignment'), data = response.data;
  expect(data.module.text).toBe(original.replace('org/support = partner', 'org/support = acme'));
  expect(data.impact.affected_owners).toEqual([{ owner: 'acme', members: ['alice'] }, { owner: 'partner', members: ['bob'] }]);
  expect(data.impact.scopes).toContainEqual({ position: 'org/support', before_owner: 'partner', after_owner: 'acme' });
  await expect(evidence(page)).toContainText('Affected owner reviews');
  const download = page.waitForEvent('download');
  await evidence(page).getByRole('button', { name: 'Download validated ownership map', exact: true }).click();
  const file = testInfo.outputPath('owners'); await (await download).saveAs(file);
  expect(await readFile(file, 'utf8')).toBe(data.module.text);
  await writeFile(testInfo.outputPath('ownership-response.json'), JSON.stringify(response, null, 2));
  expect(await service.projectState()).toEqual(before);
  // Draft scopes do not replace the actual working-context selector.
  await expect(page.getByLabel('Working locus', { exact: true })).toHaveValue('');
});

test('Assignment removal previews native inherited ownership and single-owner mode', async ({ page, service }) => {
  await open(page, service);
  let assignment = form(page).getByRole('group', { name: 'Scope assignment', exact: true });
  await assignment.getByLabel('Assignment to edit').selectOption({ label: 'org/support' });
  const removed = (await apply(page, 'Validate assignment removal')).data;
  expect(removed.impact.scopes).toContainEqual({ position: 'org/support', before_owner: 'partner', after_owner: 'acme' });
  assignment = form(page).getByRole('group', { name: 'Scope assignment', exact: true });
  await assignment.getByLabel('Assignment to edit').selectOption({ label: 'org' });
  const single = (await apply(page, 'Validate assignment removal')).data;
  expect(single.ownership.mode).toBe('single_owner'); expect(single.impact.mode_changed).toBe(true);
  await expect(evidence(page)).toContainText('With no position assignments, DNA uses single-owner admission');
});

test('Membership and hosting edits use native affected-owner review membership', async ({ page, service }) => {
  await open(page, service);
  const membership = form(page).getByRole('group', { name: 'Owner membership', exact: true });
  await membership.getByLabel('Membership to edit').selectOption({ label: 'partner' });
  await membership.getByLabel('Members', { exact: true }).fill('bob, carol');
  const changed = (await apply(page, 'Validate membership')).data;
  expect(changed.impact.affected_owners).toEqual([{ owner: 'partner', members: ['bob', 'carol'] }]);
  await form(page).getByLabel('Host owner', { exact: true }).fill('neutral');
  const hosted = (await apply(page, 'Validate hosting party')).data;
  expect(hosted.ownership.host_owner).toBe('neutral'); expect(hosted.impact.host_changed).toBe(true);
  expect(hosted.impact.affected_owners.map(row => row.owner)).toEqual(['acme', 'partner']);
  await membership.getByLabel('Membership to edit').selectOption({ label: 'partner' });
  const removed = (await apply(page, 'Validate membership removal')).data;
  expect(removed.impact.affected_owners.find(row => row.owner === 'partner').members).toEqual(['bob']);
});

test('New scopes and memberships preserve Unicode without HTML interpretation', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, service);
  const assignment = form(page).getByRole('group', { name: 'Scope assignment', exact: true });
  await assignment.getByLabel('Scope path').fill('org/équipe'); await assignment.getByLabel('Assigned owner').fill('north');
  const scoped = (await apply(page, 'Validate assignment')).data;
  expect(scoped.ownership.positions).toContainEqual({ position: 'org/équipe', owner: 'north' });
  expect(scoped.impact.affected_owners.find(row => row.owner === 'north').members).toEqual([]);
  await expect(evidence(page)).toContainText('No review members declared');
  const membership = form(page).getByRole('group', { name: 'Owner membership', exact: true });
  await membership.getByLabel('Owner name').fill('north'); await membership.getByLabel('Members', { exact: true }).fill('Élodie, <script>');
  const member = (await apply(page, 'Validate membership')).data;
  expect(member.ownership.memberships).toContainEqual({ owner: 'north', members: ['Élodie', '<script>'] });
  await expect(evidence(page).locator('script')).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await evidence(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('ownership-mobile.png') });
});

test('Raw source invalidates exports and native syntax refusals retain the draft', async ({ page, service }) => {
  const original = await open(page, service);
  await apply(page, 'Validate ownership map');
  await editor(page).getByText('Ownership source', { exact: true }).click();
  await source(page).fill(original + 'org/support/ = duplicate\n');
  await expect(form(page)).toHaveCount(0); await expect(evidence(page)).toBeEmpty();
  await editor(page).getByRole('button', { name: 'Validate ownership map', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('normalized position is declared more than once');
  await expect(source(page)).toHaveValue(original + 'org/support/ = duplicate\n');
  await source(page).fill(original + '# preserved comment\n');
  const validated = (await apply(page, 'Validate ownership map')).data;
  expect(validated.impact.affected_owners).toEqual([]); await expect(form(page)).toBeVisible();
});

test('A committed ownership race clears forms and candidates before reloading', async ({ page, service }) => {
  const original = await open(page, service);
  await service.changeOwnership(original.replace('partner: bob', 'partner: carol'));
  await editor(page).getByRole('button', { name: 'Validate ownership map', exact: true }).click();
  await expect(source(page)).toHaveCount(0); await expect(form(page)).toHaveCount(0);
  await expect(page.getByText('carol', { exact: true })).toBeVisible();
});

test('Access loss clears the ownership draft and editing controls', async ({ page, service }) => {
  await open(page, service);
  await page.route(endpoint, route => route.fulfill({ status: 401, json: errorBody('unauthenticated', 'Sign in again') }));
  await editor(page).getByRole('button', { name: 'Validate ownership map', exact: true }).click();
  await expect(source(page)).toHaveCount(0); await expect(page.getByRole('button', { name: 'Edit ownership', exact: true })).toHaveCount(0);
});

test('Ownership draft HTTP boundary binds principal and source and rejects invalid input', async ({ page, service }) => {
  const before = await service.projectState();
  const get = await page.request.get(path(service)); expect(get.status()).toBe(200); const data = (await get.json()).data;
  const body = payload(service, data, data.module.text);
  const headers = { Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' };
  for (const [change, status] of [[{ principal: { mode: 'local', name: 'different-person' } }, 409], [{ base: { ...data.base, module_digest: 'changed' } }, 409], [{ extra: true }, 400], [{ source_text: 'not an ownership statement' }, 422], [{ source_text: 'host = a\nhost = b\n' }, 422], [{ source_text: 'x'.repeat(16385) }, 413]]) {
    const response = await page.request.post(path(service), { headers, data: { ...body, ...change } }); expect(response.status(), await response.text()).toBe(status);
  }
  const foreign = await page.request.post(path(service), { headers: { ...headers, Origin: 'https://example.invalid' }, data: body }); expect(foreign.status()).toBe(403);
  const query = await page.request.get(path(service) + '?ignored=1'); expect(query.status()).toBe(400);
  expect(await service.projectState()).toEqual(before);
});

test.describe('Ownership source opt-in', () => {
  test.use({ organizationDrafts: false });
  test('Unavailable preparation stays visible and disabled', async ({ page, service }) => {
    await page.goto(service.url('organization'));
    await expect(editor(page).getByRole('button', { name: 'Edit ownership', exact: true })).toBeDisabled();
    const response = await page.request.get(path(service)); expect(response.status()).toBe(503);
  });
});
