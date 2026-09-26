// Browser verdict conformance over native canonical candidate reads.
// Scripted receipts do not prove native authority, correlation or durability.
import { test, expect } from './harness.mjs';
import { STORAGE_PREFIX, recoveryMetadata, scriptedCommands } from './command-fixture.mjs';
import { CALLS } from './command-wire.mjs';

test.use({ commandSubject: true });
const NOTE = 'Exact candidate checked — 第二版\nKeep <img src=x onerror="window.__verdictInjected=true"> literal.';
const panel = page => page.getByRole('region', { name: 'Review intervention', exact: true });
const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const scriptFor = (page, service, options = {}) => scriptedCommands(page, service, { reviewProfile: true, ...options });

async function prepare(page, service, decision = 'Approve', note = NOTE) {
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: decision, exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill(note);
}
async function confirm(page) {
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
}
async function submit(page) {
  await confirm(page);
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
}

test('verdict browser contract: the plain head, and a slice without ReviewVerdict, cannot enable a decision', async ({ page, service }) => {
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  const script = await scriptedCommands(page, service, { reviewWriteOnly: true });
  await page.reload();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.posts).toHaveLength(0);
});

test('verdict browser contract: confirm the same-snapshot canonical candidate and submit its distinct Review identity', async ({ page, service }, testInfo) => {
  const script = await scriptFor(page, service);
  const before = await service.refs();
  await prepare(page, service);
  expect(script.candidateReads).toHaveLength(1);
  expect(script.candidateReads[0].searchParams.get('snapshot')).toBe(script.source.record_head);
  await confirm(page);
  await expect(panel(page)).toContainText(service.pending_text);
  await expect(panel(page)).toContainText(service.pending_practice);
  await expect(panel(page)).toContainText(service.pending_review);
  await expect(panel(page)).toContainText(NOTE);
  expect(script.posts).toHaveLength(0);
  await panel(page).screenshot({ path: testInfo.outputPath('verdict-confirmation.png') });
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
  await expect.poll(() => script.posts.length).toBe(1);
  const body = script.posts[0].body;
  expect(body).toEqual({
    call: 'ReviewVerdict',
    payload: { request_id: expect.any(String), review_id: service.pending_review, subject_digest: service.pending_practice, verdict: 'approve', comment: NOTE },
  });
  expect(body.payload.review_id).not.toBe(body.payload.subject_digest);
  expect(script.savedBeforeSend[0].value).toEqual({
    version: 2, application_id: service.application, principal: script.principal,
    request_id: body.payload.request_id, operation: 'dna.review.verdict', operation_version: '1',
    position_id: 'org', target_kind: 'dna.review', target_id: service.pending_review, subject_digest: service.pending_practice,
  });
  expect(JSON.stringify(await recoveryMetadata(page))).not.toContain(NOTE);
  expect(await page.evaluate(() => window.__verdictInjected)).toBeUndefined();
  await expect(recovery(page)).toContainText(/recorded/i);
  expect(await service.refs()).toBe(before);
});

test('verdict browser contract: choose deliberately; empty notes are valid and do not choose a verdict', async ({ page, service }) => {
  const script = await scriptFor(page, service, { verdictStage: 'accepted_pending' });
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await expect(page.getByRole('radio', { checked: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Submit decision', exact: true })).toHaveCount(0);
  await page.getByRole('radio', { name: 'Reject', exact: true }).check();
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  expect(script.posts[0].body.payload).toMatchObject({ verdict: 'reject', comment: '' });
  await expect(recovery(page)).toContainText(/accepted/i);
  await expect(recovery(page)).toContainText(/pending/i);
  await expect(recovery(page)).toContainText(/unknown/i);
});

test('verdict browser contract: accepted rejection is command success and does not claim adoption', async ({ page, service }) => {
  const script = await scriptFor(page, service, { verdictStage: 'settled' });
  await prepare(page, service, 'Reject');
  await submit(page);
  await expect(recovery(page)).toContainText(/succeeded/i);
  await expect(recovery(page)).toContainText(/rejected/i);
  await expect(recovery(page)).toContainText(/unknown/i);
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: a refused command cannot borrow another command’s settled approval or adoption', async ({ page, service }) => {
  await scriptFor(page, service, { verdictStage: 'refused_other_approved' });
  await prepare(page, service, 'Reject');
  await submit(page);
  await expect(recovery(page)).toContainText(/refused/i);
  await expect(recovery(page)).toContainText(/approved/i);
  await expect(recovery(page)).toContainText('Adopted');
  await expect(recovery(page)).not.toContainText(/Command state · succeeded/i);
});

test('verdict browser contract: approval and later adoption refusal stay separate', async ({ page, service }) => {
  const script = await scriptFor(page, service, { verdictStage: 'accepted_pending' });
  await prepare(page, service);
  await submit(page);
  await expect(recovery(page)).toContainText(/accepted/i);
  await expect(recovery(page)).toContainText(/pending/i);
  script.verdictStage = 'activation_refused';
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(recovery(page)).toContainText(/approved/i);
  await expect(recovery(page)).toContainText('The candidate could not replace its predecessor.');
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: lost reply recovers the original operation through a Review read outage', async ({ page, service }) => {
  const script = await scriptFor(page, service, { postMode: 'lost', getMode: 'unavailable' });
  await prepare(page, service, 'Request revision');
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const requestID = script.posts[0].body.payload.request_id;
  script.getMode = 'receipt';
  script.reviewsUnavailable = true;
  script.reviewAuthorized = false;
  await page.reload();
  await expect(recovery(page)).toContainText(requestID);
  await expect(recovery(page)).toContainText(/recorded/i);
  expect(script.gets).toContain(requestID);
  expect(script.posts).toHaveLength(1);
  await expect(page.locator('body')).not.toContainText(NOTE);
  await expect(page.locator('body')).not.toContainText(service.pending_text);
});

test('verdict browser contract: mismatched and protected candidates cannot enable decisions', async ({ page, service }) => {
  const script = await scriptFor(page, service, { candidateMismatch: true });
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await expect(page.getByRole('button', { name: 'Submit decision', exact: true })).toHaveCount(0);
  await expect.poll(async () => page.getByRole('button', { name: 'Prepare decision', exact: true }).isEnabled().catch(() => false)).toBe(false);
  script.candidateMismatch = false;
  await page.reload();
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await expect(panel(page)).toContainText(service.pending_text);
  await service.mutate('redact-pending');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.locator('body')).not.toContainText(service.pending_text);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.posts).toHaveLength(0);
});

test('verdict browser contract: mutation, quorum, non-board and legacy unknown Review kinds cannot enable decisions', async ({ page, service }) => {
  const script = await scriptFor(page, service, { reviewMutation: true });
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  script.reviewMutation = false;
  script.reviewApprovers = 'alice,bob';
  await page.reload();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  script.reviewApprovers = '';
  script.reviewAuthority = 'owner';
  await page.reload();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  script.reviewAuthority = '';
  script.missingReviewFacts = true;
  await page.reload();
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect(script.candidateReads).toHaveLength(0);
  expect(script.posts).toHaveLength(0);
});

test('verdict browser contract: identity refusal clears the candidate and retains only the original scoped key', async ({ page, service }) => {
  const script = await scriptFor(page, service, { postMode: 'identity_changed' });
  await prepare(page, service);
  await submit(page);
  await expect(page.getByRole('heading', { name: 'Sign-in identity changed', exact: true })).toBeVisible();
  await expect(page.locator('body')).not.toContainText(service.pending_text);
  await expect(page.locator('body')).not.toContainText(NOTE);
  expect((await recoveryMetadata(page))[0].value.principal).toEqual(script.principal);
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: wrong returned choice fails closed without losing the request', async ({ page, service }) => {
  const script = await scriptFor(page, service, { wrongChoice: true, verdictStage: 'accepted_pending' });
  await prepare(page, service, 'Approve');
  await submit(page);
  await expect(recovery(page)).toContainText(/unknown|invalid|incomplete|could not/i);
  await expect(recovery(page)).not.toContainText(/Command state · succeeded/i);
  expect(await recoveryMetadata(page)).toHaveLength(1);
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: existing v1 practice recovery remains unchanged and blocks a competing verdict', async ({ page, service }) => {
  const script = await scriptFor(page, service);
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill('New practice');
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill('Migration conformance');
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  await expect(recovery(page)).toContainText(/recorded/i);
  await page.evaluate(prefix => {
    const key = Object.keys(localStorage).find(key => key.startsWith(prefix));
    const value = JSON.parse(localStorage.getItem(key));
    const { application_id, principal, request_id, target_id, subject_digest } = value;
    localStorage.setItem(key, JSON.stringify({ version: 1, application_id, principal, request_id, target_id, subject_digest }));
  }, STORAGE_PREFIX);
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await expect(recovery(page)).toContainText(/recorded/i);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  expect((await recoveryMetadata(page))[0].value.version).toBe(1);
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: proposal and verdict tabs share one request reservation', async ({ page, context, service }) => {
  const script = await scriptFor(page, service);
  const second = await context.newPage();
  try {
    await scriptFor(second, service, { posts: script.posts });
    await prepare(page, service);
    await confirm(page);
    await second.goto(service.url('practices', { id: service.practice }));
    await second.getByRole('button', { name: 'Propose revision', exact: true }).click();
    await second.getByRole('textbox', { name: 'Proposed text', exact: true }).fill('Competing practice change');
    await second.getByRole('textbox', { name: 'Rationale', exact: true }).fill('Another tab');
    await second.getByRole('button', { name: 'Review proposal', exact: true }).click();
    await Promise.all([
      page.getByRole('button', { name: 'Submit decision', exact: true }).click(),
      second.getByRole('button', { name: 'Submit proposal', exact: true }).click(),
    ]);
    await expect.poll(() => script.posts.length).toBe(1);
    const saved = (await recoveryMetadata(page))[0].value;
    expect(saved.request_id).toBe(script.posts[0].body.payload.request_id);
    expect(CALLS[saved.operation]).toBe(script.posts[0].body.call);
  } finally { await second.close(); }
});

test('verdict browser contract: complete proposal handoff keeps the exact candidate and requires explicit slot release', async ({ page, service }) => {
  // This candidate already exists in the native fixture. The scripted proposal
  // receipt tests navigation and handoff only; it does not prove native creation.
  const script = await scriptFor(page, service, {
    stage: 'created', proposalCandidate: service.pending_practice,
    proposalReview: service.pending_review, verdictStage: 'adopted',
  });
  const before = await service.refs();
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill(service.pending_text);
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill('Browser handoff conformance only');
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  await page.getByRole('link', { name: 'Open proposal review', exact: true }).click();
  await expect(panel(page)).toContainText(service.pending_text);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  const firstKey = script.posts[0].body.payload.request_id;
  await page.getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: 'Approve', exact: true }).check();
  await submit(page);
  await expect(recovery(page)).toContainText('Adopted');
  expect(script.posts).toHaveLength(2);
  expect(script.posts[1].body.payload.request_id).not.toBe(firstKey);
  expect(script.posts[1].body.payload.review_id).toBe(service.pending_review);
  expect(script.posts[1].body.payload.subject_digest).toBe(service.pending_practice);
  expect(await service.refs()).toBe(before);
});

test('verdict browser contract: dismissing a completed request refreshes candidate eligibility before another decision', async ({ page, service }) => {
  const script = await scriptFor(page, service, { verdictStage: 'settled' });
  await prepare(page, service, 'Reject');
  await submit(page);
  await expect(recovery(page)).toContainText(/succeeded/i);
  const originalSource = script.source.record_head;
  await service.mutate('redact-pending');
  await page.getByRole('button', { name: 'Dismiss completed request', exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  await expect.poll(() => script.source.record_head).not.toBe(originalSource);
  await expect(page.getByRole('button', { name: 'Prepare decision', exact: true })).toBeDisabled();
  await expect(page.locator('body')).not.toContainText(service.pending_text);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  expect(script.posts).toHaveLength(1);
});

test('verdict browser contract: narrow keyboard confirmation preserves literal candidate and byte bounds', async ({ page, service }, testInfo) => {
  const script = await scriptFor(page, service);
  await page.setViewportSize({ width: 390, height: 844 });
  await prepare(page, service);
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill('界'.repeat(683));
  await confirm(page);
  await expect(page.getByRole('button', { name: 'Submit decision', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill(NOTE);
  await page.getByRole('button', { name: 'Review decision', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(panel(page)).toContainText(service.pending_text);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await panel(page).screenshot({ path: testInfo.outputPath('verdict-confirmation-mobile.png') });
  await page.getByRole('button', { name: 'Submit decision', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => script.posts.length).toBe(1);
});
