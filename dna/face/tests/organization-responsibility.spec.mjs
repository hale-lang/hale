// UI CONTRACT ONLY: scripted source-impact objects exercise the production
// validator/renderer. They do not establish native responsibility counts,
// source grants, complete history, admission safety or real service freshness.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './http-fixture.mjs';

const SOURCE = { record_id: 'a'.repeat(40), record_head: 'b'.repeat(40), record_revision: '90' };
const REVIEW = { id: 'source-' + 'c'.repeat(64), organization_source: true, subject_digest: 'd'.repeat(40), organization_source_digest: 'sha256:' + 'e'.repeat(64) };
const COUNTS = { workflow_tasks: '9007199254740993', open_workflow_tasks: '3', open_works: '5', outstanding_attempts: '2', legacy_tasks: '8', open_legacy_tasks: '2', handed_tasks: '1', unresolved_intents: '1', schedules: '2', pending_effects: '1', unknown_effects: '1' };
const data = () => ({ profile: 'dna.organization.source-impact.v1', review_id: REVIEW.id, mutation_id: REVIEW.id, source: { base_commit: '1'.repeat(40), module_digest: 'sha256:' + '2'.repeat(64), candidate_commit: REVIEW.subject_digest, source_digest: REVIEW.organization_source_digest }, basis: { record_head: SOURCE.record_head, record_revision: SOURCE.record_revision, memory: 'record', scope: 'application', position_binding: 'unavailable', admission_fence: 'unavailable' }, state: 'obligations_observed', reason_code: 'impact_obligations_observed', counts: { ...COUNTS } });
const region = page => page.getByRole('region', { name: 'Current responsibility check', exact: true });
const category = (page, name) => region(page).getByRole('button', { name, exact: true });
const inspector = page => region(page).getByRole('group', { name: 'Selected responsibility category', exact: true });
const test = base.extend({
  page: async ({ page }, use) => { const errors = []; page.on('pageerror', error => errors.push(error.message)); await use(page); expect(errors).toEqual([]); },
  host: async ({}, use) => {
    const assets = new Map(await Promise.all(['organization-draft.js', 'styles.css'].map(async name => [name, await readFile(new URL('../web/' + name, import.meta.url))])));
    const requests = [], host = await httpFixture((req, res) => {
      requests.push({ method: req.method, path: req.url }); res.setHeader('cache-control', 'no-store');
      if (req.url === '/') { res.setHeader('content-type', 'text/html; charset=utf-8'); res.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/organization-draft.js" defer></script></head><body><main class="workspace"></main></body></html>'); return; }
      const name = req.url.slice(1); if (!assets.has(name)) { res.writeHead(404).end(); return; }
      res.setHeader('content-type', (name.endsWith('.js') ? 'text/javascript' : 'text/css') + '; charset=utf-8'); res.end(assets.get(name));
    });
    try { await use({ ...host, requests }); } finally { await host.close(); }
  }
});
async function mount(page, host, input = data(), unavailableReason = '') {
  await page.goto(host.origin);
  await page.evaluate(({ input, review, source, unavailableReason }) => {
    window.refreshes = 0;
    const model = input === null ? null : window.FaceOrganizationImpact.validate(input, review, source);
    document.querySelector('main').replaceChildren(window.FaceOrganizationImpact.render(model, { inspectedAt: new Date('2026-09-19T22:00:00Z'), unavailableReason, onRefresh: () => { window.refreshes++; } }));
  }, { input, review: REVIEW, source: SOURCE, unavailableReason });
}

test('Organization responsibility: exact closed wire joins and honest unavailable states validate without candidate plaintext', async ({ page, host }) => {
  await page.goto(host.origin);
  const result = await page.evaluate(({ input, review, source }) => {
    const valid = (value, r = review, s = source) => { try { window.FaceOrganizationImpact.validate(value, r, s); return true; } catch { return false; } };
    const none = structuredClone(input); none.state = 'none_observed'; none.reason_code = 'impact_none_observed';
    for (const key of Object.keys(none.counts)) if (!['workflow_tasks', 'legacy_tasks'].includes(key)) none.counts[key] = '0';
    const unavailable = ['impact_read_limit', 'impact_source_invalid', 'impact_source_unavailable', 'impact_unsupported_history'].map(reason_code => valid({ ...input, state: 'unavailable', reason_code, counts: null }));
    return { observed: valid(input), none: valid(none), unavailable, optionalDigest: valid(input, { ...review, organization_source_digest: '' }) };
  }, { input: data(), review: REVIEW, source: SOURCE });
  expect(result).toEqual({ observed: true, none: true, unavailable: [true, true, true, true], optionalDigest: true });
});

test('Organization responsibility: malformed identity, scope, counts and contradictory outcomes are rejected', async ({ page, host }) => {
  await page.goto(host.origin);
  const result = await page.evaluate(({ input, review, source }) => {
    const changes = [
      value => { value.profile = 'future'; }, value => { value.extra = true; }, value => { value.review_id += 'other'; }, value => { value.mutation_id += 'other'; },
      value => { value.source.candidate_commit = 'f'.repeat(40); }, value => { value.source.source_digest = 'sha256:' + 'f'.repeat(64); }, value => { value.source.base_commit = 'not-a-commit'; },
      value => { value.basis.record_head = 'f'.repeat(40); }, value => { value.basis.record_revision = '91'; }, value => { value.basis.memory = 'ledger'; }, value => { value.basis.scope = 'position'; }, value => { value.basis.position_binding = 'org/support'; }, value => { value.basis.admission_fence = 'safe'; },
      value => { value.counts.open_works = 5; }, value => { value.counts.open_works = '-1'; }, value => { value.counts.open_works = '01'; }, value => { value.counts.open_works = '9223372036854775808'; }, value => { delete value.counts.handed_tasks; }, value => { value.counts.secret_count = '1'; },
      value => { value.counts.open_workflow_tasks = '9007199254740994'; }, value => { value.counts.open_legacy_tasks = '9'; }, value => { value.counts.handed_tasks = '3'; },
      value => { value.state = 'unavailable'; value.reason_code = 'impact_source_unavailable'; }, value => { value.state = 'none_observed'; value.reason_code = 'impact_none_observed'; }, value => { value.reason_code = 'impact_none_observed'; },
      value => { for (const key of Object.keys(value.counts)) value.counts[key] = '0'; }, value => { value.counts = null; }, value => { value.state = 'unavailable'; value.counts = null; value.reason_code = 'revealed_private_data'; }
    ];
    return changes.map(change => { const value = structuredClone(input); change(value); try { window.FaceOrganizationImpact.validate(value, review, source); return 'accepted'; } catch { return 'rejected'; } });
  }, { input: data(), review: REVIEW, source: SOURCE });
  expect(result).toHaveLength(28); expect(result.every(value => value === 'rejected')).toBe(true);
});

test('Organization responsibility: category exploration preserves overlapping exact counts and narrow keyboard context', async ({ page, host }, info) => {
  await mount(page, host);
  await expect(region(page)).toHaveAttribute('data-state', 'obligations_observed');
  await expect(region(page)).toContainText('not attributed to particular changed positions');
  await expect(category(page, 'Tasks')).toHaveAttribute('aria-pressed', 'true');
  await expect(inspector(page).locator('[data-count="workflow_tasks"] dd')).toHaveText('9007199254740993');
  await expect(inspector(page)).toContainText('included in open legacy Tasks');
  await category(page, 'Work').focus(); await page.keyboard.press('Enter');
  await expect(category(page, 'Work')).toBeFocused(); await expect(category(page, 'Work')).toHaveAttribute('aria-pressed', 'true');
  await expect(inspector(page).locator('[data-count="open_works"] dd')).toHaveText('5');
  await expect(inspector(page).locator('[data-count="outstanding_attempts"] dd')).toHaveText('2');
  await category(page, 'Schedules').click(); await expect(inspector(page).locator('[data-count="schedules"] dd')).toHaveText('2');
  await category(page, 'Uncertain work & effects').click(); await expect(inspector(page).locator('[data-count="unknown_effects"] dd')).toHaveText('1');
  await expect(region(page)).toContainText('Do not add them into a total obligation count');
  await expect(region(page)).toContainText('does not authorize source application');
  await expect(region(page)).toContainText('does not retire responsibilities');
  await expect(region(page).getByRole('link')).toHaveCount(0);
  await expect(region(page).getByRole('button', { name: /apply|approve|retire|cancel|retry/i })).toHaveCount(0);
  await region(page).screenshot({ path: info.outputPath('responsibility-desktop.png') });
  await page.setViewportSize({ width: 390, height: 1000 });
  await category(page, 'Tasks').focus(); await page.keyboard.press('Space'); await expect(category(page, 'Tasks')).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await region(page).screenshot({ path: info.outputPath('responsibility-mobile.png') });
  await region(page).getByRole('button', { name: 'Refresh responsibility check', exact: true }).click();
  expect(await page.evaluate(() => window.refreshes)).toBe(1);
  expect(host.requests.every(request => request.method === 'GET' && !request.path.startsWith('/api/'))).toBe(true);
});

test('Organization responsibility: zero stays bounded and unavailable refreshes remove every former count', async ({ page, host }) => {
  const none = data(); none.state = 'none_observed'; none.reason_code = 'impact_none_observed';
  for (const key of Object.keys(none.counts)) if (!['workflow_tasks', 'legacy_tasks'].includes(key)) none.counts[key] = '0';
  await mount(page, host, none);
  await expect(region(page)).toContainText('None observed in this bounded read');
  await expect(inspector(page).locator('[data-count="workflow_tasks"] dd')).toHaveText('9007199254740993');
  await expect(region(page)).toContainText('not an assessment retained at decision time');
  for (const input of [{ ...data(), state: 'unavailable', reason_code: 'impact_source_unavailable', counts: null }, null]) {
    await page.evaluate(({ input, review, source }) => { const model = input === null ? null : window.FaceOrganizationImpact.validate(input, review, source); document.querySelector('main').replaceChildren(window.FaceOrganizationImpact.render(model, { unavailableReason: 'Read unavailable — <img src=x onerror="window.injected=true">', onRefresh() {} })); }, { input, review: REVIEW, source: SOURCE });
    await expect(region(page)).toHaveAttribute('data-state', 'unavailable');
    await expect(region(page).locator('[data-count]')).toHaveCount(0); await expect(region(page).getByRole('group', { name: 'Responsibility categories', exact: true })).toHaveCount(0);
    await expect(region(page)).not.toContainText('9007199254740993'); await expect(region(page)).toContainText('Counts are withheld');
    await expect(region(page)).toContainText('<img src=x'); expect(await page.evaluate(() => window.injected)).toBeUndefined();
  }
});
