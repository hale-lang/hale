// Real binding proposals, canonical Reviews, native effects and graph readback.
import { test as base, expect } from '@playwright/test';
import { startBindingService, bindingEnvironmentPresent, bindingGrant } from './native-knowledge-binding-harness.mjs';

const test = base.extend({
  grants: [undefined, { option: true }],
  service: async ({ grants }, use, testInfo) => {
    const service = await startBindingService({ grants });
    try { await use(service); }
    finally { await service.stop(); await testInfo.attach('native-binding-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); expect(service.processes()).toEqual([]); }
  },
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
});
test.skip(!bindingEnvironmentPresent(), 'Supply matching native API, Body, relay and Knowledge service binaries.');
test.setTimeout(120_000);
const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const receipt = page => page.getByRole('region', { name: 'Knowledge binding request', exact: true });
const decision = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const verdictReceipt = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const rationale = 'Apply exact evidence — café 東京 🧭. Keep <img src=x onerror="window.__bindingInjected=true"> literal.';
const saved = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('face.knowledge-recovery.v1:')).map(([, value]) => JSON.parse(value)));
const trackPosts = page => { const values = []; page.on('request', request => { if (request.method() === 'POST' && new URL(request.url()).pathname.endsWith('/dna/knowledge/commands')) values.push(request.postDataJSON()); }); return values; };

async function prepare(page, service, { idea = service.practice, target = 'org/support', bindingId = '', filter = '' } = {}) {
  await service.quiesce(); await page.goto(service.url('knowledge', { id: idea, ...(filter ? { target: filter } : {}) }));
  if (bindingId) await page.locator('[data-binding-id="' + bindingId + '"]').getByRole('button', { name: 'Remove this binding', exact: true }).click();
  else {
    await page.getByRole('button', { name: 'Add locus binding', exact: true }).click();
    await editor(page).getByLabel('Requested authoring locus', { exact: true }).fill('org');
    await editor(page).getByLabel('Target locus', { exact: true }).fill(target);
  }
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(rationale);
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
}
async function send(page, service) {
  const pending = page.waitForResponse(response => new URL(response.url()).pathname === service.commandPath && response.request().method() === 'POST');
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click();
  const response = await pending; return { status: response.status(), body: await response.json(), command: response.request().postDataJSON() };
}
async function propose(page, service, options) {
  await prepare(page, service, options); const sent = await send(page, service); expect(sent.status).toBe(202);
  const native = await service.waitBinding(sent.command.request_id, value => value.binding.proposal_state === 'created'); await service.quiesce();
  await receipt(page).getByRole('button', { name: 'Check binding request', exact: true }).click(); await expect(receipt(page)).toContainText('Proposal created');
  return { ...sent, native, canonical: service.candidate(native.binding.candidate_digest) };
}
async function decide(page, service, proposal, { verdict = 'approve', effect = proposal.command.operation.endsWith('.unbind') ? 'unbound' : 'bound' } = {}) {
  await service.quiesce(); await page.goto(service.url('reviews', { id: proposal.native.binding.review_id }));
  await expect(decision(page).getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled(); await expect(decision(page)).toContainText('You are its recorded proposer');
  await service.asActor('bob'); await page.reload();
  await expect(decision(page).getByRole('button', { name: 'Prepare decision', exact: true })).toBeEnabled();
  expect(JSON.parse(await decision(page).locator('.binding-canonical-document').textContent())).toEqual(proposal.canonical);
  expect(proposal.canonical.format).toBe('dna.knowledge-binding-change/1'); expect(proposal.canonical.idea_id).toBe(proposal.command.target.id);
  await expect(decision(page)).toContainText(proposal.command.arguments.target); await expect(decision(page).locator('img')).toHaveCount(0);
  await decision(page).getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await decision(page).getByRole('radio', { name: verdict === 'approve' ? 'Approve' : 'Reject', exact: true }).check();
  await decision(page).getByRole('textbox', { name: 'Decision note', exact: true }).fill('Decide the exact binding tuple and unchanged original idea.');
  await decision(page).getByRole('button', { name: 'Review decision', exact: true }).click();
  const pending = page.waitForResponse(response => new URL(response.url()).pathname === service.apiPath + '/commands' && response.request().method() === 'POST');
  await decision(page).getByRole('button', { name: 'Submit decision', exact: true }).click();
  const response = await pending; expect([200, 202]).toContain(response.status());
  await service.waitCommand(response.request().postDataJSON().request_id, value => value.verdict.state === 'accepted');
  await service.quiesce();
  await verdictReceipt(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await verdictReceipt(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await expect(verdictReceipt(page)).toHaveCount(0); await expect(decision(page).getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(JSON.parse(await decision(page).locator('.binding-canonical-document').textContent())).toEqual(proposal.canonical);
  await service.asActor('alice'); return service.waitBinding(proposal.command.request_id, value => value.binding.effect_state === effect);
}
async function openResult(page, service, proposal, extra = {}) {
  await service.quiesce(); await page.goto(service.url('knowledge', { id: proposal.command.target.id, ...extra }));
  await expect(receipt(page).getByRole('button', { name: 'Check binding request', exact: true })).toBeEnabled();
}
async function dismiss(page) { await receipt(page).getByRole('button', { name: 'Dismiss binding request', exact: true }).click(); await expect(receipt(page)).toHaveCount(0); }

test('native bindings: reviewed applicability reaches a new branch and exact removal preserves descendant binding', async ({ page, service }, testInfo) => {
  const idea = await service.createItem(), before = service.candidate(idea), posts = trackPosts(page);
  expect((await service.bindings(idea, 'org/support')).length).toBe(0);
  const binding = await propose(page, service, { idea });
  const graph = await service.request(service.apiPath + '/dna/knowledge/nodes?limit=25'); expect(graph.status).toBe(200); expect(graph.body.data.items.some(row => row.id === binding.native.binding.candidate_digest)).toBe(false);
  expect(binding.command.arguments).toEqual({ idea_id: idea, author: 'org', target: 'org/support', rationale });
  expect((await saved(page))[0]).toMatchObject({ version: 4, operation: 'dna.knowledge.binding.bind', target_id: idea, binding: { author: 'org', target: 'org/support' } });
  expect(JSON.stringify(await saved(page))).not.toContain(rationale); await expect(receipt(page)).not.toContainText('Binding observed');
  await decide(page, service, binding); await openResult(page, service, binding, { target: 'org/support' });
  await expect(receipt(page)).toContainText('Binding observed'); expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('target')).toBe('org/support');
  await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('binding-observed-desktop.png') }); await dismiss(page);
  const descendant = await service.applyBinding(idea, 'org/support/urgent');
  const removal = await propose(page, service, { idea, bindingId: binding.native.binding.binding_id, filter: 'org/support' });
  expect(removal.command.arguments).toEqual({ binding_id: binding.native.binding.binding_id, idea_id: idea, author: 'org', target: 'org/support', rationale });
  expect(removal.command.arguments).not.toHaveProperty('class'); expect(removal.command.arguments).not.toHaveProperty('applicability');
  await decide(page, service, removal); await openResult(page, service, removal, { target: 'org/support/urgent' });
  await expect(receipt(page)).toContainText('Binding removal observed');
  const rows = await service.bindings(idea); expect(rows.some(row => row.id === binding.native.binding.binding_id)).toBe(false); expect(rows.some(row => row.id === descendant.receipt.binding.binding_id)).toBe(true);
  expect(service.candidate(idea)).toEqual(before); expect(await page.evaluate(() => window.__bindingInjected)).toBeUndefined(); expect(posts).toHaveLength(2);
  await page.setViewportSize({ width: 390, height: 844 }); await receipt(page).scrollIntoViewIfNeeded(); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); await page.screenshot({ path: testInfo.outputPath('binding-removal-observed-mobile.png') });
});

test('native bindings: lost unbind reply restarts all services and recovers by GET with original tuple', async ({ page, service }) => {
  const initial = await service.applyBinding(service.practice, 'org/support'), posts = trackPosts(page);
  await prepare(page, service, { bindingId: initial.receipt.binding.binding_id }); await service.pauseDelivery(); let admitted;
  await page.route('**/dna/knowledge/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue(); const response = await route.fetch(); expect(response.status()).toBe(202); admitted = route.request().postDataJSON(); await route.abort('failed');
  });
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click(); await expect(receipt(page)).toContainText('could not be confirmed');
  expect((await saved(page))[0]).toMatchObject({ version: 4, operation: 'dna.knowledge.binding.unbind', request_id: admitted.request_id });
  await page.unroute('**/dna/knowledge/commands'); await service.restart(); await service.waitBinding(admitted.request_id, value => value.binding.proposal_state === 'created'); await service.quiesce();
  await page.reload(); await expect(receipt(page)).toContainText('Proposal created'); await expect(receipt(page)).toContainText('org/support'); expect(posts).toHaveLength(1);
  expect((await service.bindings(service.practice)).some(row => row.id === initial.receipt.binding.binding_id)).toBe(true);
});

test('native bindings: rejected Review leaves the binding effect declined and graph unchanged', async ({ page, service }) => {
  const proposal = await propose(page, service); await decide(page, service, proposal, { verdict: 'reject', effect: 'declined' }); await openResult(page, service, proposal);
  await expect(receipt(page).getByRole('button', { name: 'Binding effect', exact: true })).toContainText('declined'); await expect(receipt(page)).not.toContainText('Binding observed');
  expect((await service.bindings(service.practice)).some(row => row.id === proposal.native.binding.binding_id)).toBe(false);
});

test('native bindings: stale admission preserves the graph and sends no replacement request', async ({ page, service }) => {
  const posts = trackPosts(page); await prepare(page, service); await service.pauseDelivery();
  const command = await service.command('binding.bind', { idea_id: service.practice, author: 'org', target: 'org/elsewhere', rationale: 'Advance native Record.' }, service.practice);
  expect((await service.post(command)).status).toBe(202); const head = service.journal().head;
  const refused = await send(page, service); expect(refused.status).toBe(409); expect(refused.body.error.code).toBe('stale_subject'); await expect(receipt(page)).toContainText('refused');
  expect(service.journal().head).toBe(head); expect((await service.lookup(refused.command.request_id)).status).toBe(404); expect(posts).toHaveLength(1); service.resumeDelivery();
});

test('native bindings: approved competing candidate reports refused effect rather than graph success', async ({ page, service }) => {
  const first = await propose(page, service); await dismiss(page); const second = await propose(page, service);
  await decide(page, service, first); await decide(page, service, second, { effect: 'refused' }); await openResult(page, service, second);
  await expect(receipt(page).getByRole('button', { name: 'Review', exact: true })).toContainText('approve'); await expect(receipt(page).getByRole('button', { name: 'Binding effect', exact: true })).toContainText('refused'); await expect(receipt(page)).not.toContainText('Binding observed');
});

test('native bindings: removal absence requires complete unfiltered pagination and survives a failed continuation', async ({ page, service }) => {
  test.setTimeout(180_000); let chosen;
  // Every tuple is a real admitted, independently reviewed native effect.
  // Bound setup lifetimes; this proves full-history restart and pagination,
  // not sustained-service operation (tracked separately as a deployment limit).
  for (let i = 0; i < 27; i++) {
    const binding = await service.applyBinding(service.practice, 'org/page/' + String(i).padStart(2, '0'));
    if (i === 0) chosen = binding;
    if ((i + 1) % 6 === 0) await service.restart();
  }
  await service.restart();
  expect(await service.bindings(service.practice)).toHaveLength(28);
  const removal = await propose(page, service, { bindingId: chosen.receipt.binding.binding_id }); await decide(page, service, removal);
  let failed = false;
  await page.route('**/dna/knowledge/bindings?*', route => {
    const query = new URL(route.request().url()).searchParams;
    if (query.has('cursor') && !query.has('target')) { failed = true; return route.fulfill({ status: 503, json: { api_version: 'hale.v1', error: { code: 'knowledge_unavailable', message: 'Continuation transport unavailable', retryable: true } } }); }
    return route.continue();
  });
  await openResult(page, service, removal); await expect(receipt(page)).toContainText('Not established'); expect(failed).toBe(true); await expect(receipt(page)).not.toContainText('Binding removal observed');
  await page.unroute('**/dna/knowledge/bindings?*'); const pages = [];
  page.on('response', async response => { const url = new URL(response.url()); if (response.status() === 200 && url.pathname.endsWith('/dna/knowledge/bindings') && !url.searchParams.has('target')) pages.push((await response.json()).data); });
  await receipt(page).getByRole('button', { name: 'Check binding request', exact: true }).click(); await expect(receipt(page)).toContainText('Binding removal observed');
  const continued = pages.filter(value => value.page.has_more || value.items.length < 25); expect(continued.some(value => value.page.has_more)).toBe(true); expect(continued.some(value => !value.page.has_more)).toBe(true);
  const rows = await service.bindings(service.practice); expect(rows).toHaveLength(27); expect(rows.some(row => row.id === chosen.receipt.binding.binding_id)).toBe(false);
});

test.describe('Independent binding permissions', () => {
  test.use({ grants: [{ ...bindingGrant, binding_bind: 'deny', binding_unbind: 'deny' }] });
  test('native bindings: node and edge authority do not authorize binding changes', async ({ page, service }) => {
    const posts = trackPosts(page); await prepare(page, service); await expect(editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true })).toBeDisabled();
    expect((await service.capability('dna.knowledge.binding.bind')).body.data.authorized).toBe(false); expect(posts).toEqual([]); expect(await saved(page)).toEqual([]);
  });
});
