import { test, expect, errorBody } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

async function openPractice(page, service) {
  await page.goto(service.url());
  await page.getByRole('link', { name: service.name, exact: true }).click();
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await expect(page.locator('body')).toContainText('Première ligne — ✓');
}

test('real Record: practice/review navigation, opaque IDs, literal multiline text and browser back', async ({ page, service }, testInfo) => {
  const before = await service.refs();
  const mutations = [];
  page.on('request', request => { if (isWrite(request)) mutations.push(request.url()); });
  await openPractice(page, service);
  await page.screenshot({ path: testInfo.outputPath('desktop-practice.png'), fullPage: true });
  await expect(page.locator('body')).toContainText(service.rationale);
  await expect(page.locator('body')).toContainText('<img src=x onerror="window.__injected=true">');
  expect(await page.locator('body').textContent()).toContain(service.text);
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(page.locator('img[src="x"], review')).toHaveCount(0);
  await expect(page.locator('body')).toContainText(/pending/i);
  await expect(page.locator('body')).toContainText(/approve/i);
  await page.getByRole('link', { name: 'Open review', exact: true }).click();
  await expect(page.getByRole('heading', { name: service.review, exact: true })).toBeVisible();
  expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('id')).toBe(service.review);
  expect(await page.locator('body').textContent()).toContain(service.question);
  await page.goBack();
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await page.getByRole('link', { name: 'Reviews', exact: true }).click();
  await page.getByRole('link', { name: service.review, exact: true }).click();
  await page.getByRole('link', { name: 'Open practice', exact: true }).click();
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(before);
});

test('real receipt redaction removes an already displayed body and review text', async ({ page, service }) => {
  await openPractice(page, service);
  await service.mutate('redact');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.locator('body')).not.toContainText('Première ligne — ✓');
  await expect(page.locator('body')).not.toContainText(service.rationale);
  await expect(page.locator('body')).toContainText(/redacted/i);
  await page.getByRole('link', { name: 'Open review', exact: true }).click();
  await expect(page.getByRole('heading', { name: service.review, exact: true })).toBeVisible();
  await expect(page.locator('body')).not.toContainText(service.question);
  await expect(page.locator('body')).not.toContainText('approve by alice');
  await expect(page.locator('body')).toContainText(/approve/i);
});

test.describe('snapshot-bound paging', () => {
  test.use({ recordCount: 27 });
  test('real Record movement returns 409 and restarts at the first page', async ({ page, service }) => {
    await page.goto(service.url());
    const next = page.getByRole('button', { name: 'Next page', exact: true });
    await expect(next).toBeEnabled();
    await service.mutate('advance');
    const conflict = page.waitForResponse(response => response.url().includes('/dna/practices') && response.status() === 409);
    await next.click();
    await conflict;
    await expect(page.locator('body')).toContainText('The Record changed. Pagination restarted from the first page.');
    await expect(page.getByRole('button', { name: 'Previous page', exact: true })).toBeDisabled();
    expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('offset') || '0').toBe('0');
    await next.click();
    await expect(page.getByRole('button', { name: 'Previous page', exact: true })).toBeEnabled();
    await page.goBack();
    await expect(page.getByRole('button', { name: 'Previous page', exact: true })).toBeDisabled();
  });
});

test('401 clears previously rendered application data and offers sign-in', async ({ page, service }) => {
  await openPractice(page, service);
  await page.route('**/api/hale/v1/**', route => route.fulfill({
    status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('authentication_required', 'Sign in to read this application')),
  }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Sign in', exact: true })).toHaveAttribute('href', '/auth/login');
  await expect(page.locator('body')).not.toContainText(service.text);
  await expect(page.locator('body')).not.toContainText(service.name);
});

test('a stalled read clears data, times out, ignores a late response and retries the real service', async ({ page, service }) => {
  await openPractice(page, service);
  await page.clock.install();
  let resume;
  const paused = new Promise(resolve => { resume = resolve; });
  let captured;
  const intercepted = new Promise(resolve => { captured = resolve; });
  const pattern = '**/dna/practices?**';
  await page.route(pattern, async route => {
    // Hold a real response to model an unresponsive connection. Successful
    // recovery below still reads the native service; no success DTO is mocked.
    const response = await route.fetch();
    captured();
    await paused;
    await route.fulfill({ response }).catch(() => {}); // timeout may close it
  });
  try {
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await intercepted;
    await expect(page.locator('body')).not.toContainText(service.text);
    await page.clock.fastForward(15_001);
    await expect(page.getByRole('heading', { name: 'Service took too long', exact: true })).toBeVisible();
    await expect(page.locator('#content')).toHaveAttribute('aria-busy', 'false');
    await expect(page.locator('body')).not.toContainText('No practices yet');
    resume();
    await page.unroute(pattern);
    await expect(page.getByRole('heading', { name: 'Service took too long', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
    await expect(page.locator('body')).toContainText(service.text);
    // Finished reads must remove their timers: an old timer cannot replace
    // recovered content with an error later.
    await page.clock.fastForward(15_001);
    await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  } finally {
    resume();
    await page.unroute(pattern);
  }
});

test('missing object and unavailable Record have distinct, recoverable states', async ({ page, service }) => {
  await page.goto(service.url('practices', { id: 'missing/object/決定' }));
  await expect(page.getByRole('heading', { name: 'Practice not found', exact: true })).toBeVisible();
  await page.route('**/api/hale/v1/**', route => route.fulfill({
    status: 503, contentType: 'application/json', body: JSON.stringify(errorBody('record_unavailable', 'the Record is unavailable')),
  }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Record unavailable', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Practice not found', exact: true })).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText('No practices yet');
  await page.unroute('**/api/hale/v1/**');
  await page.goto(service.url());
  await expect(page.getByRole('link', { name: service.name, exact: true })).toBeVisible();
});

test('narrow viewport keeps navigation, list, details and back usable', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openPractice(page, service);
  await page.screenshot({ path: testInfo.outputPath('narrow-practice.png'), fullPage: true });
  await page.getByRole('link', { name: 'Open review', exact: true }).click();
  await expect(page.getByRole('heading', { name: service.review, exact: true })).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await page.goBack();
  await expect(page.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  await page.getByRole('link', { name: 'Reviews', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Reviews', exact: true })).toBeVisible();
});

test.describe('mobile paged navigation', () => {
  test.use({ recordCount: 52 });
  test('selection brings detail into view and back restores the same list page', async ({ page, service }, testInfo) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(service.url());
    await page.getByRole('button', { name: 'Next page', exact: true }).click();
    const register = page.getByRole('region', { name: 'Practice register', exact: true });
    await expect(page.getByRole('button', { name: 'Previous page', exact: true })).toBeEnabled();
    await expect(register.getByRole('link')).toHaveCount(25);
    const query = () => new URLSearchParams(new URL(page.url()).hash.split('?')[1]);
    const snapshot = query().get('snapshot');
    expect(query().get('offset')).toBe('25');
    expect(snapshot).toBeTruthy();
    const first = register.getByRole('link').first();
    const name = await first.getAttribute('aria-label');
    await first.click();
    const detail = page.getByRole('region', { name: 'Practice detail', exact: true });
    await expect(detail).toBeFocused();
    await expect(detail.getByRole('heading', { name, exact: true })).toBeInViewport();
    await page.screenshot({ path: testInfo.outputPath('mobile-paged-detail.png') });
    await page.getByRole('link', { name: 'Back to practices', exact: true }).click();
    await expect(register).toBeFocused();
    await expect(register.getByRole('heading', { name: 'Practice register', exact: true })).toBeInViewport();
    await expect(register.getByRole('link').first()).toHaveAttribute('aria-label', name);
    expect(query().get('offset')).toBe('25');
    expect(query().get('snapshot')).toBe(snapshot);
    expect(query().get('id')).toBeNull();
    await expect(page.getByRole('button', { name: 'Previous page', exact: true })).toBeEnabled();
  });
});

test.describe('empty Record', () => {
  test.use({ recordCount: 0 });
  test('empty practices and reviews are distinct from source errors', async ({ page, service }) => {
    await page.goto(service.url());
    await expect(page.locator('body')).toContainText('No practices yet');
    await expect(page.locator('body')).not.toContainText('Record unavailable');
    await page.getByRole('link', { name: 'Reviews', exact: true }).click();
    await expect(page.locator('body')).toContainText('No reviews yet');
    await expect(page.locator('body')).not.toContainText('Record unavailable');
  });
});

test('obsolete detail response cannot restore content after workspace navigation', async ({ page, service }) => {
  await page.goto(service.url());
  let release;
  let entered;
  const gate = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  let finished;
  const complete = new Promise(resolve => { finished = resolve; });
  await page.route('**/dna/practices?*', async route => {
    if (!new URL(route.request().url()).searchParams.has('id')) return route.continue();
    const response = await route.fetch();
    entered();
    await gate;
    try { await route.fulfill({ response }); } catch { /* aborted fetch is correct too */ }
    finished();
  });
  try {
    await page.getByRole('link', { name: service.name, exact: true }).click();
    await started;
    await page.getByRole('link', { name: 'Organization', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Organization', exact: true })).toBeVisible();
    release();
    await complete;
    await expect(page.getByRole('heading', { name: 'Organization', exact: true })).toBeVisible();
    await expect(page.locator('body')).not.toContainText(service.text);
    await expect(page.getByRole('heading', { name: service.name, exact: true })).toHaveCount(0);
  } finally { release(); }
});

test('a late authenticated response cannot restore content after session loss', async ({ page, service }) => {
  await page.goto(service.url());
  await expect(page.getByRole('link', { name: service.name, exact: true })).toBeVisible();
  let release;
  let entered;
  let finished;
  const gate = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  const complete = new Promise(resolve => { finished = resolve; });
  await page.route('**/api/hale/v1/**', async route => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/practices') && url.searchParams.has('id')) {
      const response = await route.fetch();
      entered();
      await gate;
      try { await route.fulfill({ response }); } catch { /* cancellation is also safe */ }
      finished();
      return;
    }
    if (url.pathname.endsWith('/reviews')) {
      await route.fulfill({ status: 401, contentType: 'application/json',
        body: JSON.stringify(errorBody('authentication_required', 'Sign in')) });
    } else await route.continue();
  });
  try {
    await page.getByRole('link', { name: service.name, exact: true }).click();
    await started;
    await page.getByRole('link', { name: 'Reviews', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
    release();
    await complete;
    await expect(page.getByRole('heading', { name: 'Sign in to read this application' })).toBeVisible();
    await expect(page.locator('body')).not.toContainText(service.text);
    await expect(page.locator('body')).not.toContainText(service.name);
  } finally { release(); }
});
