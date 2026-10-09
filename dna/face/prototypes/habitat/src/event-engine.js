  // Presentation envelopes over authored event samples. No network transport or
  // backend event schema is implied. Replays never mutate the captured graph.
  const activitySources = { record:'Recorded change', runtime:'Runtime observation', telemetry:'Telemetry sample', head:'Head notification' };
  const ACTIVITY_LOG_LIMIT = 32;
  const activityIndex = new Map(Object.entries(activityData).flatMap(([part,page])=>page.events.map(event=>[event.id,{...event,part}])));
  let activityCursors = validatedActivityCursors(savedState?.activityCursors);
  let activityLog = validatedActivityLog(savedState?.activityLog);
  let activityLogSequence = Math.max(0,...activityLog.map(entry=>entry.key));
  let activityLogFocusKey = 0;
  const activityExpandedLogs = new Set();
  let activityScope = '';
  let activityPlaying = false;
  let activityTimer = 0;
  let activityAutoTimer = 0;
  let activityHeld = false;
  let activityTrigger = 'Page opened';
  let activityPersistRun = false;
  let activityAnimation = 0;
  let activityStarted = -Infinity;
  let activityElapsed = 3000;
  let activityPaused = false;
  let activityScene = null;
  const activityMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  function validatedActivityCursors(incoming) {
    const result = {};
    if (!incoming || typeof incoming !== 'object') return result;
    Object.entries(graphData).forEach(([part,page]) => page.views.forEach(view => {
      const key = part + ':' + view.id, count = (activityData[part]?.events || []).filter(event=>event.view===view.id).length;
      if (Number.isInteger(incoming[key]) && incoming[key]>=-1 && incoming[key]<count) result[key]=incoming[key];
    }));
    return result;
  }
  function validatedActivityLog(incoming) {
    if (!Array.isArray(incoming)) return [];
    const seen=new Set();
    return incoming.filter(entry=>{
      if (!entry || !Number.isSafeInteger(entry.key) || entry.key<=0 || seen.has(entry.key) || !activityIndex.has(entry.eventId) || !/^\d{2}:\d{2}:\d{2}$/.test(entry.seenAt)) return false;
      seen.add(entry.key);return true;
    }).slice(-ACTIVITY_LOG_LIMIT).map(({key,eventId,seenAt})=>({key,eventId,seenAt}));
  }
  function restoreActivityState(state) {
    const incoming = validatedActivityCursors(state?.activityCursors);
    // An echo of our own saved cursor must not restart or cancel playback.
    if (Array.from(new Set([...Object.keys(incoming),...Object.keys(activityCursors)])).some(key=>incoming[key]!==activityCursors[key])) {
      stopActivityPlayback(); activityCursors=incoming; activityElapsed=3000;activityStarted=-Infinity;activityPaused=false;
    }
    activityLog=validatedActivityLog(state?.activityLog);
    activityLogSequence=Math.max(activityLogSequence,...activityLog.map(entry=>entry.key));
  }
  function activityKey() { return currentPart + ':' + (currentGraphView()?.id || ''); }
  function activityEvents() { return (activityData[currentPart]?.events || []).filter(event=>event.view===currentGraphView()?.id); }
  function currentActivityEvent() { return activityEvents()[activityCursors[activityKey()] ?? -1] || null; }
  function activityIsVisible() { return signedIn && activeView==='organism' && currentOrganism==='Atlas' && !document.hidden && Boolean(partContent.querySelector('.activity-panel')); }
  function ensureActivityScope() {
    if (!activityIsVisible()) return;
    const key=activityKey();
    if (key!==activityScope) {
      stopActivityPlayback();activityScope=key;activityScene=null;activityStarted=-Infinity;activityElapsed=3000;activityPaused=false;
      activityHeld=false;activityCursors[key]=-1;
      queueActivitySimulation('Page opened',null,400,false);
    }
  }
  function leaveActivityScope() {
    stopActivityPlayback();activityScope='';activityScene=null;
  }
  function matchingActivityIndex(target) {
    const events=activityEvents(), visible=visibleGraph();
    if (!visible?.nodes.length) return -1;
    const nodes=new Set(visible.nodes.map(node=>node.id)), edges=new Set(visible.edges.map(edge=>edge.id));
    return events.findIndex(event=>{
      if (target) return target.kind==='edge' ? event.edgeIds.includes(target.id) && edges.has(target.id) : event.entityIds.includes(target.id) && nodes.has(target.id);
      return event.entityIds.some(id=>(visible.primary || nodes).has(id)) || event.edgeIds.some(id=>edges.has(id));
    });
  }
  function queueActivitySimulation(reason,target=null,delay=280,persist=true) {
    if (activityHeld || !activityIsVisible() || inspector.open) return;
    // A connected-context entity may have no event in this perspective. Do not
    // invent one or move to another perspective just to produce an animation.
    const index=matchingActivityIndex(target);
    if (index<0) {
      if (!target) {stopActivityPlayback();activityCursors[activityKey()]=-1;activityPaused=false;activityTrigger='No matching sample';renderActivityPanel();drawActivityOverlay();}
      return;
    }
    stopActivityPlayback();activityPaused=false;activityTrigger=reason;activityPersistRun=persist;
    const key=activityKey();
    activityAutoTimer=setTimeout(()=>{
      activityAutoTimer=0;
      if (key!==activityKey() || activityHeld || !activityIsVisible() || inspector.open) {renderActivityPanel();return;}
      activityPlaying=true;presentActivity(index);scheduleActivity();
      if (activityPersistRun) save();
    },delay);
  }
  function holdActivity() {
    activityElapsed=activityProgress()*2400;
    stopActivityPlayback();activityHeld=true;activityPaused=Boolean(currentActivityEvent());
  }
  function stopActivityPlayback() {
    clearTimeout(activityTimer); activityTimer=0;
    clearTimeout(activityAutoTimer); activityAutoTimer=0;
    activityPlaying=false;
    cancelAnimationFrame(activityAnimation);activityAnimation=0;
  }
  function activityTime(event) {
    const first=activityEvents()[0]?.at || 0;
    const seconds=Math.max(0,event.at-first);
    return '+'+Math.floor(seconds/60)+':'+String(seconds%60).padStart(2,'0');
  }
  function activityTraceLabel(event) {
    if (!event.edgeIds.length) return 'Node update';
    return event.effect==='flow'?'Message traffic':event.kind==='projection.refreshed'?'Read trace':'Event trace';
  }
  function renderActivityPanel() {
    const host=partContent.querySelector('.activity-panel');
    if (!host) return;
    ensureActivityScope();
    const events=activityEvents(), event=currentActivityEvent(), cursor=activityCursors[activityKey()] ?? -1;
    const finished=cursor===events.length-1 && cursor>=0;
    host.dataset.playing=String(activityPlaying);
    host.dataset.event=event?.id || '';
    const play=host.querySelector('[data-activity-play]');
    play.textContent=activityPlaying || activityAutoTimer?'Pause':activityPaused?'Resume':finished?'Replay sample':'Play sample';
    play.disabled=!events.length;
    host.querySelector('[data-activity-next]').disabled=finished || !events.length;
    host.querySelector('[data-activity-reset]').disabled=cursor<0;
    host.querySelector('.activity-play-state').textContent=(activityAutoTimer?'Starting':activityPlaying?'Playing':activityPaused || activityHeld?'Paused':finished?'Complete':'Ready')+' · '+Math.max(0,cursor+1)+' / '+events.length;
    host.querySelector('.activity-current-label').textContent=event?event.label:activityAutoTimer?'Starting this perspective’s sample…':activityTrigger==='No matching sample'?'No sample events match the visible entities.':'Explore this page or play its sample sequence.';
    host.querySelector('.activity-current-meta').textContent=event?activityTime(event)+' · '+(event.kind==='projection.refreshed'?'Derived read':activitySources[event.source])+' · '+activityTraceLabel(event)+' · '+activityTrigger:'';
    host.querySelector('[data-activity-inspect]').hidden=!event;
    host.querySelector('.activity-sequence-count').textContent='· '+events.length;
    const list=host.querySelector('.activity-event-list');
    if (list.dataset.scope!==activityScope) {
      list.dataset.scope=activityScope;
      list.innerHTML=events.map((item,index)=>buttonHTML(`<time>${activityTime(item)}</time><span>${esc(item.label)}</span><small>${esc(item.kind==='projection.refreshed'?'Derived read':activitySources[item.source])}</small>`,`data-activity-seek="${index}" aria-pressed="false"`,'activity-event')).join('');
    }
    list.querySelectorAll('[data-activity-seek]').forEach(button=>button.setAttribute('aria-pressed',String(Number(button.dataset.activitySeek)===cursor)));
    const visible=visibleGraph(), nodes=new Set(visible?.nodes.map(node=>node.id)), edges=new Set(visible?.edges.map(edge=>edge.id));
    const hiddenNodes=event?event.entityIds.filter(id=>!nodes.has(id)).length:0;
    const hiddenEdges=event?event.edgeIds.filter(id=>!edges.has(id)).length:0;
    const hidden=host.querySelector('.activity-hidden-targets');
    hidden.hidden=!(hiddenNodes||hiddenEdges);
    hidden.querySelector('span').textContent=[hiddenNodes?hiddenNodes+' '+(hiddenNodes===1?'entity':'entities'):'',hiddenEdges?hiddenEdges+' '+(hiddenEdges===1?'relationship':'relationships'):''].filter(Boolean).join(' and ')+' hidden by this view’s filters.';
    updateActivityOutline();
    renderActivityLog();
  }
  function recordActivityEvent(event) {
    const now=new Date(), two=value=>String(value).padStart(2,'0');
    activityLog.push({key:++activityLogSequence,eventId:event.id,seenAt:[now.getHours(),now.getMinutes(),now.getSeconds()].map(two).join(':')});
    activityLogFocusKey=activityLogSequence;
    activityLog=activityLog.slice(-ACTIVITY_LOG_LIMIT);
  }
  function renderActivityLog() {
    const host=partContent.querySelector('.activity-log');if(!host)return;
    const entries=activityLog.filter(entry=>activityIndex.get(entry.eventId)?.part===currentPart).slice(-8);
    const expanded=activityExpandedLogs.has(currentPart), shown=expanded?entries:entries.slice(-4);
    const attention=entries.filter(entry=>activityIndex.get(entry.eventId).effect==='attention').length;
    host.querySelector('.activity-log-summary').textContent='Sample replay · '+entries.length+' '+(entries.length===1?'event':'events')+(attention?' · '+attention+' need attention':'');
    host.querySelector('[data-activity-clear-log]').hidden=!entries.length;
    const more=host.querySelector('[data-activity-log-more]');more.hidden=entries.length<=4;more.textContent=expanded?'Show recent events':'Show '+(entries.length-4)+' earlier events';
    const feed=host.querySelector('.activity-log-feed');
    if (!shown.length) {
      if (!feed.querySelector('.activity-log-empty')) feed.innerHTML='<p class="activity-log-empty">This part’s simulated events appear here as its scenario runs.</p>';
      return;
    }
    feed.querySelector('.activity-log-empty')?.remove();
    const keys=new Set(shown.map(entry=>String(entry.key)));
    feed.querySelectorAll('[data-activity-log-entry]').forEach(row=>{if(!keys.has(row.dataset.activityLogEntry))row.remove();});
    shown.forEach((entry,index)=>{
      let row=feed.querySelector(`[data-activity-log-entry="${entry.key}"]`);
      if (!row) {
        const event=activityIndex.get(entry.eventId);
        row=document.createElement('button');row.type='button';row.className='activity-log-entry cursor-interaction';row.dataset.activityLogEntry=entry.key;row.dataset.effect=event.effect;
        row.setAttribute('aria-label','Highlight '+event.label);
        row.innerHTML=`<time>${entry.seenAt}</time><i class="activity-log-mark" aria-hidden="true"></i><span class="activity-log-message">${esc(event.label)}</span><small>${event.effect==='attention'?'Attention':event.effect==='flow'?'Traffic':event.kind==='projection.refreshed'?'Read refreshed':activitySources[event.source]}</small>`;
        feed.insertBefore(row,feed.children[index]||null);
      }
      const focused=activityLog.find(item=>item.key===activityLogFocusKey && item.eventId===currentActivityEvent()?.id) || [...entries].reverse().find(item=>item.eventId===currentActivityEvent()?.id);
      row.setAttribute('aria-pressed',String(entry.key===focused?.key));
    });
  }
  function setActivityScene(canvas,points,routed) {
    activityScene={canvas,points,routed,key:activityKey()};
    drawActivityOverlay();
  }
  function updateActivityOutline() {
    const event=currentActivityEvent();
    partContent.querySelectorAll('.graph-outline-entry').forEach(entry=>{
      const id=entry.querySelector('[data-graph-node]')?.dataset.graphNode;
      if (event?.entityIds.includes(id)) entry.dataset.activity=event.effect==='attention'?'attention':'true';
      else delete entry.dataset.activity;
    });
  }
  function activityProgress() {
    if (activityMotion.matches) return 1;
    const elapsed=activityPaused?activityElapsed:performance.now()-activityStarted;
    return Math.max(0,Math.min(1,elapsed/2400));
  }
  function pointAlongActivityRoute(route,progress) {
    const lengths=route.slice(1).map((p,i)=>Math.hypot(p.x-route[i].x,p.y-route[i].y));
    const total=lengths.reduce((sum,len)=>sum+len,0);
    let remaining=total*progress;
    for (let i=0;i<lengths.length;i++) {
      if (remaining<=lengths[i] || i===lengths.length-1) {
        const fraction=lengths[i]?remaining/lengths[i]:0;
        return {x:route[i].x+(route[i+1].x-route[i].x)*fraction,y:route[i].y+(route[i+1].y-route[i].y)*fraction};
      }
      remaining-=lengths[i];
    }
    return route[0];
  }
  function drawActivityOverlay() {
    if (!activityScene || activityScene.key!==activityKey() || !activityScene.canvas.isConnected) return;
    const {canvas,points,routed}=activityScene, svg=canvas.querySelector('.spatial-svg');
    if (!svg) return;
    let layer=svg.querySelector('.activity-overlay');
    if (!layer) {layer=document.createElementNS('http://www.w3.org/2000/svg','g');layer.setAttribute('class','activity-overlay');svg.append(layer);}
    const event=currentActivityEvent();
    if (!event) {layer.replaceChildren();return;}
    const progress=activityProgress(), marks=[];
    routed.filter(item=>event.edgeIds.includes(item.edge.id)).forEach(({edge,route})=>{
      const d=route.map((p,i)=>(i?'L':'M')+p.x.toFixed(2)+','+p.y.toFixed(2)).join(' ');
      const color=`--route:var(--route-${edge.dimension})`;
      marks.push(`<path class="activity-route" data-kind="${esc(edge.kind)}" style="${color};opacity:${.55+Math.sin(progress*Math.PI)*.3}" d="${d}"/>`);
      // Every authored relationship can be traced. A hollow diamond denotes an
      // event/read trace; only observed message traffic uses a solid packet.
      // Geometry and timing are presentation, never new transport evidence.
      if (!activityMotion.matches && progress<1) {
        const travelKind=event.effect==='flow'?'message':'trace';
        const routeLength=route.slice(1).reduce((sum,p,index)=>sum+Math.hypot(p.x-route[index].x,p.y-route[index].y),0);
        const tailFraction=Math.min(.3,52/Math.max(1,routeLength));
        const trail=Array.from({length:10},(_,index)=>pointAlongActivityRoute(route,Math.max(0,progress-tailFraction+tailFraction*index/9)));
        const trailPath=trail.map((p,index)=>(index?'L':'M')+p.x.toFixed(2)+','+p.y.toFixed(2)).join(' ');
        marks.push(`<path class="activity-travel-glow" style="${color}" d="${trailPath}"/><path class="activity-travel-tail" data-travel="${travelKind}" style="${color}" d="${trailPath}"/>`);
        const p=pointAlongActivityRoute(route,progress);
        marks.push(`<circle class="activity-pulse-glow" style="${color}" cx="${p.x}" cy="${p.y}" r="11"/>`);
        if (travelKind==='message') marks.push(`<circle class="activity-packet" data-edge="${esc(edge.id)}" style="${color}" cx="${p.x}" cy="${p.y}" r="5.5"/><circle class="activity-pulse-core" cx="${p.x}" cy="${p.y}" r="2.2"/>`);
        else marks.push(`<path class="activity-trace-pulse" data-edge="${esc(edge.id)}" data-x="${p.x}" data-y="${p.y}" style="${color}" d="M${p.x},${p.y-6} L${p.x+6},${p.y} L${p.x},${p.y+6} L${p.x-6},${p.y} Z"/>`);
      }
    });
    event.entityIds.forEach(id=>{
      const p=points.get(id);if(!p)return;
      const arrival=event.edgeIds.length?Math.max(0,(progress-.8)/.2):progress;
      const radius=16+arrival*10;
      marks.push(`<circle class="activity-node-ring" data-effect="${event.effect}" cx="${p.x}" cy="${p.y}" r="${radius}" opacity="${arrival>0?.75-arrival*.45:.25}"/>`);
    });
    layer.dataset.event=event.id;layer.dataset.progress=progress.toFixed(3);layer.innerHTML=marks.join('');
  }
  function animateActivity() {
    cancelAnimationFrame(activityAnimation);
    function frame() {
      if (!activityIsVisible()) {stopActivityPlayback();return;}
      drawActivityOverlay();
      if (!activityPaused && activityProgress()<1) activityAnimation=requestAnimationFrame(frame);
    }
    frame();
  }
  function presentActivity(index,animate=true,record=true) {
    const events=activityEvents();
    if (index<0 || index>=events.length) return false;
    activityCursors[activityKey()]=index;
    if (record) recordActivityEvent(events[index]);
    activityPaused=false;activityElapsed=0;activityStarted=animate?performance.now():-Infinity;
    renderActivityPanel();animateActivity();return true;
  }
  function scheduleActivity() {
    clearTimeout(activityTimer);
    const key=activityKey();
    activityTimer=setTimeout(()=>{
      if (!activityPlaying || key!==activityKey() || !activityIsVisible()) {stopActivityPlayback();renderActivityPanel();return;}
      const visible=visibleGraph(), nodes=new Set(visible?.nodes.map(node=>node.id)), edges=new Set(visible?.edges.map(edge=>edge.id));
      const next=activityEvents().findIndex((event,index)=>index>(activityCursors[key] ?? -1) && (event.entityIds.some(id=>nodes.has(id)) || event.edgeIds.some(id=>edges.has(id))));
      if (!presentActivity(next)) {stopActivityPlayback();renderActivityPanel();if(activityPersistRun)save();return;}
      if(activityPersistRun)save();scheduleActivity();
    },3000);
  }
  function openActivityInspector() {
    const event=currentActivityEvent();if(!event)return;
    // Inspecting freezes the sample so the graph and the dialog name the same event.
    holdActivity();renderActivityPanel();
    inspectorContent.innerHTML=`<section class="activity-event-detail"><span class="graph-kind-label">Simulated event · ${esc(activityTime(event))}</span><h2>${esc(event.label)}</h2><div class="activity-kind">${esc(event.kind)}</div><p>${esc(event.detail)}</p><dl class="ui-kv"><dt>Source</dt><dd>${esc(event.kind==='projection.refreshed'?'Derived API read':activitySources[event.source])}</dd><dt>Perspective</dt><dd>${esc(titleForPart(currentPart)+' · '+currentGraphView().label)}</dd><dt>Presentation</dt><dd>${esc(activityTraceLabel(event))}${event.effect==='flow'?' · solid message pulse':event.edgeIds.length?' · hollow pulse follows the event’s relationships; not a transport observation':' · affected entity highlighted'}</dd></dl><p>Sample replay over the captured graph. No operation is sent.</p><div class="activity-targets">${event.entityIds.map(id=>buttonHTML('Inspect '+esc(graphEntities.get(id)?.label||id),`data-activity-target="${esc(id)}"`)).join('')}</div></section>`;
    inspector.showModal();
  }
  root.addEventListener('click',event=>{
    const button=event.target.closest('button');if(!button)return;
    if (button.hasAttribute('data-activity-play')) {
      if (activityPlaying || activityAutoTimer) {holdActivity();renderActivityPanel();}
      else {
        activityHeld=false;activityPersistRun=true;activityTrigger='Manual replay';
        const cursor=activityCursors[activityKey()] ?? -1;
        activityPlaying=true;
        if (activityPaused && cursor>=0) {activityStarted=performance.now()-activityElapsed;activityPaused=false;renderActivityPanel();animateActivity();}
        else presentActivity(cursor>=activityEvents().length-1?0:cursor+1);
        scheduleActivity();
      }
    } else if (button.hasAttribute('data-activity-next')) {holdActivity();activityTrigger='Stepped event';presentActivity((activityCursors[activityKey()] ?? -1)+1);}
    else if (button.hasAttribute('data-activity-reset')) {holdActivity();activityCursors[activityKey()]=-1;activityPaused=false;renderActivityPanel();drawActivityOverlay();}
    else if (button.hasAttribute('data-activity-seek')) {holdActivity();activityTrigger='Selected event';presentActivity(Number(button.dataset.activitySeek));}
    else if (button.hasAttribute('data-activity-inspect')) {openActivityInspector();return;}
    else if (button.hasAttribute('data-activity-reveal')) {graphFilters[currentPart]={};graphFocus=null;spatialHiddenDimensions=[];renderGraphWorkspace();}
    else if (button.dataset.activityTarget) {graphSelection={kind:'node',id:button.dataset.activityTarget};inspector.close();renderGraphWorkspace();}
    else if (button.hasAttribute('data-activity-clear-log')) {activityLog=activityLog.filter(entry=>activityIndex.get(entry.eventId)?.part!==currentPart);renderActivityLog();}
    else if (button.hasAttribute('data-activity-log-more')) {if(activityExpandedLogs.has(currentPart))activityExpandedLogs.delete(currentPart);else activityExpandedLogs.add(currentPart);renderActivityLog();}
    else if (button.dataset.activityLogEntry) {
      const entry=activityLog.find(item=>item.key===Number(button.dataset.activityLogEntry)), selected=activityIndex.get(entry?.eventId);
      if (!selected) return;
      activityLogFocusKey=entry.key;
      if (selected.view!==currentGraphView()?.id && graphFilters[currentPart]) {graphFilters[currentPart].kind='all';graphFilters[currentPart].source='all';}
      stopActivityPlayback();graphPerspectives[currentPart]=selected.view;graphFocus=null;renderGraphWorkspace();
      holdActivity();activityTrigger='Log replay';
      presentActivity(activityEvents().findIndex(item=>item.id===selected.id),true,false);
      partContent.querySelector('.activity-panel')?.scrollIntoView({block:'center',behavior:activityMotion.matches?'auto':'smooth'});
    }
    else return;
    save();
  });
  document.addEventListener('visibilitychange',()=>{
    if (document.hidden && (activityPlaying || activityAutoTimer)) {holdActivity();renderActivityPanel();}
    else if (!document.hidden) {ensureActivityScope();renderActivityPanel();}
  });
  activityMotion.addEventListener('change',()=>{drawActivityOverlay();});
