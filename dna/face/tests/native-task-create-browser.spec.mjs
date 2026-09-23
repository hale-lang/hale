// Actual native task creation: the face's ask lands as the CLI's
// intent.requested row in a real Record through the real composed API. No
// relay or organism runs here, so the ask stays `requested`; offer, refusal
// and birth are the organism's later facts and a follow-up case.
import { test as base, expect } from '@playwright/test';
import fs from 'node:fs';
import { startTaskService, nativeTaskEnvironmentPresent } from './native-task-harness.mjs';

const taskPolicy = (application_id, name) => ({ format: 'dna.task-authority/1', application_id, owner: 'operations', members: ['alex', 'blair'], grants: [{ mode: 'local', name, reassign: true, recover: true }] });
const OUTCOME = 'Confirm the supplier handover — équipe\nKeep the signed schedule.';
const test = base.extend({
  service: async ({}, use, info) => {
    const service = await startTaskService({ taskPolicy });
    try { await use(service); }
    finally { await service.stop(); await info.attach('native-task-create-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); }
  },
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
});
test.skip(!nativeTaskEnvironmentPresent(), 'Supply matching native Task seed and configured command API binaries.');
test.setTimeout(45_000);
const region = page => page.getByRole('region', { name: 'New task', exact: true });
const recovery = page => page.getByRole('region', { name: 'New task request', exact: true });
const confirmation = page => page.getByRole('group', { name: 'Confirm new task', exact: true });
const asks = service => service.journal().rows.filter(row => row.kind === 'intent.requested');
const stored = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('iris.practice-recovery.v1:')).map(([key, value]) => ({ key, value: JSON.parse(value) })));
async function prepare(page, service) {
  await page.goto(service.origin + '/#/tasks?' + new URLSearchParams({ app: service.application }));
  await expect(region(page)).toBeVisible();
  await region(page).getByRole('textbox', { name: 'What should happen', exact: true }).fill(OUTCOME);
  await region(page).getByRole('button', { name: 'Review new task', exact: true }).click();
  await expect(confirmation(page)).toContainText('Keep the signed schedule.');
}

test('native ask lands as the CLI row: one POST, a real receipt, one intent.requested row in the Record', async ({ page, service }, info) => {
  const posts = []; page.on('request', request => { if (request.method() === 'POST') posts.push(request.postDataJSON()); });
  const capabilities = await service.read(service.prefix + '/capabilities');
  expect(capabilities.json.data.writes.task_create).toBe(true); expect(capabilities.json.data.task_create_commands.profile).toBe('dna.task.create.v1');
  fs.writeFileSync(service.evidence + '/capabilities-response.json', JSON.stringify(capabilities.json, null, 2));
  const before = service.journal().head;
  await prepare(page, service); expect(posts).toHaveLength(0); expect(asks(service)).toHaveLength(0);
  const reply = page.waitForResponse(r => r.request().method() === 'POST' && new URL(r.url()).pathname === service.prefix + '/commands');
  await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  const response = await reply; expect(response.status()).toBe(200); const receipt = (await response.json()).data;
  fs.writeFileSync(service.evidence + '/task-create-response.json', JSON.stringify(receipt, null, 2));
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested');
  expect(receipt.operation).toBe('dna.task.create'); expect(receipt.task_create.intent_state).toBe('requested'); expect(receipt.task_create.task_id).toBe('');
  expect(receipt.task_create.intent_id).toMatch(/^i[0-9a-f]{1,16}$/); expect(receipt.subject_digest).toBe(before);
  expect(posts).toHaveLength(1); expect(posts[0].preconditions.record_head).toBe(before); expect(posts[0].arguments).toEqual({ outcome: OUTCOME, to: 'org' });
  const rows = asks(service); expect(rows).toHaveLength(1); const row = rows[0];
  expect(row.entity).toBe(receipt.task_create.intent_id); expect(row.author).toBe('riley');
  const body = JSON.parse(row.body);
  expect(Object.keys(body).slice(0, 3)).toEqual(['outcome', 'from', 'to']);
  expect(body).toMatchObject({ outcome: OUTCOME, from: 'riley', to: 'org', command_format: 'dna.task-create-command/1', command_id: receipt.command_id, command_record_head: before, command_authority: 'riley' });
  expect(body).not.toHaveProperty('via'); expect(body).not.toHaveProperty('intent_id');
  expect(row.body.startsWith(JSON.stringify({ outcome: OUTCOME, from: 'riley', to: 'org' }).replace(/":"/g, '": "').replace(/","/g, '", "').slice(0, -1))).toBe(true);
  expect(service.journal().head).toBe(receipt.task_create.event_id);
  const lookup = await service.read(service.prefix + '/commands?' + new URLSearchParams({ request_id: posts[0].request_id }));
  expect(lookup.status).toBe(200); expect(lookup.json.data.command_id).toBe(receipt.command_id); expect(lookup.json.data.task_create).toEqual(receipt.task_create);
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested'); expect(posts).toHaveLength(1); expect(asks(service)).toHaveLength(1);
  await page.screenshot({ path: info.outputPath('actual-native-task-create.png') });
});

test('lost native POST reply survives API restart and reload with GET-only exact-key recovery', async ({ page, service }) => {
  let savedCommand, received; const nativeResponse = new Promise(resolve => { received = resolve; }); let postCount = 0;
  page.on('request', request => { if (request.method() === 'POST') postCount += 1; });
  await page.route('**/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    savedCommand = route.request().postDataJSON();
    const response = await route.fetch(); received({ status: response.status(), json: await response.json() });
    await route.abort('failed');
  });
  await prepare(page, service); await confirmation(page).getByRole('button', { name: 'Confirm new task', exact: true }).click();
  const committed = await nativeResponse; expect(committed.status).toBe(200); expect(committed.json.data.task_create.intent_state).toBe('requested');
  await expect(recovery(page)).toBeVisible(); const before = await stored(page); expect(before).toHaveLength(1);
  expect(before[0].value).toMatchObject({ version: 7, operation: 'dna.task.create', request_id: savedCommand.request_id, target_kind: 'dna.record', target_id: service.application });
  expect(Object.keys(before[0].value)).not.toContain('arguments'); expect(asks(service)).toHaveLength(1);
  await page.unroute('**/commands'); await service.restart(); await page.reload();
  await expect(recovery(page)).toHaveAttribute('data-intent-state', 'requested');
  expect((await stored(page))[0].value.request_id).toBe(savedCommand.request_id); expect(postCount).toBe(1); expect(asks(service)).toHaveLength(1);
  const lookup = await service.read(service.prefix + '/commands?' + new URLSearchParams({ request_id: savedCommand.request_id }));
  expect(lookup.status).toBe(200); expect(lookup.json.data.task_create).toEqual(committed.json.data.task_create);
  const stale = await service.post({ ...savedCommand, request_id: 'stale-' + savedCommand.request_id });
  expect(stale.status).toBe(409); expect(stale.json.error.code).toBe('stale_subject'); expect(asks(service)).toHaveLength(1);
});
