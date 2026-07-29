// iris render/web — the flower over real fused state.
//
// Design (DESIGN §11: an instrument, not a scene):
//   LAYOUT    layered left->right by observed data flow (the
//             pipeline IS the x-axis), quiet strip for unwired
//             processes, smooth reflow on topology change.
//   HIERARCHY structure is slate and quiet; color is EARNED by
//             activity: amber = emitting, cyan = reacting,
//             ribbon color = latency, red = alerts, white =
//             focus. Plumbing (heartbeat/ctl.*) is hairlines.
//   LABELS    on demand: at rest only flower names + the few
//             hottest loci; hover/pin a flower or ribbon for
//             everything else. No labels stamped over labels.
//   ALTITUDE  wheel zoom + drag pan; 30fps cap; rates derived
//             client-side from consecutive SSE snapshots.

const canvas = document.getElementById("c");
const ctx = canvas.getContext("2d");
const connEl = document.getElementById("conn");
const statsEl = document.getElementById("stats");
const topicsEl = document.getElementById("topics");
const eventsEl = document.getElementById("events");
const tipEl = document.getElementById("tip");

const FRAME_MS = 1000 / 30;
const SLATE = "148,163,184";
const AMBER = "252,190,88";
const CYAN = "94,220,244";
const RED = "248,81,73";

let snap = null, prev = null;
const rates = { edges: new Map(), procs: new Map(), loci: new Map(), topics: new Map() };
let lastDraw = 0, lastFrameT = 0;
let showPlumbing = false;
let dpr = 1;

// view transform (world -> screen: s = w*k + o)
const view = { k: 1, x: 0, y: 0 };
let mouse = { x: 0, y: 0, wx: 0, wy: 0, down: false, dragged: false };
let hover = null;          // {type:'flower',pid} | {type:'ribbon',key}
let pinned = null;         // same shape, click-to-pin
let hoverTopic = null, pinnedTopic = null;
const deadSince = new Map();

// topic-family palette: hues for the bus planes (md.*, risk.*,
// strategy.*, ...). 8 muted-but-distinct hues on dark; red is
// reserved for alerts, amber/cyan for petal activity accents.
const FAM_HUES = [210, 265, 320, 165, 130, 95, 20, 240];
const famOf = (t) => t.split(".")[0];
function famHue(fam) {
  let h = 0;
  for (const ch of fam) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return FAM_HUES[h % FAM_HUES.length];
}
const famColor = (fam, a) => `hsla(${famHue(fam)}, 60%, 58%, ${a})`;
let colorMode = "topic"; // 'topic' | 'latency' (key c)

const fmt = (n) =>
  n >= 1e6 ? (n / 1e6).toFixed(1) + "M" : n >= 1e3 ? (n / 1e3).toFixed(1) + "k" : Math.round(n);
const isPlumbing = (t) => t === "heartbeat" || t.startsWith("ctl.");

function resize() {
  dpr = window.devicePixelRatio || 1;
  canvas.width = innerWidth * dpr;
  canvas.height = innerHeight * dpr;
}
addEventListener("resize", resize);
resize();

// ---- ingest ------------------------------------------------

function ingest(s) {
  prev = snap;
  snap = s;
  if (!prev || !snap) return;
  const dt = (snap.ts - prev.ts) / 1e9;
  if (dt <= 0) return;
  const pe = new Map(prev.edges.map(e => [e.topic + e.from + e.to, e]));
  for (const e of snap.edges) {
    const p = pe.get(e.topic + e.from + e.to);
    rates.edges.set(e.topic + e.from + e.to, p ? Math.max(0, (e.matched - p.matched) / dt) : 0);
  }
  const pt = new Map((prev.topics || []).map(tp => [tp.name, tp]));
  for (const tp of snap.topics || []) {
    const o = pt.get(tp.name);
    const inst = o ? Math.max(0, ((tp.dlv - o.dlv) + (tp.pub - o.pub)) / 2 / dt) : 0;
    const ema = rates.topics.get(tp.name) || 0;
    rates.topics.set(tp.name, ema * 0.8 + inst * 0.2);
  }
  const pp = new Map(prev.processes.map(p => [p.pid, p]));
  for (const p of snap.processes) {
    const q = pp.get(p.pid);
    rates.procs.set(p.pid, q ? Math.max(0, (p.records - q.records) / dt) : 0);
    const prevLoci = new Map((q?.loci || []).map(l => [l.id, l]));
    for (const l of p.loci || []) {
      const o = prevLoci.get(l.id);
      rates.loci.set(p.pid + ":" + l.id, {
        pub: o ? Math.max(0, (l.pub - o.pub) / dt) : 0,
        dlv: o ? Math.max(0, (l.dlv - o.dlv) / dt) : 0,
      });
    }
  }
}

function connect() {
  const es = new EventSource("/events");
  es.onopen = () => { connEl.textContent = "live"; connEl.className = "live"; };
  es.onerror = () => { connEl.textContent = "reconnecting…"; connEl.className = "dead"; };
  es.onmessage = (m) => { try { ingest(JSON.parse(m.data)); } catch (e) {} };
}
connect();

// ---- pair ribbons ------------------------------------------
// One ribbon per (from,to) process pair; per-topic detail lives
// in the tooltip. Plumbing pairs render as hairlines.

function buildRibbons() {
  const ribbons = new Map(), plumbs = new Map();
  for (const e of snap.edges || []) {
    const key = e.from + ">" + e.to;
    const bucket = isPlumbing(e.topic) ? plumbs : ribbons;
    if (!bucket.has(key))
      bucket.set(key, { key, from: e.from, to: e.to, topics: [], rate: 0, latW: 0, lost: 0 });
    const r = bucket.get(key);
    const er = rates.edges.get(e.topic + e.from + e.to) || 0;
    const mean = e.matched ? e.latSum / e.matched / 1000 : 0;
    r.topics.push({ name: e.topic, rate: er, mean, matched: e.matched,
                    lost: Math.max(0, e.sends - e.delivers) });
    r.rate += er;
    r.latW += mean * er;
    r.lost += Math.max(0, e.sends - e.delivers);
  }
  for (const r of ribbons.values()) {
    r.mean = r.rate > 0 ? r.latW / r.rate
      : r.topics.reduce((a, t) => a + t.mean, 0) / Math.max(1, r.topics.length);
    r.topics.sort((a, b) => b.rate - a.rate);
    r.fam = famOf(r.topics[0]?.name || "?");
  }
  return { ribbons: [...ribbons.values()], plumbs: [...plumbs.values()] };
}

// ---- layered layout ----------------------------------------
// x = layer (longest path over non-plumbing pair edges),
// y = barycenter-ordered, packed by flower radius. Unwired
// processes sit in a quiet strip along the bottom. Positions
// lerp toward targets; layout recomputes on topology change.

const nodePos = new Map();   // pid -> {x,y,tx,ty,r}
let topoSig = "";

function flowerRadius(p) {
  const n = (p.loci || []).length;
  if (n === 0) return 26;
  const ring = n <= 8 ? 40 : n <= 16 ? 32 : 24;
  const depth = Math.min(3, 1 + (n > 1 ? 1 : 0) + (n > 12 ? 1 : 0));
  return 26 + ring * depth * 0.9;
}

function relayout(ribbons) {
  const procs = (snap.processes || []).filter(p => p.state === "live");
  const pids = procs.map(p => p.pid);
  const idset = new Set(pids);
  const adj = new Map(pids.map(p => [p, []]));
  const radj = new Map(pids.map(p => [p, []]));
  for (const r of ribbons) {
    if (!idset.has(r.from) || !idset.has(r.to) || r.from === r.to) continue;
    adj.get(r.from).push(r.to);
    radj.get(r.to).push(r.from);
  }
  const wired = new Set();
  for (const r of ribbons) { wired.add(r.from); wired.add(r.to); }

  // longest-path layering with cycle guard
  const layer = new Map();
  const state = new Map();
  const depth = (p, stack) => {
    if (layer.has(p)) return layer.get(p);
    if (stack.has(p)) return 0;
    stack.add(p);
    let d = 0;
    for (const q of radj.get(p) || []) d = Math.max(d, depth(q, stack) + 1);
    stack.delete(p);
    layer.set(p, d);
    return d;
  };
  for (const p of pids) if (wired.has(p)) depth(p, new Set());

  const layers = [];
  for (const p of pids) {
    if (!wired.has(p)) continue;
    const d = layer.get(p) || 0;
    (layers[d] = layers[d] || []).push(p);
  }
  // barycenter ordering, two sweeps
  const orderOf = new Map();
  layers.forEach(L => L.forEach((p, i) => orderOf.set(p, i)));
  for (let sweep = 0; sweep < 2; sweep++) {
    for (let li = 0; li < layers.length; li++) {
      const L = layers[li];
      L.sort((a, b) => {
        const ma = meanOrder(a), mb = meanOrder(b);
        return ma - mb || a - b;
      });
      L.forEach((p, i) => orderOf.set(p, i));
    }
  }
  function meanOrder(p) {
    const ns = [...(radj.get(p) || []), ...(adj.get(p) || [])];
    if (!ns.length) return orderOf.get(p) || 0;
    return ns.reduce((s, q) => s + (orderOf.get(q) || 0), 0) / ns.length;
  }

  const byPid = new Map(procs.map(p => [p.pid, p]));
  const W = innerWidth, H = innerHeight;
  const nL = Math.max(1, layers.length);
  const colW = Math.max(230, (W - 200) / nL);
  const x0 = 130;
  layers.forEach((L, li) => {
    const rs = L.map(p => flowerRadius(byPid.get(p)));
    const gap = 56;
    const total = rs.reduce((a, r) => a + 2 * r + gap, -gap);
    let cy = Math.max(90, (H - 130) / 2 - total / 2);
    L.forEach((p, i) => {
      const r = rs[i];
      setTarget(p, x0 + li * colW + colW / 2, cy + r, r);
      cy += 2 * r + gap;
    });
  });
  // quiet strip: unwired, small, along the bottom
  const quiet = pids.filter(p => !wired.has(p));
  quiet.forEach((p, i) => {
    setTarget(p, 150 + i * 150, H - 70, Math.min(30, flowerRadius(byPid.get(p))));
  });
}

function setTarget(pid, tx, ty, r) {
  const n = nodePos.get(pid);
  if (n) { n.tx = tx; n.ty = ty; n.r = r; }
  else nodePos.set(pid, { x: tx, y: ty, tx, ty, r });
}

function layoutTick(ribbons, dt) {
  const sig = (snap.processes || []).filter(p => p.state === "live").map(p => p.pid).sort().join(",")
    + "|" + ribbons.map(r => r.key).sort().join(",");
  if (sig !== topoSig) { topoSig = sig; relayout(ribbons); }
  const a = Math.min(1, dt * 3.5);
  for (const n of nodePos.values()) {
    n.x += (n.tx - n.x) * a;
    n.y += (n.ty - n.y) * a;
  }
}

// ---- flower internals (tree + aggregation, from v1) --------

function treeLayout(loci, cx, cy, ring) {
  const byId = new Map(loci.map(l => [l.id, l]));
  const kids = new Map(); const roots = [];
  for (const l of loci) {
    if (l.parent && byId.has(l.parent)) {
      if (!kids.has(l.parent)) kids.set(l.parent, []);
      kids.get(l.parent).push(l);
    } else roots.push(l);
  }
  const pos = new Map();
  const place = (l, a0, a1, depth) => {
    const a = (a0 + a1) / 2, r = depth * ring;
    pos.set(l.id, { x: cx + Math.cos(a) * r, y: cy + Math.sin(a) * r, angle: a, depth });
    const c = kids.get(l.id) || [];
    c.forEach((k, i) => {
      const w = (a1 - a0) / c.length;
      place(k, a0 + i * w, a0 + (i + 1) * w, depth + 1);
    });
  };
  roots.forEach((rt, i) => {
    const w = Math.PI * 2 / Math.max(1, roots.length);
    place(rt, i * w - Math.PI / 2, (i + 1) * w - Math.PI / 2, 0);
  });
  return pos;
}

const KEEP = 2, AGG_MIN = 5;
function condenseLoci(pid, loci) {
  const internal = loci.filter(l => l.type.startsWith("__"));
  loci = loci.filter(l => !l.type.startsWith("__"));
  const groups = new Map();
  for (const l of loci) {
    const k = l.parent + "|" + l.type;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(l);
  }
  const act = (id) => {
    const r = rates.loci.get(pid + ":" + id) || { pub: 0, dlv: 0 };
    return r.pub + r.dlv;
  };
  const out = [];
  for (const [k, members] of groups) {
    if (members.length < AGG_MIN) { out.push(...members); continue; }
    members.sort((x, y) => act(y.id) - act(x.id));
    out.push(...members.slice(0, KEEP));
    const rest = members.slice(KEEP);
    let pub = 0, dlv = 0;
    for (const m of rest) {
      const r = rates.loci.get(pid + ":" + m.id) || { pub: 0, dlv: 0 };
      pub += r.pub; dlv += r.dlv;
    }
    let hh = 0; for (const ch of k) hh = (hh * 31 + ch.charCodeAt(0)) % 100000;
    out.push({ id: -(hh + 1), type: members[0].type, parent: members[0].parent,
               agg: true, count: rest.length, aggPub: pub, aggDlv: dlv });
  }
  if (internal.length) {
    let pub = 0, dlv = 0;
    for (const m of internal) {
      const r = rates.loci.get(pid + ":" + m.id) || { pub: 0, dlv: 0 };
      pub += r.pub; dlv += r.dlv;
    }
    out.push({ id: -999999, type: "runtime", parent: 0, agg: true, internal: true,
               count: internal.length, aggPub: pub, aggDlv: dlv });
  }
  return out;
}

// ---- drawing helpers ---------------------------------------

const cbez = (u, x1, y1, cx1, cy1, cx2, cy2, x2, y2) => {
  const v = 1 - u;
  const a = v * v * v, b = 3 * v * v * u, c = 3 * v * u * u, d = u * u * u;
  return [a * x1 + b * cx1 + c * cx2 + d * x2,
          a * y1 + b * cy1 + c * cy2 + d * y2];
};

function ribbonGeom(r) {
  const f = nodePos.get(r.from), t = nodePos.get(r.to);
  if (!f || !t) return null;
  const x1 = f.x + f.r * 0.9, y1 = f.y, x2 = t.x - t.r * 0.9, y2 = t.y;
  if (t.x >= f.x) {
    const dx = Math.max(60, (x2 - x1) * 0.45);
    return [x1, y1, x1 + dx, y1, x2 - dx, y2, x2, y2];
  }
  // back-edge: arc below everything
  const dy = Math.max(120, Math.abs(y2 - y1) * 0.5 + 140);
  return [f.x, f.y + f.r * 0.9, f.x, f.y + dy, t.x, t.y + dy, t.x, t.y + t.r * 0.9];
}

const pulses = new Map();
function stepPulses(key, rate, dt) {
  let list = pulses.get(key);
  if (!list) { list = []; pulses.set(key, list); }
  for (const p of list) p.u += 0.5 * dt;
  while (list.length && list[0].u > 1) list.shift();
  const want = rate <= 0 ? 0 : Math.min(12, 1.5 + Math.log10(1 + rate) * 2.6);
  const gap = 1 / Math.max(want, 1);
  if (want > 0 && (!list.length || list[list.length - 1].u > gap)) list.push({ u: 0 });
  return list;
}

function latColor(meanUs, a) {
  const h = 130 - 90 * Math.min(1, meanUs / 200);
  return `hsla(${h}, 65%, 52%, ${a})`;
}

function dimFor(kind, id) {
  // focus model: when something is hovered/pinned, everything
  // else recedes. Returns an alpha multiplier.
  const f = pinned || hover;
  const topicSel = pinnedTopic || hoverTopic;
  if (!f && !topicSel) return 1;
  if (topicSel) {
    if (kind === "ribbon") return id.topics.some(t => t.name === topicSel) ? 1 : 0.12;
    if (kind === "flower") {
      const touches = (snap.edges || []).some(e => e.topic === topicSel && (e.from === id || e.to === id));
      return touches ? 1 : 0.22;
    }
    return 0.4;
  }
  if (f.type === "flower") {
    if (kind === "flower") return id === f.pid ? 1 : 0.22;
    if (kind === "ribbon") return (id.from === f.pid || id.to === f.pid) ? 1 : 0.10;
  }
  if (f.type === "ribbon") {
    if (kind === "ribbon") return id.key === f.key ? 1 : 0.10;
    if (kind === "flower") {
      const [a, b] = f.key.split(">").map(Number);
      return (id === a || id === b) ? 1 : 0.22;
    }
  }
  return 1;
}

// ---- flower ------------------------------------------------

function drawFlower(p, t, focus) {
  const n = nodePos.get(p.pid);
  if (!n) return;
  const { x: cx, y: cy } = n;
  const dim = dimFor("flower", p.pid);
  const dead = p.state === "dead";
  const loci = condenseLoci(p.pid, p.loci || []);
  const nLoci = loci.length;
  const ring = nLoci <= 8 ? 40 : nLoci <= 16 ? 32 : 24;
  const pos = treeLayout(loci, cx, cy, ring);
  const focused = focus || (hover?.type === "flower" && hover.pid === p.pid)
    || (pinned?.type === "flower" && pinned.pid === p.pid);

  ctx.globalAlpha = dim;

  // stems
  ctx.lineWidth = 1;
  ctx.strokeStyle = `rgba(${SLATE}, ${dead ? 0.10 : 0.20})`;
  for (const l of loci) {
    const a = pos.get(l.parent), b = pos.get(l.id);
    if (!a || !b) continue;
    ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
  }

  // petals: slate at rest, colored only by activity
  const hot = [];
  for (const l of loci) {
    const q = pos.get(l.id);
    if (!q || q.depth === 0) continue;
    const act = l.agg ? { pub: l.aggPub, dlv: l.aggDlv }
      : rates.loci.get(p.pid + ":" + l.id) || { pub: 0, dlv: 0 };
    const tx = Math.min(1, Math.log10(1 + act.pub) / 4);
    const rx = Math.min(1, Math.log10(1 + act.dlv) / 4);
    if (tx + rx > 0.02) hot.push({ l, q, v: act.pub + act.dlv });
    const actScale = 1 + 0.18 * Math.max(tx, rx);
    const scale = (l.agg ? 1 + Math.min(1.2, Math.log2(1 + l.count) / 4) : 1) * actScale;
    const pw = (ring * 0.42) * scale, ph = (ring * 0.16 + 2) * scale;
    const bloom = dead ? 0 : 0.5 + 0.5 * Math.sin(t * 1.6 + (l.id % 97) * 1.7);

    ctx.save();
    ctx.translate(q.x, q.y);
    ctx.rotate(q.angle);
    // emit halo (amber) under the petal
    if (!dead && tx > 0.02) {
      ctx.beginPath();
      ctx.ellipse(pw * 0.55, 0, pw * 1.15, ph * 1.9, 0, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(${AMBER}, ${0.10 + 0.22 * tx})`;
      ctx.fill();
    }
    ctx.beginPath();
    ctx.ellipse(pw * 0.55, 0, pw * (0.9 + 0.1 * bloom), ph * (0.9 + 0.25 * bloom), 0, 0, Math.PI * 2);
    if (l.internal) ctx.fillStyle = `rgba(${SLATE}, 0.07)`;
    else if (dead) ctx.fillStyle = `rgba(${SLATE}, 0.08)`;
    else {
      const warm = Math.min(0.55, tx * 0.7);
      ctx.fillStyle = warm > 0.05
        ? `rgba(${AMBER}, ${0.10 + warm * 0.5})`
        : `rgba(${SLATE}, ${0.13 + 0.05 * bloom})`;
    }
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = l.internal ? `rgba(${SLATE},0.10)` : `rgba(${SLATE},${dead ? 0.12 : 0.28})`;
    ctx.stroke();
    // react ring (cyan), flicker rises with load
    if (!dead && rx > 0.02) {
      const flick = 0.55 + 0.45 * Math.sin(t * (5 + 9 * rx) + (l.id % 31));
      ctx.lineWidth = 1.2 + 1.6 * rx;
      ctx.strokeStyle = `rgba(${CYAN}, ${(0.25 + 0.6 * rx) * flick})`;
      ctx.stroke();
    }
    ctx.restore();
  }

  // core
  const rec = rates.procs.get(p.pid) || 0;
  const pulse = dead ? 0 : Math.min(1, rec / 300000);
  ctx.beginPath();
  ctx.arc(cx, cy, 9 + 3 * pulse, 0, Math.PI * 2);
  ctx.fillStyle = dead ? "rgba(60,34,38,0.8)" : `hsl(${145 - 60 * pulse}, 45%, ${34 + 14 * pulse}%)`;
  ctx.fill();
  ctx.strokeStyle = dead ? `rgba(${RED},0.7)` : "rgba(28,36,48,1)";
  ctx.lineWidth = 2;
  ctx.stroke();

  // labels: SCREEN-constant size (divide by zoom — zoom grows
  // geometry, not text), side-anchored at petal tips so they
  // never cross the flower, instance ids only when the type is
  // duplicated within this flower. Semantic zoom: a flower
  // filling enough screen reveals all its labels unaided.
  const fs = (px) => `${(px / view.k).toFixed(2)}px ui-monospace, monospace`;
  ctx.textAlign = "center";
  ctx.font = "600 " + fs(12);
  ctx.fillStyle = dead ? `rgba(${RED},0.8)` : "rgba(230,237,243,0.92)";
  ctx.fillText(p.name || "pid " + p.pid, cx, cy + n.r + 16 / view.k);
  const bits = [];
  if (!dead && rec > 500) bits.push(fmt(rec) + "/s");
  if (p.restarts > 0) bits.push(p.restarts + " restarts");
  if (bits.length) {
    ctx.font = fs(10);
    ctx.fillStyle = p.restarts > 0 ? `rgba(${RED},0.75)` : `rgba(${SLATE},0.6)`;
    ctx.fillText(bits.join(" · "), cx, cy + n.r + 29 / view.k);
  }
  const typeCount = new Map();
  for (const l of loci) typeCount.set(l.type, (typeCount.get(l.type) || 0) + 1);
  const nameOf = (l) => l.agg ? `${l.type} ×${l.count}`
    : (typeCount.get(l.type) > 1 ? `${l.type}#${l.id}` : l.type);
  hot.sort((a, b) => b.v - a.v);
  const zoomed = n.r * view.k > 130;           // semantic zoom
  const showAll = focused || zoomed;
  const labelSet = showAll
    ? loci.map(l => ({ l, q: pos.get(l.id) })).filter(e => e.q && e.q.depth > 0)
    : (view.k < 0.55 ? [] : hot.slice(0, 3));  // fleet altitude: names only
  ctx.font = fs(9.5);
  for (const { l, q } of labelSet) {
    const tipR = q.depth * ring + ring * 0.55 + 6 / view.k;
    const dx = Math.cos(q.angle), dy = Math.sin(q.angle);
    const lx = cx + dx * tipR, ly = cy + dy * tipR;
    ctx.textAlign = dx > 0.3 ? "left" : dx < -0.3 ? "right" : "center";
    const voff = Math.abs(dx) <= 0.3 ? (dy > 0 ? 9 : -4) / view.k : 3 / view.k;
    ctx.fillStyle = showAll ? `rgba(${SLATE},0.85)` : `rgba(${SLATE},0.55)`;
    ctx.fillText(nameOf(l), lx, ly + voff);
  }
  ctx.globalAlpha = 1;
}

// ---- frame -------------------------------------------------

function frame(now) {
  requestAnimationFrame(frame);
  if (now - lastDraw < FRAME_MS) return;
  const dt = Math.min(0.1, (now - lastFrameT) / 1000 || 0.033);
  lastDraw = now; lastFrameT = now;
  const t = now / 1000;

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, innerWidth, innerHeight);
  if (!snap) return;

  const { ribbons, plumbs } = buildRibbons();
  layoutTick(ribbons, dt);

  ctx.setTransform(dpr * view.k, 0, 0, dpr * view.k, dpr * view.x, dpr * view.y);

  // ghosts age out
  const procs = (snap.processes || []).filter(p => {
    if (p.state !== "dead") { deadSince.delete(p.pid); return true; }
    if (!deadSince.has(p.pid)) deadSince.set(p.pid, now);
    return now - deadSince.get(p.pid) < 60000;
  });

  // plumbing hairlines (toggle 'h')
  if (showPlumbing) {
    ctx.lineWidth = 0.6;
    for (const r of plumbs) {
      const g = ribbonGeom(r);
      if (!g) continue;
      ctx.beginPath();
      ctx.moveTo(g[0], g[1]);
      ctx.bezierCurveTo(g[2], g[3], g[4], g[5], g[6], g[7]);
      ctx.strokeStyle = `rgba(${SLATE}, 0.10)`;
      ctx.stroke();
    }
  }

  // ribbons: volume must read across ORDERS OF MAGNITUDE, so
  // width, brightness, glow, and pulse size all step with
  // log10(rate): 1/s, 100/s, and 10k/s are three different
  // animals at a glance. Color stays latency.
  for (const r of ribbons) {
    const g = ribbonGeom(r);
    if (!g) continue;
    const dim = dimFor("ribbon", r);
    const tier = r.rate <= 0 ? 0 : Math.log10(1 + r.rate); // 0..~5
    const sel = (pinned?.type === "ribbon" && pinned.key === r.key)
      || (hover?.type === "ribbon" && hover.key === r.key);
    ctx.globalAlpha = dim;
    ctx.beginPath();
    ctx.moveTo(g[0], g[1]);
    ctx.bezierCurveTo(g[2], g[3], g[4], g[5], g[6], g[7]);
    const col = (al) => colorMode === "topic" ? famColor(r.fam, al) : latColor(r.mean, al);
    // heavy flows get an under-glow first
    if (tier > 2.2) {
      ctx.lineWidth = 2.5 + 3.2 * tier;
      ctx.strokeStyle = col(0.05 + 0.03 * tier);
      ctx.stroke();
    }
    ctx.lineWidth = (tier <= 0 ? 0.7 : 1.1 + 1.9 * tier) + (sel ? 1 : 0);
    ctx.strokeStyle = sel ? "rgba(230,237,243,0.8)"
      : col(tier <= 0 ? 0.12 : Math.min(0.7, 0.22 + 0.11 * tier));
    ctx.stroke();
    // latency ALERT: a slow path earns red regardless of mode
    if (r.mean > 300 && r.rate > 0) {
      ctx.save();
      ctx.setLineDash([6, 8]);
      ctx.lineWidth = 1.2;
      ctx.strokeStyle = `rgba(${RED}, 0.55)`;
      ctx.stroke();
      ctx.restore();
    }
    for (const p of stepPulses(r.key, r.rate, dt)) {
      const [px, py] = cbez(p.u, ...g);
      const fade = Math.sin(Math.PI * Math.min(1, Math.max(0, p.u)));
      ctx.beginPath();
      ctx.arc(px, py, 1.6 + 1.1 * tier, 0, Math.PI * 2);
      ctx.fillStyle = sel ? `rgba(230,237,243,${0.9 * fade})` : col(0.9 * fade);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  for (const p of procs) drawFlower(p, t, false);

  // HUD
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const live = procs.filter(p => p.state === "live");
  const totRec = live.reduce((a, p) => a + (rates.procs.get(p.pid) || 0), 0);
  statsEl.textContent =
    `${live.length} processes · ${fmt(totRec)} records/s · ${ribbons.length} flows`;
  renderTopics();
  eventsEl.innerHTML = (snap.events || [])
    .filter(x => !x.includes("__")).slice(-5).map(x => `<div>${x}</div>`).join("");
}
requestAnimationFrame(frame);

// ---- topic panel -------------------------------------------

let topicsHtml = "";
let topicOrder = [], lastRank = 0;
function renderTopics() {
  const now = performance.now();
  const active = (snap.topics || []).filter(tp => tp.pub > 0 || tp.dlv > 0);
  if (now - lastRank > 5000 || !topicOrder.length) {
    lastRank = now;
    topicOrder = active.slice()
      .sort((a, b) => (rates.topics.get(b.name) || 0) - (rates.topics.get(a.name) || 0)
                   || (b.pub + b.dlv) - (a.pub + a.dlv))
      .map(tp => tp.name);
  }
  const pos = new Map(topicOrder.map((n, i) => [n, i]));
  const rows = active
    .sort((a, b) => (pos.has(a.name) ? pos.get(a.name) : 999) - (pos.has(b.name) ? pos.get(b.name) : 999))
    .map(tp => {
      const cls = tp.name === pinnedTopic ? "trow pinned" : "trow";
      const rt = rates.topics.get(tp.name) || 0;
      const rate = rt >= 0.5
        ? `<span style="color:#9ecbff">${fmt(rt)}/s</span>`
        : `<span class="z">idle</span>`;
      const dot = `<span style="color:${famColor(famOf(tp.name), 0.9)}">●</span>`;
      return `<div class="${cls}" data-t="${tp.name}">${dot} ${tp.name} ${rate} <span class="z">· ${fmt(Math.max(tp.pub, tp.dlv))}</span></div>`;
    }).join("");
  if (rows !== topicsHtml) { topicsHtml = rows; topicsEl.innerHTML = rows; }
}
topicsEl.addEventListener("mouseover", (e) => {
  hoverTopic = e.target.closest(".trow")?.dataset.t || null;
});
topicsEl.addEventListener("mouseout", () => { hoverTopic = null; });
topicsEl.addEventListener("click", (e) => {
  const t = e.target.closest(".trow")?.dataset.t;
  pinnedTopic = pinnedTopic === t ? null : (t || null);
});

// ---- interaction -------------------------------------------

function hitTest(wx, wy) {
  if (!snap) return null;
  for (const p of snap.processes || []) {
    const n = nodePos.get(p.pid);
    if (!n) continue;
    const d = Math.hypot(wx - n.x, wy - n.y);
    if (d < n.r + 14) return { type: "flower", pid: p.pid };
  }
  const { ribbons } = buildRibbons();
  let best = null, bestD = 9 / view.k + 3;
  for (const r of ribbons) {
    const g = ribbonGeom(r);
    if (!g) continue;
    for (let i = 1; i < 24; i++) {
      const [px, py] = cbez(i / 24, ...g);
      const d = Math.hypot(wx - px, wy - py);
      if (d < bestD) { bestD = d; best = { type: "ribbon", key: r.key, r }; }
    }
  }
  return best;
}

function tipFor(h) {
  if (h.type === "ribbon") {
    const r = h.r;
    const f = (snap.processes || []).find(p => p.pid === r.from);
    const t = (snap.processes || []).find(p => p.pid === r.to);
    const rows = r.topics.map(tp =>
      `<span style="color:${famColor(famOf(tp.name), 0.9)}">●</span> ${tp.name} — ${fmt(tp.rate)}/s · ${tp.mean.toFixed(0)}µs${tp.lost ? ` · <span style="color:rgb(${RED})">${fmt(tp.lost)} lost</span>` : ""}`
    ).join("<br>");
    return `<b>${f?.name || r.from} → ${t?.name || r.to}</b><br>${rows}`;
  }
  const p = (snap.processes || []).find(q => q.pid === h.pid);
  if (!p) return "";
  const rec = rates.procs.get(p.pid) || 0;
  return `<b>${p.name}</b> · pid ${p.pid}<br>` +
    `${(p.loci || []).length} loci · ${fmt(rec)} rec/s · ${p.restarts} restarts` +
    (p.state === "dead" ? `<br><span style="color:rgb(${RED})">dissolved</span>` : "");
}

canvas.addEventListener("mousemove", (e) => {
  mouse.wx = (e.clientX - view.x) / view.k;
  mouse.wy = (e.clientY - view.y) / view.k;
  if (mouse.down) {
    view.x += e.clientX - mouse.x; view.y += e.clientY - mouse.y;
    mouse.x = e.clientX; mouse.y = e.clientY;
    mouse.dragged = true;
    tipEl.style.display = "none";
    return;
  }
  mouse.x = e.clientX; mouse.y = e.clientY;
  hover = hitTest(mouse.wx, mouse.wy);
  canvas.style.cursor = hover ? "pointer" : "default";
  if (hover) {
    tipEl.innerHTML = tipFor(hover);
    tipEl.style.display = "block";
    tipEl.style.left = Math.min(innerWidth - 320, e.clientX + 14) + "px";
    tipEl.style.top = (e.clientY + 14) + "px";
  } else tipEl.style.display = "none";
});
canvas.addEventListener("mousedown", (e) => {
  mouse.down = true; mouse.dragged = false;
  mouse.x = e.clientX; mouse.y = e.clientY;
});
addEventListener("mouseup", () => {
  if (mouse.down && !mouse.dragged) {
    pinned = hover ? { ...hover } : null;
  }
  mouse.down = false;
});
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const k2 = Math.min(3, Math.max(0.3, view.k * Math.exp(-e.deltaY * 0.0012)));
  view.x = e.clientX - (e.clientX - view.x) * (k2 / view.k);
  view.y = e.clientY - (e.clientY - view.y) * (k2 / view.k);
  view.k = k2;
}, { passive: false });
canvas.addEventListener("dblclick", () => { view.k = 1; view.x = 0; view.y = 0; pinned = null; });
addEventListener("keydown", (e) => {
  if (e.key === "h") showPlumbing = !showPlumbing;
  if (e.key === "c") colorMode = colorMode === "topic" ? "latency" : "topic";
  if (e.key === "Escape") { pinned = null; pinnedTopic = null; }
});
