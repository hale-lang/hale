import { useEffect, useMemo, useRef, useState, useId } from 'react';
import { Loading, Unavailable, Empty, Mono, ago, count, micros } from '@hale/components';
import { useGet, type UsageRecord } from '../api/client';
import { subscribeAdminEvents } from '../api/events';
import type { ScreenProps } from '../App';
import { buildGraph, edgePath, layoutGraph, resizeLayout, requestEdges, requestFlowEdges, requestVertices, matchingRequests, graphKinds, kindName, type Fleet, type Graph, type Kind, type Point } from './network-model';
import { demoFleet, demoInFlight, demoCompletions, demoContext, DEMO_TICK_MS } from './network-demo';
import s from './Network.module.css';
import { FACETS, filterOptions, filterFleet, type Filters } from './network-filters';

const EMPTY: Fleet = { nodes: [], accounts: [], projects: [], usage: [] };

export function Network(_: ScreenProps) {
  const health = useGet('/readyz');
  const nodes = useGet('/admin/v1/nodes');
  const accounts = useGet('/admin/v1/accounts');
  const projects = useGet('/admin/v1/projects');
  const usage = useGet('/admin/v1/usage', { params: { query: { limit: 100 } } });
  const [choice, setChoice] = useState<'api' | 'demo' | null>(() => {
    try { const saved = sessionStorage.getItem('voice.network.source'); return saved === 'api' || saved === 'demo' ? saved : null; } catch { return null; }
  });
  const chooseSource = (source: 'api' | 'demo') => {
    setChoice(source);
    try { sessionStorage.setItem('voice.network.source', source); } catch { /* Storage can be disabled. */ }
  };
  const stub = health.data?.version?.includes('stub') ?? false;
  const source = choice ?? (stub ? 'demo' : 'api');
  const queries = [nodes, accounts, projects, usage];
  const failure = queries.find((q) => q.isError);
  const pending = queries.some((q) => q.isPending);
  const fleet = useMemo<Fleet>(() => ({ nodes: nodes.data?.data ?? [], accounts: accounts.data?.data ?? [], projects: projects.data?.data ?? [], usage: usage.data?.data ?? [] }), [nodes.data, accounts.data, projects.data, usage.data]);

  return <div className={s.screen}>
    <header className={s.header}>
      <div><h1>Network</h1><p className="muted">Neural observatory / inference fleet</p></div>
      <div className={s.sources} role="group" aria-label="Network data source">
        <button aria-pressed={source === 'api'} onClick={() => chooseSource('api')}>API {stub ? 'fixture' : 'data'}</button>
        <button aria-pressed={source === 'demo'} onClick={() => chooseSource('demo')}>Demo fleet</button>
      </div>
    </header>
    {source === 'api' && failure ? <Unavailable what="Network" detail={failure.error?.message} /> : source === 'api' && pending ? <Loading what="the fleet" /> :
      <FleetNetwork key={source} fleet={source === 'demo' ? EMPTY : fleet} demo={source === 'demo'} stub={stub} />}
  </div>;
}

type Pulse = { record: UsageRecord; at: number; inFlight?: boolean; color?: string };

function FleetNetwork({ fleet, demo, stub }: { fleet: Fleet; demo: boolean; stub: boolean }) {
  const [clock, setClock] = useState(() => ({ tick: 0, now: Date.now() }));
  const [paused, setPaused] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [matchMode, setMatchMode] = useState<'all' | 'any'>('all');
  const [query, setQuery] = useState('');
  const selected = selectedIds.at(-1) ?? null;
  const [hovered, setHovered] = useState<string | null>(null);
  const [traceSelection, setTrace] = useState<UsageRecord | null>(null);
  const [pulses, setPulses] = useState<Pulse[]>([]);
  const [arrivals, setArrivals] = useState<UsageRecord[]>([]);
  const [filters, setFilters] = useState<Filters>({});
  const [hidden, setHidden] = useState<Kind[]>([]);
  const seen = useRef(new Set<string>());
  const initialDemo = useMemo(() => demoFleet(0, Date.now()), []);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const data = useMemo(() => demo ? { ...demoFleet(clock.tick, clock.now), usage: initialDemo.usage } : fleet, [demo, clock, fleet, initialDemo]);
  const trace = traceSelection ? arrivals.find(r => r.id === traceSelection.id) ?? traceSelection : null;
  const running = useMemo(() => demo ? demoInFlight(data, clock.tick, clock.now) : [], [demo,data,clock]);
  const activeIds = new Set(running.map(job => job.record.id));
  const traceInFlight = !!trace && activeIds.has(trace.id);
  const motion: Pulse[] = demo ? running.map(job => ({ record: job.record, at: clock.now, inFlight: true, color: job.color })) : pulses;
  const fullData = useMemo(() => {
    const usage = [...new Map([...data.usage, ...(trace ? [trace] : []), ...running.map(job => job.record), ...arrivals].map(r => [r.id, r])).values()];
    return { ...data, usage, contexts: demo ? Object.fromEntries(usage.map(r => [r.id, demoContext(r)])) : undefined };
  }, [data, arrivals, demo, trace, running]);
  const filtered = useMemo(() => filterFleet(fullData, filters), [fullData, filters]);
  const options = useMemo(() => filterOptions(fullData), [fullData]);
  const fullGraph = useMemo(() => buildGraph(fullData), [fullData]);
  const graph = useMemo(() => {
    const g = buildGraph(filtered);
    const vertices = g.vertices.filter(v => !hidden.includes(v.kind));
    const ids = new Set(vertices.map(v => v.id));
    return { ...g, vertices, edges: g.edges.filter(e => ids.has(e.source) && ids.has(e.target)) };
  }, [filtered, hidden]);
  const changeFilter = (key: keyof Filters, value: string) => { setFilters(old => ({ ...old, [key]: value })); setSelectedIds([]); setTrace(null); setHovered(null); };
  const filterCount = Object.values(filters).filter(Boolean).length;

  useEffect(() => {
    const interval = setInterval(() => {
      const now = Date.now();
      setPulses((old) => old.filter((p) => now - p.at < 7000));
      if (demo && !pausedRef.current) setClock((old) => ({ tick: old.tick + 1, now }));
    }, DEMO_TICK_MS);
    return () => clearInterval(interval);
  }, [demo]);

  useEffect(() => {
    if (!demo || clock.tick === 0) return;
    const records = demoCompletions(demoFleet(clock.tick, clock.now), clock.tick, clock.now);
    if (records.length) {
      setArrivals(old => [...records, ...old].slice(0, 12));
      setTrace(current => records.find(r => r.id === current?.id) ?? current);
    }
  }, [demo, clock]);

  useEffect(() => {
    if (demo) return;
    return subscribeAdminEvents((event) => {
      if (event.type === 'reset') { setArrivals([]); setPulses([]); seen.current.clear(); return; }
      if (event.type !== 'usage' || seen.current.has(event.data.id)) return;
      const record = event.data;
      seen.current.add(record.id);
      if (seen.current.size > 512) seen.current.delete(seen.current.values().next().value!);
      setArrivals((old) => [record, ...old].slice(0, 8));
      // Replayed historical rows remain history; they are never presented as live pulses.
      const age = Date.now() - Date.parse(event.at);
      if (!pausedRef.current && age >= 0 && age < 30_000) setPulses((old) => [...old.slice(-3), { record, at: Date.now() }]);
    });
  }, [demo]);

  const records = useMemo(() => filtered.usage.filter(r => !activeIds.has(r.id)).sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at)).slice(0, 6), [filtered.usage]);
  const vertex = graph.vertices.find((v) => v.id === selected);
  const criteria = selectedIds.length ? selectedIds : hovered ? [hovered] : [];
  const focus = criteria.at(-1);
  const cohort = trace ? [trace] : criteria.length ? matchingRequests(criteria, filtered.usage, graph, matchMode) : [];
  const adjacent = criteria.length === 1 && !trace;
  const focusedEdges = new Set(cohort.length ? cohort.flatMap(r => [...requestEdges(r, graph)]) : adjacent ? graph.edges.filter(e => e.source === focus || e.target === focus).map(e => e.id) : []);
  const focusedIds = new Set(cohort.length ? cohort.flatMap(r => [...requestVertices(r, graph)]) : graph.edges.filter(e => focusedEdges.has(e.id)).flatMap(e => [e.source, e.target]));
  if (!trace) for (const id of criteria) focusedIds.add(id);
  const visibleRecords = selectedIds.length ? matchingRequests(selectedIds, filtered.usage.filter(r => !activeIds.has(r.id)).sort((a,b) => Date.parse(b.created_at)-Date.parse(a.created_at)), graph, matchMode).slice(0,6) : records;
  const kinds = graphKinds(fullGraph);
  const searchResults = query.trim() ? fullGraph.vertices.filter(v => `${v.name} ${v.id} ${kindName(v.kind)}`.toLowerCase().includes(query.trim().toLowerCase())).slice(0,8) : [];
  const choose = (id: string) => { setSelectedIds(old => old.includes(id) ? old.filter(value => value !== id) : [...old,id]); setTrace(null); setQuery(''); setHovered(null); };
  const clear = () => { setSelectedIds([]); setTrace(null); setHovered(null); setQuery(''); };
  const inFlight = data.nodes.reduce((sum, n) => sum + (n.in_flight ?? 0), 0);

  return <>
    <div className={s.readout}>
      <span className={demo || stub ? 'c-wait' : 'muted'}>{demo ? 'DEMO · SIMULATED TRAFFIC · NO REQUESTS SENT' : stub ? 'API FIXTURE · CANNED SNAPSHOTS' : 'API DATA · PULSES ON NEW USAGE EVENTS'}</span>
      <span>{data.projects.length} projects · {fullGraph.vertices.filter(v => v.kind === 'repository').length} repositories · {data.accounts.length} accounts · {fullGraph.vertices.filter(v => v.kind === 'model').length} models · {data.nodes.length} nodes · {data.nodes.reduce((sum, n) => sum + n.seats.length, 0)} seats · {inFlight} {demo ? 'simulated' : 'reported'} in flight</span>
    </div>
    <section className={s.explorer} aria-label="Explore the network">
      <div className={s.exploreBar}><label className={s.search}>Find context<input aria-label="Find context" value={query} onChange={e => setQuery(e.target.value)} placeholder="Search anything in the network…" /></label><span className={s.matchCount}>{graph.vertices.length} entities · {graph.edges.length} connections</span>{(selectedIds.length > 0 || trace) && <button onClick={clear}>Clear focus</button>}</div>
      {query && <div className={s.searchResults} aria-label="Matching entities">{searchResults.length ? searchResults.map(v => <button key={v.id} onClick={() => choose(v.id)} aria-pressed={selectedIds.includes(v.id)}><span>{kindName(v.kind)}</span>{v.name}</button>) : <span className="muted">No matching entities.</span>}</div>}
      {selectedIds.length > 0 && <div className={s.focusChips}><span>WITH</span>{selectedIds.map(id => { const v = fullGraph.vertices.find(v => v.id === id); return <button key={id} onClick={() => choose(id)}>{v ? `${kindName(v.kind)} / ${v.name}` : id} ×</button>; })}<select aria-label="Combine selected context" value={matchMode} onChange={e => setMatchMode(e.target.value as 'all' | 'any')}><option value="all">All selected</option><option value="any">Any selected</option></select><span>{cohort.length} matching requests</span></div>}
      {selectedIds.length > 1 && !cohort.length && <p className={s.noMatch}>No recorded request contains {matchMode === 'all' ? 'all' : 'any'} of this context. The network stays visible.</p>}
      <details className={s.refine}><summary>Refine network{filterCount || hidden.length ? ` · ${filterCount + hidden.length} active` : ''}</summary>
        <div className={s.facets}>{FACETS.filter(([key]) => options[key].length).map(([key,label]) => <label key={key}>{label}<select value={filters[key] ?? ''} onChange={e => changeFilter(key,e.target.value)}><option value="">All</option>{options[key].map(value => <option key={value}>{value}</option>)}</select></label>)}</div>
        <div className={s.layers}>{kinds.map(kind => <button key={kind} className={s[kind] ?? s.context} aria-pressed={!hidden.includes(kind)} onClick={() => { setHidden(old => old.includes(kind) ? old.filter(k => k !== kind) : [...old,kind]); clear(); }}><Marker kind={kind} />{kindName(kind)}</button>)}<button disabled={!filterCount && !hidden.length} onClick={() => { setFilters({}); setHidden([]); clear(); }}>Reset filters</button></div>
      </details>
    </section>
    {demo && <section className={s.inFlight} aria-label="Projects in flight"><span>{running.length} / 3 PROJECTS IN FLIGHT</span>{running.map(job => <button key={job.record.id} style={{ borderColor: job.color, color: job.color }} aria-pressed={trace?.id === job.record.id} onClick={() => { setTrace(job.record); setSelectedIds([]); setHovered(null); }}><strong>{job.record.project}</strong><span>→ {job.record.served!.model}</span><small>{Math.round((job.duration-job.elapsed)*DEMO_TICK_MS/1000)}s remaining</small></button>)}<small>Simulated · select a request to follow its branch</small></section>}
    <div className={s.workspace}>
      <div className={s.mapPanel}>
        <div className={s.mapTools}>
          <span className="muted">{trace ? 'Selected request path' : selectedIds.length ? `${selectedIds.length} selected · ${cohort.length} matching requests` : 'One network · click entities to combine context'}</span>
          <div><button disabled={!records.length} onClick={() => { setTrace(records[0]!); setSelectedIds([]); setHovered(null); }}>Trace latest request</button><button onClick={() => { setPaused(!paused); setPulses([]); }}>{paused ? 'Resume' : 'Pause'} {demo ? 'demo' : 'motion'}</button>{(selected || trace) && <button onClick={clear}>Clear selection</button>}</div>
        </div>
        {graph.vertices.length === 0 ? <Empty>{filterCount || hidden.length ? "No entities match these filters. Reset filters to see the fleet." : "No nodes, seats, accounts or projects have been reported yet."}</Empty> :
          <Constellation layout={fullGraph} graph={graph} selected={selectedIds} focusedIds={focusedIds} focusedEdges={focusedEdges} paused={paused} pulses={paused && !demo ? [] : motion.filter(p => filtered.usage.some(r => r.id === p.record.id) && (!focus && !trace || cohort.some(r => r.id === p.record.id)))} onSelect={choose} onHover={setHovered} />}
        <div className={s.legend}>{kinds.map((kind) => <span key={kind} className={s[kind] ?? s.context}><Marker kind={kind} />{kindName(kind)}</span>)}</div>
        <div className={s.mapNote}>Requests follow project → API → host → seat → account → served model. Dotted links carry request context; hosts contain seats; the account layer shows budget attribution. Missing context is simply absent.</div>
        {graph.total > 160 && <p className="c-wait">Layout is limited to the first 160 of {graph.total} matching entities; hidden layers may reduce the visible count.</p>}
      </div>
      <aside className={s.detail} aria-label="Network inspector">
        {vertex ? <>
          <h3>{kindName(vertex.kind)}</h3><h2 className="mono">{vertex.name}</h2>
          <dl>{vertex.details.map(([name, value]) => <div key={name}><dt>{name}</dt><dd>{value}</dd></div>)}</dl>
          <p className="muted">{cohort.length} requests match the selected context. Add another entity to narrow the same network.</p>
          <h3>Connected to</h3><div className={s.connections}>{graph.edges.filter((e) => e.source === vertex.id || e.target === vertex.id).map((e) => { const other = graph.vertices.find((v) => v.id === (e.source === vertex.id ? e.target : e.source))!; return <button key={e.id} onClick={() => choose(other.id)}><span className="muted">{other.kind}</span> <Mono>{other.name}</Mono></button>; })}</div>
          {vertex.href && !demo && <a href={vertex.href}>Open {vertex.kind} →</a>}
        </> : trace ? <>
          <h3>{demo ? traceInFlight ? 'Simulated request · in flight' : 'Simulated request' : 'Recorded request'}</h3><h2 className="mono">{trace.project}</h2>
          <div className={s.participants}>{[...requestVertices(trace, fullGraph)].map(id => { const v = fullGraph.vertices.find(v => v.id === id)!; return <button key={id} onClick={() => choose(id)}><span>{kindName(v.kind)}</span>{v.name}</button>; })}</div>
          <dl>{[['Request ID',trace.id], ['Requested model',trace.requested_model], ['Served model',trace.served?.model ?? 'not attributed'], ['Outcome',traceInFlight ? 'in flight' : trace.status], ['Input / output',traceInFlight ? 'available on completion' : `${count(trace.input_tokens)} / ${count(trace.output_tokens)} tokens`], ['Wall / queue / load',traceInFlight ? 'available on completion' : `${trace.timing.wall_ms} / ${trace.timing.queue_ms} / ${trace.timing.load_ms} ms`], ['Price',traceInFlight ? 'available on completion' : micros(trace.price_micros,trace.currency)], ['Recorded',trace.created_at]].map(([name,value]) => <div key={name}><dt>{name}</dt><dd>{value}</dd></div>)}</dl>
          {!demo && <a href={`#/usage/${encodeURIComponent(trace.id)}`}>Open usage record →</a>}
        </> : <>
          <h3>One connected system</h3><h2>Follow the work.</h2>
          {!demo && <p className={s.small}>Only reported participants appear. API-instance identity is not exposed by UsageRecord yet; repository and other metadata are caller-supplied labels.</p>}
          <p className="muted">The whole fleet shares one field. Directional forces spread execution toward the models, while related entities settle around each other. Models are execution targets; projects, repositories, accounts and other context stay attached to the same request.</p>
          <div className={s.key}><span className="c-active">●</span><span>Reported in-flight work or a newly completed request</span><span className="c-wait">◐</span><span>Paused, pending or waiting</span><span className="c-fail">✕</span><span>Failed, down or exhausted</span></div>
          <p className="muted">Click any combination of entities to find their shared requests. Search includes every reported dimension. Optional context appears when supplied; selecting it never rearranges the network.</p>
          <p className={s.small}>{demo ? 'Homelab-inspired demo: studio1–3, h1, h2 and m1. Seats, models and activity are illustrative; no hosts are contacted.' : 'Links show configuration and sampled usage, not routing eligibility. No hardware telemetry is inferred.'}</p>
        </>}
      </aside>
    </div>
    <section className={s.activity} aria-label="Recent network activity">
      <div className={s.activityTitle}><h3>{demo ? 'Simulated completions' : 'Recent usage'}</h3><span className="muted">{demo ? paused ? 'paused' : 'up to 3 projects in flight' : 'latest records · select to trace'}</span></div>
      {visibleRecords.length === 0 ? <p className="muted">No matching usage has been reported.</p> : visibleRecords.map((r) => <button key={r.id} className={`${s.request} ${trace?.id === r.id ? s.requestSelected : ''}`} onClick={() => { setTrace(r); setSelectedIds([]); setHovered(null); }} aria-label={`Trace ${r.id}: ${r.project}, ${r.requested_model}, ${r.status}`}>
        <span className={r.status === 'served' ? 'c-active' : 'c-fail'}>{r.status === 'served' ? '●' : '✕'} {r.status}</span><span className="mono">{r.project}</span><span className="mono">{r.requested_model}</span><span className="muted">{r.served?.seat ?? 'no seat'}</span><span>{count(r.output_tokens)} out</span><span className="muted">{ago(r.created_at)} ago</span>
      </button>)}
    </section>
  </>;
}

function Marker({ kind }: { kind: Kind }) {
  return <svg viewBox="0 0 40 40" aria-hidden="true">{kind.startsWith('context:') ? <path d="M20 7L33 30H7Z" /> : kind === 'repository' ? <path d="M8 12H18L22 16H33V30H8Z" /> : kind === 'api' ? <path d="M20 5L34 13V27L20 35L6 27V13Z" /> : kind === 'node' ? <rect x="8" y="8" width="24" height="24" /> : kind === 'account' ? <path d="M20 7L33 20L20 33L7 20Z" /> : kind === 'project' ? <rect x="12" y="12" width="16" height="16" /> : <circle cx="20" cy="20" r={kind === 'seat' ? 6 : 11} />}</svg>;
}

function Constellation({ layout, graph, selected, focusedIds, focusedEdges, pulses, paused, onSelect, onHover }: { layout: Graph; graph: Graph; selected: string[]; focusedIds: Set<string>; focusedEdges: Set<string>; pulses: Pulse[]; paused: boolean; onSelect: (id: string) => void; onHover: (id: string | null) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 700, height: 610 });
  useEffect(() => { const el = ref.current!; const observer = new ResizeObserver(([entry]) => { if (entry) setSize({ width: Math.max(280, entry.contentRect.width), height: entry.contentRect.height }); }); observer.observe(el); return () => observer.disconnect(); }, []);
  // State/occupancy changes do not rerun layout. Only topology and viewport do.
  const topology = layout.vertices.map((v) => `${v.id}/${v.parent ?? ''}`).join(',') + layout.edges.map((e) => `${e.id}/${e.source}/${e.kind}/${e.flow}`).sort().join(',');
  const [pins, setPins] = useState<Map<string, Point>>(() => new Map());
  const drag = useRef<{ id: string; startX: number; startY: number; offsetX: number; offsetY: number; moved: boolean } | null>(null);
  const skipClick = useRef(false);
  const settled = useRef<{ points: Map<string, Point>; size: typeof size } | undefined>(undefined);
  const arrowId = useId();
  const points = useMemo(() => {
    const last = settled.current;
    const previous = last ? resizeLayout(last.points, { x: last.size.width, y: last.size.height }, { x: size.width, y: size.height }) : undefined;
    return layoutGraph(layout, size.width, size.height, pins, previous);
  }, [topology,size.width,size.height,pins]);
  useEffect(() => { settled.current = { points, size }; }, [points,size]);
  const pin = (id: string, point: Point) => setPins(old => new Map(old).set(id, point));
  const activeEdges = new Map<string, boolean>();
  for (const p of pulses) for (const id of requestEdges(p.record, graph)) activeEdges.set(id, !p.inFlight && p.record.status !== 'served');
  const litIds = new Set(pulses.flatMap(p => [...requestVertices(p.record, graph)]));
  const failedIds = new Set(graph.edges.filter((e) => activeEdges.get(e.id) === true).flatMap((e) => [e.source, e.target]));
  const focused = focusedIds.size > 0;
  return <>
    <div className={s.layoutTools}><span>NEURAL LAYERS <span className="muted">/ drag to arrange · select to inspect</span></span><button onClick={() => { settled.current = undefined; setPins(new Map()); }}>Reset layout{pins.size ? ` · ${pins.size} pinned` : ''}</button></div>
    <p className={s.dragHint}>Flow moves toward models → · drag to pin · arrow keys to move · Escape to release</p>
    <div className={s.mapScroll}><div ref={ref} className={s.map} style={{ minHeight: Math.max(740, (layout.vertices.filter(v => v.kind === 'model').length - 1) * 32 + 160) }} aria-label="Fleet relationship graph">
    <div className={s.flowGuides} aria-hidden="true"><span>CONTEXT</span><span>PROJECT → API → HOST → SEAT</span><span>MODELS / INFERENCE</span></div>
    <svg className={s.wires} width="100%" height="100%" viewBox={`0 0 ${size.width} ${size.height}`} aria-hidden="true">
      <defs><marker id={arrowId} markerWidth="8" markerHeight="8" refX="25" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path d="M0 0L8 4L0 8Z" fill="#80bdcf" /></marker></defs>
      {graph.edges.map((e) => <path markerEnd={e.flow ? `url(#${arrowId})` : undefined} key={e.id} className={`${s.wire} ${e.flow ? s.flowWire : s.observed} ${focused && !focusedEdges.has(e.id) ? s.dim : ''} ${focusedEdges.has(e.id) ? s.focusWire : ''}`} d={edgePath(points.get(e.source)!, points.get(e.target)!, e.id, e.kind === 'hosts')} />)}
      {pulses.flatMap((p) => graph.edges.filter((e) => requestEdges(p.record, graph).has(e.id)).map((e) => <path key={`${p.record.id}:${e.id}`} style={{ stroke: p.color, color: p.color, animationPlayState: paused ? 'paused' : 'running' }} d={edgePath(points.get(e.source)!, points.get(e.target)!, e.id, e.kind === 'hosts')} className={`${requestFlowEdges(p.record, graph).has(e.id) ? s.signal : s.contextSignal} ${p.inFlight ? s.inFlightSignal : p.record.status !== 'served' ? s.failedSignal : ''} ${focused && !focusedEdges.has(e.id) ? s.dim : ''}`} />))}
    </svg>
    {graph.vertices.map((v) => {
      const p = points.get(v.id)!;
      const bad = ['down', 'failed', 'exhausted', 'refused', 'revoked'].includes(v.state);
      const waiting = ['paused', 'pending', 'starting', 'offline', 'disabled'].includes(v.state);
      const active = v.active > 0 || litIds.has(v.id);
      return <button key={v.id} className={`${s.entity} ${s[v.kind] ?? s.context} ${bad || failedIds.has(v.id) ? s.bad : waiting ? s.waiting : active ? s.active : ''} ${selected.includes(v.id) ? s.selected : ''} ${focusedIds.has(v.id) ? s.fused : ''} ${focused && !focusedIds.has(v.id) ? s.dim : ''}`} style={{ left: p.x, top: p.y }} onClick={() => { if (skipClick.current) { skipClick.current = false; return; } onSelect(v.id); }}
        onPointerDown={e => {
          if (e.button !== 0) return;
          skipClick.current = false;
          const rect = ref.current!.getBoundingClientRect();
          drag.current = { id: v.id, startX: e.clientX, startY: e.clientY, offsetX: e.clientX - rect.left - p.x, offsetY: e.clientY - rect.top - p.y, moved: false };
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerMove={e => {
          const d = drag.current;
          if (!d || d.id !== v.id) return;
          if (!d.moved && Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < 5) return;
          d.moved = true;
          const rect = ref.current!.getBoundingClientRect();
          pin(v.id, { x: e.clientX - rect.left - d.offsetX, y: e.clientY - rect.top - d.offsetY });
        }}
        onPointerUp={e => { skipClick.current = drag.current?.moved ?? false; drag.current = null; e.currentTarget.releasePointerCapture(e.pointerId); }}
        onPointerCancel={() => { drag.current = null; skipClick.current = false; }}
        onKeyDown={e => {
          const moves: Record<string, Point> = { ArrowLeft: { x: -20, y: 0 }, ArrowRight: { x: 20, y: 0 }, ArrowUp: { x: 0, y: -20 }, ArrowDown: { x: 0, y: 20 } };
          const move = moves[e.key];
          if (move) { e.preventDefault(); pin(v.id, { x: p.x + move.x, y: p.y + move.y }); }
          if (e.key === 'Escape') setPins(old => { const next = new Map(old); next.delete(v.id); return next; });
        }}
        onMouseEnter={() => onHover(v.id)} onMouseLeave={() => onHover(null)} onFocus={() => onHover(v.id)} onBlur={() => onHover(null)} aria-label={`${v.kind} ${v.kind === 'seat' ? v.id.slice(5) : v.name}, ${v.state}${v.active ? `, ${v.active} in flight` : ''}`} aria-pressed={selected.includes(v.id)}>
        <Marker kind={v.kind} />{pins.has(v.id) && <span className={s.pinMark} aria-label="Pinned">⌖</span>}<span className={`${s.entityLabel} ${v.kind === 'seat' && !focusedIds.has(v.id) ? s.quietLabel : ''}`}>{v.name.length > 18 ? `${v.name.slice(0, 16)}…` : v.name}</span>{(bad || waiting) && <span className={s.stateLabel}>{bad ? '✕' : '◐'} {v.state}</span>}
      </button>;
    })}
    </div></div>
  </>;
}
