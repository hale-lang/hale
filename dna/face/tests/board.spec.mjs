import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

// The face reads through the commands relay too: a POST whose line is a
// read call (`Index`, `Org`, `Tasks`, `BatchPreview`) or the describe is no write.
const READ_LINE = /^\s*\{\s*"(describe|call"\s*:\s*"(Index|Org|Tasks|BatchPreview)")/;
const isMutation = request => isWrite(request) && !READ_LINE.test(request.postData() || '');

// The board: a pending Review is a card in review, a settled one is done, opening a card shows the Review and its question, and reading
// the board writes nothing.
test('a pending Review is in review, a settled one is done, and nothing is written', async ({ page, service }, testInfo) => {
  const mutations = [];
  page.on('request', request => { if (isMutation(request)) mutations.push(request.url()); });
  const before = await service.refs();
  await page.goto(service.url('board'));
  const needs = page.getByRole('listitem', { name: 'In review' });
  const settled = page.getByRole('listitem', { name: 'Done' });
  await expect(needs.getByRole('link', { name: service.pending_review })).toBeVisible();
  await expect(settled.getByRole('link', { name: service.review })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('board.png'), fullPage: true });
  await needs.getByRole('link', { name: service.pending_review }).click();
  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  await expect(inspector).toContainText(service.pending_review);
  await expect(inspector).toContainText(service.pending_question);
  await expect(inspector).toContainText(/pending/);
  expect(new URL(page.url()).hash).toContain(encodeURIComponent(service.pending_review));
  await page.goBack();
  await expect(inspector.getByRole('heading', { name: 'Select a card', exact: true })).toBeVisible();
  expect(mutations).toEqual([]);
  expect(await service.refs()).toBe(before);
});

test('the To do lane says why it is empty when the head cannot read tasks', async ({ page, service }) => {
  await page.goto(service.url('board'));
  const handed = page.getByRole('listitem', { name: 'To do' });
  await expect(handed).toContainText(/nothing here|not readable/);
});
