// UI module contract only. All callback values are scripted; these tests prove
// context, exact draft arguments and callback reuse, not native authority,
// command persistence, Review, adoption or graph effects.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './runtime-harness.mjs';

const ID = 'sha256:' + 'ab'.repeat(32), BINDING = 'sha256:' + 'cd'.repeat(32);
const ORIGINAL = '  Original practice — équipe\r\nKeep literal <img src=x onerror="window.injected=true">\r\nSecond line\n';
const ITEM = { id: ID, kind: 'practice', name: '  Evidence practice  ', text: ORIGINAL, author: 'org/source' };
const BIND = { id: BINDING, idea_id: ID, author: 'org/source', target: 'org/support', class: 'initiative', applicability: 'exact' };
const EMPTY = { items: [], page: { has_more: false } };
const editor = page => page.getByRole('region', { name: 'Practice change editor', exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['knowledge-draft.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const requests = [], host = await httpFixture((req, res) => {
      requests.push({ method: req.method, url: req.url }); res.setHeader('cache-control', 'no-store');
      if (req.url === '/') { res.setHeader('content-type', 'text/html; charset=utf-8'); res.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/knowledge-draft.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      const name = req.url.slice(1); if (!assets.has(name)) { res.writeHead(404).end(); return; }
      res.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); res.end(assets.get(name));
    });
    try { await use({ ...host, requests }); } finally { await host.close(); }
  }
});

async function mount(page, host, changes = {}) {
  await page.goto(host.origin);
  const options = { applicationId: 'app-contract', principal: { mode: 'local', name: 'alice' }, basis: { projection_record_head: 'a'.repeat(40) }, snapshot: 'captured-snapshot', target: 'org/filter', item: ITEM, relationships: EMPTY, bindings: { ...EMPTY, items: [BIND] }, ...changes };
  await page.evaluate(options => {
    window.reviews = []; window.submissions = []; window.previews = []; window.invalidations = []; window.returns = 0;
    window.handle = window.IrisKnowledgeDraft.mount(document.querySelector('main'), { ...options,
      onReview: async ({ operation, relatedId }) => { window.reviews.push({ operation, relatedId }); return { relationships: options.relationships, bindings: options.bindings, commandCapability: { enabled: true, mode: 'review' } }; },
      onSubmit: async (draft, current) => window.submissions.push({ draft, current: current() }),
      onPreview: draft => window.previews.push(draft), onInvalidate: error => window.invalidations.push(error.status), onReturn: () => { window.returns++; }
    });
  }, options);
  return options;
}
async function reviewAndSubmit(page, rationale = 'Explain exact change — approved route') {
  await page.getByRole('textbox', { name: 'Reason for practice change', exact: true }).fill(rationale);
  await page.getByRole('button', { name: 'Review practice draft', exact: true }).click();
  await page.getByRole('button', { name: 'Submit practice change', exact: true }).click();
  const rows = await page.evaluate(() => window.submissions); expect(rows).toHaveLength(1); expect(rows[0].current).toBe(true); return rows[0].draft;
}

test('Practice context: ordinary Knowledge remains closed with its existing action labels and kinds', async ({ page, host }) => {
  await mount(page, host, { item: null });
  await expect(page.getByRole('region', { name: 'Knowledge change editor', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Change kind', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => [window.reviews.length, window.submissions.length, window.previews.length])).toEqual([0, 0, 0]);
  await page.getByRole('button', { name: 'Prepare knowledge change', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Knowledge kind', exact: true })).toBeEnabled();
  await expect(page.getByRole('combobox', { name: 'Knowledge kind', exact: true })).toHaveValue('idea');
  await mount(page, host);
  await page.getByRole('button', { name: 'Prepare knowledge change', exact: true }).click();
  expect(await page.getByRole('combobox', { name: 'Change kind', exact: true }).locator('option').allTextContents()).toContain('Add relationship');
  await expect(page.getByRole('textbox', { name: 'Target locus', exact: true })).toHaveValue('org/filter');
});

test('Practice context: creation opens only a pinned Practice draft and uses the unchanged node operation', async ({ page, host }, testInfo) => {
  const options = { item: null, target: 'org/support', practice: { action: 'create', author: 'org', target: 'org/support' } };
  await mount(page, host, options);
  await expect(page.getByRole('combobox', { name: 'Change kind', exact: true })).toHaveValue('node.propose');
  expect(await page.getByRole('combobox', { name: 'Change kind', exact: true }).locator('option').allTextContents()).toEqual(['New practice']);
  await expect(page.getByRole('combobox', { name: 'Practice kind', exact: true })).toBeDisabled();
  await expect(page.getByRole('combobox', { name: 'Practice kind', exact: true })).toHaveValue('practice');
  await expect(page.getByRole('textbox', { name: 'Requested authoring locus', exact: true })).toHaveValue('org');
  await expect(page.getByRole('textbox', { name: 'Requested target locus', exact: true })).toHaveValue('org/support');
  await expect(editor(page)).toContainText('A requested locus is not authority');
  expect(await page.evaluate(() => [window.reviews.length, window.submissions.length])).toEqual([0, 0]);
  await page.getByRole('textbox', { name: 'Practice name', exact: true }).fill('Careful evidence — équipe');
  await page.getByRole('textbox', { name: 'Practice text', exact: true }).fill('Keep <script>window.injected=true</script> literal.');
  const result = await reviewAndSubmit(page);
  expect(result.profile).toBe('iris.knowledge.change-draft.v1');
  expect(result.operation).toBe('node.propose');
  expect(result.arguments).toEqual({ kind: 'practice', name: 'Careful evidence — équipe', text: 'Keep <script>window.injected=true</script> literal.', author: 'org', target: 'org/support', rationale: 'Explain exact change — approved route' });
  expect(result.base).toEqual({ snapshot: 'captured-snapshot', basis: { projection_record_head: 'a'.repeat(40) } });
  await page.getByRole('region', { name: 'Practice draft preview', exact: true }).screenshot({ path: testInfo.outputPath('practice-create-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await editor(page).screenshot({ path: testInfo.outputPath('practice-create-mobile.png') });
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  await mount(page, host, options);
  expect(await page.evaluate(() => [window.reviews.length, window.submissions.length])).toEqual([0, 0]);
  expect(host.requests.every(request => request.method === 'GET')).toBe(true);
});

test('Practice context: revision preserves exact captured bytes and canonical target instead of the graph filter', async ({ page, host }) => {
  await mount(page, host, { practice: { action: 'revise', author: 'org/requested', target: 'org/canonical' } });
  await expect(page.getByRole('textbox', { name: 'Practice name', exact: true })).toHaveValue(ITEM.name);
  await expect(page.getByRole('textbox', { name: 'Requested authoring locus', exact: true })).toHaveValue(ITEM.author);
  await expect(page.getByRole('textbox', { name: 'Requested target locus', exact: true })).toHaveValue('org/canonical');
  await expect(editor(page)).toContainText('Extra applicability bindings are not automatically carried');
  expect(await page.evaluate(() => window.previews.at(-1).arguments.text)).toBe(ORIGINAL);
  const result = await reviewAndSubmit(page);
  expect(result.operation).toBe('node.revise');
  expect(result.arguments).toEqual({ supersedes: ID, kind: 'practice', name: ITEM.name, text: ORIGINAL, author: ITEM.author, target: 'org/canonical', rationale: 'Explain exact change — approved route' });
  await page.getByRole('button', { name: 'Discard practice draft', exact: true }).click();
  expect(await page.evaluate(() => window.handle.begin({ operation: 'edge.link' }))).toBe(false);
  expect(await page.evaluate(() => window.previews.at(-1).active)).toBe(false);
});

test('Practice context: wrong-kind, missing-item and create-with-item contexts fail closed', async ({ page, host }) => {
  for (const options of [
    { practice: { action: 'create', author: 'org' } },
    ...['revise', 'retire', 'applicability'].flatMap(action => [{ item: { ...ITEM, kind: 'idea' }, practice: { action, author: 'org' } }, { item: null, practice: { action, author: 'org' } }]),
    { practice: { action: 'edge.link', author: 'org' } },
    { practice: { action: 'revise', author: 'org', target: 23 } }
  ]) {
    await mount(page, host, options);
    await expect(editor(page)).toBeVisible();
    await expect(page.getByRole('combobox', { name: 'Change kind', exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Prepare practice change', exact: true })).toBeHidden();
    expect(await page.evaluate(() => window.handle.begin())).toBe(false);
    expect(await page.evaluate(() => [window.reviews.length, window.submissions.length, window.previews.length])).toEqual([0, 0, 0]);
  }
});

test('Practice context: applicability keeps exact returned tuple removal and requested binding defaults', async ({ page, host }) => {
  const options = { practice: { action: 'applicability', author: 'org', target: 'org/new-target' } };
  await mount(page, host, options);
  const choices = page.getByRole('combobox', { name: 'Change kind', exact: true });
  await expect(choices).toHaveValue('binding.bind');
  expect(await choices.locator('option').allTextContents()).toEqual(['Add practice applicability', 'Remove practice applicability']);
  const bound = await reviewAndSubmit(page);
  expect(bound.operation).toBe('binding.bind');
  expect(bound.arguments).toEqual({ idea_id: ID, author: 'org', target: 'org/new-target', rationale: 'Explain exact change — approved route' });
  await mount(page, host, options);
  await choices.selectOption('binding.unbind');
  await expect(page.getByRole('combobox', { name: 'Binding to remove', exact: true })).toHaveValue(BINDING);
  const unbound = await reviewAndSubmit(page);
  expect(unbound.operation).toBe('binding.unbind');
  // Preserve the old draft export shape. The app's existing native-command
  // adapter, not this module, maps id to the closed binding_id wire argument.
  expect(unbound.arguments).toEqual({ ...BIND, rationale: 'Explain exact change — approved route' });
  await page.getByRole('button', { name: 'Back to practice', exact: true }).click();
  expect(await page.evaluate(() => window.returns)).toBe(1);
});

test('Practice context: retirement retains history and reuses exact node retirement with explicit review', async ({ page, host }) => {
  await mount(page, host, { practice: { action: 'retire', author: 'org', target: 'org/canonical' } });
  await expect(page.getByRole('combobox', { name: 'Change kind', exact: true })).toHaveValue('node.retire');
  await expect(editor(page)).toContainText('retaining its history');
  expect(await page.evaluate(() => [window.reviews.length, window.submissions.length])).toEqual([0, 0]);
  const retired = await reviewAndSubmit(page);
  expect(retired.operation).toBe('node.retire');
  expect(retired.arguments).toEqual({ id: ID, rationale: 'Explain exact change — approved route' });
  expect(await page.evaluate(() => window.reviews)).toEqual([{ operation: 'node.retire', relatedId: '' }]);
  await page.evaluate(() => window.handle.destroy());
  expect(await page.evaluate(() => window.previews.at(-1).active)).toBe(false);
  await expect(editor(page)).toHaveCount(0);
});
