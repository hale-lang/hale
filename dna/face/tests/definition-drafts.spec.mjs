import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { test, expect, errorBody } from './harness.mjs';

test.skip(!process.env.HALE_FACE_CATALOG_BIN, 'Definition authoring needs an explicitly supplied native application catalog.');
test.use({ definitions: true });
const revision = '9007199254740993';
const next = '9007199254740994';
const nativePath = '**/api/hale/v1/applications/*/dna/definitions/draft';
const workspace = page => page.getByRole('region', { name: 'Definition workspace', exact: true });
const validation = page => page.getByRole('region', { name: 'Definition validation', exact: true });
const postResponse = page => page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname.endsWith('/dna/definitions/draft'));
const hash = value => 'sha256:' + createHash('sha256').update(value).digest('hex');
async function draft(page, service, id = 'collect@' + revision) {
  await page.goto(service.url('definitions', { id }));
  await workspace(page).getByRole('button', { name: 'Draft next revision', exact: true }).click();
  await expect(workspace(page).getByLabel('Definition title', { exact: true })).toBeFocused();
}
async function submit(page) {
  const response = postResponse(page);
  await validation(page).getByRole('button', { name: 'Validate draft', exact: true }).click();
  return response;
}

test('Definition draft validates a full native catalog and exports exact Hale without changing source', async ({ page, service }, testInfo) => {
  const refs = await service.refs();
  await draft(page, service);
  const title = 'Evidence — équipe 🧬 <draft>';
  const objective = 'Collect the evidence\nwith Unicode: café 🧬\n<img src=x onerror="window.__draftInjected=true">';
  await workspace(page).getByLabel('Definition title', { exact: true }).fill(title);
  await workspace(page).getByLabel('Objective', { exact: true }).fill(objective);
  await workspace(page).getByLabel('Cost ceiling', { exact: true }).fill('-9223372036854775808');
  const response = await submit(page);
  expect(response.status()).toBe(200);
  const body = response.request().postDataJSON();
  const result = (await response.json()).data;
  expect(body.candidate.revision).toBe(next);
  expect(body.candidate.steps[0].members[0].leaf.cost_ceiling).toBe('-9223372036854775808');
  expect(body.candidate.steps[0].members[0].leaf.objective).toBe(objective);
  expect(result.candidate_digest).toBe(hash(JSON.stringify(body.candidate)));
  expect(result.validation.valid).toBe(true);
  expect(result.impact.direct_dependents).toEqual([{ id: 'close@1', definition_id: 'close', revision: '1', step: '0', key: 'gather' }]);
  await expect(validation(page)).toContainText('Native catalog validation passed');
  await expect(validation(page).getByRole('status')).toBeFocused();
  await validation(page).getByText('Generated Hale and validation evidence', { exact: true }).click();
  await expect(validation(page)).toContainText('Publication');
  await expect(validation(page)).toContainText('Unavailable — not published');
  expect(await page.evaluate(() => window.__draftInjected)).toBeUndefined();
  await expect(workspace(page).locator('img')).toHaveCount(0);
  const downloaded = page.waitForEvent('download');
  await validation(page).getByRole('button', { name: 'Download generated Hale', exact: true }).click();
  const file = await downloaded;
  expect(file.suggestedFilename()).toBe('workflows.generated.hl');
  const artifactPath = testInfo.outputPath('workflows.generated.hl');
  await file.saveAs(artifactPath);
  const exported = await readFile(artifactPath, 'utf8');
  expect(exported).toBe(result.artifact.text);
  expect(hash(exported)).toBe(result.artifact.digest);
  expect(exported).toContain('iris_register_workflows');
  expect(exported).toContain('9007199254740993');
  expect(exported).toContain('9007199254740994');
  expect(exported).toContain('procedure-25');
  expect(await service.refs()).toBe(refs);
  const catalog = await (await page.request.get(service.origin + service.apiPath + '/dna/definitions')).json();
  expect(catalog.data.page.total).toBe(29);
  expect(catalog.data.items.some(item => item.id === 'collect@' + next)).toBe(false);
  expect(await page.evaluate(() => Object.values(localStorage).some(value => value.includes('with Unicode:')))).toBe(false);
  await page.evaluate(() => scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('definition-draft-desktop.png'), fullPage: true });
});

test('Definition draft edits Steps and members with local checks before any native request', async ({ page, service }) => {
  let posts = 0;
  page.on('request', request => { if (request.method() === 'POST') posts += 1; });
  await draft(page, service);
  await workspace(page).getByRole('button', { name: 'Add Step', exact: true }).click();
  await expect(validation(page)).toContainText('Step 2 requires at least one member');
  await expect(validation(page).getByRole('button', { name: 'Validate draft', exact: true })).toBeDisabled();
  await workspace(page).getByRole('button', { name: 'Add leaf to Step 2', exact: true }).click();
  await workspace(page).getByLabel('Member key', { exact: true }).fill('second');
  await workspace(page).getByLabel('Objective', { exact: true }).fill('Second Step');
  await workspace(page).getByRole('button', { name: 'Move Step 2 earlier', exact: true }).click();
  await expect(workspace(page).getByRole('button', { name: 'Inspect member second in Step 1', exact: true })).toBeVisible();
  await workspace(page).getByRole('button', { name: 'Add leaf to Step 1', exact: true }).click();
  await workspace(page).getByLabel('Member key', { exact: true }).fill('second');
  await expect(validation(page)).toContainText('distinct member keys');
  await expect(validation(page).getByRole('button', { name: 'Validate draft', exact: true })).toBeDisabled();
  await workspace(page).getByRole('button', { name: 'Remove member', exact: true }).click();
  await workspace(page).getByRole('button', { name: 'Remove Step 1', exact: true }).click();
  await workspace(page).getByRole('button', { name: 'Draft JSON', exact: true }).click();
  const data = JSON.parse(await workspace(page).locator('.dd-json-panel pre').textContent());
  expect(data.steps).toHaveLength(1);
  expect(data.steps[0].members[0].key).toBe('human');
  expect(data.steps[0].members[0].leaf.cost_ceiling).toBe('9223372036854775807');
  expect(posts).toBe(0);
});

test('Native definition validation rejects missing child references and existing next revisions', async ({ page, service }) => {
  await draft(page, service);
  await workspace(page).getByRole('button', { name: 'Add child to Step 1', exact: true }).click();
  await workspace(page).getByLabel('Child definition ID', { exact: true }).fill('missing-child');
  await workspace(page).getByLabel('Child revision', { exact: true }).fill('-9007199254740993');
  const missing = await submit(page);
  expect(missing.status()).toBe(200);
  expect((await missing.json()).data.validation.valid).toBe(false);
  await expect(validation(page)).toContainText('Native catalog validation refused');
  await expect(validation(page).getByRole('button', { name: 'Download generated Hale', exact: true })).toHaveCount(0);
  await draft(page, service, 'close@1');
  const collision = await submit(page);
  expect(collision.status()).toBe(200);
  expect((await collision.json()).data.validation.valid).toBe(false);
  await expect(validation(page)).toContainText('Native catalog validation refused');
});

test('Changed draft fields invalidate verified results and ignore late native responses', async ({ page, service }) => {
  await draft(page, service);
  await submit(page);
  await expect(validation(page).getByRole('button', { name: 'Download generated Hale', exact: true })).toBeVisible();
  await workspace(page).getByLabel('Definition title', { exact: true }).fill('Changed after validation');
  await expect(validation(page).getByRole('button', { name: 'Download generated Hale', exact: true })).toHaveCount(0);
  let release, captured;
  const held = new Promise(resolve => { release = resolve; });
  const arrived = new Promise(resolve => { captured = resolve; });
  await page.route(nativePath, async route => {
    const response = await route.fetch();
    captured();
    await held;
    await route.fulfill({ response }).catch(() => {});
  });
  await validation(page).getByRole('button', { name: 'Validate draft', exact: true }).click();
  await arrived;
  await workspace(page).getByLabel('Definition title', { exact: true }).fill('Changed while validating');
  release();
  await expect(validation(page)).toContainText('native validation not established');
  await expect(validation(page).getByRole('button', { name: 'Download generated Hale', exact: true })).toHaveCount(0);
  await page.unrouteAll({ behavior: 'wait' });
});

test('Definition source movement and authentication loss clear drafts and generated text', async ({ page, service }) => {
  await draft(page, service);
  await workspace(page).getByLabel('Definition title', { exact: true }).fill('PRIVATE-DRAFT-TITLE');
  await service.changeCatalog('updated');
  const moved = await submit(page);
  expect(moved.status()).toBe(409);
  await expect(workspace(page).getByRole('button', { name: 'Draft next revision', exact: true })).toBeVisible();
  await expect(page.locator('body')).not.toContainText('PRIVATE-DRAFT-TITLE');
  await expect(page.locator('#source')).toContainText('fixture-catalog-v2');
  await workspace(page).getByRole('button', { name: 'Draft next revision', exact: true }).click();
  await workspace(page).getByLabel('Definition title', { exact: true }).fill('SECOND-PRIVATE-DRAFT');
  await page.route(nativePath, route => route.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify(errorBody('unauthenticated', 'Sign in')) }));
  await submit(page);
  await expect(page.getByRole('heading', { name: 'Sign in to read this application', exact: true })).toBeVisible();
  await expect(page.locator('body')).not.toContainText('SECOND-PRIVATE-DRAFT');
  await expect(page.locator('body')).not.toContainText('fixture-catalog-v2');
});

test('Definition export refuses changed artifact bytes, mismatched candidate and principal', async ({ page, service }) => {
  await draft(page, service);
  for (const field of ['artifact', 'candidate', 'principal']) {
    await page.route(nativePath, async route => {
      const response = await route.fetch();
      const body = await response.json();
      expect(body.data.validation.valid).toBe(true);
      if (field === 'artifact') body.data.artifact.text += '\n// altered';
      else if (field === 'candidate') body.data.candidate_digest = 'sha256:' + '0'.repeat(64);
      else body.data.principal.name = 'another-principal';
      await route.fulfill({ response, json: body });
    });
    await submit(page);
    if (field === 'principal') {
      await expect(workspace(page).getByRole('button', { name: 'Draft next revision', exact: true })).toBeVisible();
      await expect(page.locator('#notice')).toContainText('changed application, principal or captured source context');
    } else await expect(validation(page)).toContainText('unverifiable validation result');
    await expect(workspace(page).getByRole('button', { name: 'Download generated Hale', exact: true })).toHaveCount(0);
    await page.unroute(nativePath);
  }
});

test('Definition drafts do not infer validation capability from read access', async ({ page, service }) => {
  await page.route('**/capabilities', async route => {
    const response = await route.fetch();
    const body = await response.json();
    delete body.data.definition_drafts;
    await route.fulfill({ response, json: body });
  });
  let posts = 0;
  page.on('request', request => { if (request.method() === 'POST') posts += 1; });
  await draft(page, service);
  await expect(validation(page)).toContainText('Native validation is unavailable');
  await expect(validation(page).getByRole('button', { name: 'Validate draft', exact: true })).toBeDisabled();
  expect(posts).toBe(0);
});

test('Definition recursive entry retains the actual occurrence path and mobile draft is usable', async ({ page, service }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(service.url('definitions', { id: 'close@1' }));
  await workspace(page).getByRole('button', { name: 'Open child collect@' + revision, exact: true }).click();
  const path = page.getByRole('navigation', { name: 'Traversed definition path', exact: true });
  await expect(path).toContainText('Step 1 · gather');
  await expect(path.getByRole('link', { name: 'close@1', exact: true })).toBeVisible();
  await workspace(page).getByRole('button', { name: 'Draft next revision', exact: true }).click();
  await expect(workspace(page).getByLabel('Definition title', { exact: true })).toBeFocused();
  await workspace(page).getByLabel('Definition title', { exact: true }).fill('Mobile evidence');
  await workspace(page).getByRole('button', { name: 'Add Step', exact: true }).focus();
  await page.keyboard.press('Enter');
  await workspace(page).getByRole('button', { name: 'Add leaf to Step 2', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(workspace(page).getByLabel('Member key', { exact: true })).toBeFocused();
  await workspace(page).getByLabel('Objective', { exact: true }).fill('Mobile second step');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await workspace(page).getByRole('heading', { name: 'Mobile evidence', exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath('definition-draft-mobile.png'), fullPage: true });
  await workspace(page).getByRole('button', { name: 'Discard draft', exact: true }).click();
  await path.getByRole('link', { name: 'close@1', exact: true }).click();
  await expect(workspace(page).getByRole('heading', { name: 'Close the month — équipe <workflow>', exact: true })).toBeVisible();
});
