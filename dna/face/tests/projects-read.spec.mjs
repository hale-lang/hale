// Real plain Record API: no operator-machine head stands behind it, so the
// shell's head probe answers 404 and the face continues unchanged.
import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

test('real Record API without a head: the shell continues to Practices, hides Projects and sends nothing but reads', async ({ page, service }) => {
  const before = await service.refs();
  const mutations = [], probes = [];
  page.on('request', request => {
    if (isWrite(request)) mutations.push(request.url());
    if (new URL(request.url()).pathname === '/api/hale/v1/head') probes.push(request.url());
  });
  const probe = page.waitForResponse(response => new URL(response.url()).pathname === '/api/hale/v1/head');
  await page.goto(service.url());
  expect((await probe).status()).toBe(404);
  await expect(page.getByRole('heading', { name: 'Practices', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: service.name, exact: true })).toBeVisible();
  await expect(page.locator('#nav-projects')).toBeHidden();
  await expect(page.getByRole('heading', { name: 'No project is attached' })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Project service unavailable' })).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Projects', exact: true })).toHaveCount(0);
  expect(probes).toHaveLength(1);

  // Moving between Record views never probes again in the same page.
  await page.getByRole('link', { name: 'Reviews', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Reviews', exact: true })).toBeVisible();
  expect(probes).toHaveLength(1);

  // The workspace remains reachable by hand and says what is missing.
  await page.goto(service.url('projects'));
  await expect(page.getByRole('heading', { name: 'Projects', exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Projects workspace', exact: true })).toContainText('No project service is running behind this API');
  await expect(page.locator('#nav-projects')).toBeHidden();
  await expect(page.locator('#principal')).toHaveText('Not connected');
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(before);
});
