const fs = require('node:fs');
const path = require('node:path');
const readSource = name => fs.readFileSync(path.join(__dirname, '..', 'src', name), 'utf8');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const graphData = Object.assign({}, ...['meaning','runtime'].map(group=>JSON.parse(readSource(`graph-data-${group}.json`))));
const activityData = Object.assign({}, ...['meaning','runtime'].map(group=>JSON.parse(readSource(`event-data-${group}.json`))));
let clock = 0;
const motion = {matches:false,addEventListener(){}};
const root = {addEventListener(){}};
const sceneLayer = {dataset:{},innerHTML:'',replaceChildren(){this.innerHTML='';}};
const svg = {querySelector(){return sceneLayer;}};
const canvas = {isConnected:true,querySelector(){return svg;}};
const ctx = vm.createContext({
  activityData,graphData,savedState:{},currentPart:'nerves',root,
  currentGraphView(){return graphData[ctx.currentPart].views[0];},
  window:{matchMedia(){return motion;}},document:{hidden:false,addEventListener(){}},
  performance:{now(){return clock;}},
  requestAnimationFrame(){return 1;},cancelAnimationFrame(){},clearTimeout(){},setTimeout(){return 1;},
  console,Map,Set,Date,Math,JSON,Number,String,Array,Object,
  esc:value=>String(value),signedIn:true,activeView:'organism',currentOrganism:'Atlas'
});
vm.runInContext(readSource('event-engine.js'),ctx);
const allEvents=Object.values(activityData).flatMap(page=>page.events);
assert.equal(allEvents.length,96);
for(const [part,page] of Object.entries(graphData)) for(const view of page.views) {
  const events=activityData[part].events.filter(event=>event.view===view.id);
  assert.equal(events.length,3);
  const nodes=new Set(view.nodes.map(node=>node.id)),edges=new Set(view.edges.map(edge=>[edge.from,edge.label,edge.to].join('::')));
  for(const event of events) {
    assert(event.entityIds.every(id=>nodes.has(id)));
    assert(event.edgeIds.every(id=>edges.has(id)));
    if(event.effect==='flow') assert.equal(event.source,'runtime');
  }
}
ctx.canvas=canvas;
const flow=activityData.nerves.events[0],edge=graphData.nerves.views[0].edges.find(edge=>flow.edgeIds.includes([edge.from,edge.label,edge.to].join('::')));
ctx.route={edge:{...edge,id:flow.edgeIds[0],dimension:'signal'},route:[{x:0,y:0},{x:0,y:60},{x:80,y:60}]};
ctx.points=new Map(flow.entityIds.map((id,index)=>[id,{x:index*80,y:index*60}]));
vm.runInContext('activityCursors[activityKey()]=0; activityStarted=0; setActivityScene(canvas,points,[route]);',ctx);
clock=600;vm.runInContext('drawActivityOverlay()',ctx);const early=sceneLayer.innerHTML;
clock=1800;vm.runInContext('drawActivityOverlay()',ctx);const late=sceneLayer.innerHTML;
assert.notEqual(early,late,'message pulse must change coordinates with time');
assert(early.includes('activity-packet') && late.includes('activity-packet'));
vm.runInContext('activityElapsed=activityProgress()*2400; activityPaused=true;',ctx);
clock=60000;vm.runInContext('drawActivityOverlay()',ctx);
assert.equal(sceneLayer.innerHTML,late,'pause freezes the packet in place');
motion.matches=true;vm.runInContext('drawActivityOverlay()',ctx);
assert(!sceneLayer.innerHTML.includes('activity-packet'),'reduced motion removes traveling packets');
assert(sceneLayer.innerHTML.includes('activity-node-ring'),'reduced motion retains event emphasis');

// Recorded events trace their authored relationships without claiming message traffic.
const reflex=activityData.reflexes.events[0];
const reflexEdge=graphData.reflexes.views[0].edges.find(edge=>[edge.from,edge.label,edge.to].join('::')===reflex.edgeIds[0]);
ctx.currentPart='reflexes';
ctx.route={edge:{...reflexEdge,id:reflex.edgeIds[0],dimension:'work'},route:[{x:0,y:0},{x:0,y:60},{x:80,y:60}]};
ctx.points=new Map(reflex.entityIds.map((id,index)=>[id,{x:index*80,y:index*60}]));
motion.matches=false;clock=0;
vm.runInContext('activityCursors[activityKey()]=0; activityStarted=0; activityPaused=false; setActivityScene(canvas,points,[route]);',ctx);
function tracePosition(markup) {
  const match=markup.match(/class="activity-trace-pulse"[^>]*data-x="([^"]+)" data-y="([^"]+)"/);
  assert(match,'recorded event must have a traveling relationship trace');
  return match.slice(1).map(Number);
}
clock=600;vm.runInContext('drawActivityOverlay()',ctx);const traceEarly=sceneLayer.innerHTML;
clock=1800;vm.runInContext('drawActivityOverlay()',ctx);const traceLate=sceneLayer.innerHTML;
assert.deepEqual(tracePosition(traceEarly),[0,35],'early trace follows the first segment of the actual route');
assert.deepEqual(tracePosition(traceLate),[45,60],'later trace turns onto the second route segment');
assert(traceEarly.includes('activity-travel-tail') && traceLate.includes('activity-travel-tail'),'traces include visible directional tails');
assert(!traceEarly.includes('activity-packet') && !traceLate.includes('activity-packet'),'recorded event must not fabricate a message packet');
vm.runInContext('activityElapsed=activityProgress()*2400; activityPaused=true;',ctx);
clock=60000;vm.runInContext('drawActivityOverlay()',ctx);
assert.equal(sceneLayer.innerHTML,traceLate,'pause freezes the relationship trace in place');
vm.runInContext('activityPaused=false;',ctx);clock=2400;vm.runInContext('drawActivityOverlay()',ctx);
assert(!sceneLayer.innerHTML.includes('activity-trace-pulse') && !sceneLayer.innerHTML.includes('activity-travel-tail'),'completed trace removes its moving marker and tail');
assert(sceneLayer.innerHTML.includes('activity-route') && sceneLayer.innerHTML.includes('activity-node-ring'),'completed trace retains the affected relationships and entities');
motion.matches=true;clock=600;vm.runInContext('drawActivityOverlay()',ctx);
assert(!sceneLayer.innerHTML.includes('activity-trace-pulse') && !sceneLayer.innerHTML.includes('activity-packet') && !sceneLayer.innerHTML.includes('activity-travel-tail'),'reduced motion removes every traveling trace');
assert(sceneLayer.innerHTML.includes('activity-route') && sceneLayer.innerHTML.includes('activity-node-ring'),'reduced motion retains static relationship and entity emphasis');

// A node-only event must not borrow a neighboring route to invent edge activity.
const nodeOnly=activityData.heart.events.find(event=>event.view===graphData.heart.views[0].id && !event.edgeIds.length);
assert(nodeOnly,'Heart supplies a real node-only fixture');
ctx.currentPart='heart';
ctx.points=new Map(nodeOnly.entityIds.map((id,index)=>[id,{x:index*80,y:index*60}]));
ctx.nodeOnlyIndex=activityData.heart.events.filter(event=>event.view===graphData.heart.views[0].id).indexOf(nodeOnly);
motion.matches=false;clock=600;
vm.runInContext('activityCursors[activityKey()]=nodeOnlyIndex; activityStarted=0; activityPaused=false; setActivityScene(canvas,points,[route]);',ctx);
assert(!sceneLayer.innerHTML.includes('activity-trace-pulse') && !sceneLayer.innerHTML.includes('activity-packet') && !sceneLayer.innerHTML.includes('activity-route'),'node-only events do not animate unrelated edges');
assert(sceneLayer.innerHTML.includes('activity-node-ring'),'node-only event still identifies affected entities');

ctx.incoming=Array.from({length:100},(_,index)=>({key:index+1,eventId:flow.id,seenAt:'10:22:33'}));
assert.equal(vm.runInContext('validatedActivityLog(incoming).length',ctx),32);
assert.equal(vm.runInContext("Object.keys(validatedActivityCursors({'nerves:routes':99})).length",ctx),0);
const longest=allEvents.reduce((a,b)=>a.id.length>b.id.length?a:b);
const budget={signedIn:true,theme:'midnight',activeView:'organism',currentOrganism:'Atlas',currentPart:'nerves',previewVersion:3,
  graphSelection:{kind:'edge',id:'x'.repeat(250)},graphPerspectives:{},graphMode:'graph',graphFilters:{},
  spatialProjection:'3d',spatialRelated:true,spatialHiddenDimensions:['structure','authority','work','signal','evidence'],
  activityCursors:{},activityLog:Array.from({length:32},(_,index)=>({key:index+1,eventId:longest.id,seenAt:'10:22:33'}))};
for(const [part,page] of Object.entries(graphData)) {
  budget.graphPerspectives[part]=page.views.reduce((a,b)=>a.id.length>b.id.length?a:b).id;
  const nodes=page.views.flatMap(view=>view.nodes);
  budget.graphFilters[part]={query:'界'.repeat(120),kind:nodes.reduce((a,b)=>a.kind.length>b.kind.length?a:b).kind,source:nodes.reduce((a,b)=>a.id.length>b.id.length?a:b).id,state:'attention',relation:'observed'};
  for(const view of page.views)budget.activityCursors[part+':'+view.id]=2;
}
const bytes=Buffer.byteLength(JSON.stringify({modelContent:{activityEvent:longest.id},privateContent:budget}));
assert(bytes<16384,`saved state ${bytes} bytes exceeds limit`);
console.log(`96 events / 32 perspectives valid; moving packets and relationship traces, pause, completion, node-only events, reduced motion, bounded restoration pass; conservative saved state ${bytes} bytes.`);
