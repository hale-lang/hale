// Browser command conformance: real native practice reads, scripted command
// responses. This does NOT prove native admission, restart durability or adoption.
import { test, expect } from './harness.mjs';
import { STORAGE_PREFIX, recoveryMetadata, scriptedCommands } from './command-fixture.mjs';

test.use({ commandSubject: true });
const TEXT = 'Proposed revision — 第二版\nKeep <img src=x onerror="window.__injected=true"> literal.';
const RATIONALE = 'Review the precise replacement.\nNo implicit adoption.';

async function openEditor(page, service) {
  await page.goto(service.url('practices', { id: service.practice }));
  await page.getByRole('button', { name: 'Propose revision', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Practice intervention', exact: true })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveValue(service.text);
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill(TEXT);
  await page.getByRole('textbox', { name: 'Rationale', exact: true }).fill(RATIONALE);
}

async function submit(page) {
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
}

test('command browser contract: default provider remains read only', async ({ page, service }) => {
  const writes = [];
  page.on('request', request => { if (request.method() !== 'GET') writes.push(request.url()); });
  await page.goto(service.url('practices', { id: service.practice }));
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Propose revision', exact: true })).toBeDisabled();
  expect(writes).toEqual([]);
});

test('command browser contract: legacy write boolean alone cannot enable submission', async ({ page, service }) => {
  await scriptedCommands(page, service, { profile: false });
  await page.goto(service.url('practices', { id: service.practice }));
  // Assert on the rendered practice, not on a page still reading: ending the
  // test with the capabilities read in flight tears the API down under it.
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Submit proposal', exact: true })).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveCount(0);
});

test('command browser contract: exact comparison, durable-before-send identity and literal content', async ({ page, service }, testInfo) => {
  const script = await scriptedCommands(page, service);
  const before = await service.refs();
  await openEditor(page, service);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Practice intervention', exact: true });
  await expect(panel).toContainText(service.text);
  await expect(panel).toContainText(TEXT);
  await expect(panel).toContainText(service.practice);
  expect(script.posts).toHaveLength(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  await page.screenshot({ path: testInfo.outputPath('practice-revision-comparison.png'), fullPage: true });
  await panel.screenshot({ path: testInfo.outputPath('practice-intervention.png') });
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).click();
  await expect.poll(() => script.posts.length).toBe(1);
  const sent = script.posts[0];
  expect(sent.body).toEqual({
    request_id: expect.any(String), operation: 'dna.practice.propose', operation_version: '1',
    context: { application_id: service.application, position_id: 'org' },
    target: { application_id: service.application, kind: 'dna.practice', id: service.practice },
    preconditions: { subject_digest: service.practice, principal: script.principal }, arguments: { text: TEXT, rationale: RATIONALE },
  });
  expect(sent.headers['x-hale-command']).toBe('1');
  expect(sent.headers['content-type']).toContain('application/json');
  expect(script.savedBeforeSend).toHaveLength(1);
  expect(script.savedBeforeSend[0].value).toEqual({
    version: 2, application_id: service.application, principal: script.principal,
    request_id: sent.body.request_id, operation: 'dna.practice.propose', operation_version: '1',
    position_id: 'org', target_kind: 'dna.practice', target_id: service.practice, subject_digest: service.practice,
  });
  const stored = JSON.stringify(await recoveryMetadata(page));
  expect(stored).not.toContain('Proposed revision');
  expect(stored).not.toContain('Review the precise replacement');
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/recorded/i);
  expect(await service.refs()).toBe(before); // scripted commands are not domain writes
});

test('command browser contract: lost reply and reload recover the original identity without POST retry', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { postMode: 'lost', getMode: 'unavailable' });
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const request = script.posts[0].body.request_id;
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toBeVisible();
  script.getMode = 'receipt';
  await page.reload();
  await expect.poll(() => script.gets.includes(request)).toBe(true);
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/recorded/i);
  expect(script.posts).toHaveLength(1);
  expect((await recoveryMetadata(page))[0].value.request_id).toBe(request);
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveCount(0);
});

test('command browser contract: unavailable and absent recovery retain the same key', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { postMode: 'lost', getMode: 'missing' });
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const request = script.posts[0].body.request_id;
  const check = page.getByRole('button', { name: 'Check request status', exact: true });
  await check.click();
  await expect.poll(() => script.gets.length).toBeGreaterThan(0);
  expect((await recoveryMetadata(page))[0].value.request_id).toBe(request);
  script.getMode = 'unavailable';
  await check.click();
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/unavailable/i);
  expect((await recoveryMetadata(page))[0].value.request_id).toBe(request);
  expect(script.posts).toHaveLength(1);
});

test('command browser contract: storage failure prevents transmission', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  await page.addInitScript(prefix => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (String(key).startsWith(prefix)) throw new DOMException('Storage unavailable', 'QuotaExceededError');
      return original.call(this, key, value);
    };
  }, STORAGE_PREFIX);
  await openEditor(page, service);
  await submit(page);
  await expect(page.getByRole('region', { name: 'Practice intervention', exact: true })).toContainText(/save|storage|recovery/i);
  expect(script.posts).toHaveLength(0);
});

test('command browser contract: invalid receipt cannot disclose another principal result', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { corrupt: true });
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(panel).toContainText(/unknown|invalid|incomplete|could not/i);
  await expect(page.locator('body')).not.toContainText('WRONG ACTOR SECRET');
  expect(await recoveryMetadata(page)).toHaveLength(1);
});

test('command browser contract: approval and adoption refusal remain separate', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { stage: 'approved' });
  await openEditor(page, service);
  await submit(page);
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(panel).toContainText(/approved/i);
  await expect(panel).toContainText(/pending/i);
  await expect(panel).toContainText('Pending — awaiting adoption');
  script.stage = 'refused';
  await page.getByRole('button', { name: 'Check request status', exact: true }).click();
  await expect(panel).toContainText(/approved/i);
  await expect(panel).toContainText('Another candidate replaced this predecessor.');
  await expect(panel).toContainText(/refused/i);
  expect(script.posts).toHaveLength(1);
});

test('command browser contract: revoked write permission does not lose an existing recovery key', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  script.authorized = false;
  await page.reload();
  await expect.poll(() => script.gets.length).toBeGreaterThan(0);
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(/recorded/i);
  expect(await recoveryMetadata(page)).toHaveLength(1);
  expect(script.posts).toHaveLength(1);
});

test('command browser contract: recovery survives an unavailable practice collection without exposing stale content', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { postMode: 'lost', getMode: 'unavailable' });
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const requestID = script.posts[0].body.request_id;
  script.readsUnavailable = true;
  script.getMode = 'receipt';
  await page.reload();
  const panel = page.getByRole('region', { name: 'Command recovery', exact: true });
  await expect(panel).toContainText(/recorded/i);
  await expect.poll(() => script.gets.includes(requestID)).toBe(true);
  await expect(page.locator('body')).not.toContainText(service.text);
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Submit proposal', exact: true })).toHaveCount(0);
  expect((await recoveryMetadata(page))[0].value.request_id).toBe(requestID);
  expect(script.posts).toHaveLength(1);
  script.authLost = true;
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(panel).toHaveCount(0);
  expect(await recoveryMetadata(page)).toHaveLength(1);
});

test('command browser contract: identity changes clear drafts and never reuse another principal key', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  await openEditor(page, service);
  script.authLost = true;
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(page.locator('body')).not.toContainText(TEXT);
  await expect(page.getByRole('textbox', { name: 'Proposed text', exact: true })).toHaveCount(0);
  script.authLost = false;
  await page.reload();
  await openEditor(page, service);
  await submit(page);
  await expect.poll(() => script.posts.length).toBe(1);
  const gets = script.gets.length;
  script.principal = { mode: 'local', name: 'another-person' };
  await page.reload();
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toHaveCount(0);
  expect(script.gets).toHaveLength(gets);
  expect(await recoveryMetadata(page)).toHaveLength(1);
});

test('command browser contract: narrow comparison and submit remain keyboard accessible', async ({ page, service }, testInfo) => {
  const script = await scriptedCommands(page, service);
  await page.setViewportSize({ width: 390, height: 844 });
  await openEditor(page, service);
  await page.getByRole('button', { name: 'Review proposal', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Submit proposal', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({ path: testInfo.outputPath('practice-revision-mobile.png'), fullPage: true });
  await page.getByRole('region', { name: 'Practice intervention', exact: true }).screenshot({ path: testInfo.outputPath('practice-intervention-mobile.png') });
  await page.getByRole('button', { name: 'Submit proposal', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => script.posts.length).toBe(1);
});

test('command browser contract: two tabs cannot overwrite an unresolved request or both submit', async ({ page, context, service }) => {
  const script = await scriptedCommands(page, service);
  const second = await context.newPage();
  try {
    await scriptedCommands(second, service, { posts: script.posts });
    await openEditor(page, service);
    await openEditor(second, service);
    await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
    await second.getByRole('button', { name: 'Review proposal', exact: true }).click();
    await Promise.all([
      page.getByRole('button', { name: 'Submit proposal', exact: true }).click(),
      second.getByRole('button', { name: 'Submit proposal', exact: true }).click(),
    ]);
    await expect.poll(() => script.posts.length).toBe(1);
    const saved = await recoveryMetadata(page);
    expect(saved).toHaveLength(1);
    expect(saved[0].value.request_id).toBe(script.posts[0].body.request_id);
    await second.reload();
    await expect(second.getByRole('region', { name: 'Command recovery', exact: true })).toContainText(saved[0].value.request_id);
    expect(script.posts).toHaveLength(1);
  } finally { await second.close(); }
});

test('command browser contract: UTF-8 byte limits reject an oversized draft before saving or sending', async ({ page, service }) => {
  const script = await scriptedCommands(page, service);
  await openEditor(page, service);
  await page.getByRole('textbox', { name: 'Proposed text', exact: true }).fill('界'.repeat(2731));
  await page.getByRole('button', { name: 'Review proposal', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Practice intervention', exact: true })).toContainText(/8192/);
  await expect(page.getByRole('button', { name: 'Submit proposal', exact: true })).toHaveCount(0);
  expect(await recoveryMetadata(page)).toHaveLength(0);
  expect(script.posts).toHaveLength(0);
});

test('command browser contract: a late receipt cannot reappear under another principal', async ({ page, service }) => {
  let release;
  const waitForPost = new Promise(resolve => { release = resolve; });
  const script = await scriptedCommands(page, service, { waitForPost, stage: 'approved' });
  try {
    await openEditor(page, service);
    await submit(page);
    await expect.poll(() => script.posts.length).toBe(1);
    script.principal = { mode: 'local', name: 'another-person' };
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await expect(page.locator('#principal')).toContainText('another-person');
    release();
    await expect(page.getByRole('region', { name: 'Command recovery', exact: true })).toHaveCount(0);
    await expect(page.locator('body')).not.toContainText('command/' + script.posts[0].body.request_id);
    expect(script.posts).toHaveLength(1);
    expect(await recoveryMetadata(page)).toHaveLength(1);
  } finally { release(); }
});

test('command browser contract: an identity-change refusal clears content and preserves the original scope', async ({ page, service }) => {
  const script = await scriptedCommands(page, service, { postMode: 'identity_changed', getMode: 'missing' });
  await openEditor(page, service);
  await submit(page);
  await expect(page.getByRole('heading', { name: 'Sign-in identity changed', exact: true })).toBeVisible();
  await expect(page.locator('body')).not.toContainText(service.text);
  await expect(page.locator('body')).not.toContainText(TEXT);
  expect(script.posts).toHaveLength(1);
  expect(script.posts[0].body.preconditions.principal).toEqual(script.principal);
  expect((await recoveryMetadata(page))[0].value.principal).toEqual(script.principal);
});
