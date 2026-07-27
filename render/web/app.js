// iris render/web — the flower over real fused state.
// Data: SSE /events (10Hz snapshots of monotonic totals from
// fuse-hl). Rates are derived here from consecutive frames.
// Render: 2D canvas, frame-capped at 30fps (DESIGN §11: the
// observer must not be the CPU hog in the system it observes).
//
// Perspectives — same fused ground truth, different bases:
//   [1] process: one flower per pid; petals laid out by the
//       supervision tree (parent chains -> stems + depth rings)
//   [2] flow: topics are the bases; processes orbit the topics
//       they publish/consume, edges route through their topic

const canvas = document.getElementById("c");
const ctx = canvas.getContext("2d");
const connEl = document.getElementById("conn");
const statsEl = document.getElementById("stats");
const topicsEl = document.getElementById("topics");
const eventsEl = document.getElementById("events");

let snap = null, prev = null, rates = { edges: new Map(), procs: new Map(), loci: new Map() };
let lastDraw = 0, lastFrameT = 0;
let view = 1;
const FRAME_MS = 1000 / 30;

addEventListener("keydown", (e) => {
  if (e.key === "1") view = 1;
  if (e.key === "2") view = 2;
});

const hue = (s) => { let h = 0; for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) % 360; return h; };
const fmt = (n) =>
  n >= 1e6 ? (n / 1e6).toFixed(1) + "M" : n >= 1e3 ? (n / 1e3).toFixed(1) + "k" : Math.round(n);

function resize() {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = innerWidth * dpr;
  canvas.height = innerHeight * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}
addEventListener("resize", resize);
resize();

// ---- data ingestion ----------------------------------------

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
  const pp = new Map(prev.processes.map(p => [p.pid, p]));
  for (const p of snap.processes) {
    const q = pp.get(p.pid);
    rates.procs.set(p.pid, q ? Math.max(0, (p.records - q.records) / dt) : 0);
    // per-locus activity: which petals are emitting / reacting
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

// ---- directional pulses ------------------------------------
// One particle pool per edge key; spawn ∝ message rate, travel
// from -> to. This is what makes flow DIRECTION legible.

const pulses = new Map(); // key -> [{u}] progress along curve

function stepPulses(key, rate, dt) {
  let list = pulses.get(key);
  if (!list) { list = []; pulses.set(key, list); }
  const speed = 0.55; // curve lengths per second
  for (const p of list) p.u += speed * dt;
  while (list.length && list[0].u > 1) list.shift();
  // spawn: visible pulse count tracks rate (log-ish, capped)
  const want = rate <= 0 ? 0 : Math.min(14, 2 + Math.log10(1 + rate) * 3);
  const gap = 1 / Math.max(want, 1);
  if (want > 0 && (!list.length || 1 - Math.max(...list.map(p => p.u)) > gap * 0.9)) {
    // steady cadence: newest pulse enters when the last has
    // travelled one gap — approximates a constant stream
  }
  if (want > 0 && (!list.length || list[list.length - 1].u > gap)) list.push({ u: 0 });
  return list;
}

const qbez = (u, x1, y1, cx, cy, x2, y2) => {
  const v = 1 - u;
  return [v * v * x1 + 2 * v * u * cx + u * u * x2,
          v * v * y1 + 2 * v * u * cy + u * u * y2];
};

function drawEdgeCurve(key, x1, y1, cx, cy, x2, y2, rate, meanUs, dt) {
  const load = Math.min(1, rate / 20000);
  const h = 130 - 90 * Math.min(1, meanUs / 200);
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.quadraticCurveTo(cx, cy, x2, y2);
  ctx.lineWidth = 1 + 4 * load;
  ctx.strokeStyle = `hsla(${h}, 70%, 50%, ${rate > 0 ? .35 : .15})`;
  ctx.stroke();
  // pulses ride the same curve, from -> to
  for (const p of stepPulses(key, rate, dt)) {
    const [px, py] = qbez(p.u, x1, y1, cx, cy, x2, y2);
    const fade = Math.sin(Math.PI * Math.min(1, Math.max(0, p.u))); // ease in/out
    ctx.beginPath();
    ctx.arc(px, py, 2.5 + 3 * load, 0, Math.PI * 2);
    ctx.fillStyle = `hsla(${h}, 90%, 70%, ${0.9 * fade})`;
    ctx.fill();
    // short comet tail pointing backwards along travel
    const [tx, ty] = qbez(Math.max(0, p.u - 0.04), x1, y1, cx, cy, x2, y2);
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(px, py);
    ctx.lineWidth = 2 + 2 * load;
    ctx.strokeStyle = `hsla(${h}, 90%, 70%, ${0.35 * fade})`;
    ctx.stroke();
  }
}

// ---- supervision-tree flower layout ------------------------
// The base of a flower is the process; WITHIN it, layout is the
// supervision tree: root at core, children fan out in their
// parent's angular sector, one ring per depth. Stems draw the
// parentage, so restarts visibly re-grow from the supervisor.

function treeLayout(loci, cx, cy) {
  const byId = new Map(loci.map(l => [l.id, l]));
  const kids = new Map();
  const roots = [];
  for (const l of loci) {
    if (l.parent && byId.has(l.parent)) {
      if (!kids.has(l.parent)) kids.set(l.parent, []);
      kids.get(l.parent).push(l);
    } else roots.push(l);
  }
  const pos = new Map(); // id -> {x, y, angle, depth}
  const RING = 46;
  const place = (l, a0, a1, depth) => {
    const a = (a0 + a1) / 2;
    const r = depth * RING;
    pos.set(l.id, { x: cx + Math.cos(a) * r, y: cy + Math.sin(a) * r, angle: a, depth });
    const c = kids.get(l.id) || [];
    c.forEach((k, i) => {
      const span = (a1 - a0);
      // children own the parent's sector; a lone child fans a
      // little wider so deep chains don't collapse to a line
      const w = span / c.length;
      place(k, a0 + i * w, a0 + (i + 1) * w, depth + 1);
    });
  };
  roots.forEach((rt, i) => {
    const w = Math.PI * 2 / roots.length;
    place(rt, i * w - Math.PI / 2, (i + 1) * w - Math.PI / 2, 0);
  });
  return { pos, kids, byId };
}

// ---- same-type aggregation ---------------------------------
// Many sibling loci of one type under one parent (worker pools)
// drown the flower. Keep the K most-active as individuals and
// collapse the rest into one aggregate petal sized by count.
const KEEP = 2, AGG_MIN = 5;

function condenseLoci(pid, loci) {
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
    const kept = members.slice(0, KEEP);
    const rest = members.slice(KEEP);
    out.push(...kept);
    let pub = 0, dlv = 0;
    for (const m of rest) {
      const r = rates.loci.get(pid + ":" + m.id) || { pub: 0, dlv: 0 };
      pub += r.pub; dlv += r.dlv;
    }
    let hh = 0;
    for (const ch of k) hh = (hh * 31 + ch.charCodeAt(0)) % 100000;
    out.push({
      id: -(hh + 1), // stable synthetic id -> stable layout slot
      type: members[0].type, parent: members[0].parent,
      agg: true, count: rest.length, aggPub: pub, aggDlv: dlv,
    });
  }
  return out;
}

function drawFlower(p, cx, cy, t) {
  const dead = p.state === "dead";
  const loci = condenseLoci(p.pid, p.loci || []);
  const { pos, kids } = treeLayout(loci, cx, cy);

  // stems: parent -> child, the supervision structure itself
  ctx.lineWidth = 1;
  for (const l of loci) {
    const a = pos.get(l.parent), b = pos.get(l.id);
    if (!a || !b) continue;
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.strokeStyle = dead ? "rgba(92,103,115,.25)" : "rgba(120,140,170,.35)";
    ctx.stroke();
  }

  let maxDepth = 0;
  for (const l of loci) {
    const q = pos.get(l.id);
    if (!q) continue;
    maxDepth = Math.max(maxDepth, q.depth);
    const h = hue(l.type);
    const bloom = dead ? 0.15 : 0.55 + 0.35 * Math.sin(t * 1.8 + l.id * 1.7);
    if (q.depth === 0) continue; // root rendered as the core below
    const act = l.agg
      ? { pub: l.aggPub, dlv: l.aggDlv }
      : rates.loci.get(p.pid + ":" + l.id) || { pub: 0, dlv: 0 };
    const rxI = Math.min(1, Math.log10(1 + act.dlv) / 4.5); // reacting (incoming)
    const txI = Math.min(1, Math.log10(1 + act.pub) / 4.5); // emitting (outgoing)
    const scale = l.agg ? 1 + Math.min(1.4, Math.log2(1 + l.count) / 4) : 1;
    ctx.save();
    ctx.translate(q.x, q.y);
    ctx.rotate(q.angle);
    // emitting: warm halo behind the petal
    if (!dead && txI > 0) {
      ctx.beginPath();
      ctx.ellipse(10 * scale, 0, (20 + 6 * bloom) * scale, (9 + 5 * bloom) * scale, 0, 0, Math.PI * 2);
      ctx.fillStyle = `hsla(38, 95%, 60%, ${0.10 + 0.16 * txI})`;
      ctx.fill();
    }
    ctx.beginPath();
    ctx.ellipse(10 * scale, 0, (16 + 6 * bloom) * scale, (5 + 5 * bloom) * scale, 0, 0, Math.PI * 2);
    ctx.fillStyle = dead
      ? `hsla(${h}, 12%, 34%, .35)`
      : `hsla(${h}, 68%, 62%, ${0.3 + 0.5 * bloom})`;
    ctx.fill();
    // reacting: perimeter pulse, flicker rate rises with load
    if (!dead && rxI > 0) {
      const flick = 0.55 + 0.45 * Math.sin(t * (6 + 10 * rxI) + l.id * 2.1);
      ctx.lineWidth = 1.2 + 1.8 * rxI;
      ctx.strokeStyle = `hsla(185, 90%, 70%, ${(0.25 + 0.7 * rxI) * flick})`;
      ctx.stroke();
    }
    ctx.restore();
    ctx.fillStyle = dead ? "rgba(92,103,115,.5)" : `hsla(${h}, 55%, 75%, .85)`;
    ctx.textAlign = "center";
    ctx.font = "10px ui-monospace, monospace";
    ctx.fillText(l.agg ? `${l.type} ×${l.count}` : `${l.type}#${l.id}`,
      cx + Math.cos(q.angle) * (q.depth * 46 + 34 + (l.agg ? 14 : 0)),
      cy + Math.sin(q.angle) * (q.depth * 46 + 34 + (l.agg ? 14 : 0)) + 3);
  }

  // core = the root locus (Main) + process vitals
  const rec = rates.procs.get(p.pid) || 0;
  const pulse = dead ? 0 : Math.min(1, rec / 500000);
  ctx.beginPath();
  ctx.arc(cx, cy, 12 + 4 * pulse, 0, Math.PI * 2);
  ctx.fillStyle = dead ? "#3d2226" : `hsl(${140 - 80 * pulse}, 60%, ${38 + 12 * pulse}%)`;
  ctx.fill();
  ctx.strokeStyle = dead ? "#f85149" : "#1c2430";
  ctx.lineWidth = 2;
  ctx.stroke();

  const labelR = (maxDepth + 1) * 46 + 28;
  ctx.fillStyle = dead ? "#f85149" : "#e6edf3";
  ctx.textAlign = "center";
  ctx.font = "12px ui-monospace, monospace";
  ctx.fillText(`pid ${p.pid}${dead ? " ✝" : ""}`, cx, cy + labelR);
  ctx.fillStyle = "#5c6773";
  ctx.font = "10px ui-monospace, monospace";
  ctx.fillText(dead ? "dissolved" : `${fmt(rec)} rec/s · ${p.restarts} restarts`,
    cx, cy + labelR + 14);
}

// ---- perspectives ------------------------------------------

function flowerCenters(n, w, h) {
  if (n === 1) return [[w / 2, h / 2]];
  if (n === 2) return [[w * 0.28, h * 0.5], [w * 0.72, h * 0.5]];
  const r = Math.min(w, h) * 0.33;
  return Array.from({ length: n }, (_, i) => {
    const a = (i / n) * Math.PI * 2 - Math.PI / 2;
    return [w / 2 + r * Math.cos(a), h / 2 + r * Math.sin(a)];
  });
}

function drawProcessView(t, dt, w, h) {
  const procs = snap.processes || [];
  const centers = flowerCenters(procs.length, w, h);
  const byPid = new Map(procs.map((p, i) => [p.pid, centers[i]]));
  for (const e of snap.edges || []) {
    const f = byPid.get(e.from), g = byPid.get(e.to);
    if (!f || !g) continue;
    const rate = rates.edges.get(e.topic + e.from + e.to) || 0;
    const meanUs = e.matched ? e.latSum / e.matched / 1000 : 0;
    const mx = (f[0] + g[0]) / 2, my = (f[1] + g[1]) / 2 - 46;
    drawEdgeCurve(e.topic + e.from + e.to, f[0], f[1], mx, my, g[0], g[1], rate, meanUs, dt);
    ctx.textAlign = "center";
    ctx.fillStyle = "#e6edf3";
    ctx.font = "11px ui-monospace, monospace";
    ctx.fillText(e.topic, mx, my + 16);
    ctx.fillStyle = "#5c6773";
    ctx.font = "10px ui-monospace, monospace";
    ctx.fillText(`${fmt(rate)}/s · ${meanUs.toFixed(0)}µs · ${fmt(Math.max(0, e.sends - e.delivers))} lost`,
      mx, my + 30);
  }
  procs.forEach((p, i) => drawFlower(p, centers[i][0], centers[i][1], t));
}

// flow view: the TOPIC is the base. Topics sit as hubs in the
// middle; processes orbit; every edge routes through its topic.
function drawFlowView(t, dt, w, h) {
  const procs = snap.processes || [];
  const topics = (snap.topics || []).filter(tp => tp.pub > 0 || tp.dlv > 0);
  const pcenters = flowerCenters(procs.length, w, h);
  const byPid = new Map(procs.map((p, i) => [p.pid, pcenters[i]]));

  const hubs = new Map();
  topics.forEach((tp, i) => {
    const y = h * (topics.length === 1 ? 0.5 : 0.22 + 0.56 * (i / (topics.length - 1)));
    hubs.set(tp.name, [w / 2, y]);
  });

  for (const e of snap.edges || []) {
    const f = byPid.get(e.from), g = byPid.get(e.to), hub = hubs.get(e.topic);
    if (!f || !g || !hub) continue;
    const rate = rates.edges.get(e.topic + e.from + e.to) || 0;
    const meanUs = e.matched ? e.latSum / e.matched / 1000 : 0;
    drawEdgeCurve("A" + e.topic + e.from, f[0], f[1],
      (f[0] + hub[0]) / 2, (f[1] + hub[1]) / 2 - 24, hub[0], hub[1], rate, meanUs, dt);
    drawEdgeCurve("B" + e.topic + e.to, hub[0], hub[1],
      (hub[0] + g[0]) / 2, (hub[1] + g[1]) / 2 - 24, g[0], g[1], rate, meanUs, dt);
  }

  for (const tp of topics) {
    const [hx, hy] = hubs.get(tp.name);
    const th = hue(tp.name);
    ctx.beginPath();
    ctx.arc(hx, hy, 9, 0, Math.PI * 2);
    ctx.fillStyle = `hsla(${th}, 70%, 55%, .9)`;
    ctx.fill();
    ctx.textAlign = "center";
    ctx.fillStyle = "#e6edf3";
    ctx.font = "11px ui-monospace, monospace";
    ctx.fillText(tp.name, hx, hy - 16);
    ctx.fillStyle = "#5c6773";
    ctx.font = "10px ui-monospace, monospace";
    ctx.fillText(`pub ${fmt(tp.pub)} · dlv ${fmt(tp.dlv)}`, hx, hy + 22);
  }
  procs.forEach((p, i) => drawFlower(p, pcenters[i][0], pcenters[i][1], t));
}

// ---- frame loop (capped) -----------------------------------

function frame(now) {
  requestAnimationFrame(frame);
  if (now - lastDraw < FRAME_MS) return; // 30fps cap
  const dt = Math.min(0.1, (now - lastFrameT) / 1000 || 0.033);
  lastDraw = now;
  lastFrameT = now;
  const t = now / 1000;
  const w = innerWidth, h = innerHeight;
  ctx.clearRect(0, 0, w, h);
  if (!snap) return;

  if (view === 1) drawProcessView(t, dt, w, h);
  else drawFlowView(t, dt, w, h);

  const procs = snap.processes || [];
  const totRec = procs.reduce((a, p) => a + (rates.procs.get(p.pid) || 0), 0);
  statsEl.textContent =
    `${procs.filter(p => p.state === "live").length}/${procs.length} processes · ${fmt(totRec)} records/s` +
    ` · view [${view === 1 ? "1" : "2"}] ${view === 1 ? "process" : "flow"} (keys 1/2)`;
  topicsEl.innerHTML = (snap.topics || [])
    .map(tp => `${tp.name} <span style="color:#5c6773">pub ${fmt(tp.pub)} · dlv ${fmt(tp.dlv)}</span>`)
    .join("<br>");
  eventsEl.innerHTML = (snap.events || []).slice(-6).map(x => `<div>${x}</div>`).join("");
}
requestAnimationFrame(frame);
