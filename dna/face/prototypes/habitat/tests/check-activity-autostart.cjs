// Deterministic lifecycle checks for the authored UI simulations. No browser or
// timers outside this VM are used; the production event engine stays unmodified.
const fs = require('node:fs');
const path = require('node:path');
const readSource = name => fs.readFileSync(path.join(__dirname, '..', 'src', name), 'utf8');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const graphData = Object.assign({}, ...['meaning', 'runtime'].map(group => JSON.parse(readSource(`graph-data-${group}.json`))));
const activityData = Object.assign({}, ...['meaning', 'runtime'].map(group => JSON.parse(readSource(`event-data-${group}.json`))));
const engine = readSource('event-engine.js');

function harness(part = 'nerves') {
  let now = 0, sequence = 0, saves = 0;
  const timers = new Map(), rootListeners = {}, documentListeners = {};
  const elements = new Map();
  function element(selector) {
    if (!elements.has(selector)) elements.set(selector, {
      dataset: {}, textContent: '', innerHTML: '', disabled: false, hidden: false,
      querySelector(child) { return element(selector + ' ' + child); },
      querySelectorAll() { return []; }, setAttribute() {}, scrollIntoView() {},
    });
    return elements.get(selector);
  }
  const panel = element('.activity-panel');
  const ctx = vm.createContext({
    activityData, graphData, savedState: {}, currentPart: part,
    signedIn: true, activeView: 'organism', currentOrganism: 'Atlas',
    graphPerspectives: {}, graphFilters: {}, graphSelection: null, graphFocus: null,
    spatialHiddenDimensions: [],
    root: { addEventListener(type, callback) { (rootListeners[type] ??= []).push(callback); } },
    document: { hidden: false, addEventListener(type, callback) { (documentListeners[type] ??= []).push(callback); } },
    window: { matchMedia() { return { matches: false, addEventListener() {} }; } },
    performance: { now: () => now },
    requestAnimationFrame() { return ++sequence; }, cancelAnimationFrame() {},
    setTimeout(callback, delay) { const id = ++sequence; timers.set(id, { callback, at: now + delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    partContent: { querySelector(selector) { return selector === '.activity-panel' ? panel : null; }, querySelectorAll() { return []; } },
    inspector: { open: false, showModal() { this.open = true; }, close() { this.open = false; } },
    inspectorContent: { innerHTML: '' },
    graphEntities: new Map(Object.values(graphData).flatMap(page => page.views.flatMap(view => view.nodes.map(node => [node.id, node])))),
    titleForPart: value => value, esc: value => String(value),
    buttonHTML: (label, attributes) => `<button ${attributes}>${label}</button>`,
    save() { saves++; },
    currentGraphView() { return graphData[ctx.currentPart].views.find(view => view.id === ctx.graphPerspectives[ctx.currentPart]) || graphData[ctx.currentPart].views[0]; },
    visibleGraph() {
      if (ctx.visibleOverride) return ctx.visibleOverride;
      const view = ctx.currentGraphView();
      return { nodes: view.nodes, edges: view.edges.map(edge => ({ ...edge, id: [edge.from, edge.label, edge.to].join('::') })), primary: null };
    },
    renderGraphWorkspace() { ctx.ensureActivityScope(); ctx.renderActivityPanel(); },
    console, Map, Set, Date, Math, JSON, Number, String, Array, Object,
  });
  vm.runInContext(engine, ctx);
  function run(code) { return vm.runInContext(code, ctx); }
  function snapshot() {
    return JSON.parse(run('JSON.stringify({scope:activityScope,playing:activityPlaying,held:activityHeld,paused:activityPaused,auto:activityAutoTimer,timer:activityTimer,cursor:activityCursors[activityKey()] ?? -1,event:currentActivityEvent()?.id ?? null,log:activityLog,started:activityStarted,trigger:activityTrigger})'));
  }
  function tick(duration) {
    const until = now + duration;
    for (let count = 0; ; count++) {
      assert(count < 1000, 'runaway simulation timer');
      const next = [...timers].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
      if (!next) break;
      now = next[1].at; timers.delete(next[0]); next[1].callback();
    }
    now = until;
  }
  function enter(nextPart = ctx.currentPart, view) {
    ctx.currentPart = nextPart; ctx.activeView = 'organism'; ctx.currentOrganism = 'Atlas';
    if (view) ctx.graphPerspectives[nextPart] = view;
    ctx.renderGraphWorkspace();
  }
  function click(attribute, value = '') {
    const name = attribute.replace(/^data-/, '').replace(/-([a-z])/g, (_, char) => char.toUpperCase());
    const button = { dataset: { [name]: value }, hasAttribute: candidate => candidate === attribute };
    for (const callback of rootListeners.click || []) callback({ target: { closest: () => button } });
  }
  return { ctx, run, snapshot, tick, enter, click, timers, panel,
    get saves() { return saves; },
    hide() { ctx.document.hidden = true; (documentListeners.visibilitychange || []).forEach(callback => callback()); },
    show() { ctx.document.hidden = false; (documentListeners.visibilitychange || []).forEach(callback => callback()); },
  };
}

const cases = [];
function test(name, callback) { cases.push([name, callback]); }

test('all 16 pages begin one authored sample after entering', () => {
  for (const part of Object.keys(graphData)) {
    const h = harness(part); h.enter();
    assert(h.snapshot().auto); assert.equal(h.snapshot().log.length, 0);
    h.tick(399); assert.equal(h.snapshot().log.length, 0);
    h.tick(1); assert.equal(h.snapshot().log.length, 1); assert.equal(h.snapshot().playing, true);
    assert.equal(h.snapshot().event, activityData[part].events.find(event => event.view === graphData[part].views[0].id).id);
  }
});

test('rapid page navigation cancels the old pending start', () => {
  const h = harness(); h.enter(); h.tick(200); h.enter('heart'); h.tick(399);
  assert.equal(h.snapshot().log.length, 0);
  h.tick(1); assert.equal(h.snapshot().log.length, 1);
  assert(h.snapshot().event.startsWith('heart'));
  h.tick(3000); assert.equal(h.snapshot().log.length, 2);
  assert(h.snapshot().log.every(entry => activityData.heart.events.some(event => event.id === entry.eventId)));
});

test('perspective changes replace the old sequence exactly once', () => {
  const h = harness(); h.enter(); h.tick(400);
  const view = graphData.nerves.views[1]; h.enter('nerves', view.id);
  assert.equal(h.snapshot().cursor, -1); h.tick(400);
  assert.equal(h.snapshot().event, activityData.nerves.events.find(event => event.view === view.id).id);
  const pending = h.snapshot().timer; h.ctx.renderGraphWorkspace(); h.ctx.renderGraphWorkspace();
  assert.equal(h.snapshot().timer, pending); h.tick(3000);
  assert.equal(h.snapshot().log.length, 3);
});

test('Pause cancels a pending start and holds against selections and filters', () => {
  const h = harness(); h.enter(); h.click('data-activity-play');
  assert.equal(h.snapshot().held, true); assert.equal(h.snapshot().auto, 0);
  h.ctx.queueActivitySimulation('Selection', { kind: 'node', id: activityData.nerves.events[0].entityIds[0] });
  h.ctx.queueActivitySimulation('Filtered view', null, 550); h.tick(10000);
  assert.equal(h.snapshot().log.length, 0); assert.equal(h.snapshot().playing, false);
});

test('Pause freezes progress and Resume continues without duplicating the event', () => {
  const h = harness(); h.enter(); h.tick(1000); h.click('data-activity-play');
  const frozen = h.run('activityProgress()'), count = h.snapshot().log.length;
  h.tick(5000); assert.equal(h.run('activityProgress()'), frozen);
  h.click('data-activity-play'); assert.equal(h.snapshot().held, false);
  assert.equal(h.snapshot().log.length, count); h.tick(3000);
  assert.equal(h.snapshot().log.length, count + 1);
});

test('Reset, Next, and Seek all cancel pending starts and keep automatic triggers held', () => {
  for (const [attribute, value] of [['data-activity-reset', ''], ['data-activity-next', ''], ['data-activity-seek', '1']]) {
    const h = harness(); h.enter(); h.click(attribute, value);
    const event = h.snapshot().event, count = h.snapshot().log.length;
    assert.equal(h.snapshot().held, true); assert.equal(h.snapshot().auto, 0);
    h.ctx.queueActivitySimulation('Filtered view', null, 550); h.tick(10000);
    assert.equal(h.snapshot().event, event); assert.equal(h.snapshot().log.length, count);
  }
});

test('same-page reentry restarts the finite scenario after leaving', () => {
  const h = harness(); h.enter(); h.tick(9400); assert.equal(h.snapshot().playing, false);
  h.ctx.activeView = 'Account'; h.ctx.leaveActivityScope(); h.tick(2000);
  assert.equal(h.snapshot().scope, '');
  h.enter(); assert(h.snapshot().auto); h.tick(400);
  assert.equal(h.snapshot().cursor, 0); assert.equal(h.snapshot().playing, true);
  assert.equal(h.snapshot().log.length, 4);
});

test('state echoes preserve the existing timer and pulse clock', () => {
  const h = harness(); h.enter(); h.tick(1100);
  const before = h.snapshot();
  h.run('restoreActivityState({activityCursors: {...activityCursors}, activityLog: activityLog.map(entry=>({...entry}))})');
  h.ctx.renderGraphWorkspace();
  const after = h.snapshot();
  assert.equal(after.timer, before.timer); assert.equal(after.started, before.started);
  assert.equal(after.playing, true); assert.equal(after.auto, 0);
  h.tick(2300); assert.equal(h.snapshot().log.length, 2);
});

test('filter typing is debounced and only the final matching sample starts', () => {
  const h = harness(); h.enter();
  h.ctx.queueActivitySimulation('Filtered view', null, 550); h.tick(100);
  h.ctx.queueActivitySimulation('Filtered view', null, 550); h.tick(100);
  const event = activityData.nerves.events.find(item => item.view === graphData.nerves.views[0].id && item.entityIds.some(id => !activityData.nerves.events[0].entityIds.includes(id)));
  const id = event.entityIds.find(id => !activityData.nerves.events[0].entityIds.includes(id));
  h.ctx.visibleOverride = { nodes: [{id}], edges: [], primary: new Set([id]) };
  const expected = h.ctx.matchingActivityIndex(null);
  h.ctx.queueActivitySimulation('Filtered view', null, 550);
  h.tick(549); assert.equal(h.snapshot().log.length, 0);
  h.tick(1); assert.equal(h.snapshot().log.length, 1); assert.equal(h.snapshot().cursor, expected);
});

test('empty filtering cancels playback without recording an invisible event', () => {
  const h = harness(); h.enter(); h.ctx.visibleOverride = { nodes: [], edges: [], primary: new Set() };
  h.ctx.queueActivitySimulation('Filtered view', null, 550); h.tick(10000);
  assert.equal(h.snapshot().auto, 0); assert.equal(h.snapshot().log.length, 0);
  assert.equal(h.snapshot().trigger, 'No matching sample');
});

test('cross-perspective log replay cancels scope autoplay and adds no log row', () => {
  const h = harness(); h.enter(); h.tick(400); const original = h.snapshot().log[0];
  h.enter('nerves', graphData.nerves.views[1].id); h.tick(400);
  h.click('data-activity-log-entry', String(original.key));
  assert.equal(h.snapshot().event, original.eventId); assert.equal(h.snapshot().held, true);
  assert.equal(h.snapshot().auto, 0); assert.equal(h.snapshot().log.length, 2);
  h.tick(10000); assert.equal(h.snapshot().event, original.eventId); assert.equal(h.snapshot().log.length, 2);
});

test('event inspection and document hiding cancel queued autostarts', () => {
  const h = harness(); h.enter(); h.tick(400);
  h.ctx.queueActivitySimulation('Selection', { kind: 'node', id: activityData.nerves.events[0].entityIds[0] });
  h.click('data-activity-inspect'); assert.equal(h.ctx.inspector.open, true);
  h.tick(10000); assert.equal(h.snapshot().log.length, 1);
  const other = harness(); other.enter(); other.hide(); other.tick(10000); other.show(); other.tick(10000);
  assert.equal(other.snapshot().held, true); assert.equal(other.snapshot().log.length, 0);
});

let failures = 0;
for (const [name, callback] of cases) {
  try { callback(); console.log('PASS ' + name); }
  catch (error) { failures++; console.error('FAIL ' + name + '\n' + error.stack); }
}
console.log(`${cases.length - failures}/${cases.length} deterministic activity lifecycle checks passed.`);
if (failures) process.exitCode = 1;
