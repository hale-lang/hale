  // Graph identity is independent of page layout and of any future RPC transport.
  const graphEntities = new Map();
  const graphRelations = new Map();
  const graphMembership = new Map();
  Object.entries(graphData).forEach(([part, page]) => page.views.forEach(view => {
    view.nodes.forEach(node => {
      if (!graphEntities.has(node.id)) graphEntities.set(node.id, node);
      if (!graphMembership.has(node.id)) graphMembership.set(node.id, []);
      graphMembership.get(node.id).push({ part, view: view.id });
    });
    view.edges.forEach(edge => {
      edge.id = edge.id || [edge.from, edge.label, edge.to].join('::');
      if (!graphRelations.has(edge.id)) graphRelations.set(edge.id, edge);
    });
  }));
  let graphSelection = savedState?.graphSelection || null;
  if (graphSelection && !(graphSelection.kind === 'node' ? graphEntities : graphRelations).has(graphSelection.id)) graphSelection = null;
  let graphPerspectives = savedState?.graphPerspectives || {};
  let graphMode = savedState?.graphMode === 'outline' ? 'outline' : 'graph';
  let graphFilters = validateGraphFilters(savedState?.graphFilters);
  let graphFocus = null;
  let graphScopePart = '';
  let graphResizeObserver = null;
  const esc = value => String(value ?? '').replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
  const buttonHTML = (label, attrs, cls = 'ui-button') => `<button type="button" class="${cls} cursor-interaction" ${attrs}>${label}</button>`;
  const titleForPart = part => parts.find(row => row[0] === part)?.[1] || part;
  const needsAttention = node => /pending|waiting|missing|refused|uncertain|unresolved|candidate|requested|unanswered|not permitted|blocked|held|awaiting/i.test(node.state || '');
  function validateGraphFilters(incoming) {
    const result = {};
    if (!incoming || typeof incoming !== 'object' || Array.isArray(incoming)) return result;
    const kinds = new Set(Object.values(graphData).flatMap(page => page.views.flatMap(view => view.nodes.map(node => node.kind))));
    const encoder = new TextEncoder();
    Object.keys(graphData).forEach(part => {
      const values = incoming[part];
      if (!values || typeof values !== 'object' || Array.isArray(values)) return;
      const filters = {};
      if (typeof values.query === 'string') {
        let query = values.query.slice(0,120);
        while (encoder.encode(JSON.stringify(query)).byteLength > 362) query = query.slice(0,-1);
        filters.query = query;
      }
      if (values.kind === 'all' || kinds.has(values.kind)) filters.kind = values.kind;
      if (values.source === 'all' || graphEntities.has(values.source)) filters.source = values.source;
      if (['all','attention'].includes(values.state)) filters.state = values.state;
      if (['all','observed','declared','proposed'].includes(values.relation)) filters.relation = values.relation;
      if (Object.keys(filters).length) result[part] = filters;
    });
    return result;
  }
  function filtersForPart() { return graphFilters[currentPart] || {}; }
  function graphHasFilters() { return Object.values(filtersForPart()).some(value => value && value !== 'all'); }
  function renderGraphFilters() {
    const host = partContent.querySelector('.graph-filters');
    const view = getSpatialView(currentGraphView());
    if (!host || !view) return;
    const key = currentPart + ':' + view.id + ':' + spatialRelated;
    if (host.dataset.view !== key) {
      const options = items => items.map(([value,label]) => `<option value="${esc(value)}">${esc(label)}</option>`).join('');
      const select = (name,label,items) => `<label class="graph-filter-field">${esc(label)}<select data-graph-filter="${name}">${options(items)}</select></label>`;
      const searchLabel = currentPart === 'memory' ? 'Find knowledge or evidence' : currentPart === 'senses' ? 'Find a reading or source' : 'Find an entity';
      let html = `<label class="graph-filter-field">${searchLabel}<input type="search" data-graph-filter="query" placeholder="Name or identity" autocomplete="off" maxlength="120"></label>`;
      if (currentPart === 'memory') html += select('kind','Entity kind',[['all','All kinds'], ...[...new Set(view.nodes.map(node=>node.kind))].map(kind=>[kind,kind])]);
      if (currentPart === 'senses') {
        const sources = view.nodes.filter(node => /host|runtime part|application|locus/i.test(node.kind));
        html += select('source','Source',[['all','All sources'],...sources.map(node=>[node.id,node.label])]);
      }
      html += select('state','State',[['all','All states'],['attention','Needs attention']]);
      if (currentPart === 'memory' || currentPart === 'senses') html += select('relation','Relationship basis',[['all','All relationships'],['observed','Observed'],['declared','Declared'],['proposed','Proposed']]);
      host.innerHTML = html;
      host.dataset.view = key;
    }
    host.querySelectorAll('[data-graph-filter]').forEach(control => {
      const value = filtersForPart()[control.dataset.graphFilter] || (control.tagName === 'SELECT' ? 'all' : '');
      if (control !== document.activeElement) control.value = value;
    });
  }
  function currentGraphView() {
    const page = graphData[currentPart];
    return page?.views.find(view => view.id === graphPerspectives[currentPart]) || page?.views[0];
  }
  function mountGraphPage() {
    graphResizeObserver?.disconnect();
    const content = document.createDocumentFragment();
    while (partContent.firstChild) content.append(partContent.firstChild);
    partContent.append(root.querySelector('#graph-workbench-template').content.cloneNode(true));
    partContent.querySelector('.graph-record-content').append(content);
    if (graphScopePart !== currentPart) { graphFocus = null; graphScopePart = currentPart; }
    renderGraphWorkspace();
    attachSpatialInteractions();
    let lastWidth = 0;
    graphResizeObserver = new ResizeObserver(entries => {
      const width = Math.round(entries[0].contentRect.width);
      if (width !== lastWidth) { lastWidth = width; if (width > 0) drawGraph(); }
    });
    graphResizeObserver.observe(partContent.querySelector('.graph-canvas'));
  }
  function visibleGraph() {
    const view = getSpatialView(currentGraphView());
    if (!view) return null;
    let nodes = view.nodes, edges = view.edges.filter(edge => !spatialHiddenDimensions.includes(edge.dimension));
    if (graphFocus && nodes.some(node => node.id === graphFocus)) {
      const adjacent = new Set([graphFocus]);
      edges.forEach(edge => { if (edge.from === graphFocus) adjacent.add(edge.to); if (edge.to === graphFocus) adjacent.add(edge.from); });
      nodes = nodes.filter(node => adjacent.has(node.id));
      edges = edges.filter(edge => adjacent.has(edge.from) && adjacent.has(edge.to));
    }
    const scopedNodeCount = nodes.length;
    const filters = filtersForPart();
    let primary = null;
    if (graphHasFilters()) {
      if (filters.relation && filters.relation !== 'all') edges = edges.filter(edge=>edge.kind===filters.relation);
      const query = (filters.query || '').trim().toLowerCase();
      let sourceNeighborhood = null;
      if (filters.source && filters.source !== 'all') {
        sourceNeighborhood = new Set([filters.source]);
        edges.forEach(edge=>{ if(edge.from===filters.source)sourceNeighborhood.add(edge.to); if(edge.to===filters.source)sourceNeighborhood.add(edge.from); });
      }
      const hasNodeFilter = query || (filters.kind && filters.kind !== 'all') || filters.state === 'attention' || sourceNeighborhood;
      primary = new Set(nodes.filter(node => {
        const search = [node.label,node.id,node.kind,node.note,...(node.facts||[]).flat()].join(' ').toLowerCase();
        return (!query || search.includes(query)) && (!filters.kind || filters.kind==='all' || node.kind===filters.kind) && (filters.state!=='attention' || needsAttention(node)) && (!sourceNeighborhood || sourceNeighborhood.has(node.id));
      }).map(node=>node.id));
      if (hasNodeFilter) edges = edges.filter(edge=>primary.has(edge.from)||primary.has(edge.to));
      else primary = new Set(edges.flatMap(edge=>[edge.from,edge.to]));
      const shown = new Set([...primary,...edges.flatMap(edge=>[edge.from,edge.to])]);
      nodes = nodes.filter(node=>shown.has(node.id));
    }
    return { view, nodes, edges, primary, scopedNodeCount };
  }
  function renderGraphWorkspace() {
    ensureActivityScope();
    const workbench = partContent.querySelector('.graph-workbench');
    const data = visibleGraph();
    if (!workbench || !data) return;
    workbench.dataset.presentation = graphMode;
    const active = document.activeElement;
    const focusAttribute = active?.tagName === 'BUTTON' ? Array.from(active.attributes).find(attr=>attr.name.startsWith('data-graph-') || attr.name.startsWith('data-spatial-') || attr.name.startsWith('data-activity-')) : null;
    const { view, nodes, edges } = data;
    renderGraphFilters();
    const filterStatus = workbench.querySelector('.graph-filter-status');
    filterStatus.hidden = !graphHasFilters();
    const matches = data.primary ? data.primary.size : nodes.length;
    filterStatus.innerHTML = `${matches} ${matches===1 ? 'match' : 'matches'} · ${data.primary ? nodes.filter(node=>!data.primary.has(node.id)).length : 0} connected entities kept for context ${buttonHTML('Clear filters','data-graph-clear-filters')}`;
    workbench.querySelector('.graph-context-part').textContent = titleForPart(currentPart);
    workbench.querySelector('.graph-perspectives').innerHTML = graphData[currentPart].views.map(item => buttonHTML(esc(item.label), `data-graph-view="${esc(item.id)}" aria-pressed="${view.id === item.id}"`, '')).join('');
    workbench.querySelectorAll('[data-graph-mode]').forEach(button => { button.setAttribute('aria-pressed', String(button.dataset.graphMode === graphMode)); button.classList.add('cursor-interaction'); });
    workbench.querySelector('.graph-question').textContent = view.summary;
    workbench.querySelector('.graph-focus-trail').hidden = !graphFocus;
    workbench.querySelector('.graph-focus-label').textContent = graphFocus ? 'Neighborhood · ' + graphEntities.get(graphFocus)?.label : '';
    workbench.querySelector('.graph-scope-name').textContent = currentOrganism + ' · ' + view.label;
    workbench.querySelector('.graph-count').textContent = nodes.length + ' entities · ' + edges.length + ' relationships';
    workbench.querySelector('.graph-canvas').hidden = graphMode === 'outline';
    workbench.querySelector('.graph-outline').hidden = graphMode !== 'outline';
    workbench.querySelector('.graph-coverage').textContent = `Bounded sample · ${nodes.length} of ${view.nodes.length} entities shown${graphHasFilters() ? ' · filtered' : ''}`;
    renderSpatialToolbar();
    drawGraph();
    renderGraphOutline();
    renderActivityPanel();
    renderGraphInspector();
    if (focusAttribute) {
      const replacement = Array.from(workbench.querySelectorAll('button')).find(button=>button.getAttribute(focusAttribute.name)===focusAttribute.value && button.offsetWidth>0 && button.offsetHeight>0);
      replacement?.focus({preventScroll:true});
    }
  }
  function selectedNeighborhood(edges) {
    if (!graphSelection) return null;
    if (graphSelection.kind === 'edge') {
      const edge = graphRelations.get(graphSelection.id);
      return edge ? new Set([edge.from, edge.to]) : null;
    }
    const ids = new Set([graphSelection.id]);
    edges.forEach(edge => { if (edge.from === graphSelection.id) ids.add(edge.to); if (edge.to === graphSelection.id) ids.add(edge.from); });
    return ids;
  }
  function drawGraph() {
    drawSpatialGraph();
  }
  function renderGraphOutline() {
    const host = partContent.querySelector('.graph-outline');
    const data = visibleGraph();
    if (!host || !data) return;
    if (!data.nodes.length) { host.innerHTML='<div class="graph-no-results"><h3>No matching entities</h3><p>Change the filters to expand this perspective.</p></div>'; return; }
    host.innerHTML = data.nodes.map(node => `<div class="graph-outline-entry">${buttonHTML(`<span>${esc(node.label)}</span><span class="ui-small ui-muted">${esc(node.kind)}</span>`, `data-graph-node="${esc(node.id)}" aria-pressed="${graphSelection?.kind === 'node' && graphSelection.id === node.id}"`, 'graph-outline-node')}<div class="graph-relations">${data.edges.filter(edge => edge.from === node.id).map(edge => buttonHTML(`<span>${esc(edge.label)}</span><span>→ ${esc(graphEntities.get(edge.to)?.label)}</span>`, `data-graph-edge="${esc(edge.id)}"`, 'graph-relation-row')).join('')}</div></div>`).join('');
  }
  function graphPartsFor(id) {
    return Array.from(new Set((graphMembership.get(id) || []).map(item => item.part)));
  }
  function renderGraphInspector() {
    const host = partContent.querySelector('.graph-inspection');
    const data = visibleGraph();
    if (!host || !data) return;
    if (!graphSelection) {
      host.innerHTML = '<div class="graph-empty-selection"><div><span class="graph-kind-label">One organism · many perspectives</span><h2>Follow an entity through the system</h2><p>Select a node to inspect its relationships, or select a relationship to see what supports it. Your selection travels between perspectives.</p></div></div>';
      return;
    }
    if (graphSelection.kind === 'edge') {
      const edge = data.view.edges.find(item => item.id === graphSelection.id) || graphRelations.get(graphSelection.id);
      if (!edge) return;
      const from = graphEntities.get(edge.from), to = graphEntities.get(edge.to);
      const outside = !data.edges.some(item => item.id === edge.id);
      host.innerHTML = `<div>${outside ? '<div class="graph-outside">Selection preserved · relationship outside the visible graph</div>' : ''}<span class="graph-kind-label">${esc(edge.kind)} relationship</span><h2>${esc(from.label)} → ${esc(to.label)}</h2><p><strong>${esc(edge.label)}</strong>${edge.note ? ' · ' + esc(edge.note) : ''}</p><div class="graph-identity">${esc(edge.id)}</div><div class="graph-linked-parts">${buttonHTML('Inspect ' + esc(from.label), `data-graph-node="${esc(from.id)}"`)}${buttonHTML('Inspect ' + esc(to.label), `data-graph-node="${esc(to.id)}"`)}${buttonHTML('Clear selection','data-graph-clear')}</div></div><div><h3>Basis for this relationship</h3><p>${esc(edge.evidence || 'Proposed read projection')}</p><dl class="ui-kv"><dt>Evidence state</dt><dd>${esc(edge.kind)} · sample</dd><dt>Scope</dt><dd>Atlas · captured at 09:42</dd></dl><p class="ui-small">The line expresses this named relation. It does not imply any other authority or causal link.</p></div>`;
      return;
    }
    const node = data.view.nodes.find(item => item.id === graphSelection.id) || graphEntities.get(graphSelection.id);
    if (!node) return;
    const outside = !data.nodes.some(item => item.id === node.id);
    const outsideReason = data.view.nodes.some(item=>item.id===node.id) ? 'Selection preserved · hidden by current filters or neighborhood' : 'Selection preserved · outside this perspective';
    const related = data.view.edges.filter(edge => edge.from === node.id || edge.to === node.id);
    const otherParts = graphPartsFor(node.id).filter(part => part !== currentPart);
    const facts = (node.facts || []).slice(0, 4).map(([key, value]) => `<dt>${esc(key)}</dt><dd>${esc(value)}</dd>`).join('');
    const relationRows = related.map(edge => {
      const outgoing = edge.from === node.id;
      const neighbor = graphEntities.get(outgoing ? edge.to : edge.from);
      return buttonHTML(`<span>${outgoing ? '' : '← '}${esc(edge.label)}</span><span>${esc(neighbor.label)}${outgoing ? ' →' : ''}</span>`, `data-graph-edge="${esc(edge.id)}"`, 'graph-relation-row');
    }).join('');
    host.innerHTML = `<div>${outside ? `<div class="graph-outside">${outsideReason}</div>` : ''}<span class="graph-kind-label">${esc(node.kind)}${node.state ? ' · ' + esc(node.state) : ''}</span><h2>${esc(node.label)}</h2><div class="graph-identity">atlas / ${esc(node.id)}</div><p>${esc(node.note)}</p>${facts ? `<dl class="ui-kv">${facts}</dl>` : ''}<div class="ui-row">${node.detail ? buttonHTML('Open details', `data-graph-detail="${esc(node.detail)}"`) : ''}${!outside && graphFocus !== node.id ? buttonHTML('Focus neighborhood', `data-graph-focus="${esc(node.id)}"`) : ''}${buttonHTML('Clear selection', 'data-graph-clear')}</div></div><div><h3>${outside ? 'Continue with this entity' : 'Direct relationships'}</h3><div class="graph-relations">${relationRows || '<p class="ui-small">No direct relationships shown in this perspective.</p>'}</div>${otherParts.length ? `<p class="ui-small">Follow the same identity into</p><div class="graph-linked-parts">${otherParts.map(part => buttonHTML(glyph(part) + esc(titleForPart(part)), `data-graph-reveal="${esc(part)}" data-entity="${esc(node.id)}"`)).join('')}</div>` : ''}</div>`;
  }
  function openGraphDetail(id) {
    let detailTemplate = null;
    root.querySelectorAll('template[id^="page-"]').forEach(template => {
      template.content.querySelectorAll('template[data-detail]').forEach(candidate => { if (candidate.dataset.detail === id) detailTemplate = candidate; });
    });
    if (!detailTemplate) return;
    inspectorContent.replaceChildren(detailTemplate.content.cloneNode(true));
    inspectorContent.querySelectorAll('button,a').forEach(button => button.classList.add('cursor-interaction'));
    inspector.showModal();
  }
  root.addEventListener('click', event => {
    const target = event.target.closest('button');
    if (!target) return;
    if (target.dataset.graphNode) graphSelection = { kind: 'node', id: target.dataset.graphNode };
    else if (target.dataset.graphEdge) graphSelection = { kind: 'edge', id: target.dataset.graphEdge };
    else if (target.dataset.graphView) { graphPerspectives[currentPart] = target.dataset.graphView; graphFocus = null; if(graphFilters[currentPart]) { graphFilters[currentPart].source='all'; graphFilters[currentPart].kind='all'; } }
    else if (target.dataset.graphMode) graphMode = target.dataset.graphMode;
    else if (target.dataset.graphFocus) graphFocus = target.dataset.graphFocus;
    else if (target.hasAttribute('data-graph-reset')) graphFocus = null;
    else if (target.hasAttribute('data-graph-clear')) graphSelection = null;
    else if (target.hasAttribute('data-graph-clear-filters')) graphFilters[currentPart] = {};
    else if (target.dataset.graphDetail) { openGraphDetail(target.dataset.graphDetail); return; }
    else if (target.dataset.graphReveal) {
      const part = target.dataset.graphReveal;
      const membership = (graphMembership.get(target.dataset.entity) || []).find(item => item.part === part);
      if (membership) graphPerspectives[part] = membership.view;
      currentPart = part; graphFocus = null; render(); save(); return;
    } else return;
    renderGraphWorkspace();
    if (target.dataset.graphNode || target.dataset.graphEdge) queueActivitySimulation('Selection',graphSelection);
    else if (target.dataset.graphFocus || target.hasAttribute('data-graph-reset') || target.hasAttribute('data-graph-clear-filters')) queueActivitySimulation('Visible neighborhood');
    renderActivityPanel();save();
  });
  function updateGraphFilter(event) {
    const target=event.target;
    if (!target.dataset.graphFilter) return;
    if (event.type==='input' && target.tagName==='SELECT') return;
    if (event.type==='change' && target.tagName==='INPUT') return;
    graphFilters = validateGraphFilters({ ...graphFilters, [currentPart]:{ ...filtersForPart(), [target.dataset.graphFilter]:target.value } });
    renderGraphWorkspace();queueActivitySimulation('Filtered view',null,550);renderActivityPanel();save();
  }
  root.addEventListener('input',updateGraphFilter);
  root.addEventListener('change',updateGraphFilter);
