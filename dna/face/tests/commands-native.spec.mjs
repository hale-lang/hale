// Real HTTP through the head's forwarding route to its api socket, with an
// explicitly scripted provider. The head's person holds the board and the
// reviewer seat, so the gates open. This proves forwarding/browser
// composition, not DNA command durability.
import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test.use({ commandSubject: true, commandAdapter: true, seats: ['board', 'reviewer'] });
test.skip(!process.env.HALE_FACE_COMMAND_BIN, 'Supply the explicit scripted native command-adapter fixture.');

async function submit(page, service) {
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill('Exact replacement — première ligne\n第二行');
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill('Native HTTP adapter conformance only.');
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  const response = page.waitForResponse(result => result.url().endsWith('/commands') && isWrite(result.request()), { timeout: 15_000 });
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  return response;
}

test('scripted native provider: real origin-checked submit and exact-key reload recovery compose with the browser', async ({ page, service }) => {
  const before = await service.refs();
  const posts = [];
  page.on('request', request => { if (isWrite(request)) posts.push(request); });
  const submitted = await submit(page, service);
  expect(submitted.status()).toBe(200);
  const accepted = await submitted.json();
  expect(accepted.caller.via).toBe('http-session'); expect(accepted.role).toBe('position');
  expect(accepted.value.ok).toBe(true); expect(accepted.value.receipt.state).toBe('recorded');
  expect(accepted.value.receipt.subject_digest).toBe(service.practice);
  const requestID = accepted.value.receipt.request_id;
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
  expect((await submit(page, service)).status()).toBe(200);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await service.changeCommandMode('approve_pending');
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText('Approved');
  await expect(panel).toContainText('Pending — awaiting adoption');
  await service.changeCommandMode('activation_refused');
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText('Refused');
  await expect(panel).toContainText('Approved');
  // The head forwards the provider's receipt as written; a receipt for
  // another application is the face's to refuse.
  await service.changeCommandMode('malformed');
  const malformed = page.waitForResponse(result => result.url().includes('/commands?'));
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  expect((await (await malformed).json()).value.receipt.application_id).toBe('another-application');
  await expect(panel).toContainText('could not be verified');
  await expect(panel).not.toContainText('Approved');
});

async function submitVerdict(page, service, choice = 'Approve') {
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: choice, exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill('Exact candidate — conformance seulement.\n第二行');
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  const response = page.waitForResponse(result => result.url().endsWith('/commands') && isWrite(result.request()), { timeout: 15_000 });
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
  return response;
}

test('scripted native provider: verdict submission and reload preserve distinct Review and subject identities', async ({ page, service }) => {
  const before = await service.refs();
  const posts = [];
  page.on('request', request => { if (isWrite(request)) posts.push(request); });
  const response = await submitVerdict(page, service, 'Request revision');
  expect(response.status()).toBe(200);
  const line = await response.json();
  expect(line.role).toBe('reviewer');
  const receipt = line.value.receipt;
  expect(receipt.operation).toBe('dna.review.verdict');
  expect(receipt.target_id).toBe(service.pending_review);
  expect(receipt.subject_digest).toBe(service.pending_practice);
  expect(receipt.verdict_value).toBe('revise');
  expect(receipt.proposal_state).toBe('');
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
  expect(submitted.status()).toBe(200);
  const original = submitted.request().postDataJSON();
  expect(original.call).toBe('PracticePropose');
  const response = await page.request.post(service.origin + service.apiPath + '/commands', {
    headers: { Origin: service.origin, 'X-Hale-Command': '1', 'Content-Type': 'application/json' },
    data: {
      call: 'ReviewVerdict',
      payload: { request_id: original.payload.request_id, review_id: service.pending_review, subject_digest: service.pending_practice, verdict: 'approve', comment: 'Reused key conformance' },
    },
  });
  // The binding answered; the provider refused the reused key.
  expect(response.status()).toBe(200);
  const refused = await response.json();
  expect(refused.value).toMatchObject({ ok: false, code: 'request_conflict' });
  const lookup = await page.request.get(service.origin + service.apiPath + '/commands?' + new URLSearchParams({ request_id: original.payload.request_id }));
  expect(lookup.status()).toBe(200);
  expect((await lookup.json()).value.receipt.operation).toBe('dna.practice.propose');
});

test('scripted native provider: a line naming its own forwarder is refused before the socket', async ({ page, service }) => {
  const response = await page.request.post(service.origin + service.apiPath + '/commands', {
    headers: { Origin: service.origin, 'X-Hale-Command': '1', 'Content-Type': 'application/json' },
    data: { call: 'CommandLookup', payload: { request_id: 'x' }, via: 'http-session' },
  });
  expect(response.status()).toBe(400);
  expect((await response.json()).error.code).toBe('invalid_command');
});
