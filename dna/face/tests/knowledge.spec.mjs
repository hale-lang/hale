import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

// The face reads through the commands relay too: a POST whose line is a
// read call (`Index`, `Org`, `Tasks`) or the describe is no write.
const READ_LINE = /^\s*\{\s*"(describe|call"\s*:\s*"(Index|Org|Tasks)")/;
const isMutation = request => isWrite(request) && !READ_LINE.test(request.postData() || '');

// Knowledge on a record-only head: every proposal the record holds, by
// kind, the practice's literal text on its page, a search that narrows the
// list, and nothing written.
test('lists the record\'s nodes by kind, shows a node\'s literal text, and writes nothing', async ({ page, service }, testInfo) => {
  const mutations = [];
  page.on('request', request => { if (isMutation(request)) mutations.push(request.url()); });
  const before = await service.refs();
  await page.goto(service.url('knowledge'));
  const list = page.locator('#kn-list');
  await expect(list.getByRole('link', { name: service.name })).toBeVisible();
  await list.getByRole('link', { name: service.name }).first().click();
  const node = page.getByRole('region', { name: 'Node' });
  await expect(node.getByRole('heading', { name: service.name, exact: true })).toBeVisible();
  expect(await node.textContent()).toContain(service.text);
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  await expect(node).toContainText('bound to');
  await page.screenshot({ path: testInfo.outputPath('knowledge.png'), fullPage: true });
  await page.getByRole('searchbox', { name: 'Search knowledge' }).fill('no-such-node-anywhere');
  await expect(page.locator('#kn-list').getByRole('link')).toHaveCount(0);
  await page.getByRole('searchbox', { name: 'Search knowledge' }).fill('');
  await expect(page.locator('#kn-list').getByRole('link', { name: service.name })).toBeVisible();
  expect(new URL(page.url()).hash).toContain(encodeURIComponent(service.practice));
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(before);
});
