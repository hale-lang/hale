// Actual native Task reassignment/recovery. Only the delivery-loss case drops
// a genuine POST response; it never substitutes a command outcome.
import { test as base, expect } from '@playwright/test';
import fs from 'node:fs';
import { startTaskService, nativeTaskEnvironmentPresent } from './native-task-harness.mjs';
import { isDescribe, isWrite } from './command-wire.mjs';

// The retire grant lets the person read carry the eligible recipients the
// reassignment picker offers.
const taskPolicy = (application_id, name) => ({ format: 'dna.task-authority/1', application_id, owner: 'operations', members: ['alex', 'blair', 'casey', 'retired'], grants: [{ mode: 'local', name, reassign: true, retire: true, recover: true }] });
const test = base.extend({
  service: async ({}, use, info) => {
    const service = await startTaskService({ taskPolicy });
    try { await use(service); }
    finally { await service.stop(); await info.attach('native-task-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); }
  },
  page: async ({ page, service }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await service.attach(page); await use(page); expect(errors).toEqual([]); },
});
test.skip(!nativeTaskEnvironmentPresent(), 'Supply matching native Task seed and configured command API binaries.');
test.setTimeout(45_000);
const task = page => page.getByRole('region', { name: 'Handed Task administration', exact: true });
const recovery = page => page.getByRole('region', { name: 'Task reassignment request', exact: true });
const confirmation = page => page.getByRole('group', { name: 'Confirm Task reassignment', exact: true });
async function prepare(page, service, to = 'blair') {
  await page.goto(service.url()); await expect(task(page)).toBeVisible();
  await task(page).getByLabel('New assignee', { exact: true }).selectOption(to);
  await task(page).getByRole('button', { name: 'Review reassignment', exact: true }).click();
  await expect(confirmation(page)).toContainText('alex → ' + to);
}
const reassignments = service => service.journal().rows.filter(row => row.kind === 'task.reassigned' && row.entity === service.task);
const stored = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('face.practice-recovery.v1:')).map(([key, value]) => ({ key, value: JSON.parse(value) })));

test('native reassignment preserves the same open Task and joins exact assignment history', async ({ page, service }, info) => {
  const posts = []; page.on('request', request => { if (isWrite(request)) posts.push(request.postDataJSON()); });
  const captured = service.journal().head;
  const one = await service.read(service.prefix + '/dna/tasks?limit=1&snapshot=' + captured);
  const two = await service.read(service.prefix + '/dna/tasks?limit=1&offset=1&snapshot=' + captured);
  expect(one.status).toBe(200); expect(two.status).toBe(200); expect(one.json.data.page.total).toBe(2);
  expect(one.json.data.items[0].id).not.toBe(two.json.data.items[0].id);
  for (const query of ['?offset=1', '?id=' + encodeURIComponent(service.task) + '&offset=1&snapshot=' + captured, '?id=one&id=two', '?position=org']) expect((await service.read(service.prefix + '/dna/tasks' + query)).status).toBe(400);
  expect((await service.read(service.prefix + '/dna/tasks?snapshot=stale')).status).toBe(409);
  expect((await service.read(service.prefix + '/dna/tasks?id=missing')).status).toBe(404);
  expect(service.journal().head).toBe(captured);
  const capabilities = await service.read(service.prefix + '/capabilities');
  fs.writeFileSync(service.evidence + '/capabilities-response.json', JSON.stringify(capabilities.json, null, 2));
  await prepare(page, service); expect(posts).toHaveLength(0); expect(reassignments(service)).toHaveLength(0);
  const reply = page.waitForResponse(r => isWrite(r.request()) && new URL(r.url()).pathname === service.prefix + '/commands');
  await confirmation(page).getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  fs.writeFileSync(service.evidence + '/command-response.json', JSON.stringify(await (await reply).json(), null, 2));
  await expect(recovery(page)).toHaveAttribute('data-observation', 'observed');
  await expect(task(page).locator('.task-current-assignment')).toContainText('blair');
  const exact = await service.read(service.prefix + '/dna/tasks?id=' + encodeURIComponent(service.task));
  fs.writeFileSync(service.evidence + '/task-reassigned-response.json', JSON.stringify(exact.json, null, 2));
  const collection = await service.read(service.prefix + '/dna/tasks');
  fs.writeFileSync(service.evidence + '/tasks-response.json', JSON.stringify(collection.json, null, 2));
  const current = exact.json.data.items[0]; expect(current.id).toBe(service.task); expect(current.state).toBe('handed');
  for (const key of ['outcome', 'obligation', 'acceptance_digest', 'acceptance_bound', 'evidence_required']) expect(current[key]).toEqual(service.initial[key]);
  expect(current.assignment_digest).not.toBe(service.initial.assignment_digest); expect(current.history).toHaveLength(2);
  expect(current.history[0]).toEqual(service.initial.history[0]); expect(current.history[1]).toMatchObject({ kind: 'task.reassigned', from: 'alex', to: 'blair', by: 'riley' });
  expect(posts).toHaveLength(1); expect(reassignments(service)).toHaveLength(1);
  expect(service.journal().rows.some(row => row.kind === 'task.done' && row.entity === service.task)).toBe(false);
  await task(page).getByRole('button', { name: /^Handoff to alex/ }).click();
  await expect(task(page).getByRole('group', { name: 'Selected assignment' })).toContainText('Handed to alex');
  await page.screenshot({ path: info.outputPath('actual-native-task-reassignment.png') });
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-observation', 'observed'); expect(posts).toHaveLength(1);
});

test('lost native POST reply survives API restart and reload with GET-only exact-key recovery', async ({ page, service }) => {
  let savedCommand, received; const nativeResponse = new Promise(resolve => { received = resolve; }); let postCount = 0;
  page.on('request', request => { if (isWrite(request)) postCount += 1; });
  await page.route('**/commands', async route => {
    if (route.request().method() !== 'POST' || isDescribe(route.request())) return route.continue();
    savedCommand = route.request().postDataJSON();
    const response = await route.fetch(); received({ status: response.status(), json: await response.json() });
    await route.abort('failed');
  });
  await prepare(page, service); await confirmation(page).getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  const committed = await nativeResponse; expect(committed.status).toBe(200); expect(committed.json.value.receipt.task.state).toBe('applied');
  await expect(recovery(page)).toBeVisible(); const before = await stored(page); expect(before).toHaveLength(1);
  expect(savedCommand.call).toBe('TaskReassign');
  expect(before[0].value).toMatchObject({ version: 5, operation: 'dna.task.reassign', request_id: savedCommand.payload.request_id, target_id: service.task });
  expect(Object.keys(before[0].value)).not.toContain('arguments'); expect(Object.keys(before[0].value)).not.toContain('recipients');
  expect(reassignments(service)).toHaveLength(1);
  await page.unroute('**/commands'); await service.restart(); await page.reload();
  await expect(recovery(page)).toHaveAttribute('data-observation', 'observed');
  expect((await stored(page))[0].value.request_id).toBe(savedCommand.payload.request_id); expect(postCount).toBe(1);
  expect(reassignments(service)).toHaveLength(1); expect((await service.current()).assignee).toBe('blair');
  const lookup = await service.lookup(savedCommand.payload.request_id);
  expect(lookup.status).toBe(200); expect(lookup.receipt.command_id).toBe(committed.json.value.receipt.command_id); expect(lookup.receipt.task).toEqual(committed.json.value.receipt.task);
});

test('native current-assignment conflict and retired/outside-policy recipients cannot reassign', async ({ page, service }, info) => {
  await page.setViewportSize({ width: 390, height: 844 }); await page.emulateMedia({ reducedMotion: 'reduce' });
  await prepare(page, service);
  const alternate = await service.post(service.command(service.initial, 'casey', 'concurrent-native-assignment'));
  expect(alternate.status).toBe(200); expect(alternate.receipt.task.state).toBe('applied');
  const response = page.waitForResponse(r => isWrite(r.request()) && new URL(r.url()).pathname === service.prefix + '/commands');
  await confirmation(page).getByRole('button', { name: 'Confirm reassignment', exact: true }).click();
  // The binding answered; the provider refused the stale subject.
  const stale = await response; expect(stale.status()).toBe(200); expect((await stale.json()).value).toMatchObject({ ok: false, code: 'stale_subject' });
  await expect(recovery(page).getByRole('alert')).toBeVisible(); expect(reassignments(service)).toHaveLength(1);
  const current = await service.current(); expect(current.assignee).toBe('casey');
  const plan = await service.read(service.prefix + '/dna/people?id=' + encodeURIComponent(current.assignee)); expect(plan.status).toBe(200);
  expect(plan.json.data.recipients).not.toContain('retired'); expect(plan.json.data.recipients).not.toContain('outside');
  for (const recipient of ['retired', 'outside']) {
    const denied = await service.post(service.command(current, recipient, 'denied-' + recipient));
    expect(denied.status).toBe(200); expect(denied.code).toBe('forbidden');
  }
  expect(reassignments(service)).toHaveLength(1); expect((await service.current()).assignee).toBe('casey');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('actual-native-task-stale-narrow.png') });
});
