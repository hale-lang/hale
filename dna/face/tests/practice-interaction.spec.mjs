// Interaction conformance over native Practice/Review reads and scripted command
// receipts. These cases do not prove native submission, authority or durability.
import { test, expect } from './harness.mjs';
import { recoveryMetadata, scriptedCommands } from './command-fixture.mjs';

test.use({ commandSubject: true });

const intervention = page => page.getByRole('region', { name: 'Practice intervention', exact: true });
const recovery = page => page.getByRole('region', { name: 'Command recovery', exact: true });
const journey = page => recovery(page).getByRole('list', { name: 'Request and outcome', exact: true });
const selectedOutcome = page => recovery(page).getByRole('group', { name: 'Selected outcome', exact: true });
const stageButton = (page, title) => journey(page).getByRole('button', { name: title, exact: true });
const RATIONALE = 'Keep the unchanged evidence visible.\nExplain the exact replacement — 第二版.';

async function prepareProposal(page, service, text) {
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill(text);
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill(RATIONALE);
}

async function submitProposal(page) {
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
}

async function expectExactComparison(page, before, after) {
  const comparison = page.locator('#proposal-comparison');
  await expect(comparison).toBeVisible();
  // toHaveText normalizes whitespace. textContent preserves the actual Unicode,
  // line endings and literal markup that the operator must be able to inspect.
  expect(await comparison.locator('.comparison-before').textContent()).toBe(before);
  expect(await comparison.locator('.comparison-after').textContent()).toBe(after);
}

async function expectSelectedStage(page, title) {
  await expect(stageButton(page, title)).toHaveAttribute('aria-pressed', 'true');
  await expect(journey(page).locator('button[aria-pressed="true"]')).toHaveCount(1);
  await expect(selectedOutcome(page)).toBeVisible();
}

// Overlay only the text of a native read to exercise formatting which the
// ordinary fixture does not contain. Its unchanged digest is not a claim that
// these replacement bytes are a real canonical Practice or can be submitted.
async function presentPracticeText(page, service, text) {
  await page.route('**/api/hale/v1/**/dna/practices?*', async route => {
    const response = await route.fetch();
    const payload = await response.json();
    for (const practice of payload.data.items) {
      if (practice.id === service.practice) practice.text = text;
    }
    await route.fulfill({ response, json: payload });
  });
}

test('practice interaction: show separated edits in exact multiline Unicode and literal markup without sending', async ({ page, service }, testInfo) => {
  const script = await scriptedCommands(page, service);
  const anchor = '<img src=x onerror="window.__interactionInjected=true"> & literal — 第一段';
  const before = `Opening stays.\nUse amber evidence.\n\n${anchor}\n\nKeep the retired route.\nClosing stays.\n`;
  const after = before.replace('amber', 'jade').replace('retired', 'current');
  await presentPracticeText(page, service, before);
  await prepareProposal(page, service, after);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();

  await expectExactComparison(page, before, after);
  await expect(page.getByRole('button', { name: 'Show changes', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Read exact text', exact: true })).toHaveAttribute('aria-pressed', 'false');
  const comparison = page.locator('#proposal-comparison');
  const removed = await comparison.locator('.comparison-before del').allTextContents();
  const added = await comparison.locator('.comparison-after ins').allTextContents();
  expect(removed.length).toBeGreaterThanOrEqual(2);
  expect(added.length).toBeGreaterThanOrEqual(2);
  expect(removed.join(' ')).toContain('amber');
  expect(removed.join(' ')).toContain('retired');
  expect(added.join(' ')).toContain('jade');
  expect(added.join(' ')).toContain('current');
  expect(removed.join(' ')).not.toContain(anchor);
  expect(added.join(' ')).not.toContain(anchor);
  await expect(intervention(page).locator('.comparison-summary')).toContainText(/\d/);
  await expect(intervention(page)).toContainText(service.practice);
  await expect(intervention(page)).toContainText(script.principal.name);
  await expect(intervention(page)).toContainText('org / Organization-wide');
  await expect(intervention(page)).toContainText(RATIONALE);
  await expect(comparison.locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__interactionInjected)).toBeUndefined();
  expect(script.posts).toHaveLength(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  await intervention(page).screenshot({ path: testInfo.outputPath('practice-separated-change-comparison.png') });
});

test('practice interaction: exact-text mode and return to editing preserve draft and CRLF predecessor', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  const before = 'First line — café\r\n\r\nSecond line <strong>literal</strong>\r\nLast line ✓\r\n';
  const after = 'First line — café\n\nSecond line <strong>revised literal</strong>\nLast line ✓\n';
  await presentPracticeText(page, service, before);
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveValue(before.replace(/\r\n/g, '\n'));
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill(RATIONALE);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await expect(intervention(page)).toContainText('Change the proposed text before reviewing a revision.');
  await expect(page.getByRole('button', { name: 'Submit proposal', exact: true })).toHaveCount(0);
  expect(script.posts).toHaveLength(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill(after);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await expectExactComparison(page, before, after);

  await page.getByRole('button', { name: 'Read exact text', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Read exact text', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Show changes', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expectExactComparison(page, before, after);
  await page.getByRole('button', { name: 'Show changes', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Show changes', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expectExactComparison(page, before, after);

  await page.getByRole('button', { name: 'Back to editing', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveValue(after);
  await expect(page.getByRole('textbox', { name: 'Rationale', exact: true })).toHaveValue(RATIONALE);
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toBeFocused();
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await expectExactComparison(page, before, after);
  expect(script.posts).toHaveLength(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
});

test('practice interaction: selecting receipt stages keeps approval distinct from adoption and preserves selection on GET refresh', async ({ page, service }, testInfo) => {
  const script = await scriptedCommands(page, service, { stage: 'approved' });
  const refs = await service.refs();
  await prepareProposal(page, service, `${service.text}\nAdd a precise practice requirement.`);
  await submitProposal(page);
  await expect(journey(page).getByRole('button')).toHaveCount(4);
  await expect(page.locator('.practice-detail').getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();

  await stageButton(page, 'Exact-candidate Review').click();
  await expectSelectedStage(page, 'Exact-candidate Review');
  await expect(selectedOutcome(page)).toContainText(/approved/i);
  await stageButton(page, 'Adoption').click();
  await expectSelectedStage(page, 'Adoption');
  await expect(selectedOutcome(page)).toContainText(/pending|awaiting adoption/i);
  const requestID = script.posts[0].body.request_id;
  const previousReads = script.gets.length;

  script.stage = 'refused';
  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect.poll(() => script.gets.length).toBeGreaterThan(previousReads);
  await expectSelectedStage(page, 'Adoption');
  await expect(selectedOutcome(page)).toContainText(/refused/i);
  await expect(selectedOutcome(page)).toContainText('Another candidate replaced this predecessor.');
  await expect(stageButton(page, 'Exact-candidate Review')).toContainText(/approved/i);
  await expect(stageButton(page, 'Command')).toContainText(/succeeded/i);
  expect(script.gets.at(-1)).toBe(requestID);
  expect(script.posts).toHaveLength(1);
  expect((await recoveryMetadata(page))[0].value.request_id).toBe(requestID);
  expect(await service.refs()).toBe(refs);
  await recovery(page).screenshot({ path: testInfo.outputPath('practice-approved-adoption-refused.png') });
});

test('practice interaction: an outcome_unknown receipt remains uncertain and retains its request without resubmitting', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  const posts = [], gets = [];
  let original;
  // Only the receipt is scripted here; native reads and the advertised command
  // profile still come through the shared fixture. This does not establish a
  // real accepted native command or recovery after a native process restart.
  await page.route('**/api/hale/v1/**/commands*', async route => {
    const request = route.request();
    const isPost = request.method() === 'POST';
    if (isPost) {
      original = request.postDataJSON();
      posts.push(original);
    } else {
      gets.push(new URL(request.url()).searchParams.get('request_id'));
    }
    await route.fulfill({
      status: isPost ? 202 : 200,
      contentType: 'application/json',
      body: JSON.stringify({
        api_version: 'hale.v1', source: script.source,
        data: {
          command_id: `command/${original.request_id}`, request_id: original.request_id,
          application_id: service.application, operation: original.operation, operation_version: '1',
          principal: script.principal, context: original.context, target: original.target,
          subject_digest: original.preconditions.subject_digest,
          fingerprint: 'sha256:' + 'c'.repeat(64), state: 'outcome_unknown',
          reason: 'The service cannot yet establish the outcome of this request.',
          proposal: { state: 'unknown', candidate_digest: '', review_id: '' },
          review: { state: 'unavailable', outcome: '', subject_digest: '' },
          activation: { state: 'unknown', reason: '' },
        },
      }),
    });
  });
  await prepareProposal(page, service, `${service.text}\nAn explicit additional requirement.`);
  await submitProposal(page);
  await expect(stageButton(page, 'Command')).toHaveAttribute('data-state', 'unknown');
  await expect(stageButton(page, 'Command')).toContainText(/unknown/i);
  await stageButton(page, 'Command').click();
  await expectSelectedStage(page, 'Command');
  await expect(selectedOutcome(page)).toHaveAttribute('data-state', 'unknown');
  await expect(selectedOutcome(page)).toContainText(/unknown/i);
  await expect(recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true })).toHaveCount(0);
  const saved = (await recoveryMetadata(page))[0].value;
  expect(saved.request_id).toBe(original.request_id);

  await recovery(page).getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect.poll(() => gets.length).toBe(1);
  await expectSelectedStage(page, 'Command');
  await expect(stageButton(page, 'Command')).toHaveAttribute('data-state', 'unknown');
  await page.reload();
  await expect.poll(() => gets.length).toBeGreaterThanOrEqual(2);
  await expect(stageButton(page, 'Command')).toHaveAttribute('data-state', 'unknown');
  await expect(stageButton(page, 'Proposal')).toHaveAttribute('data-state', 'unknown');
  await expect(recovery(page).getByRole('button', { name: 'Dismiss completed request', exact: true })).toHaveCount(0);
  expect((await recoveryMetadata(page))[0].value).toEqual(saved);
  expect(gets.every(id => id === saved.request_id)).toBe(true);
  expect(posts).toHaveLength(1);
});

test('practice interaction: a refused verdict remains refused while independently inspecting another settled approval and adoption', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { reviewProfile: true, verdictStage: 'refused_other_approved' });
  await page.goto(service.url('reviews', { id: service.pending_review }));
  await page.getByRole('button', { name: 'Prepare decision', exact: true }).click();
  await page.getByRole('radio', { name: 'Reject', exact: true }).check();
  await page.getByRole('textbox', { name: 'Decision note', exact: true }).fill('This refusal must remain attributable to this request.');
  await page.getByRole('button', { name: 'Review decision', exact: true }).click();
  await page.getByRole('button', { name: 'Submit decision', exact: true }).click();
  await expect(page.locator('.review-detail').getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  await expect(journey(page).getByRole('button')).toHaveCount(4);

  await stageButton(page, 'Command').click();
  await expectSelectedStage(page, 'Command');
  await expect(selectedOutcome(page)).toContainText(/refused/i);
  await expect(selectedOutcome(page)).not.toContainText(/succeeded/i);
  await stageButton(page, 'This verdict').click();
  await expectSelectedStage(page, 'This verdict');
  await expect(selectedOutcome(page)).toContainText(/reject/i);
  await expect(selectedOutcome(page)).toContainText(/refused/i);
  await stageButton(page, 'Review settlement').click();
  await expectSelectedStage(page, 'Review settlement');
  await expect(selectedOutcome(page)).toContainText(/approved/i);
  await stageButton(page, 'Adoption').click();
  await expectSelectedStage(page, 'Adoption');
  await expect(selectedOutcome(page)).toContainText('Adopted');
  await expect(stageButton(page, 'Command')).toContainText(/refused/i);
  await expect(stageButton(page, 'This verdict')).toContainText(/refused/i);
  await expect(recovery(page)).toContainText(/other.*decision|another.*decision/i);
  expect(script.posts).toHaveLength(1);
});

test('practice interaction: recovery follows its exact candidate and remains available during a collection outage', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, {
    reviewProfile: true, stage: 'created',
    proposalCandidate: service.pending_practice, proposalReview: service.pending_review,
  });
  await prepareProposal(page, service, service.pending_text);
  await submitProposal(page);
  await expect(page.locator('.practice-detail').getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  // the recorder counts the POST only after it has read what the page
  // saved, so the recovery region can show first (GH #1022)
  await expect.poll(() => script.posts.length).toBe(1);
  const requestID = script.posts[0].body.request_id;

  await recovery(page).getByRole('link', { name: 'Open proposal review', exact: true }).click();
  await expect(page.locator('.review-detail').getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  await expect(recovery(page)).toContainText(requestID);
  await recovery(page).getByRole('link', { name: 'Open proposed practice', exact: true }).click();
  await expect(page.locator('.practice-detail').getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  await expect(recovery(page)).toContainText(requestID);

  await page.goto(service.url('reviews', { id: service.review }));
  await expect(recovery(page)).toBeVisible();
  await expect(page.locator('.review-detail').getByRole('region', { name: 'Command recovery', exact: true })).toHaveCount(0);
  script.reviewsUnavailable = true;
  await page.reload();
  await expect(recovery(page)).toContainText(requestID);
  await expect(journey(page).getByRole('button')).toHaveCount(4);
  await expect(page.locator('.review-detail').getByRole('region', { name: 'Command recovery', exact: true })).toHaveCount(0);
  await expect(recovery(page)).toContainText(/created/i);
  expect(script.posts).toHaveLength(1);
  expect(script.gets).toContain(requestID);
});

test('practice interaction: narrow reduced-motion comparison and outcome stages are keyboard usable without overflow', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const script = await scriptedCommands(page, service, { stage: 'approved' });
  const text = `${service.text}\nReference: ${'precise-unbroken-reference-'.repeat(9)}\nHuman action — 確認 ✓`;
  await prepareProposal(page, service, text);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expectExactComparison(page, service.text, text);
  await page.getByRole('button', { name: 'Read exact text', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Read exact text', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Read exact text', exact: true })).toBeFocused();
  await expectExactComparison(page, service.text, text);
  expect(script.posts).toHaveLength(0);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
  await intervention(page).screenshot({ path: testInfo.outputPath('practice-exact-comparison-narrow.png') });

  await page.getByRole('button', { name: 'Submit proposal', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(journey(page).getByRole('button')).toHaveCount(4);
  await stageButton(page, 'Command').focus();
  await page.keyboard.press('Tab');
  await expect(stageButton(page, 'Proposal')).toBeFocused();
  await page.keyboard.press('Enter');
  await expectSelectedStage(page, 'Proposal');
  await expect(stageButton(page, 'Proposal')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(stageButton(page, 'Exact-candidate Review')).toBeFocused();
  await page.keyboard.press('Space');
  await expectSelectedStage(page, 'Exact-candidate Review');
  await expect(selectedOutcome(page)).toContainText(/approved/i);
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
  expect(script.posts).toHaveLength(1);
  await recovery(page).screenshot({ path: testInfo.outputPath('practice-outcome-path-narrow.png') });
});
