// Real HTTP through the Hale command adapter, with an explicitly scripted
// provider. This proves adapter/browser composition, not DNA command durability.
import { test, expect } from './harness.mjs';

test.use({ commandSubject: true, commandAdapter: true });
test.skip(!process.env.HALE_COCKPIT_COMMAND_BIN, 'Supply the explicit scripted native command-adapter fixture.');

async function submit(page, service) {
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill('Exact replacement — première ligne\n第二行');
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill('Native HTTP adapter conformance only.');
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  const response = page.waitForResponse(result => result.url().endsWith('/commands') && result.request().method() === 'POST', { timeout: 15_000 });
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  return response;
}

test('scripted native provider: real origin-checked submit and exact-key reload recovery compose with the browser', async ({ page, service }) => {
  const before = await service.refs();
  const posts = [];
  page.on('request', request => { if (request.method() === 'POST') posts.push(request); });
  const submitted = await submit(page, service);
  expect(submitted.status()).toBe(202);
  const accepted = await submitted.json();
  expect(accepted.data.subject_digest).toBe(service.practice);
  const requestID = accepted.data.request_id;
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/recorded/i);
  const recovered = page.waitForResponse(result => new URL(result.url()).searchParams.get('request_id') === requestID);
  await page.reload();
  expect((await recovered).status()).toBe(200);
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(requestID);
  expect(posts).toHaveLength(1);
  // The fake provider is deliberately not an implementation of domain writes.
  expect(await service.refs()).toBe(before);
});

test('scripted native provider: approval, adoption refusal and malformed-provider failure stay distinct', async ({ page, service }) => {
  expect((await submit(page, service)).status()).toBe(202);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await service.changeCommandMode('approve_pending');
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText('Approved');
  await expect(panel).toContainText('Pending — awaiting adoption');
  await service.changeCommandMode('activation_refused');
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText('Refused');
  await expect(panel).toContainText('Approved');
  await service.changeCommandMode('malformed');
  const refused = page.waitForResponse(result => result.url().includes('/commands?') && result.status() === 503);
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await refused;
  await expect(panel).toContainText(/unavailable/i);
  await expect(panel).not.toContainText('Approved');
});

async function submitVerdict(page, service, choice = 'Approve') {
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: choice, exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill('Exact candidate — conformance seulement.\n第二行');
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  const response = page.waitForResponse(result => result.url().endsWith('/commands') && result.request().method() === 'POST', { timeout: 15_000 });
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
  return response;
}

test('scripted native provider: verdict submission and reload preserve distinct Review and subject identities', async ({ page, service }) => {
  const before = await service.refs();
  const posts = [];
  page.on('request', request => { if (request.method() === 'POST') posts.push(request); });
  const response = await submitVerdict(page, service, 'Request revision');
  expect(response.status()).toBe(202);
  const receipt = (await response.json()).data;
  expect(receipt.operation).toBe('dna.review.verdict');
  expect(receipt.target.id).toBe(service.pending_review);
  expect(receipt.subject_digest).toBe(service.pending_practice);
  expect(receipt.verdict.value).toBe('revise');
  expect(receipt).not.toHaveProperty('proposal');
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/recorded/i);
  const recovered = page.waitForResponse(result => new URL(result.url()).searchParams.get('request_id') === receipt.request_id);
  await page.reload();
  expect((await recovered).status()).toBe(200);
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(receipt.request_id);
  expect(posts).toHaveLength(1);
  expect(await service.refs()).toBe(before);
});

test('scripted native provider: own verdict acceptance can precede Review settlement without adoption', async ({ page, service }) => {
  await service.changeCommandMode('verdict_pending');
  const response = await submitVerdict(page, service, 'Reject');
  expect(response.status()).toBe(200);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(panel).toContainText(/accepted/i);
  await expect(panel).toContainText(/pending/i);
  await service.changeCommandMode('verdict_settled');
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText(/rejected/i);
  await expect(panel).toContainText(/unknown/i);
});

test('scripted native provider: refused verdict remains refused beside another command’s adoption', async ({ page, service }) => {
  await service.changeCommandMode('verdict_refused_after_adoption');
  expect((await submitVerdict(page, service, 'Reject')).status()).toBe(200);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(panel).toContainText(/refused/i);
  await expect(panel).toContainText(/approved/i);
  await expect(panel).toContainText('Adopted');
  await expect(panel).not.toContainText(/Command state · succeeded/i);
});

test('scripted native provider: one recovery namespace rejects a request key reused across operations', async ({ page, service }) => {
  const submitted = await submit(page, service);
  expect(submitted.status()).toBe(202);
  const original = submitted.request().postDataJSON();
  const response = await page.request.post(service.origin + service.apiPath + '/commands', {
    headers: { Origin: service.origin, 'X-Hale-Command': '1' },
    data: {
      ...original, operation: 'dna.review.verdict',
      target: { ...original.target, kind: 'dna.review', id: service.pending_review },
      preconditions: { ...original.preconditions, subject_digest: service.pending_practice, review_state: 'pending' },
      arguments: { verdict: 'approve', comment: 'Reused key conformance' },
    },
  });
  expect(response.status()).toBe(409);
  expect((await response.json()).error.code).toBe('request_conflict');
  const lookup = await page.request.get(service.origin + service.apiPath + '/commands?' + new URLSearchParams({ request_id: original.request_id }));
  expect(lookup.status()).toBe(200);
  expect((await lookup.json()).data.operation).toBe('dna.practice.propose');
});
