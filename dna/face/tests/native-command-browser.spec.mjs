// Actual browser → command API → host relay → DNA body → Record outcomes.
// The lost-reply case drops only transport after a real native POST completed.
import { test as base, expect } from '@playwright/test';
import { nativeCommandEnvironmentPresent, startService } from './native-command-harness.mjs';

const test = base.extend({
  service: async ({}, use, testInfo) => {
    const service = await startService();
    try { await use(service); }
    finally {
      await service.stop();
      await testInfo.attach('native-service-evidence', { path: service.evidence + '/service.json', contentType: 'application/json' });
    }
  },
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await use(page);
    expect(errors, 'No unhandled face JavaScript error').toEqual([]);
  },
});
test.skip(true, "The HTTP record-command route was cut (GH #1104 piece 5, PR #1129): record commands are the head socket's gated topics, which a browser cannot reach; this lane waits for the face's write path.");
test.skip(!nativeCommandEnvironmentPresent(), 'Supply explicit HALE_NATIVE_COMMAND_API/BODY/RELAY binaries for real native browser acceptance.');
test.setTimeout(75_000);

const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const intervention = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const stage = (page, name) => recovery(page).getByRole('list', { name: 'Request and outcome', exact: true }).getByRole('button', { name, exact: true });
const postRequests = page => {
  const posts = [];
  page.on('request', request => { if (request.method() === 'POST' && new URL(request.url()).pathname.endsWith('/commands')) posts.push(request.postDataJSON()); });
  return posts;
};
const metadata = page => page.evaluate(() => Object.entries(localStorage)
  .filter(([key]) => key.startsWith('face.practice-recovery.v1:')).map(([, value]) => JSON.parse(value)));

async function prepareProposal(page, service, text, rationale = 'Browser-operated native replacement — evidence stays exact.') {
  await service.quiesce();
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill(text);
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill(rationale);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  expect(await page.locator('#proposal-comparison .comparison-after').textContent()).toBe(text);
}
async function send(page, service, label) {
  const response = page.waitForResponse(result => new URL(result.url()).pathname === service.apiPath + '/commands' && result.request().method() === 'POST');
  await page.getByRole('button', { name: label, exact: true }).click();
  const result = await response;
  expect([200, 202]).toContain(result.status());
  const payload = await result.json();
  expect(payload.source.record_id).toBe(service.application);
  expect(payload.data.principal).toEqual(service.principal);
  return { receipt: payload.data, command: result.request().postDataJSON() };
}
async function showOutcome(page, service, request, predicate) {
  const native = await service.waitCommand(request.receipt.request_id, predicate);
  await service.quiesce();
  const response = page.waitForResponse(result => result.request().method() === 'GET' && new URL(result.url()).searchParams.get('request_id') === request.receipt.request_id);
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  const result = await response;
  expect(result.status()).toBe(200);
  const receipt = (await result.json()).data;
  expect(receipt.command_id).toBe(native.command_id);
  expect(receipt.fingerprint).toBe(native.fingerprint);
  expect(predicate(receipt)).toBe(true);
  await expect(recovery(page)).toContainText(request.receipt.request_id);
  return receipt;
}
async function propose(page, service, text) {
  await prepareProposal(page, service, text);
  const sent = await send(page, service, 'Submit proposal');
  const receipt = await showOutcome(page, service, sent, value => value.proposal?.state === 'created');
  expect(service.candidate(receipt.proposal.candidate_digest).text).toBe(text);
  expect(service.facts('practice.proposed', receipt.command_id)).toHaveLength(1);
  return { ...sent, receipt, text };
}
async function dismiss(page) {
  await recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  expect(await metadata(page)).toEqual([]);
}
async function prepareDecision(page, service, proposal, { followLink = false } = {}) {
  await service.quiesce();
  if (followLink) {
    await recovery(page).getByRole('link', { name: 'Open proposal review', exact: true }).click();
    await expect(page).toHaveURL(service.url('reviews', { id: proposal.receipt.proposal.review_id }));
    await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
    await dismiss(page);
  } else {
    await page.goto(service.url('reviews', { id: proposal.receipt.proposal.review_id }));
  }
  const text = intervention(page).locator('.intervention-document .document-text');
  await expect(text).toBeVisible();
  expect(await text.textContent()).toBe(proposal.text);
  await expect(intervention(page)).toContainText(proposal.receipt.proposal.candidate_digest);
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: 'Approve', exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill('Approve the exact canonical candidate — café 東京 🧭.');
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  await expect(intervention(page)).toContainText(proposal.receipt.proposal.review_id);
  await expect(intervention(page)).toContainText(proposal.receipt.proposal.candidate_digest);
}
function adoptionFacts(service, proposal, decision) {
  const candidate = proposal.receipt.proposal.candidate_digest;
  const review = proposal.receipt.proposal.review_id;
  const facts = service.facts('review.command_decided', decision.command_id);
  expect(facts).toHaveLength(1);
  expect(facts[0].data).toMatchObject({ command_id: decision.command_id, review_id: review, subject_digest: candidate, accepted: true, settled: true, outcome: 'approve', reviewer: 'alice' });
  expect(service.facts('knowledge.ratified', candidate)).toHaveLength(1);
  expect(service.facts('knowledge.retired', service.practice)).toHaveLength(1);
  expect(service.facts('knowledge.retired', service.practice)[0].data.by).toBe(candidate);
}

test('real native browser: propose, inspect the exact Review, approve and follow adoption', async ({ page, service }, testInfo) => {
  const posts = postRequests(page);
  const text = 'Collect the exact receipt.\nKeep <img src=x onerror="window.__nativeInjected=true"> literal — café 東京 🧭.\n';
  const proposal = await propose(page, service, text);
  expect(proposal.command.preconditions.subject_digest).toBe(service.practice);
  expect(proposal.command.arguments.text).toBe(text);
  await expect(stage(page, 'Proposal')).toContainText('created');
  await expect(stage(page, 'Adoption')).toContainText('Unknown');
  await prepareDecision(page, service, proposal, { followLink: true });
  expect(await page.evaluate(() => window.__nativeInjected)).toBeUndefined();
  await expect(intervention(page).locator('img')).toHaveCount(0);
  await intervention(page).screenshot({ path: testInfo.outputPath('native-exact-review.png') });
  const sent = await send(page, service, 'Submit decision');
  expect(sent.command.target.id).toBe(proposal.receipt.proposal.review_id);
  expect(sent.command.preconditions.subject_digest).toBe(proposal.receipt.proposal.candidate_digest);
  expect(sent.command.target.id).not.toBe(sent.command.preconditions.subject_digest);
  const decision = await showOutcome(page, service, sent, value => value.activation?.state === 'adopted');
  await expect(stage(page, 'This verdict')).toContainText('accepted');
  await expect(stage(page, 'Review settlement')).toContainText('Approved');
  await expect(stage(page, 'Adoption')).toContainText('Adopted');
  adoptionFacts(service, proposal, decision);
  await recovery(page).screenshot({ path: testInfo.outputPath('native-browser-adopted.png') });
  await recovery(page).getByRole('link', { name: 'Open candidate practice', exact: true }).click();
  await expect(page.locator('.practice-detail .detail-kicker')).toContainText('Ratified');
  expect(await page.locator('.practice-detail > .detail-section').first().locator('.document-text').textContent()).toBe(text);
  expect(posts).toHaveLength(2);
});

test('real native browser: a lost reply survives API/body restart and reload recovers by GET only', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const posts = postRequests(page);
  const text = 'Retain this exact request after a lost browser reply.\n東京 🧭';
  await prepareProposal(page, service, text);
  await service.pauseDelivery();
  let delivered;
  await page.route('**/commands', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    // This is transport fault injection after actual native admission. No
    // successful response or outcome is synthesized by the browser fixture.
    const response = await route.fetch({ maxRetries: 0 });
    expect([200, 202]).toContain(response.status());
    delivered = (await response.json()).data;
    await route.abort('failed');
  });
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  await expect.poll(() => delivered?.request_id).toBeTruthy();
  await expect(recovery(page)).toContainText(/unconfirmed|could not|not confirmed|unavailable/i);
  expect(posts).toHaveLength(1);
  const saved = await metadata(page);
  expect(saved).toHaveLength(1); expect(saved[0].request_id).toBe(delivered.request_id);
  expect(JSON.stringify(saved)).not.toContain(text);
  expect(service.admissions(delivered.request_id)).toHaveLength(1);
  expect(service.facts('practice.proposed', delivered.command_id)).toHaveLength(0);
  await page.unroute('**/commands');
  const origin = service.origin;
  await service.restart();
  expect(service.origin).toBe(origin);
  const settled = await service.waitCommand(delivered.request_id, value => value.proposal?.state === 'created');
  await service.quiesce();
  const response = page.waitForResponse(result => result.request().method() === 'GET' && new URL(result.url()).searchParams.get('request_id') === delivered.request_id);
  await page.reload();
  expect((await response).status()).toBe(200);
  await expect(stage(page, 'Proposal')).toContainText('created');
  await expect(recovery(page)).toContainText(delivered.request_id);
  expect(await metadata(page)).toEqual(saved);
  expect(posts).toHaveLength(1);
  expect(settled.command_id).toBe(delivered.command_id);
  expect(settled.fingerprint).toBe(delivered.fingerprint);
  expect(service.admissions(delivered.request_id)).toHaveLength(1);
  expect(service.candidate(settled.proposal.candidate_digest).text).toBe(text);
  await recovery(page).scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await recovery(page).screenshot({ path: testInfo.outputPath('native-browser-recovered-mobile.png') });
});

test('real native browser: competing replacements keep Review approval separate from adoption refusal', async ({ page, service }, testInfo) => {
  const posts = postRequests(page);
  const first = await propose(page, service, 'First replacement of the shared predecessor.');
  await dismiss(page);
  const second = await propose(page, service, 'Second replacement of the same exact predecessor.');
  await dismiss(page);
  await prepareDecision(page, service, first);
  const firstSent = await send(page, service, 'Submit decision');
  const accepted = await showOutcome(page, service, firstSent, value => value.activation?.state === 'adopted');
  adoptionFacts(service, first, accepted);
  await dismiss(page);
  await prepareDecision(page, service, second);
  const secondSent = await send(page, service, 'Submit decision');
  const declined = await showOutcome(page, service, secondSent, value => value.activation?.state === 'refused');
  expect(declined.state).toBe('succeeded'); expect(declined.verdict.state).toBe('accepted');
  expect(declined.review.outcome).toBe('approve');
  await expect(stage(page, 'This verdict')).toContainText('accepted');
  await expect(stage(page, 'Review settlement')).toContainText('Approved');
  await expect(stage(page, 'Adoption')).toContainText('Refused');
  await stage(page, 'Adoption').click();
  await expect(recovery(page).getByRole('group', { name: 'Selected outcome', exact: true })).toContainText('refused');
  const facts = service.facts('knowledge.refused', second.receipt.proposal.candidate_digest);
  expect(facts).toHaveLength(1);
  expect(facts[0].data.retired_by).toBe(first.receipt.proposal.candidate_digest);
  expect(service.facts('knowledge.ratified', second.receipt.proposal.candidate_digest)).toHaveLength(0);
  expect(service.facts('knowledge.retired', service.practice)[0].data.by).toBe(first.receipt.proposal.candidate_digest);
  expect(posts).toHaveLength(4);
  await recovery(page).screenshot({ path: testInfo.outputPath('native-browser-adoption-refused.png') });
});
