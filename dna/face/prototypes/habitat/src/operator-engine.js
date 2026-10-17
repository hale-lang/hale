  // The overview is a curated perspective over the same fixture identities.
  // Attention is explicitly authored; a pending/held state is not an alarm.
  let operatorViews = validatedOperatorViews(savedState?.operatorViews);
  function validatedOperatorViews(incoming) {
    return Object.fromEntries(Object.keys(graphData).filter(part=>incoming?.[part]==='explore').map(part=>[part,'explore']));
  }
  function operatorIsActive() { return operatorViews[currentPart]!=='explore'; }
  function operatorGraph() {
    const focus=operatorData[currentPart]?.focus, view=graphData[currentPart]?.views.find(item=>item.id===focus?.view);
    if (!view) return null;
    return {view,nodes:view.nodes.filter(node=>focus.nodes.includes(node.id)),edges:view.edges.filter(edge=>focus.edges.includes(edge.id)),primary:null,scopedNodeCount:focus.nodes.length};
  }
  function operatorTargetAttrs(target) {
    return `data-operator-part="${esc(target.part)}" data-operator-view="${esc(target.view)}" data-operator-node="${esc(target.node)}"`;
  }
  function renderPageDepth() {
    const host=partContent.querySelector('.graph-workbench');if(!host)return;
    const active=operatorIsActive();
    host.dataset.depth=active?'overview':'explore';
    host.querySelector('.operator-overview').hidden=!active;
    host.querySelector('.graph-explorer').hidden=active;
    host.querySelectorAll('[data-page-depth]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.pageDepth===(active?'overview':'explore'))));
  }
  function renderOperatorPage() {
    const host=partContent.querySelector('.operator-overview'), page=operatorData[currentPart];
    if(!host || !page)return;
    host.querySelector('.operator-question').textContent=page.question;
    host.querySelector('.operator-headline').textContent=page.headline;
    host.querySelector('.operator-summary').textContent=page.summary;
    host.querySelector('.operator-checks').innerHTML=page.checks.map(item=>buttonHTML(`<span>${esc(item.label)}</span><strong>${esc(item.value)}</strong>`,operatorTargetAttrs(item.target),'operator-check')).join('');
    host.querySelector('.operator-attention').innerHTML='<h3>Next up</h3>'+(page.attention.length?page.attention.map(item=>`<article class="operator-next"><h4>${esc(item.title)}</h4><p>${esc(item.why)}</p><p class="operator-next-step">${esc(item.next)}</p>${buttonHTML(esc(item.action)+' →',operatorTargetAttrs(item.target))}</article>`).join(''):'<p class="operator-quiet">No intervention flagged in this sample. The facts above open the working brief and its evidence.</p>');
    host.querySelector('.operator-map-caption').textContent=page.focus.nodes.length+' selected entities · '+currentGraphView().label+' · full context in Explore';
    drawOperatorContext();
    renderActivityPanel();
  }
  function drawOperatorContext() {
    const canvas=partContent.querySelector('.operator-map'), graph=operatorGraph();
    if(!canvas || !operatorIsActive() || !graph)return;
    const {nodes,edges}=graph, width=Math.max(200,canvas.clientWidth), height=canvas.clientHeight || 255;
    const layout=computeSpatialLayout(nodes,edges,nodes[0]?.id), coords=[...layout.values()];
    const xMin=Math.min(...coords.map(p=>p.x)),xMax=Math.max(...coords.map(p=>p.x)),yMin=Math.min(...coords.map(p=>p.y)),yMax=Math.max(...coords.map(p=>p.y));
    const points=new Map(nodes.map(node=>{
      const p=layout.get(node.id);
      return [node.id,{x:xMax===xMin?width/2:58+(p.x-xMin)/(xMax-xMin)*(width-116),y:yMax===yMin?height/2:32+(p.y-yMin)/(yMax-yMin)*(height-88)}];
    }));
    const routed=edges.map(edge=>{
      const a=points.get(edge.from),b=points.get(edge.to);
      return {edge,route:[a,{x:a.x+(b.x-a.x)*.4,y:a.y+(b.y-a.y)*.55},{x:a.x+(b.x-a.x)*.7,y:a.y+(b.y-a.y)*.55},b]};
    });
    let svg=`<svg class="spatial-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="${esc(titleForPart(currentPart))}: ${nodes.length} selected entities and ${edges.length} authored relationships">`;
    routed.forEach(({edge,route})=>{
      const d=route.map((p,index)=>(index?'L':'M')+p.x.toFixed(2)+','+p.y.toFixed(2)).join(' '), color=`--route:var(--route-${edge.dimension})`;
      const a=route[route.length-2],b=route[route.length-1],length=Math.hypot(b.x-a.x,b.y-a.y)||1,ux=(b.x-a.x)/length,uy=(b.y-a.y)/length,x=b.x-ux*10,y=b.y-uy*10;
      svg+=`<path class="spatial-edge" data-kind="${esc(edge.kind)}" style="${color}" d="${d}"/><path class="spatial-direction" style="${color}" d="M${x-ux*4-uy*2.5},${y-uy*4+ux*2.5} L${x},${y} L${x-ux*4+uy*2.5},${y-uy*4-ux*2.5}"/>`;
    });
    nodes.forEach(node=>{const p=points.get(node.id);svg+=`<circle class="operator-station" cx="${p.x}" cy="${p.y}" r="5"/>`;});
    canvas.innerHTML=svg+'</svg>';
    const occupied=[];
    nodes.forEach(node=>{
      const p=points.get(node.id), labelHeight=Math.max(matchMedia('(pointer:coarse)').matches?44:28,Math.ceil(node.label.length/15)*15+6), labelWidth=Math.min(106,width-8);
      const candidates=[];
      for(const distance of [12,30,50,72]) for(const angle of [Math.PI/2,-Math.PI/2,0,Math.PI,Math.PI/4,-Math.PI/4,Math.PI*3/4,-Math.PI*3/4]) {
        const dx=Math.cos(angle),dy=Math.sin(angle);
        candidates.push({x:Math.max(2,Math.min(width-labelWidth-2,p.x+dx*(labelWidth/2+distance)-labelWidth/2)),y:Math.max(2,Math.min(height-labelHeight-2,p.y+dy*(labelHeight/2+distance)-labelHeight/2))});
      }
      const overlap=pos=>occupied.reduce((sum,r)=>sum+Math.max(0,Math.min(pos.x+labelWidth+4,r.x+r.w+4)-Math.max(pos.x,r.x))*Math.max(0,Math.min(pos.y+labelHeight+4,r.y+r.h+4)-Math.max(pos.y,r.y))*1000,0)+[...points.values()].filter(q=>q.x>pos.x-7 && q.x<pos.x+labelWidth+7 && q.y>pos.y-7 && q.y<pos.y+labelHeight+7).length*1000;
      candidates.sort((a,b)=>overlap(a)-overlap(b));
      if(overlap(candidates[0])) {
        const fallback=[];
        for(let y=2;y<=height-labelHeight-2;y+=8)for(let x=2;x<=width-labelWidth-2;x+=8)fallback.push({x,y});
        const distance=pos=>Math.hypot(pos.x+labelWidth/2-p.x,pos.y+labelHeight/2-p.y);
        fallback.sort((a,b)=>overlap(a)-overlap(b)||distance(a)-distance(b));
        if(fallback.length && overlap(fallback[0])<overlap(candidates[0]))candidates.unshift(fallback[0]);
      }
      const pos=candidates[0];occupied.push({...pos,w:labelWidth,h:labelHeight});
      const near={x:Math.max(pos.x,Math.min(pos.x+labelWidth,p.x)),y:Math.max(pos.y,Math.min(pos.y+labelHeight,p.y))};
      if(Math.hypot(near.x-p.x,near.y-p.y)>22)canvas.querySelector('svg').insertAdjacentHTML('beforeend',`<path class="operator-label-leader" d="M${p.x},${p.y} L${near.x},${near.y}"/>`);
      canvas.insertAdjacentHTML('beforeend',buttonHTML(esc(node.label),`${operatorTargetAttrs({part:currentPart,view:graph.view.id,node:node.id})} style="left:${pos.x}px;top:${pos.y}px;width:${labelWidth}px" aria-label="${esc(node.label+' · '+node.state+' · inspect evidence')}"`,'operator-node'));
    });
    setActivityScene(canvas,points,routed);
  }
  function renderOperatorActivity() {
    const host=partContent.querySelector('.operator-activity');if(!host)return;
    const event=currentActivityEvent(),events=activityEvents(),cursor=activityCursors[activityKey()] ?? -1,finished=cursor>=0 && cursor===events.length-1;
    host.querySelector('.operator-activity-status').textContent='Sample activity · '+(activityPlaying?'playing':activityAutoTimer?'starting':activityPaused || activityHeld?'paused':finished?'complete':'ready');
    const label=host.querySelector('.operator-activity-event');label.textContent=event?.label || 'A short scenario follows this working context.';label.disabled=!event;
    const play=host.querySelector('[data-operator-play]');play.textContent=activityPlaying || activityAutoTimer?'Pause':activityPaused?'Resume':'Replay';
  }
  function openOperatorEvidence(target) {
    const view=graphData[target.part]?.views.find(item=>item.id===target.view),node=view?.nodes.find(item=>item.id===target.node);
    if(!node)return;
    holdActivity();renderActivityPanel();
    inspectorContent.innerHTML=`<section class="operator-evidence"><div class="operator-evidence-heading"><span class="graph-kind-label">${esc(titleForPart(target.part))} · ${esc(node.kind)}</span><small>Sample evidence</small></div><h2>${esc(node.label)}</h2><p>${esc(node.note)}</p><dl class="ui-kv"><dt>State</dt><dd>${esc(node.state)}</dd>${(node.facts||[]).map(([key,value])=>`<dt>${esc(key)}</dt><dd>${esc(value)}</dd>`).join('')}</dl>${buttonHTML('Explore these relationships →',`data-operator-explore ${operatorTargetAttrs(target)}`)}<p><small>This opens supporting evidence; no operation is sent.</small></p></section>`;
    inspector.showModal();
  }
  function exploreOperatorTarget(target) {
    const changedPart=currentPart!==target.part;
    currentPart=target.part;operatorViews[currentPart]='explore';graphPerspectives[currentPart]=target.view;graphSelection={kind:'node',id:target.node};graphFocus=target.node;graphFilters[currentPart]={};spatialHiddenDimensions=[];
    if(inspector.open)inspector.close();
    if(changedPart)render();else renderGraphWorkspace();
    holdActivity();renderActivityPanel();
    partContent.querySelector('.graph-inspection')?.scrollIntoView({block:'nearest',behavior:activityMotion.matches?'auto':'smooth'});
  }
  root.addEventListener('click',event=>{
    const button=event.target.closest('button');if(!button)return;
    if(button.dataset.pageDepth || button.hasAttribute('data-operator-full-graph') || button.hasAttribute('data-operator-log')) {
      if(operatorIsActive() && button.dataset.pageDepth!=='overview') {graphPerspectives[currentPart]=operatorData[currentPart].focus.view;graphFocus=null;graphFilters[currentPart]={};}
      operatorViews[currentPart]=button.dataset.pageDepth==='overview'?'overview':'explore';
      renderGraphWorkspace();
      if(button.hasAttribute('data-operator-log'))partContent.querySelector('.activity-log')?.scrollIntoView({block:'center',behavior:activityMotion.matches?'auto':'smooth'});
    } else if(button.hasAttribute('data-operator-play')) {
      partContent.querySelector('.activity-panel [data-activity-play]').click();return;
    } else if(button.dataset.operatorNode) {
      const target={part:button.dataset.operatorPart,view:button.dataset.operatorView,node:button.dataset.operatorNode};
      if(button.hasAttribute('data-operator-explore'))exploreOperatorTarget(target);else openOperatorEvidence(target);
    } else return;
    save();
  });
