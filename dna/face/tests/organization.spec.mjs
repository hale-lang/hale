import { test, expect } from './harness.mjs';
import { isWrite } from './command-wire.mjs';

// The face reads through the commands relay too: a POST whose line is a
// read call (`Index`, `Org`, `Tasks`, `BatchPreview`) or the describe is no write.
const READ_LINE = /^\s*\{\s*"(describe|call"\s*:\s*"(Index|Org|Tasks|BatchPreview)")/;
const isMutation = request => isWrite(request) && !READ_LINE.test(request.postData() || '');

// The org chart as the graph states it: a position the record seats is
// drawn under the purpose with its holder, selecting it shows its mandate
// state, and reading the chart writes nothing.
test.describe('the org chart', () => {
  test.use({ seats: ['board', 'api/dev'] });

  test('draws the record\'s positions with their holders and writes nothing', async ({ page, service }, testInfo) => {
    const mutations = [];
    page.on('request', request => { if (isMutation(request)) mutations.push(request.url()); });
    const before = await service.refs();
    await page.goto(service.url('org'));
    const chart = page.getByRole('region', { name: 'Org chart' });
    await expect(chart.getByRole('link', { name: /^board/ })).toBeVisible();
    await expect(chart.getByRole('link', { name: /^api\/dev/ })).toBeVisible();
    await expect(chart).toContainText(`held by ${process.env.USER}`);
    await page.screenshot({ path: testInfo.outputPath('chart.png'), fullPage: true });
    await chart.getByRole('link', { name: /^board/ }).click();
    const inspector = page.getByRole('complementary', { name: 'Inspector' });
    await expect(inspector.getByRole('heading', { name: 'board', exact: true })).toBeVisible();
    await expect(inspector).toContainText('position:board');
    await expect(inspector).toContainText(process.env.USER);
    await expect(inspector).toContainText(/mandate/);
    expect(decodeURIComponent(new URL(page.url()).hash)).toContain('position:board');
    await page.goBack();
    await expect(inspector.getByRole('heading', { name: 'Select a position', exact: true })).toBeVisible();
    expect(mutations).toEqual([]);
    expect(await service.refs()).toBe(before);
  });

  // Drawing the chart by hand: a stated position offers to say what it
  // does, to move, and to retire; checking a change previews the chart
  // lines it would add without writing anything.
  test('a position offers its changes, and checking one writes nothing', async ({ page, service }) => {
    const mutations = [];
    page.on('request', request => { if (isMutation(request)) mutations.push(request.url()); });
    const before = await service.refs();
    await page.goto(service.url('org', { id: 'position:api/dev' }));
    const inspector = page.getByRole('complementary', { name: 'Inspector' });
    await expect(inspector.getByRole('heading', { name: 'api/dev', exact: true })).toBeVisible();
    await expect(inspector.getByRole('heading', { name: /say what this position does|change what this position does/ })).toBeVisible();
    await expect(inspector.getByRole('heading', { name: 'move this position', exact: true })).toBeVisible();
    await expect(inspector.getByRole('heading', { name: 'retire this position', exact: true })).toBeVisible();
    await inspector.getByPlaceholder('one sentence on what this position does').fill('Keeps the api correct and fast.');
    await inspector.getByRole('button', { name: 'Check the change' }).first().click();
    await expect(inspector).toContainText('+ keeps position:api/dev Keeps the api correct and fast.');
    await expect(inspector.getByRole('button', { name: 'Propose it' }).first()).toBeVisible();
    expect(mutations).toEqual([]);
    expect(await service.refs()).toBe(before);
  });

  test('a position the chart shows is one the record states', async ({ page, service }) => {
    await page.goto(service.url('org', { id: 'position:nowhere' }));
    const inspector = page.getByRole('complementary', { name: 'Inspector' });
    await expect(inspector.getByRole('heading', { name: 'Select a position', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Org chart' }).getByRole('link', { name: /^board/ })).toBeVisible();
  });
});
