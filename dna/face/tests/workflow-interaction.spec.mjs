// Interaction over the native recorded-fact projection, not a live executor.
// The final case explicitly overlays pending-member fields to check that the
// browser does not invent a destination for an unknown or ambiguous key.
import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test.skip(!process.env.HALE_FACE_WORKFLOWS_BIN, 'Requires the explicitly supplied native recorded workflow fixture.');
test.use({ workflows: true, organization: true });

const detail = page => page.getByRole('region', { name: 'Execution', exact: true });
const steps = page => page.getByRole('list', { name: 'Ordered execution Steps', exact: true });
const focus = page => page.locator('#execution-focus-panel');
const work = page => page.getByRole('region', { name: 'Work and attempt results', exact: true });
const selectedStep = page => page.getByRole('region', { name: 'Selected Step', exact: true });
const selectedWorkflow = page => page.getByRole('region', { name: 'Selected workflow', exact: true });
const selectedAttempt = page => page.getByRole('region', { name: 'Selected attempt', exact: true });
const params = page => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
const attemptLink = (page, id) => work(page).getByRole('link', { name: 'Inspect attempt ' + id + ' ·', exact: false });
const refresh = page => page.getByRole('button', { name: 'Refresh', exact: true }).click();
async function open(page, service, extra = {}) {
  const reading = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname.endsWith('/dna/workflows') && url.searchParams.get('id') === service.execution;
  });
  await page.goto(service.url('workflows', { id: service.execution, ...extra }));
  const response = await reading;
  expect(response.status(), await response.text()).toBe(200);
  const body = await response.json();
  await expect(steps(page)).toBeVisible();
  return body.data.items[0];
}

function watchMutations(page) {
  const writes = [];
  page.on('request', request => { if (isWrite(request)) writes.push(request.method() + ' ' + request.url()); });
  return writes;
}

test('workflow interaction: follow a native waiting member into its child and outstanding human Work', async ({ page, service }) => {
  const writes = watchMutations(page), refs = await service.refs();
  const execution = await open(page, service, { locus: 'org/support' });
  const child = execution.nodes.find(node => node.id === service.child);
  const spawningStep = execution.nodes.find(node => node.id === child.spawning_step);
  expect(spawningStep.pending).toBe(child.member_key);
  const waiting = steps(page).getByRole('link', { name: 'Follow member evidence', exact: true });
  const target = new URLSearchParams(new URL(await waiting.getAttribute('href'), page.url()).hash.split('?')[1]);
  expect(target.get('node')).toBe(child.id);
  await waiting.click();
  await expect(selectedWorkflow(page)).toBeVisible();
  await expect(focus(page)).toBeFocused();
  expect(params(page).get('node')).toBe(child.id);
  await expect(steps(page).locator(':scope > li')).toHaveCount(1);
  await expect(detail(page).getByRole('navigation', { name: 'Execution ancestry', exact: true }).getByRole('link', { name: 'monthly-close@7 · ' + service.execution, exact: true })).toBeVisible();
  await steps(page).getByRole('link', { name: 'Inspect work ' + service.human, exact: true }).click();
  await expect(work(page)).toBeVisible();
  await expect(focus(page)).toBeFocused();
  const attempt = execution.attempts.find(value => value.work_id === service.human);
  await expect(selectedAttempt(page)).toContainText(attempt.id);
  await expect(selectedAttempt(page)).toContainText(/outstanding/i);
  await expect(selectedAttempt(page)).toContainText(/human/i);
  await expect(work(page)).toContainText('No accepted Work result is recorded');
  expect(params(page).get('node')).toBe(service.human);
  expect(params(page).get('locus')).toBe('org/support');
  expect(writes).toEqual([]);
  expect(await service.refs()).toBe(refs);
});

test('workflow interaction: desktop Steps and selected Work share the stage while all native attempt summaries remain visible', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 980 });
  const execution = await open(page, service, { node: service.bank });
  const bank = execution.nodes.find(node => node.id === service.bank);
  const attempts = execution.attempts.filter(attempt => attempt.work_id === service.bank);
  await expect(detail(page).locator('.execution-stage')).toHaveCount(1);
  await expect(detail(page).locator('.execution-stage .execution-steps')).toBeVisible();
  await expect(detail(page).locator('.execution-stage #execution-focus-panel')).toBeVisible();
  const [stepBounds, focusBounds, detailBounds, contentBounds] = await Promise.all([
    steps(page).boundingBox(), focus(page).boundingBox(), detail(page).boundingBox(), page.locator('#content').boundingBox(),
  ]);
  expect(stepBounds.x + stepBounds.width).toBeLessThanOrEqual(focusBounds.x + 1);
  expect(Math.min(stepBounds.y + stepBounds.height, focusBounds.y + focusBounds.height)).toBeGreaterThan(Math.max(stepBounds.y, focusBounds.y));
  expect(detailBounds.width).toBeGreaterThan(contentBounds.width * 0.9);
  await expect(work(page).locator('.attempt-card')).toHaveCount(attempts.length);
  for (const attempt of attempts) {
    const card = work(page).locator('.attempt-card').filter({ has: page.getByRole('link', { name: 'Inspect attempt ' + attempt.id + ' ·', exact: false }) });
    await expect(card).toContainText(attempt.disposition);
    await expect(card).toContainText(attempt.accepted ? 'Accepted by the Work' : 'Not an accepted Work result');
  }
  await expect(selectedAttempt(page)).toContainText(bank.attempt_id);
  await expect(selectedAttempt(page)).toContainText('Reconciled ✓');
  await expect(work(page)).toContainText('exact historical binding v7');
  await expect(work(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.bad)).toBeUndefined();
  await steps(page).getByRole('link', { name: 'Inspect work ' + service.bank, exact: true }).click();
  await expect(focus(page)).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('workflow-stage-selected-work.png') });
});

test('workflow interaction: exact attempt selection survives URL history and reload without borrowing another Work attempt', async ({ page, service }) => {
  const writes = watchMutations(page);
  const execution = await open(page, service, { node: service.bank, locus: 'org/support' });
  const failed = execution.attempts.find(attempt => attempt.work_id === service.bank && attempt.disposition === 'failed');
  const accepted = execution.attempts.find(attempt => attempt.work_id === service.bank && attempt.accepted);
  const otherWorkAttempt = execution.attempts.find(attempt => attempt.work_id === service.human);
  await expect(selectedAttempt(page)).toContainText(accepted.id);
  await attemptLink(page, failed.id).click();
  await expect(selectedAttempt(page)).toBeFocused();
  await expect(selectedAttempt(page)).toContainText(failed.id);
  await expect(selectedAttempt(page)).toContainText(failed.result);
  await expect(attemptLink(page, failed.id)).toHaveAttribute('aria-current', 'true');
  await expect(work(page).locator('.attempt-card.selected')).toHaveAttribute('data-accepted', 'false');
  await expect(selectedAttempt(page)).not.toContainText(accepted.result);
  expect(params(page).get('attempt')).toBe(failed.id);
  expect(params(page).get('node')).toBe(service.bank);
  expect(params(page).get('id')).toBe(service.execution);
  expect(params(page).get('locus')).toBe('org/support');
  const failedURL = page.url();
  await attemptLink(page, accepted.id).click();
  await expect(selectedAttempt(page)).toBeFocused();
  await expect(selectedAttempt(page)).toContainText(accepted.id);
  await expect(attemptLink(page, accepted.id)).toHaveAttribute('aria-current', 'true');
  await expect(work(page).locator('.attempt-card.selected')).toHaveAttribute('data-accepted', 'true');
  expect(params(page).get('attempt')).toBe(accepted.id);
  await page.goBack();
  await expect(page).toHaveURL(failedURL);
  await expect(selectedAttempt(page)).toContainText(failed.id);
  await expect(selectedAttempt(page)).toBeFocused();
  await page.reload();
  await expect(selectedAttempt(page)).toContainText(failed.id);
  await expect(selectedAttempt(page)).not.toContainText(accepted.result);
  expect(params(page).get('attempt')).toBe(failed.id);

  // This ID exists in the same native execution but belongs to another Work.
  // It must remain unavailable here rather than falling back to the accepted one.
  await open(page, service, { node: service.bank, attempt: otherWorkAttempt.id });
  await expect(selectedAttempt(page)).toContainText(/unavailable/i);
  await expect(selectedAttempt(page)).not.toContainText(accepted.result);
  await expect(selectedAttempt(page)).not.toContainText(/Accepted by the Work/);
  expect(params(page).get('attempt')).toBe(otherWorkAttempt.id);

  await page.getByRole('region', { name: 'Execution register', exact: true }).getByRole('link', { name: 'refused-close', exact: true }).click();
  await expect(detail(page)).toContainText('Required binding is unavailable');
  await expect(selectedAttempt(page)).toHaveCount(0);
  expect(params(page).get('id')).toBe('refused-close');
  expect(params(page).has('node')).toBe(false);
  expect(params(page).has('attempt')).toBe(false);
  expect(writes).toEqual([]);
});

test('workflow interaction: completed members keep the selected native Step barrier open until its own completion exists', async ({ page, service }) => {
  const execution = await open(page, service);
  const first = execution.nodes.find(node => node.kind === 'step' && node.parent_id === service.execution && node.step_index === '0');
  await steps(page).getByRole('link', { name: 'Step 1', exact: true }).click();
  await expect(selectedStep(page)).toBeVisible();
  await expect(focus(page)).toBeFocused();
  expect(params(page).get('node')).toBe(first.id);
  await service.mutate('members');
  await refresh(page);
  await expect(selectedStep(page)).toContainText(first.id);
  await expect(steps(page)).toContainText('All registered members settled done; Step completion has not been recorded');
  await expect(steps(page).locator(':scope > li').nth(0)).toHaveAttribute('data-state', 'activated');
  await expect(steps(page).locator(':scope > li').nth(1)).toHaveAttribute('data-state', 'bound');
  await expect(steps(page).getByRole('link', { name: 'Follow member evidence', exact: true })).toHaveCount(0);
  expect(params(page).get('node')).toBe(first.id);
});

test('workflow interaction: narrow reduced-motion keyboard navigation returns through the containing and spawning Steps without losing context', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const execution = await open(page, service, { locus: 'org/support' });
  const human = execution.nodes.find(node => node.id === service.human);
  const child = execution.nodes.find(node => node.id === service.child);
  await steps(page).getByRole('link', { name: 'Follow member evidence', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(focus(page)).toBeFocused();
  await steps(page).getByRole('link', { name: 'Inspect work ' + service.human, exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(work(page)).toBeFocused();
  await expect(selectedAttempt(page)).toContainText(/outstanding/i);
  await page.screenshot({ path: testInfo.outputPath('workflow-human-attempt-mobile.png') });
  await work(page).getByRole('link', { name: 'Inspect containing Step', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(selectedStep(page)).toBeFocused();
  expect(params(page).get('node')).toBe(human.parent_id);
  const selectedURL = page.url();
  await focus(page).getByRole('button', { name: 'Back to workflow', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(steps(page)).toBeFocused();
  await expect(page).toHaveURL(selectedURL);
  await detail(page).getByRole('link', { name: 'Return to spawning Step', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(selectedStep(page)).toBeFocused();
  expect(params(page).get('node')).toBe(child.spawning_step);
  expect(params(page).get('locus')).toBe('org/support');
  await expect(steps(page).locator(':scope > li')).toHaveCount(2);
  expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true);
  expect(await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth)).toBeLessThanOrEqual(1);
});

test('workflow interaction: browser-only unknown and ambiguous waiting-key overlays do not invent member navigation', async ({ page, service }) => {
  let mode = 'unknown';
  const unknown = '<img src=x onerror="window.__workflowInteractionInjected=true">';
  await page.route('**/api/hale/v1/**/dna/workflows?*', async route => {
    if (new URL(route.request().url()).searchParams.get('id') !== service.execution) {
      await route.continue();
      return;
    }
    const response = await route.fetch(), body = await response.json();
    for (const execution of body.data.items) {
      if (execution.id !== service.execution) continue;
      const child = execution.nodes.find(node => node.id === service.child);
      const step = execution.nodes.find(node => node.id === child.spawning_step);
      step.pending = mode === 'unknown' ? unknown : child.member_key;
      // Synthetic duplicate key only; all native node identities and containment
      // remain intact. This does not claim the native executor emits this state.
      if (mode === 'ambiguous') execution.nodes.find(node => node.id === service.bank).member_key = child.member_key;
    }
    await route.fulfill({ response, json: body });
  });
  await open(page, service);
  await expect(steps(page)).toContainText(unknown);
  await expect(steps(page).getByRole('link', { name: /^Follow member / })).toHaveCount(0);
  await expect(steps(page).locator('img')).toHaveCount(0);
  expect(await page.evaluate(() => window.__workflowInteractionInjected)).toBeUndefined();
  mode = 'ambiguous';
  await refresh(page);
  await expect(steps(page)).toContainText('Completion not recorded for · evidence');
  await expect(steps(page).getByRole('link', { name: 'Follow member evidence', exact: true })).toHaveCount(0);
  await expect(steps(page).getByRole('link', { name: 'Enter child ' + service.child, exact: true })).toBeVisible();
  await expect(steps(page).getByRole('link', { name: 'Inspect work ' + service.bank, exact: true })).toBeVisible();
});
