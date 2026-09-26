// Relationship changes and their recovery are forwarded lines of the head's
// api wire on …/commands (GH #1129); a refusal is the reply's code.
import { test as base, expect } from '@playwright/test';
import { startEdgeReviewService, edgeReviewEnvironmentPresent, edgeReviewGrant } from './native-knowledge-edge-review-harness.mjs';
import { callOf, isKnowledgeCall, settleKnowledge } from './command-wire.mjs';
const test = base.extend({
  service: async ({}, use, testInfo) => {
    const service = await startEdgeReviewService();
    try { await use(service); }
    finally { await service.stop(); await testInfo.attach('native-edge-review-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); expect(service.processes()).toEqual([]); }
  },
  page: async ({ page, service }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await service.attach(page); await use(page); expect(errors).toEqual([]); },
});
test.skip(!edgeReviewEnvironmentPresent(), 'Supply matching API, Body, relay and Knowledge service binaries.');
test.setTimeout(90_000);
const editor = page => page.getByRole('region', { name: 'Knowledge change editor', exact: true });
const map = page => page.getByRole('region', { name: 'Knowledge relationship map', exact: true });
const receipt = page => page.getByRole('region', { name: 'Knowledge relationship request', exact: true });
const decision = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const verdictReceipt = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const label = 'clarifies café 東京 🧭 <edge>';
const rationale = 'Exact direction and label. Keep <img src=x onerror="window.__edgeInjected=true"> literal.';
const saved = page => page.evaluate(() => Object.entries(localStorage).filter(([key]) => key.startsWith('face.knowledge-recovery.v1:')).map(([, value]) => JSON.parse(value)));
const trackPosts = page => { const values = []; page.on('request', request => { if (isKnowledgeCall(request)) values.push(request.postDataJSON()); }); return values; };
// A payload's change, without the request identity, prepared head and target.
const change = ({ request_id, record_head, target_id, ...rest }) => rest;
async function prepare(page, service, { other = service.practice, edge = null, rel = label } = {}) {
  await service.quiesce(); await page.goto(service.url('knowledge', { id: service.practice }));
  if (edge) {
    await map(page).getByRole('button', { name: 'Inspect relationship ' + edge.id, exact: true }).click();
    await map(page).getByRole('button', { name: 'Remove this relationship', exact: true }).click();
  } else {
    await map(page).getByRole('button', { name: 'Add relationship', exact: true }).click();
    if (other === service.practice) await editor(page).getByLabel('Other knowledge identity', { exact: true }).fill(other);
    else await editor(page).getByLabel('Choose visible knowledge item', { exact: true }).selectOption(other);
    await editor(page).getByLabel('Relationship direction', { exact: true }).selectOption('incoming');
    await editor(page).getByLabel('Relationship label', { exact: true }).fill(rel);
  }
  await editor(page).getByLabel('Reason for knowledge change', { exact: true }).fill(rationale);
  await editor(page).getByRole('button', { name: 'Review knowledge draft', exact: true }).click();
  // whether a relationship is reviewed is the policy's; its receipt says so
  await expect(editor(page).getByRole('status')).toContainText('proposed for a native Review');
}
async function send(page, service) {
  const pending = page.waitForResponse(r => new URL(r.url()).pathname === service.commandPath && isKnowledgeCall(r.request()));
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click(); const response = await pending;
  const line = response.request().postDataJSON(), settled = settleKnowledge(response.status(), await response.json());
  expect(settled.status).toBe(200); expect(settled.code).toBe(''); return { ...settled, line, request_id: line.payload.request_id };
}
async function propose(page, service, options) {
  await prepare(page, service, options); const sent = await send(page, service);
  const native = await service.waitRelationship(sent.request_id, r => r.relationship.proposal_state === 'created'); await service.quiesce();
  await receipt(page).getByRole('button', { name: 'Check relationship request', exact: true }).click(); await expect(receipt(page)).toContainText('Proposal created');
  return { ...sent, native, canonical: service.candidate(native.relationship.candidate_digest) };
}
async function decide(page, service, proposal, { verdict = 'approve', effect = proposal.line.call === 'KnowledgeEdgeUnlink' ? 'unlinked' : 'linked' } = {}) {
  await service.quiesce(); await page.goto(service.url('reviews', { id: proposal.native.relationship.review_id }));
  await expect(decision(page)).toContainText('You are its recorded proposer'); await expect(decision(page).getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  await service.asActor('bob'); await page.reload();
  expect(JSON.parse(await decision(page).locator('.relationship-canonical-document').textContent())).toEqual(proposal.canonical);
  await expect(decision(page)).toContainText(proposal.line.payload.rel); await expect(decision(page).locator('img')).toHaveCount(0);
  await decision(page).getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await decision(page).getByRole('radio', { name: verdict === 'approve' ? 'Approve' : 'Reject', exact: true }).check();
  await decision(page).getByRole('textbox', { name: 'Decision note', exact: true }).fill('Decide the exact directed tuple independently.');
  await decision(page).getByRole('button', { name: 'Review decision', exact: true }).click();
  const pending = page.waitForResponse(r => new URL(r.url()).pathname === service.apiPath + '/commands' && callOf(r.request()) === 'ReviewVerdict');
  await decision(page).getByRole('button', { name: 'Submit decision', exact: true }).click(); const response = await pending; expect(response.status()).toBe(200);
  await service.waitCommand(response.request().postDataJSON().payload.request_id, r => r.verdict.state === 'accepted'); await service.quiesce();
  await verdictReceipt(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(verdictReceipt(page)).toContainText('Reported by relationship request');
  await verdictReceipt(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click(); await expect(verdictReceipt(page)).toHaveCount(0);
  expect(JSON.parse(await decision(page).locator('.relationship-canonical-document').textContent())).toEqual(proposal.canonical);
  await service.asActor('alice'); return service.waitRelationship(proposal.request_id, r => r.relationship.effect_state === effect);
}
async function openResult(page, service) { await service.quiesce(); await page.goto(service.url('knowledge', { id: service.practice })); }
async function dismiss(page) { await receipt(page).getByRole('button', { name: 'Dismiss relationship request', exact: true }).click(); await expect(receipt(page)).toHaveCount(0); }

test('reviewed relationships: exact directed creation and selected removal require independent Reviews and preserve other tuples', async ({ page, service }, testInfo) => {
  const other = await service.createItem(), posts = trackPosts(page);
  const link = await propose(page, service, { other });
  expect(link.line.call).toBe('KnowledgeEdgeLink');
  expect(change(link.line.payload)).toEqual({ from_id: other, to_id: service.practice, rel: label, rationale });
  expect(link.receipt).toHaveProperty('relationship'); expect((await service.edges()).some(e => e.id === link.native.edge_id)).toBe(false);
  expect((await saved(page))[0]).toMatchObject({ version: 2, operation: 'dna.knowledge.edge.link' }); expect((await saved(page))[0]).not.toHaveProperty('mode');
  await expect(receipt(page)).not.toContainText('Observed in graph'); await decide(page, service, link); await openResult(page, service);
  await expect(receipt(page)).toContainText('Observed in graph'); await receipt(page).scrollIntoViewIfNeeded(); await page.screenshot({ path: testInfo.outputPath('reviewed-link-desktop.png') }); await dismiss(page);
  // A separate real direct request adds the reverse tuple; it cannot satisfy or
  // disappear with the reviewed forward tuple. Historical mode is per request.
  // Dismissing the receipt starts a read refresh. Leave that document before
  // deliberately replacing its API so the next navigation is a fresh load.
  await page.goto('about:blank');
  await service.setGrants([{ ...edgeReviewGrant, edge_link: 'direct' }]);
  const reverse = await service.command('edge.link', { from_id: service.practice, to_id: other, rel: label, rationale: 'Keep the reverse tuple independent.' }, service.practice);
  const direct = await service.post(reverse); expect(direct.code).toBe(''); expect(direct.receipt).not.toHaveProperty('relationship');
  await service.setGrants([edgeReviewGrant]);
  const edge = (await service.edges()).find(e => e.id === link.native.edge_id);
  const removal = await propose(page, service, { edge }); expect(removal.line.payload.edge_id).toBe(link.native.edge_id);
  await decide(page, service, removal); await openResult(page, service); await expect(receipt(page)).toContainText('Removal observed');
  const rows = await service.edges(); expect(rows.some(e => e.id === link.native.edge_id)).toBe(false); expect(rows.some(e => e.id === direct.receipt.edge_id)).toBe(true);
  expect(posts).toHaveLength(2); expect(await page.evaluate(() => window.__edgeInjected)).toBeUndefined();
  await page.setViewportSize({ width: 390, height: 844 }); await receipt(page).scrollIntoViewIfNeeded(); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); await page.screenshot({ path: testInfo.outputPath('reviewed-removal-mobile.png') });
});

test('reviewed relationships: lost reply recovers legacy metadata by GET after policy mode and full service restart', async ({ page, service }) => {
  const posts = trackPosts(page); await prepare(page, service); await service.pauseDelivery(); let command, admitted;
  await page.route('**/commands', async route => { if (!isKnowledgeCall(route.request())) return route.fallback(); const response = await route.fetch(); expect(response.status()).toBe(200); command = route.request().postDataJSON().payload; admitted = settleKnowledge(200, await response.json()).receipt; expect(admitted.relationship.proposal_state).toBe('pending'); await route.abort('failed'); });
  await editor(page).getByRole('button', { name: 'Submit knowledge change', exact: true }).click(); await expect(receipt(page)).toContainText('could not be confirmed');
  await page.evaluate(() => { const [key, raw] = Object.entries(localStorage).find(([key]) => key.startsWith('face.knowledge-recovery.v1:')); const m = JSON.parse(raw); m.version = 1; delete m.operation; localStorage.setItem(key, JSON.stringify(m)); });
  await page.unroute('**/commands'); await service.setGrants([{ ...edgeReviewGrant, edge_link: 'direct' }]);
  await service.waitRelationship(command.request_id, r => r.relationship.proposal_state === 'created'); await service.quiesce(); await page.reload();
  await expect(receipt(page)).toContainText('Proposal created'); await expect(receipt(page)).toContainText('Relationship effect'); await expect(receipt(page)).not.toContainText('Observed in graph');
  expect(posts).toHaveLength(1); expect((await service.edges()).length).toBe(0); const recovered = (await service.lookup(command.request_id)).receipt; expect(recovered).toMatchObject({ command_id: admitted.command_id, event_id: admitted.event_id, fingerprint: admitted.fingerprint }); expect(recovered).toHaveProperty('relationship');
  expect(service.journal().rows.filter(row => ['knowledge.edge.linked', 'knowledge.edge.unlinked', 'knowledge.edge.reviewed_linked', 'knowledge.edge.reviewed_unlinked'].includes(row.kind))).toEqual([]);
  expect(service.journal().rows.filter(row => row.kind === 'knowledge.edge.requested' && row.data?.request_id === admitted.command_id)).toHaveLength(1);
});

test('reviewed relationships: rejected Review declines the effect without graph success', async ({ page, service }) => {
  const proposal = await propose(page, service); await decide(page, service, proposal, { verdict: 'reject', effect: 'declined' }); await openResult(page, service);
  await expect(receipt(page).getByRole('button', { name: 'Relationship effect', exact: true })).toContainText('declined'); await expect(receipt(page)).not.toContainText('Observed in graph'); expect(await service.edges()).toEqual([]);
});

test('reviewed relationships: changed exact tuple basis leaves approved Review and refused effect separate', async ({ page, service }) => {
  const first = await propose(page, service); await dismiss(page); const second = await propose(page, service);
  await decide(page, service, first); await decide(page, service, second, { effect: 'refused' }); await openResult(page, service);
  await expect(receipt(page).getByRole('button', { name: 'Review', exact: true })).toContainText('approve'); await expect(receipt(page).getByRole('button', { name: 'Relationship effect', exact: true })).toContainText('refused'); await expect(receipt(page)).not.toContainText('Observed in graph');
  expect((await service.edges()).filter(e => e.id === first.native.edge_id)).toHaveLength(1);
});
