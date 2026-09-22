import { test as base, expect } from '@playwright/test';
import { cockpitHost, scriptedObserver, observation, nativeObserver, pause } from './runtime-harness.mjs';

const status = page => page.getByRole('region', { name: 'Runtime observer status', exact: true });
const connect = page => page.getByRole('button', { name: 'Connect observer', exact: true }).click();
const refresh = page => page.getByRole('button', { name: 'Refresh observation', exact: true }).click();

const test = base.extend({
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await use(page);
    expect(errors, 'runtime raised no unhandled JavaScript errors').toEqual([]);
  },
  observer: async ({}, use) => {
    const observer = await scriptedObserver();
    try { await use(observer); } finally { await observer.close(); }
  },
  host: async ({ observer }, use) => {
    const host = await cockpitHost(observer.origin);
    try { await use(host); } finally { await host.close(); }
  },
});

test('Runtime without DNA: an unconfigured connection only prepares a safe external link', async ({ page }) => {
  const host = await cockpitHost();
  try {
    await page.goto(host.url);
    await expect(page.getByRole('heading', { name: 'Runtime', exact: true })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Organization', exact: true })).toBeHidden();
    const input = page.getByLabel('Observer URL', { exact: true });
    await input.fill('javascript:window.__runtimeInjected=true');
    await connect(page);
    await expect(page.getByRole('link', { name: 'Open runtime observer', exact: true })).toHaveCount(0);
    await input.fill('https://observer.example.invalid/');
    await connect(page);
    await expect(page.getByRole('link', { name: 'Open runtime observer', exact: true })).toHaveAttribute('href', 'https://observer.example.invalid/');
    expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
    expect(await page.evaluate(() => window.__runtimeInjected)).toBeUndefined();
  } finally { await host.close(); }
});

test('Runtime without DNA: explicit connection renders literal native shapes and omits credentials', async ({ page, context, observer, host }, testInfo) => {
  await context.addCookies([{ name: 'observer-secret', value: 'must-not-send', url: observer.origin }]);
  await page.goto(host.url);
  await expect(status(page)).toBeVisible();
  expect(observer.requests).toHaveLength(0);
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  await expect(page.locator('body')).toContainText(observation().processes[0].name);
  await page.getByRole('button', { name: 'Inspect process 41', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('0011223344556677');
  expect(await page.evaluate(() => window.__runtimeInjected)).toBeUndefined();
  expect(observer.requests.every(request => request.method === 'GET' && request.url === '/snapshot')).toBe(true);
  expect(observer.requests.every(request => !request.headers.cookie && !request.headers.authorization && !request.headers.referer)).toBe(true);
  expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('runtime-observed-fleet.png'), fullPage: true });
  await page.getByRole('button', { name: 'Topic routes', exact: true }).click();
  await expect(page.locator('body')).toContainText('work.ready');
});

test('Runtime distinguishes empty observation from unavailable source and recovers explicitly', async ({ page, observer, host }) => {
  observer.snapshot = { ts: 1, processes: [], topics: [], edges: [], events: [] };
  await page.goto(host.url);
  await connect(page);
  await expect(page.locator('body')).toContainText(/no processes/i);
  observer.snapshot = observation();
  await refresh(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  observer.status = 503;
  await refresh(page);
  await expect(status(page)).toContainText(/unavailable|failed|could not/i);
  await expect(page.locator('body')).not.toContainText(observation().processes[0].name);
  observer.status = 200;
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
});

test('Runtime preserves topic shape ambiguity and distinguishes unknown measurements from zero', async ({ page, observer, host }) => {
  observer.snapshot.processes[0].records = Number.MAX_SAFE_INTEGER + 1;
  observer.snapshot.processes[0].overruns = 7;
  observer.snapshot.processes[0].loci[1].parent = 999;
  observer.snapshot.topics.push({ name: 'work.ready', shape: '{message:String}', pub: 0, dlv: 0 });
  observer.snapshot.edges[0].to = 99;
  await page.goto(host.url);
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Inspect process 41', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText(/unavailable/i);
  await expect(page.locator('body')).toContainText(/overrun|loss/i);
  await page.getByRole('button', { name: 'Topic routes', exact: true }).click();
  await expect(page.locator('body')).toContainText(/ambiguous|multiple shapes/i);
  await expect(page.locator('body')).toContainText(/unobserved|not observed|unresolved/i);
  const topics = page.getByRole('button', { name: /^Inspect topic work.ready/ });
  await expect(topics).toHaveCount(2);
  await topics.nth(0).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('{id:Int}');
  await topics.nth(1).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('{message:String}');
  expect(await page.locator('body').innerText()).not.toContain(String(Number.MAX_SAFE_INTEGER + 1));
});

test('Runtime rejects unsafe identities, duplicate identities, cycles and oversized observations', async ({ page, observer, host }) => {
  await page.goto(host.url);
  for (const makeInvalid of [
    snapshot => { snapshot.processes[0].pid = Number.MAX_SAFE_INTEGER + 1; },
    snapshot => { snapshot.processes.push(structuredClone(snapshot.processes[0])); },
    snapshot => { snapshot.processes[0].loci[0].parent = 2; },
    snapshot => { snapshot.events = Array(65).fill('not displayed'); },
  ]) {
    observer.snapshot = observation();
    makeInvalid(observer.snapshot);
    await connect(page);
    await expect(status(page)).toContainText(/invalid|unsupported|unavailable|limit/i);
    await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
  }
  observer.raw = ' '.repeat(2_097_153);
  await connect(page);
  await expect(status(page)).toContainText(/limit|large|unavailable/i);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
});

test('Runtime timeout clears prior observations; disconnected late replies cannot restore them', async ({ page, observer, host }) => {
  await page.goto(host.url);
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  let release;
  observer.wait = new Promise(resolve => { release = resolve; });
  try {
    await refresh(page);
    await expect(status(page)).toContainText(/timed out|timeout|unavailable|could not/i);
    await expect(page.locator('body')).not.toContainText(observation().processes[0].name);
    const before = observer.requests.length;
    await connect(page);
    await expect.poll(() => observer.requests.length).toBeGreaterThan(before);
    await page.getByRole('button', { name: 'Disconnect observer', exact: true }).click();
    release(); observer.wait = null;
    await expect(status(page)).toContainText(/disconnected/i);
    await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
  } finally { release(); }
});

test('Runtime clears departed selections and pauses observation when hidden', async ({ page, observer, host }) => {
  await page.goto(host.url);
  await connect(page);
  await page.getByRole('button', { name: 'Inspect process 41', exact: true }).click();
  observer.snapshot.processes.shift();
  observer.snapshot.edges = [];
  await refresh(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).not.toContainText('0011223344556677');
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect(page.getByRole('button', { name: 'Inspect process 42', exact: true })).toHaveCount(0);
  await expect(status(page)).toContainText(/paused|hidden/i);
  const count = observer.requests.length;
  await pause(1200); // one poll interval: a hidden document must not keep polling
  expect(observer.requests).toHaveLength(count);
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 42', exact: true })).toBeVisible();
});

test('Runtime rejects redirects and unconfigured targets before receiving external content', async ({ page, observer, host }) => {
  const other = await scriptedObserver();
  try {
    observer.redirect = other.origin + '/snapshot';
    await page.goto(host.url);
    await connect(page);
    await expect(status(page)).toContainText(/unavailable|failed|could not/i);
    expect(other.requests).toHaveLength(0);
    await page.getByLabel('Observer URL', { exact: true }).fill(other.origin);
    await connect(page);
    await expect(page.getByRole('link', { name: 'Open runtime observer', exact: true })).toHaveAttribute('href', other.origin + '/');
    expect(other.requests).toHaveLength(0);
  } finally { await other.close(); }
});

test('Runtime does not mistake repeated or regressing observer timestamps for fresh evidence', async ({ page, observer, host }) => {
  observer.advance = false;
  await page.goto(host.url);
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  await expect(status(page)).toContainText(/stale|stopped advancing|not advance|unchanged|unavailable/i);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
  observer.snapshot.ts = 100;
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  observer.snapshot.ts = 50;
  await refresh(page);
  await expect(status(page)).toContainText(/regress|backward|reset|unavailable/i);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
});

test('Runtime navigation disposes pending observation and returning requires a new connection', async ({ page, observer, host }) => {
  await page.goto(host.url);
  await connect(page);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toBeVisible();
  let release;
  observer.wait = new Promise(resolve => { release = resolve; });
  try {
    await refresh(page);
    await expect.poll(() => observer.inFlight).toBe(1);
    await page.getByRole('button', { name: 'Back to DNA inspection', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Runtime observer', exact: true })).toHaveCount(0);
    release(); observer.wait = null;
    await expect.poll(() => observer.inFlight).toBe(0);
    await expect(page.locator('body')).not.toContainText(observation().processes[0].name);
    const requests = observer.requests.length;
    await page.goto(host.url);
    await expect(status(page)).toContainText(/disconnected/i);
    expect(observer.requests).toHaveLength(requests);
    expect(observer.maximumInFlight).toBe(1);
  } finally { release(); }
});

test('Runtime rejects malformed connection metadata without fetching an observer', async ({ page, observer }) => {
  const host = await cockpitHost(observer.origin, { profile: 'hale.iris.observer.v0', origin: observer.origin + '/private/snapshot' });
  try {
    await page.goto(host.url);
    await expect(status(page)).toContainText(/configuration is unavailable/i);
    expect(observer.requests).toHaveLength(0);
    await page.getByLabel('Observer URL', { exact: true }).fill(observer.origin);
    await connect(page);
    await expect(page.getByRole('link', { name: 'Open runtime observer', exact: true })).toBeVisible();
    expect(observer.requests).toHaveLength(0);
  } finally { await host.close(); }
});

test('Runtime refuses rounded identity lexemes and withholds rounded measurement lexemes', async ({ page, observer, host }) => {
  observer.raw = JSON.stringify(observation()).replace('"pid":41', '"pid":41.0000000000000001');
  await page.goto(host.url);
  await connect(page);
  await expect(status(page)).toContainText(/unsupported|invalid|unavailable/i);
  await expect(page.getByRole('button', { name: 'Inspect process 41', exact: true })).toHaveCount(0);
  observer.raw = JSON.stringify(observation()).replace('"records":24', '"records":24.0000000000000001');
  await connect(page);
  await page.getByRole('button', { name: 'Inspect process 41', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText(/unavailable/i);
});

test('Runtime has a narrow keyboard path through a selected observed process', async ({ page, observer, host }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(host.url);
  await page.getByRole('button', { name: 'Connect observer', exact: true }).focus();
  await page.keyboard.press('Enter');
  const process = page.getByRole('button', { name: 'Inspect process 41', exact: true });
  await expect(process).toBeVisible();
  await process.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('0011223344556677');
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  const inspector = await page.getByRole('complementary', { name: 'Runtime inspector', exact: true }).boundingBox();
  const navigation = await page.getByRole('banner', { name: 'Cockpit navigation', exact: true }).boundingBox();
  expect(inspector.y).toBeGreaterThanOrEqual(navigation.y + navigation.height);
  expect(inspector.y).toBeLessThan(844);
  await page.screenshot({ path: testInfo.outputPath('runtime-mobile-inspector-viewport.png') });
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('runtime-mobile.png'), fullPage: true });
});

test('Runtime native integration: observe a real plain Hale app without any DNA service or Record', async ({ page }, testInfo) => {
  test.skip(!process.env.HALE_COCKPIT_OBSERVER_BIN || !process.env.HALE_COCKPIT_PLAIN_APP_BIN,
    'Supply explicit native observer and plain Hale application binaries for the real observation gate.');
  const observer = await nativeObserver(process.env.HALE_COCKPIT_OBSERVER_BIN, process.env.HALE_COCKPIT_PLAIN_APP_BIN);
  let host;
  try {
    const captured = await observer.snapshot();
    expect(captured.processes.map(process => process.pid)).toEqual([observer.appPID]);
    host = await cockpitHost(observer.origin);
    await page.goto(host.url);
    await connect(page);
    await page.getByRole('button', { name: `Inspect process ${observer.appPID}`, exact: true }).click();
    await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText(String(observer.appPID));
    await expect(page.locator('body')).toContainText('Worker');
    await page.getByRole('button', { name: `Enter process ${observer.appPID}`, exact: true }).click();
    const canvas = page.getByRole('region', { name: 'Observed processes', exact: true });
    const process = captured.processes.find(item => item.pid === observer.appPID);
    const root = process.loci.find(item => item.parent === 0);
    expect(root).toBeTruthy();
    await page.getByRole('button', { name: `Enter locus ${root.id} in process ${observer.appPID}`, exact: true }).click();
    await expect(canvas).toHaveAttribute('data-scope-locus', String(root.id));
    const children = process.loci.filter(item => item.parent === root.id);
    expect(children.length).toBeGreaterThan(0);
    for (const child of children) await expect(canvas.getByRole('button', { name: `Inspect locus ${child.id}`, exact: true })).toBeVisible();
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: testInfo.outputPath('runtime-native-plain-hale.png'), fullPage: true });
    await page.getByRole('button', { name: 'Topic routes', exact: true }).click();
    await expect(page.locator('body')).toContainText('orders');
    expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
    await observer.stopObserver();
    await refresh(page);
    await expect(status(page)).toContainText(/unavailable|failed|could not/i);
    await expect(page.getByRole('button', { name: `Inspect process ${observer.appPID}`, exact: true })).toHaveCount(0);
    await testInfo.attach('native-snapshot.json', { body: JSON.stringify(captured, null, 2), contentType: 'application/json' });
  } finally {
    await testInfo.attach('observer.log', { body: observer.log(), contentType: 'text/plain' });
    if (host) await host.close();
    await observer.close();
  }
});

function recursiveObservation() {
  const snapshot = observation();
  snapshot.processes[0].loci = [
    { id: 1, type: 'Intake', parent: 0, pub: 12, dlv: 0 },
    { id: 2, type: 'Dispatch', parent: 1, pub: 12, dlv: 12 },
    { id: 7, type: 'Worker <script>window.__runtimeInjected=true</script>', parent: 2, pub: 0, dlv: 12 },
    { id: 4, type: 'Audit', parent: 1, pub: 0, dlv: 12 },
    { id: 5, type: 'Detached', parent: 999, pub: 0, dlv: 0 },
  ];
  return snapshot;
}
const containment = page => page.getByRole('region', { name: 'Observed processes', exact: true });
const containmentPath = page => page.getByRole('navigation', { name: 'Runtime containment path', exact: true });
const enterProcess = (page, pid = 41) => page.getByRole('button', { name: `Enter process ${pid}`, exact: true }).click();
const enterLocus = (page, id, pid = 41) => page.getByRole('button', { name: `Enter locus ${id} in process ${pid}`, exact: true }).click();
async function expectImmediateLoci(page, ids) {
  const actual = await containment(page).getByRole('list', { name: 'Immediate observed contents', exact: true }).getByRole('button', { name: /^Inspect locus \d+$/ }).evaluateAll(buttons => buttons.map(button => Number(button.getAttribute('aria-label').replace('Inspect locus ', ''))));
  expect(actual.sort((a, b) => a - b)).toEqual([...ids].sort((a, b) => a - b));
}

test('Runtime recursive focus changes immediate contents while inspection and routes preserve containment scope', async ({ page, observer, host }, testInfo) => {
  observer.snapshot = recursiveObservation();
  await page.goto(host.url);
  await connect(page);
  await enterProcess(page);
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'process');
  // The missing parent is explicit; it must not acquire an edge to Intake.
  await expect(containment(page)).toContainText(/999.*unobserved|unobserved.*999/i);
  await enterLocus(page, 1);
  await expectImmediateLoci(page, [2, 4]);
  await expect(containment(page)).toHaveAttribute('data-scope-locus', '1');
  await containment(page).getByRole('button', { name: 'Inspect locus 2', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).toContainText('Dispatch');
  await expect(containment(page)).toHaveAttribute('data-scope-locus', '1');
  await enterLocus(page, 2);
  await expectImmediateLoci(page, [7]);
  await expect(containmentPath(page).getByRole('button', { name: 'Locus 1 in process 41', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Topic routes', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Observed routing', exact: true })).toContainText(/process-to-process/);
  await page.getByRole('button', { name: 'Process containment', exact: true }).click();
  await expect(containment(page)).toHaveAttribute('data-scope-locus', '2');
  await expectImmediateLoci(page, [7]);
  await page.getByRole('button', { name: 'Accessible list', exact: true }).click();
  await expectImmediateLoci(page, [7]);
  await containmentPath(page).getByRole('button', { name: 'Locus 1 in process 41', exact: true }).click();
  await expectImmediateLoci(page, [2, 4]);
  await page.getByRole('button', { name: 'Process containment', exact: true }).click();
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('runtime-recursive-contents.png'), fullPage: true });
  expect(await page.evaluate(() => window.__runtimeInjected)).toBeUndefined();
  expect(host.requests.filter(path => path.startsWith('/api/'))).toEqual([]);
  expect(observer.requests.every(request => request.method === 'GET' && request.url === '/snapshot')).toBe(true);
});

test('Runtime containment breadcrumbs use observed parents and expose an unresolved ancestor', async ({ page, observer, host }) => {
  observer.snapshot = recursiveObservation();
  await page.goto(host.url);
  await connect(page);
  await enterLocus(page, 5);
  await expect(containmentPath(page)).toContainText(/999.*unobserved|unobserved.*999/i);
  await expect(containmentPath(page).getByRole('button', { name: /999/ })).toHaveCount(0);
  await expectImmediateLoci(page, []);
  await expect(containment(page)).toContainText(/no.*observed.*(child|content)|no.*(child|content).*observed/i);
  await containmentPath(page).getByRole('button', { name: 'Observed fleet', exact: true }).click();
  await enterLocus(page, 7);
  await expect(containmentPath(page).getByRole('button', { name: 'Locus 2 in process 41', exact: true })).toBeVisible();
  observer.snapshot.processes[0].loci.find(item => item.id === 7).parent = 4;
  await refresh(page);
  await expect(containmentPath(page).getByRole('button', { name: 'Locus 4 in process 41', exact: true })).toBeVisible();
  await expect(containmentPath(page).getByRole('button', { name: 'Locus 2 in process 41', exact: true })).toHaveCount(0);
});

test('Runtime containment scope survives samples and moves outward when the focused object departs', async ({ page, observer, host }) => {
  observer.snapshot = recursiveObservation();
  await page.goto(host.url);
  await connect(page);
  await enterLocus(page, 2);
  await refresh(page);
  await expect(containment(page)).toHaveAttribute('data-scope-locus', '2');
  observer.snapshot.processes[0].loci = observer.snapshot.processes[0].loci.filter(item => item.id !== 2);
  await refresh(page);
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'process');
  await expect(page.getByRole('status', { name: 'Runtime viewing scope', exact: true })).toContainText(/no longer|departed|not.*observed/i);
  await expect(containmentPath(page).getByRole('button', { name: 'Locus 2 in process 41', exact: true })).toHaveCount(0);
  observer.snapshot.processes.shift();
  observer.snapshot.edges = [];
  await refresh(page);
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'fleet');
  await expect(containmentPath(page).getByRole('button', { name: 'Process 41', exact: true })).toHaveCount(0);
});

test('Runtime resets local scope and selection after a reported restart and reconnect', async ({ page, observer, host }) => {
  observer.snapshot = recursiveObservation();
  await page.goto(host.url);
  await connect(page);
  await enterLocus(page, 1);
  await containment(page).getByRole('button', { name: 'Inspect locus 2', exact: true }).click();
  observer.snapshot.processes[0].restarts = 1;
  await refresh(page);
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'fleet');
  await expect(page.getByRole('complementary', { name: 'Runtime inspector', exact: true })).not.toContainText('Dispatch');
  await expect(page.getByRole('status', { name: 'Runtime viewing scope', exact: true })).toContainText(/restart/i);
  await enterLocus(page, 1);
  observer.status = 503;
  await refresh(page);
  await expect(status(page)).toContainText(/unavailable|failed|could not/i);
  await expect(containmentPath(page)).toHaveCount(0);
  observer.status = 200;
  await connect(page);
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'fleet');
});

test('Runtime focused large child sets remain bounded and reveal all immediate contents', async ({ page, observer, host }) => {
  observer.snapshot = recursiveObservation();
  const root = observer.snapshot.processes[0].loci[0];
  const children = Array.from({ length: 90 }, (_, index) => ({ id: index + 2, type: `Worker ${index + 2}`, parent: 1, pub: 0, dlv: 0 }));
  observer.snapshot.processes[0].loci = [root, ...children, { id: 150, type: 'Nested', parent: 2, pub: 0, dlv: 0 }];
  await page.goto(host.url);
  await connect(page);
  await enterLocus(page, 1);
  const visible = containment(page).getByRole('list', { name: 'Immediate observed contents', exact: true }).getByRole('button', { name: /^Inspect locus \d+$/ });
  expect(await visible.count()).toBeGreaterThan(0);
  expect(await visible.count()).toBeLessThan(90);
  await expect(containment(page)).toContainText(/of 90/);
  await containment(page).getByRole('button', { name: /Show more/ }).click();
  await expectImmediateLoci(page, children.map(item => item.id));
  await refresh(page);
  await expectImmediateLoci(page, children.map(item => item.id));
});

test('Runtime recursive focus supports narrow keyboard navigation without poll-driven scrolling', async ({ page, observer, host }, testInfo) => {
  observer.snapshot = recursiveObservation();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(host.url);
  await connect(page);
  const enter = page.getByRole('button', { name: 'Enter process 41', exact: true });
  await enter.focus();
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'Enter locus 1 in process 41', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expectImmediateLoci(page, [2, 4]);
  const path = containmentPath(page);
  const box = await path.boundingBox();
  const navigation = await page.getByRole('banner', { name: 'Cockpit navigation', exact: true }).boundingBox();
  expect(box.y).toBeGreaterThanOrEqual(navigation.y + navigation.height);
  expect(box.y).toBeLessThan(844);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  const scroll = await page.evaluate(() => scrollY);
  const requests = observer.requests.length;
  await expect.poll(() => observer.requests.length).toBeGreaterThan(requests);
  expect(await page.evaluate(() => scrollY)).toBe(scroll);
  await page.screenshot({ path: testInfo.outputPath('runtime-recursive-mobile.png') });
  await path.getByRole('button', { name: 'Observed fleet', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(containment(page)).toHaveAttribute('data-scope-kind', 'fleet');
});
