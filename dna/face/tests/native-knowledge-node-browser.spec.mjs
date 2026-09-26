// Browser -> real Knowledge command service -> host/body -> canonical Review
// -> domain activation -> projected graph. No authored outcome facts.
// Knowledge changes and their recovery are forwarded lines of the head's
// api wire on …/commands (GH #1129); a refusal is the reply's code.
import { test as base, expect } from '@playwright/test';
import { startNodeService, nodeEnvironmentPresent } from './native-knowledge-node-harness.mjs';
import { callOf, isKnowledgeCall, isKnowledgeLookup, isWrite, settleKnowledge } from './command-wire.mjs';

const test = base.extend({
  grants: [undefined, { option: true }],
  service: async ({ grants }, use, testInfo) => {
    const service = await startNodeService({ grants });
    try { await use(service); }
    finally { await service.stop(); await testInfo.attach('native-node-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); expect(service.processes()).toEqual([]); }
  },
  page: async ({ page, service }, use) => {
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await service.attach(page);
    await use(page); expect(errors).toEqual([]);
  },
});
test.skip(!nodeEnvironmentPresent(), 'Supply matching native Review API, Body, relay and Knowledge service binaries.');
test.setTimeout(90_000);

const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const receipt = page => page.getByRole('region', { name: 'Knowledge change request', exact: true });
const decision = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const verdictReceipt = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const detail = page => page.getByRole('region', { name: 'Knowledge item', exact: true });
const saved = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('face.knowledge-recovery.v1:')).map(([, value]) => JSON.parse(value)));
// Every line that asks for a change; recovery reads are not among them.
const posts = page => { const values = []; page.on('request', request => { if (isWrite(request) && !isKnowledgeLookup(request) && new URL(request.url()).pathname.endsWith('/commands')) values.push(request.postDataJSON()); }); return values; };
// The answer to a Knowledge call ('POST'), its recovery ('GET'), or a named call.
const statusResponse = (page, path, which) => page.waitForResponse(response => new URL(response.url()).pathname === path
  && (which === 'POST' ? isKnowledgeCall(response.request()) : which === 'GET' ? isKnowledgeLookup(response.request()) : callOf(response.request()) === which));
const NODE_CALLS = ['KnowledgeNodePropose', 'KnowledgeNodeRevise', 'KnowledgeNodeRetire'];
const originalText = 'A non-Practice idea — café 東京 🧭.\nKeep <img src=x onerror="window.__nodeInjected=true"> as literal evidence.\n';

async function prepare(page, service, { operation = 'node.propose', id = '', name = 'Evidence compass', text = originalText } = {}) {
  await service.quiesce(); await page.goto(service.url('knowledge', id ? { id } : {}));
  if (operation === 'node.revise') await page.getByRole('button', { name: 'Revise focused item', exact: true }).click();
  else { await editor(page).getByRole('button', { name: 'Prepare knowledge change', exact: true }).click(); await editor(page).getByLabel('Change kind', { exact: true }).selectOption(operation); }
  if (operation !== 'node.retire') {
    await editor(page).getByLabel('Knowledge kind', { exact: true }).selectOption('idea');
    await editor(page).getByLabel('Knowledge name', { exact: true }).fill(name);
    await editor(page).getByLabel('Knowledge text', { exact: true }).fill(text);
    await editor(page).getByLabel('Requested authoring locus', { exact: true }).fill('org');
    await editor(page).getByLabel('Target locus', { exact: true }).fill('org');
  }
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill('Preserve exact evidence and its history.');
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
}
async function send(page, service) {
  const pending = statusResponse(page, service.commandPath, 'POST');
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click();
  const response = await pending; const line = response.request().postDataJSON();
  return { ...settleKnowledge(response.status(), await response.json()), line, request_id: line.payload.request_id };
}
async function show(page, service, requestId, predicate = value => value.node.proposal_state === 'created') {
  const native = await service.waitNode(requestId, predicate); await service.quiesce();
  const pending = statusResponse(page, service.commandPath, 'GET');
  await receipt(page).getByRole('button', { name: 'Check knowledge request', exact: true }).click();
  expect((await pending).status()).toBe(200);
  await expect(receipt(page)).toContainText(requestId);
  return native;
}
async function propose(page, service, options = {}) {
  await prepare(page, service, options); await expect(editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true })).toBeEnabled();
  const sent = await send(page, service); expect(sent.status).toBe(200); expect(sent.code).toBe('');
  const native = await show(page, service, sent.request_id);
  const canonical = service.candidate(native.node.candidate_digest);
  if (options.operation !== 'node.retire') { expect(canonical.kind).toBe('idea'); expect(canonical.text).toBe(options.text ?? originalText); }
  return { ...sent, native, canonical };
}
async function dismissNode(page) {
  await receipt(page).getByRole('button', { name: 'Dismiss knowledge request', exact: true }).click();
  await expect(receipt(page)).toHaveCount(0); expect(await saved(page)).toEqual([]);
}
async function approve(page, service, proposal, activation = 'adopted') {
  await service.quiesce(); await page.goto(service.url('reviews', { id: proposal.native.node.review_id }));
  await expect(decision(page).getByRole('button', { name: 'Prepare decision', exact: true })).toBeEnabled();
  expect(await decision(page).locator('.intervention-document .document-text').textContent()).toBe(proposal.canonical.text);
  await expect(decision(page)).toContainText(proposal.native.node.candidate_digest);
  await decision(page).getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await decision(page).getByRole('radio', { name: 'Approve', exact: true }).check();
  await decision(page).getByRole('textbox', { name: 'Decision note', exact: true }).fill('Approve this exact canonical Knowledge candidate.');
  await decision(page).getByRole('button', { name: 'Review decision', exact: true }).click();
  const response = statusResponse(page, service.apiPath + '/commands', 'ReviewVerdict');
  await decision(page).getByRole('button', { name: 'Submit decision', exact: true }).click();
  const submitted = await response; expect(submitted.status()).toBe(200);
  const verdict = submitted.request().postDataJSON();
  expect(verdict.call).toBe('ReviewVerdict');
  await service.waitCommand(verdict.payload.request_id, value => value.verdict.state === 'accepted');
  const native = await service.waitNode(proposal.request_id, value => value.node.activation_state === activation);
  expect(native.node.review_outcome).toBe('approve'); await service.quiesce();
  await verdictReceipt(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(verdictReceipt(page).getByRole('button', { name: 'Dismiss completed request', exact: true })).toBeVisible();
  await verdictReceipt(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  return native;
}
async function openObserved(page, service, id, retiring = false) {
  await service.quiesce(); await page.goto(service.url('knowledge', { id }));
  await expect(receipt(page)).toContainText(retiring ? 'Retirement observed' : 'Adoption observed');
  await expect(detail(page)).toContainText(id);
}

test('native Knowledge nodes: create a generic idea, decide its exact Review, revise and retire with history retained', async ({ page, service }, testInfo) => {
  const submitted = posts(page);
  const first = await propose(page, service);
  // A proposal names its collection; the head derives the target from it.
  expect(first.line.call).toBe('KnowledgeNodePropose');
  expect(first.line.payload).toMatchObject({ target: 'org', author: 'org', kind: 'idea' }); expect(first.line.payload).not.toHaveProperty('target_id');
  expect((await saved(page))[0]).toMatchObject({ version: 3, operation: 'dna.knowledge.node.propose', target_kind: 'dna.knowledge.collection', target_id: 'org' });
  expect(JSON.stringify(await saved(page))).not.toContain('Evidence compass');
  await expect(receipt(page)).toContainText('Proposal created'); await expect(receipt(page)).not.toContainText('Adoption observed');
  await approve(page, service, first); const firstId = first.native.node.candidate_digest;
  await openObserved(page, service, firstId);
  expect(await page.evaluate(() => window.__nodeInjected)).toBeUndefined(); await expect(detail(page).locator('img')).toHaveCount(0);
  await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('generic-idea-adopted.png') });
  await dismissNode(page);
  const revised = await propose(page, service, { operation: 'node.revise', id: firstId, text: originalText + 'Revised through its own native Review.\n' });
  expect(revised.line.payload.supersedes).toBe(firstId);
  await approve(page, service, revised); const revisedId = revised.native.node.candidate_digest;
  await openObserved(page, service, revisedId);
  await receipt(page).getByRole('link', { name: 'Open prior knowledge', exact: true }).click();
  await expect(detail(page)).toContainText(firstId); await expect(detail(page)).toContainText('retired');
  expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('id')).toBe(firstId);
  await dismissNode(page);
  const retirement = await propose(page, service, { operation: 'node.retire', id: revisedId });
  expect(retirement.line.payload).toEqual({ request_id: retirement.request_id, record_head: retirement.line.payload.record_head, id: revisedId, rationale: 'Preserve exact evidence and its history.' });
  await approve(page, service, retirement); await openObserved(page, service, revisedId, true);
  await expect(detail(page)).toContainText('retired'); await expect(detail(page)).toContainText('Revised through its own native Review.');
  expect(submitted.filter(value => NODE_CALLS.includes(value.call)).map(value => value.call)).toEqual(NODE_CALLS);
  await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('generic-idea-retired-with-history.png') });
});

test('native Knowledge nodes: lost creation response restarts all services and recovers by GET without another proposal', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 }); const submitted = posts(page);
  await prepare(page, service); await service.pauseDelivery(); let admitted;
  await page.route('**/commands', async route => {
    if (!isKnowledgeCall(route.request())) return route.fallback();
    const response = await route.fetch(); expect(response.status()).toBe(200); admitted = route.request().postDataJSON().payload; await route.abort('failed');
  });
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click();
  await expect(receipt(page)).toContainText('Knowledge outcome could not be confirmed');
  expect((await saved(page))[0]).toMatchObject({ operation: 'dna.knowledge.node.propose', request_id: admitted.request_id, target_kind: 'dna.knowledge.collection', target_id: 'org' });
  await page.unroute('**/commands'); await service.restart();
  await service.waitNode(admitted.request_id, value => value.node.proposal_state === 'created'); await service.quiesce();
  await page.reload(); await expect(receipt(page)).toContainText('Proposal created');
  expect(submitted).toHaveLength(1); expect(submitted[0].payload.request_id).toBe(admitted.request_id);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('generic-idea-recovered-mobile.png') });
});

test('native Knowledge nodes: stale Record precondition refuses admission without a second request', async ({ page, service }) => {
  const submitted = posts(page); await prepare(page, service); await service.pauseDelivery();
  const other = await service.command('node.propose', { kind: 'idea', name: 'Other writer', text: 'Independent evidence.', author: 'org', target: 'org', rationale: 'Advance the native Record.' }, 'org');
  expect((await service.post(other)).code).toBe(''); const head = service.journal().head;
  const rejected = await send(page, service); expect(rejected.status).toBe(200); expect(rejected.code).toBe('stale_subject');
  await expect(receipt(page)).toContainText('The service refused this Knowledge request');
  expect(service.journal().head).toBe(head); expect((await service.lookup(rejected.request_id)).code).toBe('command_not_found'); expect(submitted).toHaveLength(1);
  service.resumeDelivery();
});

test('native Knowledge nodes: approved competing revision reports adoption refusal separately', async ({ page, service }, testInfo) => {
  const original = await propose(page, service); await approve(page, service, original);
  const id = original.native.node.candidate_digest; await openObserved(page, service, id); await dismissNode(page);
  const first = await propose(page, service, { operation: 'node.revise', id, text: 'First independently reviewed revision.' }); await dismissNode(page);
  const second = await propose(page, service, { operation: 'node.revise', id, text: 'Second independently reviewed revision.' });
  await approve(page, service, first); await approve(page, service, second, 'refused');
  const observed = await show(page, service, second.request_id, value => value.node.activation_state === 'refused');
  expect(observed.state).toBe('succeeded'); expect(observed.node.review_outcome).toBe('approve');
  await expect(receipt(page)).toContainText('refused'); await expect(receipt(page)).not.toContainText('Adoption observed');
  await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('generic-revision-approved-activation-refused.png') });
});

test('native Knowledge nodes: graph observation requires the final receipt at the same Record head', async ({ page, service }) => {
  const proposal = await propose(page, service); await approve(page, service, proposal); await service.pauseDelivery();
  let lookups = 0, changedHead;
  await page.route('**/commands', async route => {
    if (!isKnowledgeLookup(route.request()) || route.request().postDataJSON().payload.request_id !== proposal.request_id) return route.fallback();
    lookups++;
    if (lookups === 2) {
      const other = await service.command('node.propose', { kind: 'idea', name: 'Independent source advance', text: 'A real second admission changes the captured Record.', author: 'org', target: 'org', rationale: 'Verify the final source check.' }, 'org');
      expect((await service.post(other)).code).toBe(''); changedHead = service.journal().head;
    }
    await route.continue();
  });
  await page.goto(service.url('knowledge', { id: proposal.native.node.candidate_digest }));
  await expect.poll(() => lookups).toBe(2);
  await expect(receipt(page).getByRole('button', { name: 'Check knowledge request', exact: true })).toBeEnabled();
  await expect(receipt(page)).toContainText('Not established'); await expect(receipt(page)).not.toContainText('Adoption observed');
  await expect(receipt(page).getByRole('button', { name: 'Activation', exact: true })).toContainText('adopted');
  expect(service.journal().head).toBe(changedHead); service.resumeDelivery();
});

test.describe('Denied generic node authority', () => {
  test.use({ grants: [{ mode: 'local', name: 'alice', authority: 'board', edge_link: 'direct', node_propose: 'deny', node_revise: 'deny', node_retire: 'deny', node_scopes: [{ author: 'org', target: 'org' }], recover: true }] });
  // The seat opens the call (the `position` gate); the policy, which grants
  // this person relationships only, refuses the proposal and admits nothing.
  test('native Knowledge nodes: read and relationship authority do not permit a node proposal', async ({ page, service }) => {
    expect(await service.slice()).toEqual(expect.arrayContaining(['KnowledgeNodePropose', 'KnowledgeLookup']));
    const submitted = posts(page); await prepare(page, service); const head = service.journal().head;
    const refused = await send(page, service); expect(refused.status).toBe(200); expect(refused.code).toBe('forbidden');
    expect(service.journal().head).toBe(head); expect(submitted).toHaveLength(1);
    expect(service.facts('knowledge.node.requested')).toEqual([]);
  });
});
