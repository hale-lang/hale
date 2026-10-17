  const spatialFamilies = [
    { id: 'structure', label: 'Structure', depth: -180 },
    { id: 'authority', label: 'Authority', depth: -90 },
    { id: 'work', label: 'Work', depth: 0 },
    { id: 'signal', label: 'Signals', depth: 90 },
    { id: 'evidence', label: 'Evidence', depth: 180 }
  ];
  let spatialProjection = savedState?.spatialProjection === '3d' ? '3d' : '2d';
  let spatialRelated = savedState?.spatialRelated !== false;
  let spatialHiddenDimensions = (savedState?.spatialHiddenDimensions || []).filter(id => spatialFamilies.some(f => f.id === id));
  let spatialUnfold = spatialProjection === '3d' ? 1 : 0;
  let spatialCamera = { yaw: .58, pitch: .57, zoom: 1, x: 0, y: 0 };
  let spatialCameraScope = '';
  let spatialAnimation = 0;
  const spatialLayoutCache = new Map();
  const spatialViewCache = new Map();
  const spatialDepth = id => spatialFamilies.find(f => f.id === id)?.depth || 0;
  function restoreSpatialState(state) {
    const projection = state?.spatialProjection === '3d' ? '3d' : '2d';
    if (projection !== spatialProjection) {
      cancelAnimationFrame(spatialAnimation);
      spatialUnfold = projection === '3d' ? 1 : 0;
    }
    spatialProjection = projection;
    spatialRelated = state?.spatialRelated !== false;
    spatialHiddenDimensions = (state?.spatialHiddenDimensions || []).filter(id => spatialFamilies.some(f => f.id === id));
  }
  function getSpatialView(view) {
    if (!view) return null;
    const key = currentPart + ':' + view.id + ':' + spatialRelated;
    if (spatialViewCache.has(key)) return spatialViewCache.get(key);
    const localIds = new Set(view.nodes.map(node => node.id));
    let nodes = [...view.nodes], edges = [...view.edges], omitted = 0;
    if (spatialRelated) {
      // Expand only documented direct neighbors. A page boundary is not an entity boundary.
      const candidates = new Map();
      graphRelations.forEach(edge => {
        if (localIds.has(edge.from) === localIds.has(edge.to)) return;
        const id = localIds.has(edge.from) ? edge.to : edge.from;
        if (graphEntities.has(id)) candidates.set(id, (candidates.get(id) || 0) + 1);
      });
      const ranked = [...candidates].sort((a,b) => b[1] - a[1] || a[0].localeCompare(b[0]));
      const extra = ranked.slice(0, Math.max(0, 28 - nodes.length)).map(([id]) => graphEntities.get(id));
      omitted = ranked.length - extra.length;
      nodes.push(...extra);
      const included = new Set(nodes.map(node => node.id));
      const edgeIds = new Set(edges.map(edge => edge.id));
      graphRelations.forEach(edge => {
        if (!edgeIds.has(edge.id) && included.has(edge.from) && included.has(edge.to) && (localIds.has(edge.from) || localIds.has(edge.to))) {
          edges.push(edge); edgeIds.add(edge.id);
        }
      });
    }
    const result = { ...view, nodes, edges, localIds, omitted, spatialKey: key };
    spatialViewCache.set(key, result);
    return result;
  }
  function renderSpatialToolbar() {
    const host = partContent.querySelector('.graph-workbench');
    const view = getSpatialView(currentGraphView());
    if (!host || !view) return;
    const unfoldButton = host.querySelector('[data-spatial-unfold]');
    unfoldButton.textContent = spatialProjection === '3d' ? 'Fold into 2D' : 'Unfold into 3D';
    unfoldButton.setAttribute('aria-pressed', String(spatialProjection === '3d'));
    host.querySelector('[data-spatial-context]').setAttribute('aria-pressed', String(spatialRelated));
    host.querySelector('.spatial-zoom-value').textContent = Math.round(spatialCamera.zoom * 100) + '%';
    host.querySelector('.spatial-gesture-hint').textContent = spatialProjection === '3d' ? 'Drag to orbit · Shift+drag to pan · Ctrl+scroll to zoom' : 'Drag to pan · Ctrl+scroll to zoom';
    host.querySelector('.spatial-layer-hint').textContent = spatialProjection === '3d' ? 'Depth separates relationship layers' : 'Color follows the relationship dimension';
    host.querySelector('.spatial-orbit-buttons').hidden = spatialProjection !== '3d';
    const counts = new Map(spatialFamilies.map(family => [family.id, view.edges.filter(edge => edge.dimension === family.id).length]));
    host.querySelector('.spatial-dimensions').innerHTML = spatialFamilies.filter(family => counts.get(family.id)).map(family => buttonHTML(`<span class="spatial-route-key" aria-hidden="true"></span>${family.label}<span class="ui-muted">${counts.get(family.id)}</span>`, `data-spatial-dimension="${family.id}" aria-pressed="${!spatialHiddenDimensions.includes(family.id)}" aria-label="${family.label} relationships" style="--route:var(--route-${family.id})"`, '')).join('');
    const coverage = host.querySelector('.graph-coverage');
    if (coverage && spatialRelated && !coverage.textContent.includes('connected context')) coverage.textContent += ` · connected context${view.omitted ? ' · ' + view.omitted + ' neighbors beyond this sample' : ''}`;
  }
  function resetSpatialCamera() { spatialCamera = { yaw: .58, pitch: .57, zoom: 1, x: 0, y: 0 }; }
  function spatialCoordinates(view) {
    if (spatialLayoutCache.has(view.spatialKey)) return spatialLayoutCache.get(view.spatialKey);
    const xy = computeSpatialLayout(view.nodes, view.edges, view.nodes.some(node => node.id === currentPart) ? currentPart : null);
    const coords = new Map(view.nodes.map(node => {
      const incident = view.edges.filter(edge => edge.from === node.id || edge.to === node.id);
      const dimensions = [...new Set(incident.map(edge => edge.dimension))];
      const z = dimensions.length ? dimensions.reduce((sum,id) => sum + spatialDepth(id), 0) / dimensions.length * .65 : 0;
      return [node.id, { ...xy.get(node.id), z, dimensions, degree: incident.length }];
    }));
    spatialLayoutCache.set(view.spatialKey, coords);
    return coords;
  }
  function spatialProject(point, width, height) {
    const yaw = spatialCamera.yaw * spatialUnfold, pitch = spatialCamera.pitch * spatialUnfold;
    const z = (point.z || 0) * spatialUnfold;
    const x1 = point.x * Math.cos(yaw) + z * Math.sin(yaw);
    const z1 = -point.x * Math.sin(yaw) + z * Math.cos(yaw);
    const y1 = point.y * Math.cos(pitch) - z1 * Math.sin(pitch);
    const z2 = point.y * Math.sin(pitch) + z1 * Math.cos(pitch);
    const perspective = 1050 / (1050 - z2);
    const scale = Math.min((width - 165) / 660, (height - 100) / 660) * spatialCamera.zoom;
    return { x: width / 2 + x1 * scale * perspective + spatialCamera.x, y: height / 2 + y1 * scale * perspective + spatialCamera.y, z: z2, size: perspective };
  }
  function spatialRoute(edge, from, to, parallelIndex) {
    const dx = to.x - from.x, dy = to.y - from.y;
    const distance = Math.hypot(dx,dy) || 1;
    const offset = parallelIndex * 12;
    const ox = -dy / distance * offset, oy = dx / distance * offset;
    const diagonal = Math.min(Math.abs(dx), Math.abs(dy)) * .5;
    const first = { x: from.x + Math.sign(dx) * diagonal + ox, y: from.y + Math.sign(dy) * diagonal + oy };
    const second = { x: to.x - Math.sign(dx) * diagonal + ox, y: to.y - Math.sign(dy) * diagonal + oy };
    const routeZ = spatialDepth(edge.dimension) * 1.55;
    return [from, { ...first, z: routeZ }, { ...second, z: routeZ }, to];
  }
  function drawSpatialGraph() {
    const canvas = partContent.querySelector('.graph-canvas');
    const data = visibleGraph();
    if (!canvas || !data || !canvas.clientWidth) return;
    const { view, nodes, edges } = data;
    if (spatialCameraScope !== view.spatialKey) { spatialCameraScope = view.spatialKey; resetSpatialCamera(); }
    const activeId = canvas.contains(document.activeElement) ? document.activeElement?.dataset.graphNode : null;
    const width = canvas.clientWidth, height = Math.max(490, Math.min(660, width * .76));
    canvas.style.height = height + 'px';
    canvas.dataset.projection = spatialProjection;
    canvas.dataset.unfold = spatialUnfold.toFixed(3);
    canvas.dataset.yaw = spatialCamera.yaw.toFixed(3);
    canvas.dataset.pitch = spatialCamera.pitch.toFixed(3);
    if (!nodes.length) { canvas.innerHTML = '<div class="graph-no-results"><h3>No matching entities</h3><p>Change the filters to expand this perspective.</p></div>'; setActivityScene(canvas,new Map(),[]); return; }
    const world = spatialCoordinates(view);
    const points = new Map(nodes.map(node => [node.id, spatialProject(world.get(node.id),width,height)]));
    const neighborhood = selectedNeighborhood(edges);
    const selectionVisible = graphSelection && (graphSelection.kind === 'node' ? points.has(graphSelection.id) : edges.some(edge => edge.id === graphSelection.id));
    const svg = [`<svg class="spatial-svg" viewBox="0 0 ${width} ${height}" aria-hidden="true">`];
    if (spatialUnfold > .05) {
      // Neutral camera reference rings do not assert containment or distance units.
      for (const radius of [120,240,360]) {
        const ring = Array.from({length:65},(_,i) => spatialProject({x:Math.cos(i/64*Math.PI*2)*radius,y:Math.sin(i/64*Math.PI*2)*radius,z:0},width,height));
        svg.push(`<path class="spatial-ring" style="opacity:${spatialUnfold*.4}" d="${ring.map((p,i)=>(i?'L':'M')+p.x.toFixed(2)+','+p.y.toFixed(2)).join(' ')} Z"/>`);
      }
    }
    const routed = [];
    const pairs = new Map();
    edges.forEach(edge => {
      const key = [edge.from,edge.to].sort().join('|');
      if (!pairs.has(key)) pairs.set(key,[]);
      pairs.get(key).push(edge.id);
    });
    edges.forEach(edge => {
      const pair = pairs.get([edge.from,edge.to].sort().join('|'));
      const parallel = pair.indexOf(edge.id) - (pair.length - 1) / 2;
      const route = spatialRoute(edge,world.get(edge.from),world.get(edge.to),parallel).map(point => spatialProject(point,width,height));
      routed.push({edge,route,depth:route.reduce((sum,p)=>sum+p.z,0)/route.length});
    });
    const edgeButtons = [];
    routed.sort((a,b)=>a.depth-b.depth).forEach(({edge,route}) => {
      const selected = graphSelection?.kind === 'edge' && graphSelection.id === edge.id;
      const muted = selectionVisible && neighborhood && !(neighborhood.has(edge.from) && neighborhood.has(edge.to));
      const path = route.map((p,i)=>(i?'L':'M')+p.x.toFixed(2)+','+p.y.toFixed(2)).join(' ');
      const style = `--route:var(--route-${edge.dimension})`;
      const tip = route[route.length-1];
      const beforeTip = [...route].reverse().find(p=>Math.hypot(p.x-tip.x,p.y-tip.y)>18);
      let arrow = '';
      if (beforeTip) {
        const length=Math.hypot(tip.x-beforeTip.x,tip.y-beforeTip.y), ux=(tip.x-beforeTip.x)/length, uy=(tip.y-beforeTip.y)/length;
        const x=tip.x-ux*17,y=tip.y-uy*17;
        arrow=`<path class="spatial-direction" style="${style};opacity:${muted ? .15 : .7}" d="M${x-ux*5-uy*2.5},${y-uy*5+ux*2.5} L${x},${y} L${x-ux*5+uy*2.5},${y-uy*5-ux*2.5}"/>`;
      }
      svg.push(`<path class="spatial-edge-glow" d="${path}" style="${style}"/><path class="spatial-edge" d="${path}" data-kind="${esc(edge.kind)}" data-dimension="${edge.dimension}" data-selected="${selected}" data-muted="${Boolean(muted)}" style="${style}"/>${arrow}<path class="spatial-edge-hit cursor-interaction" d="${path}" data-spatial-edge="${esc(edge.id)}" data-tooltip="${esc(graphEntities.get(edge.from)?.label + ' ' + edge.label + ' → ' + graphEntities.get(edge.to)?.label + ' · ' + edge.dimension)}"/>`);
      if (selected) {
        const a = route[1], b = route[2], mx = Math.max(65,Math.min(width-65,(a.x+b.x)/2)), my = Math.max(20,Math.min(height-20,(a.y+b.y)/2));
        edgeButtons.push(buttonHTML(esc(edge.label),`data-graph-edge="${esc(edge.id)}" style="left:${mx}px;top:${my}px" aria-pressed="true"`,'graph-edge-label'));
      }
    });
    svg.push('</svg>');
    const nodeMarkup = [...nodes].sort((a,b)=>points.get(a.id).z-points.get(b.id).z).map(node => {
      const pos = points.get(node.id), source = world.get(node.id);
      const selected = graphSelection?.kind === 'node' && graphSelection.id === node.id;
      const radius = Math.min(7.5,4.5 + source.degree * .33) * Math.min(1.2,pos.size);
      const dimensions = source.dimensions.filter(id => !spatialHiddenDimensions.includes(id));
      const marks = dimensions.map((id,i) => {
        const circumference = 2*Math.PI*(radius+3), length = circumference / dimensions.length;
        return `<circle cx="16" cy="16" r="${radius+3}" fill="none" stroke="var(--route-${id})" stroke-width="2" stroke-dasharray="${Math.max(1,length-2)} ${circumference-Math.max(1,length-2)}" transform="rotate(${i*360/dimensions.length-90} 16 16)"/>`;
      }).join('');
      const attention = needsAttention(node);
      const label = `<span class="spatial-label">${esc(node.label)}${selected ? `<span class="spatial-label-kind">${esc(node.kind)}</span>` : ''}</span>`;
      const mark = `<svg class="spatial-station-mark" viewBox="0 0 32 32" aria-hidden="true">${selected ? '<circle class="spatial-selection-ring" cx="16" cy="16" r="15"/>' : ''}${marks}<circle class="spatial-station-core" cx="16" cy="16" r="${radius}"/>${attention ? '<circle cx="16" cy="16" r="2" fill="var(--habitat-warn)"/>' : ''}</svg>`;
      return buttonHTML(mark + label,`data-graph-node="${esc(node.id)}" aria-label="${esc(node.label + ' · ' + node.kind + (node.state ? ' · ' + node.state : ''))}" aria-pressed="${selected}" data-context="${Boolean(data.primary && !data.primary.has(node.id))}" data-muted="${Boolean(selectionVisible && neighborhood && !neighborhood.has(node.id))}" data-local="${view.localIds.has(node.id)}" style="left:${pos.x-16}px;top:${pos.y-16}px;z-index:${selected?25:Math.round(10+(pos.z+500)/100)}"`,'graph-node');
    }).join('');
    canvas.innerHTML = svg.join('') + nodeMarkup + edgeButtons.join('');
    positionSpatialLabels(canvas,points,world,view,width,height);
    setActivityScene(canvas,points,routed);
    if (activeId) [...canvas.querySelectorAll('[data-graph-node]')].find(button=>button.dataset.graphNode===activeId)?.focus({preventScroll:true});
    partContent.querySelector('.spatial-zoom-value').textContent = Math.round(spatialCamera.zoom * 100) + '%';
  }
  function positionSpatialLabels(canvas,points,world,view,width,height) {
    const boxes = [...points.values()].map(p=>({left:p.x-16,right:p.x+16,top:p.y-16,bottom:p.y+16}));
    const canvasBox = canvas.getBoundingClientRect();
    canvas.querySelectorAll('.graph-edge-label').forEach(label=>{
      const r=label.getBoundingClientRect(); boxes.push({left:r.left-canvasBox.left,right:r.right-canvasBox.left,top:r.top-canvasBox.top,bottom:r.bottom-canvasBox.top});
    });
    const buttons = [...canvas.querySelectorAll('[data-graph-node]')].sort((a,b)=>{
      const priority = el => (el.getAttribute('aria-pressed')==='true'?1000:0)+(view.localIds.has(el.dataset.graphNode)?100:0)+world.get(el.dataset.graphNode).degree;
      return priority(b)-priority(a);
    });
    buttons.forEach(button=>{
      const label = button.querySelector('.spatial-label'), p = points.get(button.dataset.graphNode);
      const w = label.offsetWidth, h = label.offsetHeight;
      const candidates = [[0,22+h/2],[0,-22-h/2],[22+w/2,0],[-22-w/2,0],[20+w/2,20+h/2],[-20-w/2,20+h/2],[20+w/2,-20-h/2],[-20-w/2,-20-h/2]];
      let chosen = null;
      for (const [dx,dy] of candidates) {
        const rect={left:p.x+dx-w/2,right:p.x+dx+w/2,top:p.y+dy-h/2,bottom:p.y+dy+h/2};
        if (rect.left<5 || rect.right>width-5 || rect.top<5 || rect.bottom>height-5) continue;
        if (!boxes.some(b=>rect.left<b.right+3 && rect.right>b.left-3 && rect.top<b.bottom+3 && rect.bottom>b.top-3)) {chosen={dx,dy,rect};break;}
      }
      if (!chosen) { label.classList.add('is-hidden'); label.style.left='16px'; label.style.top='44px'; button.setAttribute('data-tooltip',button.getAttribute('aria-label')); }
      else { label.style.left=16+chosen.dx+'px'; label.style.top=16+chosen.dy+'px'; boxes.push(chosen.rect); }
    });
  }
  function animateSpatialUnfold() {
    cancelAnimationFrame(spatialAnimation);
    const target = spatialProjection === '3d' ? 1 : 0, initial = spatialUnfold;
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {spatialUnfold=target;drawGraph();return;}
    const start = performance.now();
    function frame(now) {
      const progress = Math.min(1,(now-start)/700), eased = progress*progress*(3-2*progress);
      spatialUnfold = initial + (target-initial)*eased;
      drawGraph();
      if (progress<1) spatialAnimation=requestAnimationFrame(frame);
    }
    spatialAnimation=requestAnimationFrame(frame);
  }
  function attachSpatialInteractions() {
    const canvas = partContent.querySelector('.graph-canvas');
    if (!canvas || canvas.dataset.spatialBound) return;
    canvas.dataset.spatialBound='true';
    let drag = null, suppressClick = false;
    canvas.addEventListener('pointerdown',event=>{
      suppressClick=false;
      if (event.button!==0 || event.target.closest('button')) return;
      drag={x:event.clientX,y:event.clientY,startX:event.clientX,startY:event.clientY,moved:false};
    });
    canvas.addEventListener('pointermove',event=>{
      if (!drag) return;
      const dx=event.clientX-drag.x,dy=event.clientY-drag.y;
      if (Math.hypot(event.clientX-drag.startX,event.clientY-drag.startY)>4) drag.moved=true;
      if (drag.moved) {
        if (!canvas.hasPointerCapture(event.pointerId)) canvas.setPointerCapture(event.pointerId);
        canvas.classList.add('is-dragging');
        if (spatialProjection==='3d' && !event.shiftKey) {spatialCamera.yaw+=dx*.007;spatialCamera.pitch=Math.max(-1.2,Math.min(1.2,spatialCamera.pitch+dy*.007));}
        else {spatialCamera.x+=dx;spatialCamera.y+=dy;}
        drawGraph();
      }
      drag.x=event.clientX;drag.y=event.clientY;
    });
    function finishDrag(event) {
      if (!drag) return;
      suppressClick=drag.moved;drag=null;canvas.classList.remove('is-dragging');
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    }
    canvas.addEventListener('pointerup',finishDrag);
    canvas.addEventListener('pointercancel',finishDrag);
    canvas.addEventListener('pointerleave',event=>{if(drag && !canvas.hasPointerCapture(event.pointerId)) finishDrag(event);});
    canvas.addEventListener('click',event=>{
      if (suppressClick) {suppressClick=false;event.stopImmediatePropagation();event.preventDefault();return;}
      const edge=event.target.closest('[data-spatial-edge]');
      if (!edge) return;
      graphSelection={kind:'edge',id:edge.dataset.spatialEdge};renderGraphWorkspace();queueActivitySimulation('Selection',graphSelection);renderActivityPanel();save();
    },true);
    canvas.addEventListener('wheel',event=>{
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();spatialCamera.zoom=Math.max(.55,Math.min(2.8,spatialCamera.zoom*Math.exp(-event.deltaY*.003)));drawGraph();
    },{passive:false});
  }
  root.addEventListener('click',event=>{
    const button=event.target.closest('button');
    if (!button) return;
    if (button.hasAttribute('data-spatial-unfold')) {spatialProjection=spatialProjection==='3d'?'2d':'3d';renderSpatialToolbar();animateSpatialUnfold();save();return;}
    if (button.hasAttribute('data-spatial-context')) {
      spatialRelated=!spatialRelated;graphFocus=null;
      const filters=filtersForPart(), view=getSpatialView(currentGraphView());
      if (filters.source && !view.nodes.some(node=>node.id===filters.source)) filters.source='all';
      if (filters.kind && !view.nodes.some(node=>node.kind===filters.kind)) filters.kind='all';
    }
    else if (button.dataset.spatialDimension) {
      const id=button.dataset.spatialDimension;
      spatialHiddenDimensions=spatialHiddenDimensions.includes(id)?spatialHiddenDimensions.filter(value=>value!==id):[...spatialHiddenDimensions,id];
    }
    else if (button.dataset.spatialZoom) spatialCamera.zoom=Math.max(.55,Math.min(2.8,spatialCamera.zoom*(button.dataset.spatialZoom==='in'?1.2:1/1.2)));
    else if (button.hasAttribute('data-spatial-reset')) resetSpatialCamera();
    else if (button.dataset.spatialOrbit) spatialCamera.yaw+=button.dataset.spatialOrbit==='left'?-.28:.28;
    else return;
    renderGraphWorkspace();
    if (button.hasAttribute('data-spatial-context') || button.dataset.spatialDimension) queueActivitySimulation('Visible relationships');
    renderActivityPanel();save();
  });
