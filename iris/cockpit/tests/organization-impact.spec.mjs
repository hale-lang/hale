// Native draft projections establish the checked source shape. These interaction
// cases do not claim publication, live reassignment or execution changes.
import { isDeepStrictEqual } from 'node:util';
import { test, expect } from './harness.mjs';

test.use({ organization: true, organizationDrafts: true });

const editor = page => page.getByRole('region', { name: 'Organization editing', exact: true });
const form = page => page.getByRole('region', { name: 'Structured organization editor', exact: true });
const validation = page => page.getByRole('region', { name: 'Organization validation', exact: true });
const source = page => editor(page).getByLabel('Organization Hale source', { exact: true });
const chart = page => validation(page).getByLabel('Checked candidate chart', { exact: true });
const inspector = page => chart(page).getByRole('group', { name: 'Selected instance impact', exact: true });
const instance = (page, id) => chart(page).getByRole('button', { name: id, exact: true });
const plane = (page, name) => chart(page).getByRole('button', { name, exact: true });
const isDraftResponse = method => response => response.request().method() === method
  && new URL(response.url()).pathname.endsWith('/dna/organization/draft');
const ids = rows => rows.map(row => row.id).sort();
const sortBy = key => (a, b) => a[key].localeCompare(b[key]);

async function checkedSharedMove(page, service) {
  await page.goto(service.url('organization', { id: 'Org.metrics' }));
  const reading = page.waitForResponse(isDraftResponse('GET'));
  await page.getByRole('button', { name: 'Edit this instance', exact: true }).click();
  const readResponse = await reading;
  expect(readResponse.status(), await readResponse.text()).toBe(200);
  const before = (await readResponse.json()).data;
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue('Org.metrics');
  const move = form(page).getByRole('group', { name: 'Move to another parent', exact: true });
  await move.getByLabel('Destination parent', { exact: true }).selectOption('Org.support');
  await move.getByLabel('Child name after move', { exact: true }).fill('telemetry');
  await move.getByLabel('I reviewed both parent declarations', { exact: true }).check();
  const checking = page.waitForResponse(isDraftResponse('POST'));
  await move.getByRole('button', { name: 'Validate move', exact: true }).click();
  const checkedResponse = await checking;
  expect(checkedResponse.status(), await checkedResponse.text()).toBe(200);
  const after = (await checkedResponse.json()).data;
  expect(after.validation).toBe('valid_draft');
  await expect(editor(page).getByRole('status')).toContainText('Native organization validation passed');
  await expect(chart(page)).toBeVisible();
  return { before, after };
}

function expectedNodes(before, after) {
  const old = new Map(before.map(row => [row.id, row]));
  const next = new Map(after.map(row => [row.id, row]));
  return [...new Set([...old.keys(), ...next.keys()])].map(id => ({
    id,
    change: !old.has(id) ? 'added' : !next.has(id) ? 'removed'
      : isDeepStrictEqual(old.get(id), next.get(id)) ? 'unchanged' : 'changed',
  })).sort(sortBy('id'));
}

function nativeEdges(rows) {
  return rows.filter(row => row.parent_id).map(row => ({
    key: `${row.parent_id} → ${row.id}`, from: row.parent_id, to: row.id,
  })).sort(sortBy('key'));
}

function expectedEdges(before, after) {
  const old = new Map(nativeEdges(before).map(edge => [edge.key, edge]));
  const next = new Map(nativeEdges(after).map(edge => [edge.key, edge]));
  return [...new Set([...old.keys(), ...next.keys()])].map(key => ({
    ...(next.get(key) || old.get(key)),
    change: !old.has(key) ? 'added' : !next.has(key) ? 'removed' : 'unchanged',
  })).sort(sortBy('key'));
}

async function readNodes(page) {
  return chart(page).locator('button[data-instance-id]').evaluateAll(nodes => nodes.map(node => ({
    id: node.dataset.instanceId, change: node.dataset.change,
  })).sort((a, b) => a.id.localeCompare(b.id)));
}

async function readEdges(page) {
  return chart(page).locator('svg [data-edge-from][data-edge-to]').evaluateAll(edges => edges.map(edge => ({
    key: `${edge.dataset.edgeFrom} → ${edge.dataset.edgeTo}`,
    from: edge.dataset.edgeFrom, to: edge.dataset.edgeTo, change: edge.dataset.change,
  })).sort((a, b) => a.key.localeCompare(b.key)));
}

async function coordinates(page) {
  return chart(page).evaluate(root => {
    const bounds = root.getBoundingClientRect();
    return Object.fromEntries([...root.querySelectorAll('button[data-instance-id]')].map(button => {
      const rect = button.getBoundingClientRect();
      return [button.dataset.instanceId, { x: rect.x - bounds.x, y: rect.y - bounds.y }];
    }));
  });
}

function observeAPI(page) {
  const requests = [];
  page.on('request', request => {
    if (new URL(request.url()).pathname.startsWith('/api/hale/v1/')) requests.push({ method: request.method(), url: request.url() });
  });
  return requests;
}

function branchIds(rows, root) {
  const result = new Set([root]);
  for (let changed = true; changed;) {
    changed = false;
    for (const row of rows) if (result.has(row.parent_id) && !result.has(row.id)) {
      result.add(row.id); changed = true;
    }
  }
  return [...result].sort();
}

test('organization impact: comparison contains exactly the native source and candidate instances and containment edges', async ({ page, service }, testInfo) => {
  const { before, after } = await checkedSharedMove(page, service);
  const oldRows = before.projection.items, newRows = after.projection.items;
  expect(ids(oldRows)).toContain('Org.metrics');
  expect(ids(newRows)).not.toContain('Org.metrics');
  expect(ids(newRows)).toEqual(expect.arrayContaining(['Org.support.telemetry', 'Org.assurance.telemetry']));
  await expect(plane(page, 'Compare changes')).toHaveAttribute('aria-pressed', 'true');
  await expect(chart(page).locator('button[data-instance-id]')).toHaveCount(expectedNodes(oldRows, newRows).length);
  expect(await readNodes(page)).toEqual(expectedNodes(oldRows, newRows));
  expect(await readEdges(page)).toEqual(expectedEdges(oldRows, newRows));
  await expect(instance(page, 'Org.metrics')).toHaveAttribute('data-change', 'removed');
  await expect(instance(page, 'Org.support.telemetry')).toHaveAttribute('data-change', 'added');
  await expect(instance(page, 'Org.assurance.telemetry')).toHaveAttribute('data-change', 'added');
  await chart(page).screenshot({ path: testInfo.outputPath('organization-shared-move-impact.png') });
});

test('organization impact: plane changes preserve instance positions, source and project without API traffic', async ({ page, service }) => {
  const project = await service.projectState();
  const { before, after } = await checkedSharedMove(page, service);
  const unionCoordinates = await coordinates(page);
  const requests = observeAPI(page);
  for (const [name, rows] of [['Current source', before.projection.items], ['Checked candidate', after.projection.items]]) {
    await plane(page, name).click();
    await expect(plane(page, name)).toHaveAttribute('aria-pressed', 'true');
    expect((await readNodes(page)).map(row => row.id)).toEqual(ids(rows));
    expect((await readEdges(page)).map(({ key, from, to }) => ({ key, from, to }))).toEqual(nativeEdges(rows));
    const currentCoordinates = await coordinates(page);
    for (const id of ids(rows)) {
      expect(Math.abs(currentCoordinates[id].x - unionCoordinates[id].x), `${name}: ${id} horizontal position`).toBeLessThanOrEqual(0.5);
      expect(Math.abs(currentCoordinates[id].y - unionCoordinates[id].y), `${name}: ${id} vertical position`).toBeLessThanOrEqual(0.5);
    }
  }
  await plane(page, 'Compare changes').click();
  expect(await coordinates(page)).toEqual(unionCoordinates);
  expect(await source(page).inputValue()).toBe(after.module.text);
  expect(requests).toEqual([]);
  expect(await service.projectState()).toEqual(project);
});

test('organization impact: inspect removed and added instances in context and edit only a checked candidate instance', async ({ page, service }) => {
  const { before, after } = await checkedSharedMove(page, service);
  const requests = observeAPI(page);
  await instance(page, 'Org.metrics').click();
  await expect(inspector(page)).toContainText('Org.metrics');
  await expect(inspector(page).getByRole('columnheader', { name: 'Current source', exact: true })).toBeVisible();
  await expect(inspector(page).getByRole('columnheader', { name: 'Checked candidate', exact: true })).toBeVisible();
  const oldMetrics = before.projection.items.find(row => row.id === 'Org.metrics');
  expect(await inspector(page).locator('tbody tr').evaluateAll(rows => rows.map(row => [...row.children].map(cell => cell.textContent)))).toEqual([
    ['Within', oldMetrics.parent_id, 'Absent'],
    ['Declaration', oldMetrics.declaration, 'Absent'],
    ['Role', oldMetrics.role, 'Absent'],
  ]);
  await expect(inspector(page).getByRole('button', { name: 'Edit candidate instance', exact: true })).toBeDisabled();

  await instance(page, 'Org.support.telemetry').click();
  await expect(inspector(page)).toContainText('Org.support.telemetry');
  const telemetry = after.projection.items.find(row => row.id === 'Org.support.telemetry');
  expect(await inspector(page).locator('tbody tr').evaluateAll(rows => rows.map(row => [...row.children].map(cell => cell.textContent)))).toEqual([
    ['Within', 'Absent', telemetry.parent_id],
    ['Declaration', 'Absent', telemetry.declaration],
    ['Role', 'Absent', telemetry.role],
  ]);
  await inspector(page).getByRole('button', { name: 'Edit candidate instance', exact: true }).click();
  await expect(form(page).getByLabel('Instance to edit', { exact: true })).toHaveValue('Org.support.telemetry');
  expect(await source(page).inputValue()).toBe(after.module.text);
  expect(requests).toEqual([]);
});

test('organization impact: entering a branch retains the native neighborhood and returns to the whole comparison', async ({ page, service }) => {
  const { before, after } = await checkedSharedMove(page, service);
  const rows = [...before.projection.items, ...after.projection.items];
  const requests = observeAPI(page);
  await instance(page, 'Org.support').click();
  await inspector(page).getByRole('button', { name: 'Enter this branch', exact: true }).click();
  const expectedIds = branchIds(rows, 'Org.support');
  expect((await readNodes(page)).map(row => row.id)).toEqual(expectedIds);
  expect(await readEdges(page)).toEqual(expectedEdges(before.projection.items, after.projection.items)
    .filter(edge => expectedIds.includes(edge.from) && expectedIds.includes(edge.to)));
  await expect(instance(page, 'Org.assurance.telemetry')).toHaveCount(0);
  await expect(instance(page, 'Org.support.telemetry')).toBeVisible();
  await chart(page).getByRole('button', { name: 'Whole organization', exact: true }).click();
  expect(await readNodes(page)).toEqual(expectedNodes(before.projection.items, after.projection.items));
  await expect(plane(page, 'Compare changes')).toHaveAttribute('aria-pressed', 'true');
  expect(await source(page).inputValue()).toBe(after.module.text);
  expect(requests).toEqual([]);
});

test('organization impact: narrow reduced-motion planes and instance inspection support the keyboard without overflow', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { after } = await checkedSharedMove(page, service);
  const requests = observeAPI(page);
  await plane(page, 'Current source').focus();
  await page.keyboard.press('Space');
  await expect(plane(page, 'Current source')).toHaveAttribute('aria-pressed', 'true');
  await expect(plane(page, 'Current source')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(plane(page, 'Checked candidate')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(plane(page, 'Checked candidate')).toHaveAttribute('aria-pressed', 'true');
  expect((await readNodes(page)).map(row => row.id)).toEqual(ids(after.projection.items));
  await instance(page, 'Org.support.telemetry').focus();
  await page.keyboard.press('Enter');
  await expect(inspector(page)).toContainText('Org.support.telemetry');
  await expect(instance(page, 'Org.support.telemetry')).toBeFocused();
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
  expect(requests).toEqual([]);
  await chart(page).screenshot({ path: testInfo.outputPath('organization-impact-narrow-keyboard.png') });
});

test('organization impact: a source edit invalidates the checked chart, instance actions and export', async ({ page, service }) => {
  const project = await service.projectState();
  const { after } = await checkedSharedMove(page, service);
  await instance(page, 'Org.support.telemetry').click();
  await expect(inspector(page)).toBeVisible();
  await expect(validation(page).getByRole('button', { name: 'Download validated Hale', exact: true })).toBeVisible();
  const requests = observeAPI(page);
  await editor(page).getByText('Hale source', { exact: true }).click();
  await source(page).fill(after.module.text + '\n// A newer unvalidated source draft.\n');
  await expect(chart(page)).toHaveCount(0);
  await expect(inspector(page)).toHaveCount(0);
  await expect(validation(page).getByRole('button', { name: 'Download validated Hale', exact: true })).toHaveCount(0);
  await expect(form(page)).toHaveCount(0);
  await expect(validation(page)).toBeEmpty();
  expect(requests).toEqual([]);
  expect(await service.projectState()).toEqual(project);
});
