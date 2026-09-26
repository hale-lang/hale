// Practice entry points use the real native node/binding provider and Reviews.
// Each case is a small fresh Record; no browser-owned outcome or graph fixtures.
import { test as base, expect } from '@playwright/test';
import { startBindingService, bindingEnvironmentPresent } from './native-knowledge-binding-harness.mjs';

const grant = { mode: 'local', name: 'alice', authority: 'board', edge_link: 'direct', edge_unlink: 'direct',
  node_propose: 'review', node_revise: 'review', node_retire: 'review', node_scopes: [{ author: 'org', target: 'org/elsewhere' }],
  binding_bind: 'review', binding_unbind: 'review', binding_scopes: [{ author: 'org', target: 'org/support' }], recover: true };
const test = base.extend({
  service: async ({}, use, info) => {
    const service = await startBindingService({ grants: [grant] });
    try { await use(service); }
    finally { await service.stop(); await info.attach('native-practice-service', { path: service.evidence + '/service.json', contentType: 'application/json' }); expect(service.processes()).toEqual([]); }
  },
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
});
test.skip(true, "The HTTP record-command route was cut (GH #1104 piece 5, PR #1129): record commands are the head socket's gated topics, which a browser cannot reach; this lane waits for the face's write path.");
test.skip(!bindingEnvironmentPresent(), 'Supply matching native API, Body, relay and Knowledge service.');
test.setTimeout(90_000);
const editor = page => page.getByRole('region', { name: 'Practice change editor', exact: true });
const receipt = page => page.getByRole('region', { name: /^Knowledge (change|binding) request$/ });
const intervention = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const text = 'A Practice with exact café 🧭 evidence.\nKeep <literal> source history.';
const reason = 'Preserve this scoped Practice and its human rationale.';
const responseFor = (page, path, method) => page.waitForResponse(response => new URL(response.url()).pathname === path && response.request().method() === method);
async function reviewAndSubmit(page, service, binding = false) {
  await editor(page).getByLabel('Reason for practice change', { exact: true }).fill(reason);
  await editor(page).getByRole('button', { name: 'Review practice draft', exact: true }).click();
  await expect(editor(page).getByRole('status')).toContainText('Draft reviewed against the current visible snapshot');
  const pending = responseFor(page, service.commandPath, 'POST');
  await editor(page).getByRole('button', { name: 'Submit practice change', exact: true }).click();
  const response = await pending; expect(response.status()).toBe(202);
  const command = response.request().postDataJSON();
  const native = binding ? await service.waitBinding(command.request_id, value => value.binding.proposal_state === 'created') : await service.waitNode(command.request_id, value => value.node.proposal_state === 'created');
  await service.quiesce();
  await receipt(page).getByRole('button', { name: binding ? 'Check binding request' : 'Check knowledge request', exact: true }).click();
  return { command, native, binding, candidate: binding ? native.binding.candidate_digest : native.node.candidate_digest, review: binding ? native.binding.review_id : native.node.review_id };
}
async function approve(page, service, proposal) {
  await receipt(page).getByRole('link', { name: proposal.binding ? 'Open exact binding Review' : 'Open exact Review', exact: true }).click();
  await expect(intervention(page)).toContainText(proposal.candidate);
  await service.asActor('bob'); await page.reload();
  await intervention(page).getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await intervention(page).getByRole('radio', { name: 'Approve', exact: true }).check();
  await intervention(page).getByLabel('Decision note', { exact: true }).fill('Independently approve the exact Practice lifecycle candidate.');
  await intervention(page).getByRole('button', { name: 'Review decision', exact: true }).click();
  const pending = responseFor(page, service.apiPath + '/commands', 'POST');
  await intervention(page).getByRole('button', { name: 'Submit decision', exact: true }).click();
  const response = await pending; expect([200, 202]).toContain(response.status());
  await service.waitCommand(response.request().postDataJSON().request_id, value => value.verdict.state === 'accepted');
  await service.quiesce();
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await service.asActor('alice');
  if (proposal.binding) await service.waitBinding(proposal.command.request_id, value => value.binding.effect_state === 'bound');
  else await service.waitNode(proposal.command.request_id, value => value.node.activation_state === 'adopted');
}
async function observed(page, service, proposal, target = '') {
  const retiring = proposal.command.operation === 'dna.knowledge.node.retire';
  const id = proposal.binding || retiring ? proposal.command.target.id : proposal.candidate;
  await service.quiesce(); await page.goto(service.url('knowledge', { id, ...(target ? { target } : {}) }));
  await expect(receipt(page)).toContainText(proposal.binding ? 'Binding observed' : retiring ? 'Retirement observed' : 'Adoption observed');
  await receipt(page).getByRole('button', { name: proposal.binding ? 'Dismiss binding request' : 'Dismiss knowledge request', exact: true }).click();
  return id;
}
async function openPractice(page, service, id) {
  await service.quiesce(); await page.goto(service.url('practices', { id }));
  await expect(page.getByRole('link', { name: 'Manage applicability', exact: true })).toBeVisible();
}
async function precreate(service) {
  const command = await service.command('node.propose', { kind: 'practice', name: 'practice/browser-applicability', text, author: 'org', target: 'org/elsewhere', rationale: reason }, 'org/elsewhere');
  expect((await service.post(command)).status).toBe(202);
  const created = await service.waitNode(command.request_id, value => value.node.proposal_state === 'created');
  await service.decide(created.node.candidate_digest, created.node.review_id); await service.waitNode(command.request_id, value => value.node.activation_state === 'adopted');
  return created.node.candidate_digest;
}

test('Practice create and revision entries retain exact native text, provenance and canonical scope', async ({ page, service }, info) => {
  await page.goto(service.url('practices'));
  await page.getByRole('link', { name: 'Create practice', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Practice administration', exact: true })).toBeVisible();
  await expect(editor(page).getByLabel('Practice kind', { exact: true })).toBeDisabled();
  await editor(page).getByLabel('Practice name', { exact: true }).fill('practice/browser-lifecycle');
  await editor(page).getByLabel('Practice text', { exact: true }).fill(text);
  await editor(page).getByLabel('Requested authoring locus', { exact: true }).fill('org');
  await editor(page).getByLabel('Requested target locus', { exact: true }).fill('org/elsewhere');
  const first = await reviewAndSubmit(page, service);
  expect(first.command.operation).toBe('dna.knowledge.node.propose'); expect(first.command.arguments.kind).toBe('practice');
  expect(first.command.arguments.text).toBe(text);
  // Proposal evidence is inspected separately; it is not graph adoption.
  expect(service.candidate(first.candidate).text).toBe(text);
  await expect(receipt(page)).toContainText('Not established');
  await approve(page, service, first); await observed(page, service, first);
  await page.getByRole('link', { name: /^Open [Pp]ractice$/ }).click();
  await expect(page.getByRole('link', { name: 'Edit practice & scope', exact: true })).toBeVisible();
  await expect(page.locator('.practice-detail')).toContainText('alice'); await expect(page.locator('.practice-detail')).toContainText(reason);
  await page.getByRole('link', { name: 'Edit practice & scope', exact: true }).click();
  await expect(editor(page).getByLabel('Requested target locus', { exact: true })).toHaveValue('org/elsewhere');
  await expect(editor(page).getByLabel('Practice text', { exact: true })).toHaveValue(text);
  const revisedText = text + '\nAdd the exact review evidence.';
  await editor(page).getByLabel('Practice text', { exact: true }).fill(revisedText);
  const revised = await reviewAndSubmit(page, service); expect(revised.command.operation).toBe('dna.knowledge.node.revise');
  expect(revised.command.arguments.supersedes).toBe(first.candidate); expect(revised.command.arguments.target).toBe('org/elsewhere');
  await approve(page, service, revised); await observed(page, service, revised);
  await page.getByRole('link', { name: /^Open [Pp]ractice$/ }).click();
  await expect(page.locator('.practice-detail')).toContainText(revisedText);
  const old = await service.read(service.apiPath + '/dna/practices?' + new URLSearchParams({ id: first.candidate }));
  expect(old.json.data.items[0]).toMatchObject({ kind: 'practice', state: 'retired', text, requester: 'alice', rationale: reason });
  await page.screenshot({ path: info.outputPath('practice-revised-native.png') });
  expect(service.journal().rows.length).toBeLessThan(100);
});

test('Practice applicability and retirement entries use exact native binding and retirement Reviews', async ({ page, service }, info) => {
  const id = await precreate(service); await openPractice(page, service, id);
  await page.getByRole('link', { name: 'Manage applicability', exact: true }).click();
  await expect(editor(page)).toBeVisible();
  await editor(page).getByLabel('Requested target locus', { exact: true }).fill('org/support');
  const binding = await reviewAndSubmit(page, service, true); expect(binding.command.operation).toBe('dna.knowledge.binding.bind'); expect(binding.command.arguments.idea_id).toBe(id);
  await approve(page, service, binding); await observed(page, service, binding, 'org/support/urgent');
  expect((await service.bindings(id)).map(row => row.target).sort()).toEqual(['org/elsewhere', 'org/support']);
  await openPractice(page, service, id); await page.getByRole('link', { name: 'Retire practice', exact: true }).click();
  const retirement = await reviewAndSubmit(page, service); expect(retirement.command.operation).toBe('dna.knowledge.node.retire'); expect(retirement.command.target.id).toBe(id);
  expect(service.candidate(retirement.candidate)).toMatchObject({ kind: 'retirement', supersedes: id, text: reason });
  await approve(page, service, retirement); await observed(page, service, retirement);
  await page.getByRole('link', { name: /^Open [Pp]ractice$/ }).click();
  await expect(page.locator('.practice-detail')).toContainText('Retired'); await expect(page.locator('.practice-detail')).toContainText(text);
  await expect(page.getByRole('link', { name: 'Retire practice', exact: true })).toHaveCount(0);
  const scope = await service.request(service.apiPath + '/dna/knowledge/nodes?' + new URLSearchParams({ target: 'org/support/urgent', limit: '25' })); expect(scope.body.data.items.filter(row => row.id === id)).toEqual([]);
  expect(await service.bindings(id)).toHaveLength(2); expect(service.journal().rows.length).toBeLessThan(100);
  await page.screenshot({ path: info.outputPath('practice-retired-native-history.png') });
  await page.goto(service.url('practices', { id: retirement.candidate }));
  await expect(page.locator('.practice-detail').getByText('Retirement document', { exact: true })).toBeVisible();
  await expect(page.locator('.practice-detail')).toContainText('it is not an active practice');
  await expect(page.locator('.practice-detail')).toContainText('Retirement · Ratified');
  await expect(page.getByRole('link', { name: 'View target version', exact: true })).toBeVisible();
  for (const label of ['Manage applicability', 'Edit practice & scope', 'Retire practice']) await expect(page.getByRole('link', { name: label, exact: true })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('practice-retirement-document-native.png') });
});
