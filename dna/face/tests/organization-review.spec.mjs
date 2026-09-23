// Renderer checks consume retained responses from the actual native candidate
// API proof. They do not establish native command delivery or host activation.
import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const evidence = process.env.HALE_ORGANIZATION_REVIEW_EVIDENCE;
const web = fileURLToPath(new URL('../web/', import.meta.url));
test.skip(!evidence, 'Supply retained native candidate-response.json and review-response.json.');
let server, origin, fixture;
test.beforeAll(async () => {
  if (!evidence) return;
  const candidate = JSON.parse(await readFile(path.join(evidence, 'candidate-response.json'), 'utf8'));
  const reviews = JSON.parse(await readFile(path.join(evidence, 'review-response.json'), 'utf8'));
  const data = candidate.data, review = reviews.data.items.find(row => row.id === data.review_id);
  expect(review).toBeTruthy();
  fixture = { data, review, applicationId: candidate.source.record_id };
  server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://localhost');
    if (url.pathname === '/fixture.json') { response.setHeader('content-type', 'application/json'); response.end(JSON.stringify(fixture)); return; }
    if (url.pathname === '/styles.css' || url.pathname === '/organization-draft.js') {
      response.setHeader('content-type', url.pathname.endsWith('.css') ? 'text/css; charset=utf-8' : 'text/javascript; charset=utf-8');
      response.end(await readFile(path.join(web, url.pathname.slice(1)))); return;
    }
    response.setHeader('content-type', 'text/html; charset=utf-8');
    response.end('<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/styles.css"><script src="/organization-draft.js"></script><main style="max-width:1100px;margin:40px auto;padding:20px"><div id="candidate"></div></main>');
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = 'http://127.0.0.1:' + server.address().port;
});
test.afterAll(async () => { if (server) await new Promise(resolve => server.close(resolve)); });
async function render(page) {
  await page.goto(origin);
  await page.evaluate(async () => {
    const f = await (await fetch('/fixture.json')).json();
    const candidate = await window.FaceOrganizationReview.validate(f.data, f.review, f.applicationId);
    window.__sourceCandidate = candidate;
    document.getElementById('candidate').replaceChildren(window.FaceOrganizationReview.render(candidate));
  });
}
test('native Organization candidate: navigate semantic changes and exact retained source', async ({ page }, testInfo) => {
  const errors = []; page.on('pageerror', error => errors.push(error.message)); await render(page);
  const comparison = page.getByRole('region', { name: 'Organization candidate comparison' });
  await expect(comparison).toContainText('ORGANIZATION / EXACT CANDIDATE');
  await expect(comparison).toContainText('Review authority ·');
  await comparison.getByRole('button', { name: 'All declarations', exact: true }).click();
  const semantic = JSON.parse(JSON.parse(fixture.data.document).verification.semantic_diff.document);
  const row = semantic.declarations[0];
  await comparison.getByRole('button', { name: 'Inspect ' + row.kind + ' ' + row.name, exact: true }).click();
  await expect(comparison.locator('.source-map-inspector')).toContainText(row.name);
  await expect(comparison.getByRole('button', { name: 'Inspect ' + row.kind + ' ' + row.name, exact: true })).toHaveAttribute('aria-pressed', 'true');
  await comparison.getByRole('button', { name: 'Full source', exact: true }).click();
  await expect(comparison.getByLabel('Captured base source', { exact: true })).toContainText('main locus');
  expect(await page.evaluate(() => window.__sourceCandidate.organization.module.text)).toBe(JSON.parse(fixture.data.document).module.text);
  await expect(comparison.locator('img')).toHaveCount(0);
  await expect(comparison).not.toContainText('/tmp/');
  await expect(comparison).not.toContainText('/home/');
  await page.screenshot({ path: testInfo.outputPath('organization-source-review-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await comparison.getByRole('button', { name: 'Changed region', exact: true }).click();
  await page.screenshot({ path: testInfo.outputPath('organization-source-review-mobile.png'), fullPage: true });
  expect(errors).toEqual([]);
});
test('native Organization candidate: changed bytes, semantic digest and wrong Review are refused before rendering', async ({ page }) => {
  await page.goto(origin);
  const result = await page.evaluate(async () => {
    const f = await (await fetch('/fixture.json')).json(); const refused = [];
    for (const alteration of ['module', 'semantic', 'review', 'application']) {
      const data = structuredClone(f.data), review = structuredClone(f.review); let app = f.applicationId;
      if (alteration === 'module' || alteration === 'semantic') {
        const c = JSON.parse(data.document);
        if (alteration === 'module') c.module.text += '\n// altered after receipt';
        else c.verification.semantic_diff.digest = 'sha256:' + '0'.repeat(64);
        data.document = JSON.stringify(c);
      } else if (alteration === 'review') review.subject_digest = '0'.repeat(40);
      else app = 'different-application';
      try { await window.FaceOrganizationReview.validate(data, review, app); refused.push(false); } catch { refused.push(true); }
    }
    return refused;
  });
  expect(result).toEqual([true, true, true, true]);
  await expect(page.locator('#candidate')).toBeEmpty();
});
