import { test, expect } from './harness.mjs';

// The calls the face sends, by name, reads and the describe left out.
const READS = new Set(['Index', 'Org', 'Uis', 'Tasks', 'BatchPreview']);
function commandsSent(page) {
  const sent = [];
  page.on('request', request => {
    if (request.method() !== 'POST' || !request.url().endsWith('/commands')) return;
    let line;
    try { line = JSON.parse(request.postData() || '{}'); } catch { return; }
    if (line.call && !READS.has(line.call)) sent.push(line.call);
  });
  return sent;
}

// Tasks are tickets for the chart's positions: opened for a position, taken
// by someone who holds it, updated, handed to another position and marked
// done, each one command, the board showing it without a reload. The head
// writes them to the record's Ledger, which it reaches through memory, so
// this needs the Knowledge fixture program (HALE_FACE_KNOWLEDGE_BIN).
test.describe('tasks for positions', () => {
  test.skip(!process.env.HALE_FACE_KNOWLEDGE_BIN, 'the Ledger is memory\'s: HALE_FACE_KNOWLEDGE_BIN was not supplied');
  test.use({ knowledge: true, seats: ['board', 'eng'], vacant: ['ops'] });

  test('open, take, update, hand on and finish a task', async ({ page, service }, testInfo) => {
    const sent = commandsSent(page);
    await page.goto(service.url('board'));
    const lane = name => page.getByRole('listitem', { name });
    const inspector = page.getByRole('complementary', { name: 'Inspector' });
    await expect(page.getByRole('button', { name: 'My tasks' })).toHaveAttribute('aria-pressed', 'true');

    // open: a task for a position I hold is mine, to do
    await page.getByRole('textbox', { name: 'What needs doing' }).fill('Write the release notes');
    await page.getByRole('combobox', { name: 'For position' }).selectOption('position:eng');
    await page.getByRole('button', { name: 'Add task' }).click();
    await expect(lane('To do')).toContainText('Write the release notes');
    await expect(lane('To do')).toContainText('for eng');
    expect(sent).toEqual(['TaskOpen']);

    // take: I hold eng, so I take it and it is in progress
    await lane('To do').getByRole('link', { name: /Write the release notes/ }).click();
    await inspector.getByRole('button', { name: 'Take it' }).click();
    await expect(lane('In progress')).toContainText('Write the release notes');
    await expect(lane('In progress')).toContainText(`${process.env.USER} is on it`);
    expect(sent).toEqual(['TaskOpen', 'TaskTake']);

    // update: the text lands in the task's history
    await inspector.getByRole('textbox', { name: 'Update' }).fill('drafted; the changelog is next');
    await inspector.getByRole('button', { name: 'Post update' }).click();
    await expect(inspector.locator('.history')).toContainText('drafted; the changelog is next');
    expect(sent).toEqual(['TaskOpen', 'TaskTake', 'TaskNote']);

    // hand on: to ops, which I do not hold, so it leaves my tasks
    await inspector.getByRole('combobox', { name: 'Hand to' }).selectOption('position:ops');
    await inspector.getByRole('button', { name: 'Hand it over' }).click();
    await expect(lane('In progress')).not.toContainText('Write the release notes');
    await expect(lane('To do')).not.toContainText('Write the release notes');
    await page.getByRole('button', { name: 'Everyone\'s' }).click();
    await expect(lane('To do')).toContainText('for ops');
    expect(sent).toEqual(['TaskOpen', 'TaskTake', 'TaskNote', 'TaskHand']);

    // the Board hands it back, and whoever holds eng finishes it
    await lane('To do').getByRole('link', { name: /Write the release notes/ }).click();
    await inspector.getByRole('combobox', { name: 'Hand to' }).selectOption('position:eng');
    await inspector.getByRole('button', { name: 'Hand it over' }).click();
    await expect(lane('To do')).toContainText('for eng');
    await inspector.getByRole('textbox', { name: 'What was done' }).fill('notes published');
    await inspector.getByRole('button', { name: 'Mark done' }).click();
    await expect(lane('Done')).toContainText('Write the release notes');
    expect(sent).toEqual(['TaskOpen', 'TaskTake', 'TaskNote', 'TaskHand', 'TaskHand', 'TaskDone']);
    await expect(inspector).toContainText('notes published');
    await expect(inspector.locator('.history')).toContainText('opened for eng');
    await expect(inspector.locator('.history')).toContainText('handed to ops');
    await page.screenshot({ path: testInfo.outputPath('tasks.png'), fullPage: true });
  });

  test('a task for a position the chart does not state is refused', async ({ page, service }) => {
    await page.goto(service.url('board'));
    const reply = await page.evaluate(async ([app]) => {
      const r = await fetch(`/api/hale/v1/applications/${app}/commands`, {
        method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Hale-Command': '1' },
        body: JSON.stringify({ call: 'TaskOpen', payload: { request_id: 'r-unknown', outcome: 'x', position: 'position:nowhere' } }),
      });
      return r.json();
    }, [service.application]);
    expect(JSON.stringify(reply)).toContain('unknown_position');
  });
});

// A chart change is one batch, one approval: a new position with its holder
// shows at once as proposed, and its Review waits under In review. It runs
// on the composed head, whose command service writes the Review, in memory.
test.describe('adding a position', () => {
  test.skip(!process.env.HALE_FACE_KNOWLEDGE_BIN || !process.env.HALE_NATIVE_COMMAND_API, 'a chart change is the composed head\'s, in memory: HALE_FACE_KNOWLEDGE_BIN and HALE_NATIVE_COMMAND_API');
  test.use({ knowledge: true, composed: true, seats: ['board'] });

  test('sends one batch and shows the proposal without a reload', async ({ page, service }) => {
    const sent = commandsSent(page);
    await page.goto(service.url('org', { id: 'position:board' }));
    const inspector = page.getByRole('complementary', { name: 'Inspector' });
    const add = inspector.getByRole('form', { name: 'Add a position' });
    await add.getByRole('textbox', { name: 'name' }).fill('support');
    await add.getByRole('textbox', { name: 'what it is for' }).fill('Answers the people who use it.');
    await add.getByRole('combobox', { name: 'held by' }).fill(process.env.USER);
    await add.getByRole('button', { name: 'Submit for approval' }).click();
    const chart = page.getByRole('region', { name: 'Org chart' });
    await expect(chart.getByRole('link', { name: /^support/ })).toContainText('proposed');
    expect(sent).toEqual(['BatchApply']);
    await page.goto(service.url('board'));
    await expect(page.getByRole('listitem', { name: 'In review' })).toContainText('open support under board');
  });
});
