// Tests curated operator data and the real graph/event/operator state machine.
// Rendering is stubbed here; use the README browser smoke checks for layout and interaction.
const fs = require('node:fs');
const path = require('node:path');
const readSource = name => fs.readFileSync(path.join(__dirname, '..', 'src', name), 'utf8');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const graphData = Object.assign({}, ...['meaning', 'runtime'].map(group => JSON.parse(readSource(`graph-data-${group}.json`))));
const activityData = Object.assign({}, ...['meaning', 'runtime'].map(group => JSON.parse(readSource(`event-data-${group}.json`))));
const operatorData = Object.assign({}, ...['meaning', 'runtime', 'work'].map(group => JSON.parse(readSource(`operator-${group}.json`))));
const production = ['graph-engine.js', 'event-engine.js', 'operator-engine.js'].map(name => readSource(name)).join('\n');
const normalize = value => JSON.parse(JSON.stringify(value));

function harness(part = 'nerves', savedState = {}) {
  let now = 0, seq = 0, saves = 0, renderCount = 0;
  const timers = new Map(), listeners = {}, elements = new Map();
  function element(selector) {
    if (!elements.has(selector)) elements.set(selector, {
      dataset: {}, textContent: '', innerHTML: '', disabled: false, hidden: false,
      querySelector(child) { return element(selector + ' ' + child); },
      querySelectorAll() { return []; }, setAttribute() {}, scrollIntoView() {},
    });
    return elements.get(selector);
  }
  const ctx = vm.createContext({
    graphData, activityData, operatorData, savedState,
    parts: Object.keys(graphData).map(p => [p,p]), currentPart: part,
    signedIn: true, activeView: 'organism', currentOrganism: 'Atlas',
    spatialHiddenDimensions: [], spatialRelated: false,
    root: { addEventListener(type, fn) { (listeners[type] ??= []).push(fn); } },
    document: { hidden: false, addEventListener() {} },
    window: { matchMedia() { return { matches: false, addEventListener() {} }; } },
    inspector: { open: false, showModal() { this.open = true; }, close() { this.open = false; } },
    inspectorContent: { innerHTML: '' },
    partContent: {
      querySelector(selector) {
        return ['.activity-panel', '.operator-activity', '.graph-inspection'].includes(selector) ? element(selector) : null;
      },
      querySelectorAll() { return []; },
    },
    performance: { now: () => now },
    setTimeout(fn, delay) { const id=++seq;timers.set(id,{fn,at:now+delay});return id; },
    clearTimeout(id) { timers.delete(id); },
    requestAnimationFrame() { return ++seq; }, cancelAnimationFrame() {},
    getSpatialView: view => view,
    save() { saves++; },
    render() { renderCount++;ctx.renderGraphWorkspace(); },
    TextEncoder, console,
  });
  vm.runInContext(production, ctx);
  // Keep actual view selection, filtering, event lifecycle and operator actions.
  // Neither real function below is relevant to DOM-free lifecycle assertions.
  vm.runInContext('renderGraphWorkspace = function() { ensureActivityScope(); renderActivityPanel(); };', ctx);
  const run = code => vm.runInContext(code, ctx);
  function snapshot() {
    return normalize(run('({part:currentPart,active:operatorIsActive(),view:currentGraphView().id,views:operatorViews,selection:graphSelection,focus:graphFocus,filters:graphFilters,dimensions:spatialHiddenDimensions,held:activityHeld,playing:activityPlaying,auto:activityAutoTimer,scope:activityScope,log:activityLog,event:currentActivityEvent()?.id||null})'));
  }
  function tick(delta) {
    const end=now+delta;
    for(let count=0;;count++) {
      assert(count<1000,'runaway timer');
      const next=[...timers].filter(([,timer])=>timer.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];
      if(!next)break;
      now=next[1].at;timers.delete(next[0]);next[1].fn();
    }
    now=end;
  }
  function click(attrs) {
    const dataset=Object.fromEntries(Object.entries(attrs).filter(([name])=>name.startsWith('data-')).map(([name,value])=>[name.slice(5).replace(/-([a-z])/g,(_,c)=>c.toUpperCase()),value]));
    const button={dataset,hasAttribute:name=>Object.hasOwn(attrs,name)};
    for(const fn of listeners.click || [])fn({target:{closest:()=>button}});
  }
  return {ctx,run,snapshot,tick,click,enter(){ctx.renderGraphWorkspace();},get saves(){return saves;},get renderCount(){return renderCount;}};
}

const cases=[];
const test=(name,fn)=>cases.push([name,fn]);
const viewFor=(part,id)=>graphData[part]?.views.find(view=>view.id===id);

test('all 16 authored operator pages have valid, bounded identities and evidence targets',()=>{
  assert.equal(Object.keys(operatorData).length,16);
  assert.deepEqual(Object.keys(operatorData).sort(),Object.keys(graphData).sort());
  for(const [part,page] of Object.entries(operatorData)) {
    assert(['steady','attention','limited'].includes(page.tone),part);
    assert.equal(page.checks.length,3,part);assert(page.attention.length<=2,part);
    for(const field of ['question','headline','summary'])assert.equal(typeof page[field],'string');
    const view=viewFor(part,page.focus.view),nodes=new Set(view.nodes.map(node=>node.id));
    assert(page.focus.nodes.length>=3 && page.focus.nodes.length<=6,part);
    assert.equal(new Set(page.focus.nodes).size,page.focus.nodes.length,part);
    page.focus.nodes.forEach(id=>assert(nodes.has(id),`${part}: ${id}`));
    for(const id of page.focus.edges) {
      const edge=view.edges.find(e=>[e.from,e.label,e.to].join('::')===id);
      assert(edge,`${part}: ${id}`);
      assert(page.focus.nodes.includes(edge.from) && page.focus.nodes.includes(edge.to),`${part}: endpoints`);
    }
    for(const row of [...page.checks,...page.attention]) {
      assert(viewFor(row.target.part,row.target.view)?.nodes.some(node=>node.id===row.target.node),`${part}: target ${JSON.stringify(row.target)}`);
    }
  }
});

test('normal pending, held evidence and historical failures do not become generic alarms',()=>{
  assert.equal(operatorData.head.attention.length,0,'a recorded request awaiting admission is normal progress');
  assert.equal(operatorData.memory.attention.length,0,'an investigation hold is not a missing-evidence alarm');
  assert.equal(operatorData.heart.attention.length,0,'retained expression is not a current failure');
  assert.equal(operatorData.body.attention.length,0,'current Up reports are respected');
  assert.equal(operatorData.hat.attention.length,0);
  assert.equal(operatorData.voice.attention.length,0,'an unestablished debug replay is not a work blocker');
  assert(!operatorData.spine.attention.some(row=>['outcome-t142','settlement-t142','schedule-month-close'].includes(row.target.node)));
  assert(operatorData.reflexes.attention.every(row=>row.target.node==='concern-reflex-api0'),'historical outage is surfaced through the recorded concern');
  assert(operatorData.reflexes.checks.some(row=>row.target.part==='body' && /Up/.test(row.value)),'later Up receipt stays visible');
  assert(operatorData.nerves.summary.includes('acknowledgement') && operatorData.nerves.summary.includes('not landed'),'transport receipt does not become answer');
});

test('Overview uses exact authored graph objects and never synthesizes shortcut edges',()=>{
  const h=harness();
  for(const part of Object.keys(operatorData)) {
    h.ctx.currentPart=part;
    const graph=h.ctx.operatorGraph(),focus=operatorData[part].focus;
    assert.deepEqual(normalize(graph.nodes.map(node=>node.id)),viewFor(part,focus.view).nodes.filter(node=>focus.nodes.includes(node.id)).map(node=>node.id));
    assert.equal(graph.edges.length,focus.edges.length);
    graph.edges.forEach(edge=>assert(viewFor(part,focus.view).edges.includes(edge),'retains source object and evidence basis'));
    assert.equal(h.ctx.visibleGraph().view,graph.view);
    assert.equal(h.ctx.currentGraphView().id,focus.view);
  }
});

test('mode state accepts only known per-part Explore choices and round-trips through persistence',()=>{
  const h=harness('nerves',{operatorViews:{nerves:'explore',spine:'overview',fake:'explore',hat:'typo',voice:true}});
  assert.deepEqual(h.snapshot().views,{nerves:'explore'});
  assert.equal(h.snapshot().active,false);
  for(const value of [null,undefined,0,'abc',[]])assert.deepEqual(normalize(h.ctx.validatedOperatorViews(value)),{});
  h.click({'data-page-depth':'overview'});
  assert.equal(h.snapshot().active,true);assert.equal(h.snapshot().view,operatorData.nerves.focus.view);
  h.click({'data-page-depth':'explore'});
  assert.equal(h.snapshot().active,false);assert(h.saves>=2);
  const restored=harness('nerves',{operatorViews:h.snapshot().views,graphPerspectives:{nerves:operatorData.nerves.focus.view}});
  assert.equal(restored.snapshot().active,false);assert.equal(restored.snapshot().view,operatorData.nerves.focus.view);
  restored.ctx.currentPart='hat';assert.equal(restored.snapshot().active,true,'unvisited page defaults to Overview');
  const shell=readSource('shell.html');
  assert(shell.includes('activityCursors, activityLog, operatorViews }'),'saved private state includes mode choices');
  assert(shell.includes('operatorViews = validatedOperatorViews(saved.privateContent?.operatorViews)'),'restored mode choices are validated');
});

test('all 16 Overview entries start an event from their curated perspective',()=>{
  for(const part of Object.keys(operatorData)) {
    const h=harness(part);h.enter();assert(h.snapshot().auto,part);
    h.tick(400);const state=h.snapshot();
    assert.equal(state.log.length,1,part);assert(state.playing,part);
    const event=activityData[part].events.find(event=>event.id===state.event);
    assert.equal(event.view,operatorData[part].focus.view,part);
    assert(event.entityIds.some(id=>operatorData[part].focus.nodes.includes(id)) || event.edgeIds.some(id=>operatorData[part].focus.edges.includes(id)),part);
    h.tick(10000);assert(!h.snapshot().playing,part+' finishes its finite scenario');
  }
});

test('opening evidence preserves context, cancels queued activity and cannot send an operation',()=>{
  const h=harness('spine');h.enter();
  const target=operatorData.spine.attention[0].target,before=h.snapshot();
  h.ctx.openOperatorEvidence(target);
  assert(h.ctx.inspector.open);assert(h.snapshot().held);assert.equal(h.snapshot().auto,0);
  assert.equal(h.snapshot().part,before.part);assert.equal(h.snapshot().view,before.view);assert.deepEqual(h.snapshot().selection,before.selection);
  assert(h.ctx.inspectorContent.innerHTML.includes('Sample evidence'));
  assert(h.ctx.inspectorContent.innerHTML.includes('data-operator-explore'));
  h.tick(20000);assert.equal(h.snapshot().log.length,0);
  h.ctx.inspector.close();h.ctx.queueActivitySimulation('Selection',{kind:'node',id:'attempt-a0'});h.tick(10000);
  assert.equal(h.snapshot().log.length,0,'closing evidence keeps inspection held');
});

test('evidence Explore crosses parts into the exact neighborhood and stays held',()=>{
  const h=harness('spine');h.enter();h.tick(400);
  const target=operatorData.spine.attention[0].target;
  h.ctx.openOperatorEvidence(target);
  h.run('graphFilters.legs={query:"invisible"};spatialHiddenDimensions=["work"];');
  h.ctx.exploreOperatorTarget(target);
  const state=h.snapshot();assert.equal(state.part,target.part);assert.equal(state.view,target.view);
  assert.equal(state.active,false);assert.deepEqual(state.selection,{kind:'node',id:target.node});assert.equal(state.focus,target.node);
  assert.deepEqual(state.filters.legs,{});assert.deepEqual(state.dimensions,[]);
  assert.equal(h.ctx.inspector.open,false);assert(state.held);assert.equal(state.auto,0);assert.equal(h.renderCount,1);
  h.tick(10000);assert.equal(h.snapshot().log.length,1,'navigation did not replay or mutate evidence');
});

test('invalid evidence targets cannot move the user or stop a valid scenario',()=>{
  const h=harness('hat');h.enter();const before=h.snapshot();
  h.ctx.openOperatorEvidence({part:'hat',view:'missing',node:'hat'});
  assert.deepEqual(h.snapshot(),before);assert(!h.ctx.inspector.open);
});

test('Explore identity navigation opens the destination view instead of its curated Overview',()=>{
  const h=harness('spine',{operatorViews:{spine:'explore'}});h.enter();h.tick(400);
  assert.equal(h.snapshot().active,false);
  // Legs defaults to its uncertainty overview, while this identity belongs to
  // the separate claim view. A mode omission would hide the requested entity.
  assert.equal(operatorData.legs.focus.view,'uncertainty');
  h.click({'data-graph-reveal':'legs','data-entity':'attempt-a0'});
  const state=h.snapshot();
  assert.equal(state.part,'legs');assert.equal(state.active,false);
  assert.equal(state.views.legs,'explore');assert.equal(state.view,'claim');
  assert.deepEqual(state.selection,{kind:'node',id:'attempt-a0'});
  assert(h.ctx.visibleGraph().nodes.some(node=>node.id==='attempt-a0'));
  assert.equal(h.renderCount,1);assert(h.saves>0);
});

test('Overview activity targets replace the event dialog with visible held evidence',()=>{
  const h=harness('spine');h.enter();h.tick(400);
  const event=activityData.spine.events.find(item=>item.id===h.snapshot().event);
  // The definition participates in the event but sits outside the compact map.
  // Its exact evidence must still be reachable from the event dialog.
  const target=event.entityIds.find(id=>!operatorData.spine.focus.nodes.includes(id)) || event.entityIds[0];
  const node=viewFor('spine',event.view).nodes.find(node=>node.id===target);
  h.click({'data-activity-inspect':''});assert(h.ctx.inspector.open);
  assert(h.ctx.inspectorContent.innerHTML.includes('activity-event-detail'));
  h.click({'data-activity-target':target});
  const state=h.snapshot();
  assert(h.ctx.inspector.open,'evidence remains visible');
  assert(h.ctx.inspectorContent.innerHTML.includes('operator-evidence'));
  assert(h.ctx.inspectorContent.innerHTML.includes(node.label));
  assert(h.ctx.inspectorContent.innerHTML.includes(`data-operator-node="${target}"`));
  assert.equal(state.part,'spine');assert.equal(state.active,true);
  assert(state.held);assert.equal(state.auto,0);assert.equal(state.playing,false);
  h.tick(10000);assert.equal(h.snapshot().log.length,1,'inspection neither advances nor duplicates the replay');
});

let failures=0;
for(const [name,fn] of cases) {
  try {fn();console.log('PASS '+name);}catch(error){failures++;console.error('FAIL '+name+'\n'+error.stack);}
}
console.log(`${cases.length-failures}/${cases.length} operator overview checks passed.`);
if(failures)process.exitCode=1;
