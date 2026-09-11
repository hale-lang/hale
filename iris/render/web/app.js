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
//   LAW       [l] overlays the model's law on the running system
//             (INSPECTOR.md): group hulls, each claim's static
//             verdict beside its witnessed state, contradicted
//             routes in alarm. Ledger is DOM; hulls are canvas.
//   ORGANISM  [5] the experience source (GH #528 B7): the organism's
//             status projection (re-projected from its Journal by
//             `hale dna run`) — tasks and their state, pending
//             Reviews and why, staged mutations, model calls, the
//             expression identity — and, on the canvas, the DNA
//             lineage tower (Task / Workflow / Step / Work / Attempt)
//             tinted violet, keyed on the core's type names.
//   MEMBRANE  [m] the typed control channel (GH #527 B6): when
//             fuse-hl was started with `hale iris --membrane`, a
//             verdict or an intent typed here is published on the
//             DNA control topics to the organism's sockets. The
//             organism decides; the panel only reports what it sent.
//   REVIEW    [4] the semantic review view (GH #527 B5): a `hale
//             model diff` document carried in /snapshot, rendered
//             as `+ locus EmailIntake`, `! fn … gains publish`,
//             `! locus … contract`, `! claim … holds -> violated`;
//             live processes still expressing the OLD artifact
//             (model hash == diff.a) are ringed stale on canvas.

const canvas = document.getElementById("c");
const ctx = canvas.getContext("2d");
const connEl = document.getElementById("conn");
const statsEl = document.getElementById("stats");
const topicsEl = document.getElementById("topics");
const eventsEl = document.getElementById("events");
const tipEl = document.getElementById("tip");
const lawEl = document.getElementById("law");
const diffEl = document.getElementById("diff");
const membraneEl = document.getElementById("membrane");
const organismEl = document.getElementById("organism");
const VIOLET = "190,140,255";

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
// phosphor afterglow: rare events must outlive their instant.
// key = topic|pair -> {born, topic, pair}
const afterglow = new Map();
const GLOW_MS = 12000;
// burn-in: paths get ETCHED in proportion to cumulative recent
// volume — a heavily-used path stays vivid through lulls,
// decaying with ~30s half-life. pair -> heat (messages).
const burnHeat = new Map();

// per-TOPIC hues (field-requested: tracing one topic across the
// canvas by color beats family grouping). Name-hash anchor, then
// golden-angle steps until >=18deg from every assigned hue —
// distinct by construction, stable within a session. Red band
// avoided (alerts); amber/cyan stay petal-activity accents.
const topicHues = new Map();
function topicHue(name) {
  if (topicHues.has(name)) return topicHues.get(name);
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  let hue = h % 360;
  const bad = (x) => x < 15 || x > 345 ||
    [...topicHues.values()].some(u => Math.min(Math.abs(u - x), 360 - Math.abs(u - x)) < 18);
  for (let i = 0; i < 24 && bad(hue); i++) hue = (hue + 137.5) % 360;
  topicHues.set(name, hue);
  return hue;
}
const topicColor = (name, a) => `hsla(${topicHue(name)}, 62%, 58%, ${a})`;
let colorMode = "topic"; // 'topic' | 'latency' (key c)

// ---- law overlay state (INSPECTOR.md, witnessed law) --------
// The model's LAW drawn on the running system: group hulls, each
// claim's static verdict beside its witnessed state, contradicted
// routes in alarm. Toggle 'l' (or '3'); the ledger panel is DOM
// (crisp text, clickable lens), hulls/highlights are canvas in
// world coordinates so they pan and zoom with the flowers.
let showLaw = false;
let selClaim = null;              // law lens: selected claim name
let petalPos = new Map();         // `${pid}|${type}` -> [{x,y}] (this frame, world)
const hue = (s) => { let h = 0; for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) % 360; return h; };

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
    const delta = p ? e.matched - p.matched : (e.matched > 0 ? 1 : 0);
    if (delta > 0 && !isPlumbing(e.topic)) {
      const pk = e.from + ">" + e.to;
      burnHeat.set(pk, (burnHeat.get(pk) || 0) + delta);
    }
    if (delta > 0 && (rates.topics.get(e.topic) || 0) < 3 && !isPlumbing(e.topic)) {
      afterglow.set(e.topic + "|" + e.from + ">" + e.to,
        { born: performance.now(), topic: e.topic, pair: e.from + ">" + e.to });
    }
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
  es.onmessage = (m) => { try { ingest(JSON.parse(m.data)); renderLawPanel(); renderDiffPanel(); renderMembranePanel(); renderOrganismPanel(); } catch (e) {} };
}
connect();

// ---- the law ledger (DOM: crisp text, clickable lens) --------

const WCOLOR = { consistent: "#7ee787", contradicted: "#f85149",
                 unwitnessed: "#d29922", not_exercised: "#8b949e",
                 unsupported: "#6e7681" };
const WGLYPH = { consistent: "witnessed consistent", contradicted: "CONTRADICTED",
                 unwitnessed: "unwitnessed", not_exercised: "not exercised",
                 unsupported: "unsupported" };

function claimFamilyAdequacy(law, c) {
  // reachability/endpoint/... -> exact | degraded, from the
  // artifact's own adequacy account. The compiler grades its own
  // proof; we only carry the grade to the pixels.
  return (law.adequacy || {})[c.family] || "?";
}

function renderLawPanel() {
  const law = snap && snap.law;
  if (!law || !showLaw) { lawEl.style.display = "none"; return; }
  lawEl.style.display = "block";
  const ev = law.evidence || {};
  let h = `<div class="hdr">law · model ${(law.model || "").slice(0, 8)} · digest ${law.digest}` +
          ` · static verdict: ${law.verdict}</div>`;
  if (ev.other_model > 0)
    h += `<div class="warn">⚠ ${ev.other_model} live process(es) built from a DIFFERENT model — their evidence is about other law</div>`;
  if (law.digest !== "verified")
    h += `<div class="warn">⚠ artifact digest ${law.digest} — claims table ${law.digest === "mismatch" ? "cleared" : "unverified"}</div>`;
  for (const c of law.claims || []) {
    const col = WCOLOR[c.witnessed] || "#6e7681";
    const adq = claimFamilyAdequacy(law, c);
    const hollow = adq === "degraded" ? " hollow" : "";
    const sel = selClaim === c.name ? " sel" : "";
    h += `<div class="claim${sel}" data-claim="${c.name}">` +
         `<span class="dot${hollow}" style="background:${col};border-color:${col}"></span>` +
         `<b>${c.name}</b> <span class="st">${c.form}</span><br>` +
         `<span style="margin-left:14px" class="st">static <b style="color:#a8b3c4">${c.static}</b>` +
         ` · <b style="color:${col}">${WGLYPH[c.witnessed] || c.witnessed}</b>` +
         ` · proof ${adq}</span>` +
         (sel ? `<div class="detail">${c.detail}</div>` : "") +
         `</div>`;
  }
  h += `<div class="st" style="margin-top:6px">click a claim to focus · [l] hides law · esc clears</div>`;
  lawEl.innerHTML = h;
}

lawEl.addEventListener("click", (e) => {
  const row = e.target.closest("[data-claim]");
  if (!row) return;
  selClaim = selClaim === row.dataset.claim ? null : row.dataset.claim;
  renderLawPanel();
});

// ---- the review view (perspective [4]) -----------------------
// The diff document is `hale model diff`'s, verbatim (fuse-hl
// carries, never interprets). Rows render in the diff's own
// vocabulary; the one thing this view adds is the live fleet:
// which processes were built from the OLD artifact (their header
// model hash equals diff.a.shape_hash), which from the new, which
// from neither.

let showDiff = false;

function modelStatus(p, doc) {
  if (!doc || !p.model) return "unknown";
  if (p.model === doc.b.shape_hash) return "current";
  if (p.model === doc.a.shape_hash) return "stale";
  return "other";
}

function isStale(p) {
  const d = snap && snap.diff && snap.diff.document;
  return !!d && p.state !== "dead" && modelStatus(p, d) === "stale";
}

function esc(s) {
  return String(s == null ? "" : s).replace(/&/g, "&amp;").replace(/</g, "&lt;");
}

function siteText(v) {
  return v && v.unit && v.span ? `<span class="site"> ${esc(v.unit)}:${v.span[0]}..${v.span[1]}</span>` : "";
}

function renderDiffPanel() {
  const diff = snap && snap.diff;
  if (!diff || !showDiff) { diffEl.style.display = "none"; if (showDiff === false) topicsEl.style.display = ""; return; }
  diffEl.style.display = "block";
  topicsEl.style.display = "none";
  const d = diff.document;
  if (!d) {
    diffEl.innerHTML = `<div class="hdr">review · ${esc(diff.path)}</div><div class="warn">⚠ diff ${esc(diff.state)}</div>`;
    return;
  }
  const a8 = (d.a.shape_hash || "").slice(0, 8), b8 = (d.b.shape_hash || "").slice(0, 8);
  let h = `<div class="hdr">review · ${a8} → ${b8} · <b>${esc(d.classification)}</b></div>`;
  // the fleet against the two models
  const live = (snap.processes || []).filter(p => p.state !== "dead");
  const counts = { current: 0, stale: 0, other: 0, unknown: 0 };
  for (const p of live) counts[modelStatus(p, d)]++;
  h += `<div class="sec">live: <span class="cur">${counts.current} on the new model</span>` +
       ` · <span class="stale">${counts.stale} still on the old</span>` +
       ` · <span class="other">${counts.other + counts.unknown} other/unknown</span></div>`;
  for (const p of live) {
    const st = modelStatus(p, d);
    if (st === "stale") h += `<div class="row stale">◌ ${esc(p.name)} pid ${p.pid} — expressing the OLD artifact</div>`;
  }
  const decls = d.declarations || [];
  const rows = [];
  for (const r of decls) {
    const k = esc(r.kind), n = esc(r.name);
    switch (r.change) {
      case "added": rows.push(`<div class="row add">+ ${k} ${n}${siteText(r.b)}</div>`); break;
      case "removed": rows.push(`<div class="row del">- ${k} ${n}${siteText(r.a)}</div>`); break;
      case "renamed": rows.push(`<div class="row ren">~ ${k} ${esc(r.from)} → ${n} <span class="site">renamed; shape unchanged</span></div>`); break;
      case "moved": rows.push(`<div class="row mv">&gt; ${k} ${n} moved ${esc(r.a && r.a.unit)} → ${esc(r.b && r.b.unit)}</div>`); break;
      case "split": rows.push(`<div class="row ren">* ${k} ${n} split into ${esc((r.into || []).join(", "))}</div>`); break;
      case "joined": rows.push(`<div class="row ren">* ${k} ${n} joined from ${esc((r.from || []).join(", "))}</div>`); break;
      case "ambiguous": rows.push(`<div class="row amb">? ${k} ${n} ${esc(r.question)} ambiguous: ${esc((r.candidates || []).join(", "))}</div>`); break;
    }
  }
  if (rows.length) h += `<div class="sec">declarations</div>` + rows.join("");
  const contracts = d.contracts || [];
  if (contracts.length) {
    h += `<div class="sec">contracts</div>`;
    for (const r of contracts) {
      const parts = [...(r.removed || []).map(x => `<span class="del">-${esc(x)}</span>`),
                     ...(r.added || []).map(x => `<span class="add">+${esc(x)}</span>`)];
      h += `<div class="row chg">! locus ${esc(r.locus)} ${esc(r.facet)}: ${parts.join("; ")}</div>`;
    }
  }
  const classes = (d.effects && d.effects.classes) || [];
  const certs = (d.effects && d.effects.certificates) || [];
  if (classes.length || certs.length) {
    h += `<div class="sec">effects</div>`;
    for (const r of classes) {
      if (r.change === "added") {
        h += `<div class="row add">+ fn ${esc(r.fn)} reaches ${esc((r.gained || []).join(", "))}</div>`;
        continue;
      }
      if (r.change === "removed") {
        h += `<div class="row del">- fn ${esc(r.fn)} reached ${esc((r.dropped || []).join(", "))}</div>`;
        continue;
      }
      const parts = [];
      if ((r.gained || []).length) parts.push(`gains ${esc(r.gained.join(", "))}`);
      if ((r.dropped || []).length) parts.push(`drops ${esc(r.dropped.join(", "))}`);
      h += `<div class="row chg">! fn ${esc(r.fn)} ${parts.join("; ")}</div>`;
    }
    for (const r of certs) {
      const cls = r.change === "added" ? "add" : r.change === "removed" ? "del" : "chg";
      const glyph = r.change === "added" ? "+" : r.change === "removed" ? "-" : "!";
      const tail = r.change === "result" ? `${esc(r.a)} → ${esc(r.b)}` : `[${esc(r.b || r.a)}]`;
      h += `<div class="row ${cls}">${glyph} certificate ${esc(r.subject)} ${esc(r.form)} ${tail}</div>`;
    }
  }
  const claims = (d.law && d.law.claims) || [];
  const adequacy = (d.law && d.law.adequacy) || [];
  const verdict = d.law && d.law.verdict;
  if (claims.length || adequacy.length || verdict) {
    h += `<div class="sec">law</div>`;
    for (const r of claims) {
      if (r.change === "added") h += `<div class="row add">+ claim ${esc(r.claim)}: ${esc(r.form)} [${esc(r.result)}]</div>`;
      else if (r.change === "removed") h += `<div class="row del">- claim ${esc(r.claim)}: ${esc(r.form)} [${esc(r.result)}]</div>`;
      else {
        const parts = Object.entries(r.changes || {}).map(([k, v]) => `${k} ${esc(v.a)} → ${esc(v.b)}`);
        h += `<div class="row chg">! claim ${esc(r.claim)}: ${parts.join("; ")}</div>`;
      }
    }
    for (const r of adequacy) h += `<div class="row chg">! adequacy ${esc(r.family)}: ${esc(r.a)} → ${esc(r.b)}</div>`;
    if (verdict) h += `<div class="row chg">! verdict: ${esc(verdict.a)} → ${esc(verdict.b)}</div>`;
  }
  const s = d.summary || {};
  if (Object.values(s).every(v => v === 0)) h += `<div class="sec">no semantic differences</div>`;
  h += `<div class="sec" style="margin-top:8px">[4] hides the review · stale flowers are ringed on the canvas</div>`;
  diffEl.innerHTML = h;
}

// ---- the organism (experience source, perspective [5]) -------
// Keyed on the DNA core's TYPE names (`…::Task`, `…::Attempt`),
// never on an application's locus names: the tower is the core's.

let showOrganism = false;
const LINEAGE = ["Task", "Workflow", "Step", "Work", "Attempt", "Review", "Metabolism", "WorkSystem", "Dna"];
function lineageKind(type) {
  const leaf = type.includes("::") ? type.slice(type.lastIndexOf("::") + 2) : type;
  return LINEAGE.includes(leaf) ? leaf : null;
}

function renderOrganismPanel() {
  const dna = snap && snap.dna;
  if (!dna || !showOrganism) { organismEl.style.display = "none"; if (!showDiff) topicsEl.style.display = ""; return; }
  organismEl.style.display = "block";
  topicsEl.style.display = "none";
  const st = dna.status;
  if (!st) {
    organismEl.innerHTML = `<div class="hdr">organism</div><div class="pend">⚠ status ${esc(dna.state)} (${esc(dna.path)})</div>`;
    return;
  }
  const e = st.expression || {};
  let h = `<div class="hdr">organism · ${esc(st.organism)}</div>`;
  h += `<div class="row">journal ${esc(st.journal && st.journal.revision)} event(s) · chain <span class="${st.journal && st.journal.chain === "verified" ? "ok" : "bad"}">${esc(st.journal && st.journal.chain)}</span>` +
       ` · attached ${esc(e.attached && e.attached.main)} @ ${esc((e.attached && e.attached.shape_hash || "").slice(0, 8))}` +
       ` · current ${esc((e.current && e.current.shape_hash || "?").slice(0, 8))} · build ${esc((e.build_digest || "?").slice(0, 8))}</div>`;
  const tasks = st.tasks || [];
  h += `<div class="sec">tasks · ${tasks.length} (${esc(st.intents && st.intents.offered)} intent(s) offered, ${(st.intents && st.intents.refused || []).length} refused)</div>`;
  for (const t of tasks) {
    const cls = t.state === "active" ? "pend" : (t.state === "done" ? "ok" : "bad");
    h += `<div class="row"><span class="${cls}">${esc(t.id)} [${esc(t.state)}]</span> ${esc(t.outcome)}${t.detail ? ` <span class="dim">${esc(t.detail)}</span>` : ""}</div>`;
  }
  const reviews = st.reviews || [];
  const pending = reviews.filter(r => r.state === "pending").length;
  h += `<div class="sec">reviews · ${pending} pending of ${reviews.length}</div>`;
  for (const r of reviews) {
    const why = r.state === "pending" ? `needs ${esc(r.required_authority)} — ${esc(r.question)}` : `settled ${esc(r.settled)}`;
    const refusals = (r.refusals || []).length;
    h += `<div class="row"><span class="${r.state === "pending" ? "pend" : "ok"}">${esc(r.id)} [${esc(r.state)}]</span> ${why}${refusals ? ` <span class="dim">(${refusals} verdict(s) refused)</span>` : ""}</div>`;
    // a mutation's Review: the evidence steps and the candidate the verdict must name
    if (r.mutation_id) h += `<div class="row dim">   ${esc(r.change_class)} · ${esc(r.evidence)} · disposition ${esc(r.disposition)} · candidate ${esc(String(r.candidate_commit || "").slice(0, 12))} — verdict digest = ${esc(r.subject_digest)}</div>`;
  }
  const muts = st.mutations || [];
  h += `<div class="sec">mutations · ${muts.length} (none applies before a human's verdict on the exact candidate)</div>`;
  for (const m of muts) {
    const cls = m.disposition === "reviewed" || m.disposition === "applied" ? "ok" : (m.disposition === "failed" || m.disposition === "rejected" || m.disposition === "deny" ? "bad" : "pend");
    h += `<div class="row">${esc(m.id || m.candidate)} <span class="${cls}">${esc(m.disposition)}</span> ${esc(m.objective || m.class || "")}${m.candidate ? ` <span class="dim">candidate ${esc(String(m.candidate).slice(0, 12))}</span>` : ""}</div>`;
  }
  const mc = st.model_calls || {};
  h += `<div class="sec">model calls · ${esc(mc.total || 0)}</div>`;
  for (const c of mc.recent || []) {
    h += `<div class="row"><span class="${c.ok ? "ok" : "bad"}">${esc(c.attempt)}</span> ${esc(c.adapter)}/${esc(c.backend)} ${esc(c.reported_model || c.requested_model)} ${c.ok ? `${esc(c.input_tokens)}+${esc(c.output_tokens)} tok · ${esc(c.elapsed)}` : esc(c.refused)}</div>`;
  }
  const deferred = st.law_deferred || [];
  if (deferred.length) h += `<div class="sec pend">law · ${deferred.length} clause(s) deferred at init</div>`;
  h += `<div class="sec">[5] hides · the lineage tower (Task → Workflow → Step → Work → Attempt) is tinted on the canvas</div>`;
  organismEl.innerHTML = h;
}

// ---- the membrane (typed control channel) --------------------
// Built once; only the counters line re-renders per snapshot, so
// typing is never clobbered by the 10 Hz stream.

let showMembrane = false;

function membraneForm() {
  return `<div class="hdr">membrane · <span id="m-dir"></span></div>` +
    `<div class="st" id="m-counts"></div>` +
    `<div class="st" style="margin-top:6px">intent → dna.intent.offered</div>` +
    `<input id="m-outcome" placeholder="outcome (what should happen)">` +
    `<input id="m-from" placeholder="from" value="iris">` +
    `<button id="m-send-intent">offer intent</button>` +
    `<div class="st" style="margin-top:8px">verdict → dna.review.verdict</div>` +
    `<input id="m-review" placeholder="review_id">` +
    `<input id="m-digest" placeholder="subject_digest (the candidate you looked at)">` +
    `<select id="m-verdict"><option>approve</option><option>revise</option><option>reject</option><option>abstain</option></select>` +
    `<input id="m-reviewer" placeholder="reviewer">` +
    `<input id="m-authority" placeholder="authority" value="maintainer">` +
    `<input id="m-comment" placeholder="comment">` +
    `<button id="m-send-verdict">send verdict</button>` +
    `<div class="log" id="m-log"></div>` +
    `<div class="st" style="margin-top:6px">[m] hides · the organism decides; this only publishes</div>`;
}

async function membranePost(path, body) {
  const log = document.getElementById("m-log");
  try {
    const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const t = await r.text();
    log.className = r.ok ? "log" : "log err";
    log.textContent = `${r.status} ${t}`;
  } catch (e) {
    log.className = "log err";
    log.textContent = String(e);
  }
}

function renderMembranePanel() {
  const m = snap && snap.membrane;
  if (!m || !showMembrane) { membraneEl.style.display = "none"; return; }
  membraneEl.style.display = "block";
  if (!membraneEl.dataset.built) {
    membraneEl.innerHTML = membraneForm();
    membraneEl.dataset.built = "1";
    const v = (id) => document.getElementById(id).value;
    document.getElementById("m-send-intent").addEventListener("click", () =>
      membranePost("/ctl/intent", { intent_id: "i" + Date.now().toString(36), outcome: v("m-outcome"), from: v("m-from"), to: "" }));
    document.getElementById("m-send-verdict").addEventListener("click", () =>
      membranePost("/ctl/review", { review_id: v("m-review"), subject_digest: v("m-digest"), verdict: v("m-verdict"),
                                    reviewer: v("m-reviewer"), authority: v("m-authority"), comment: v("m-comment") }));
    // keys typed into the form must not toggle views
    membraneEl.addEventListener("keydown", (e) => e.stopPropagation());
  }
  document.getElementById("m-dir").textContent = m.dir;
  document.getElementById("m-counts").textContent = `published: ${m.verdicts} verdict(s) · ${m.intents} intent(s)`;
}

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
    r.top = r.topics[0]?.name || "?";
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
const pulseCounters = new Map();
function pickTopic(r, n) {
  // deterministic weighted round-robin over the pair's topics,
  // so pulse colors appear in proportion to each topic's rate
  if (!r || !r.topics.length) return null;
  const tot = r.topics.reduce((a, t) => a + t.rate, 0);
  if (tot <= 0) return r.topics[0].name;
  let u = ((n * 0.6180339887) % 1) * tot;
  for (const t of r.topics) { u -= t.rate; if (u <= 0) return t.name; }
  return r.topics[0].name;
}
function stepPulses(key, rate, dt, r) {
  let list = pulses.get(key);
  if (!list) { list = []; pulses.set(key, list); }
  for (const p of list) p.u += 0.5 * dt;
  while (list.length && list[0].u > 1) list.shift();
  const want = rate <= 0 ? 0 : Math.min(12, 1.5 + Math.log10(1 + rate) * 2.6);
  const gap = 1 / Math.max(want, 1);
  if (want > 0 && (!list.length || list[list.length - 1].u > gap)) {
    const n = (pulseCounters.get(key) || 0) + 1;
    pulseCounters.set(key, n);
    list.push({ u: 0, t: pickTopic(r, n) });
  }
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

  // review view: a flower still expressing the OLD artifact wears
  // a dashed amber ring — the drift is a fact about the fleet,
  // shown where the fleet is drawn.
  if (showDiff && isStale(p)) {
    ctx.save();
    ctx.setLineDash([5, 5]);
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = `rgba(${AMBER}, 0.75)`;
    ctx.beginPath();
    ctx.arc(cx, cy, ring + 14, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }

  // petals: slate at rest, colored only by activity
  const hot = [];
  for (const l of loci) {
    const q = pos.get(l.id);
    if (!q || q.depth === 0) continue;
    // law overlay looks petals up by (pid, type) — world coords
    const pk = p.pid + "|" + l.type;
    if (!petalPos.has(pk)) petalPos.set(pk, []);
    petalPos.get(pk).push({ x: q.x, y: q.y });
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
    const lineage = showOrganism && !dead && lineageKind(l.type);
    if (l.internal) ctx.fillStyle = `rgba(${SLATE}, 0.07)`;
    else if (dead) ctx.fillStyle = `rgba(${SLATE}, 0.08)`;
    else if (lineage) ctx.fillStyle = `rgba(${VIOLET}, ${0.22 + 0.10 * bloom})`;
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

// ---- law overlay ---------------------------------------------
// Drawn in WORLD coordinates after the flowers, so hulls sit on the
// petals they enclose at any pan/zoom; text and stroke widths are
// divided by view.k to stay screen-constant, like the flower labels.

function groupPoints(law, gname) {
  const pts = [];
  const members = (law.groups || {})[gname] || [];
  for (const [k, list] of petalPos) {
    const type = k.split("|")[1];
    if (members.includes(type)) pts.push(...list);
  }
  return pts;
}

function drawHull(pts, hueDeg, label, bright, dashed) {
  if (!pts.length) return null;
  const k = view.k;
  let cx = 0, cy = 0;
  for (const p of pts) { cx += p.x; cy += p.y; }
  cx /= pts.length; cy /= pts.length;
  let r = 30;
  for (const p of pts) r = Math.max(r, Math.hypot(p.x - cx, p.y - cy) + 30);
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.setLineDash(dashed ? [6 / k, 5 / k] : []);
  ctx.fillStyle = `hsla(${hueDeg}, 60%, 55%, ${bright ? 0.10 : 0.05})`;
  ctx.fill();
  ctx.lineWidth = (bright ? 2 : 1) / k;
  ctx.strokeStyle = `hsla(${hueDeg}, 65%, 62%, ${bright ? 0.85 : 0.35})`;
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.textAlign = "center";
  ctx.font = `${(11 / k).toFixed(2)}px ui-monospace, monospace`;
  ctx.fillStyle = `hsla(${hueDeg}, 55%, 72%, ${bright ? 0.95 : 0.55})`;
  ctx.fillText(label, cx, cy - r - 6 / k);
  return { x: cx, y: cy, r };
}

function ringPetals(types, color) {
  for (const [k, list] of petalPos) {
    const type = k.split("|")[1];
    if (!types.includes(type)) continue;
    for (const q of list) {
      ctx.beginPath();
      ctx.arc(q.x, q.y, 15, 0, Math.PI * 2);
      ctx.lineWidth = 2 / view.k;
      ctx.strokeStyle = color;
      ctx.stroke();
    }
  }
}

function drawLawOverlay() {
  const law = snap.law;
  if (!law) return;
  const k = view.k;
  ctx.globalAlpha = 1;
  const sel = (law.claims || []).find(c => c.name === selClaim) || null;
  const parts = new Set(); // groups the selected claim touches
  if (sel) for (const g of [sel.group, sel.src, sel.dst]) if (g) parts.add(g);

  // group hulls: bright when selected-or-no-selection, dim otherwise
  const hulls = new Map();
  for (const gname of Object.keys(law.groups || {})) {
    const bright = !sel || parts.has(gname);
    const adqDegraded = sel && parts.has(gname) &&
      claimFamilyAdequacy(law, sel) === "degraded";
    hulls.set(gname, drawHull(groupPoints(law, gname), hue(gname), gname, bright, adqDegraded));
  }

  // claim annotations on the canvas
  for (const c of law.claims || []) {
    if (sel && c.name !== sel.name) continue;
    if (c.witnessed === "contradicted") {
      if (c.route) {
        // forbid: the observed one-hop route, in alarm
        const a = hulls.get(c.src), b = hulls.get(c.dst);
        if (a && b) {
          ctx.beginPath();
          ctx.moveTo(a.x, a.y);
          ctx.setLineDash([9 / k, 6 / k]);
          const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2 - 40;
          ctx.quadraticCurveTo(mx, my, b.x, b.y);
          ctx.lineWidth = 3 / k;
          ctx.strokeStyle = `rgba(${RED},.9)`;
          ctx.stroke();
          ctx.setLineDash([]);
          ctx.fillStyle = "#f85149";
          ctx.textAlign = "center";
          ctx.font = `${(11 / k).toFixed(2)}px ui-monospace, monospace`;
          ctx.fillText(`⚠ ${c.route.pub} → ${c.route.topic} → ${c.route.dlv}`, mx, my + 14 / k);
        }
      } else if (c.witnesses) {
        // count: ring every observed writer red; the set IS the finding
        ringPetals(c.witnesses, `rgba(${RED},.9)`);
      }
    }
    if (sel && c.witnessed === "consistent" && c.witnesses)
      ringPetals(c.witnesses, "rgba(126,231,135,.7)");
  }
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
  petalPos = new Map();

  const { ribbons, plumbs } = buildRibbons();
  layoutTick(ribbons, dt);

  ctx.setTransform(dpr * view.k, 0, 0, dpr * view.k, dpr * view.x, dpr * view.y);

  // ghosts age out
  const procs = (snap.processes || []).filter(p => {
    if (p.state !== "dead") { deadSince.delete(p.pid); return true; }
    if (!deadSince.has(p.pid)) deadSince.set(p.pid, now);
    return now - deadSince.get(p.pid) < 60000;
  });

  for (const [k, gl] of afterglow) if (now - gl.born > GLOW_MS) afterglow.delete(k);
  const bdecay = Math.exp(-dt / 45);
  for (const [k, h] of burnHeat) {
    const h2 = h * bdecay;
    if (h2 < 1) burnHeat.delete(k); else burnHeat.set(k, h2);
  }

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
    const col = (al) => colorMode === "topic" ? topicColor(r.top, al) : latColor(r.mean, al);
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
    // burn-in: cumulative recent volume etches a hot core that
    // survives lulls (~30s half-life). More volume = more vivid.
    const heat = burnHeat.get(r.key) || 0;
    const burn = Math.min(1, Math.log10(1 + heat) / 5.5);
    if (burn > 0.06) {
      const hue = colorMode === "topic" ? topicHue(r.top)
        : 130 - 90 * Math.min(1, r.mean / 200);
      ctx.lineWidth = Math.max(0.8, (1.1 + 1.9 * tier) * 0.45);
      ctx.strokeStyle = `hsla(${hue}, 70%, ${58 + 24 * burn}%, ${0.10 + 0.55 * burn})`;
      ctx.stroke();
    }
    // afterglow: a rare message's 12s of persistence, in its
    // topic's own hue, decaying — so one-off control/state
    // messages are seeable without staring at the instant.
    for (const gl of afterglow.values()) {
      if (gl.pair !== r.key) continue;
      const f = 1 - (now - gl.born) / GLOW_MS;
      if (f <= 0) continue;
      const ease = f * f;
      ctx.beginPath();
      ctx.moveTo(g[0], g[1]);
      ctx.bezierCurveTo(g[2], g[3], g[4], g[5], g[6], g[7]);
      ctx.lineWidth = 3 + 5 * ease;
      ctx.strokeStyle = topicColor(gl.topic, 0.30 * ease);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(g[6], g[7], 3 + 6 * ease, 0, Math.PI * 2);
      ctx.fillStyle = topicColor(gl.topic, 0.5 * ease);
      ctx.fill();
    }
    // latency ALERT: a slow path earns red regardless of mode
    if (r.mean > 300 && r.rate > 0) {
      ctx.save();
      ctx.setLineDash([6, 8]);
      ctx.lineWidth = 1.2;
      ctx.strokeStyle = `rgba(${RED}, 0.55)`;
      ctx.stroke();
      ctx.restore();
    }
    for (const p of stepPulses(r.key, r.rate, dt, r)) {
      const [px, py] = cbez(p.u, ...g);
      const fade = Math.sin(Math.PI * Math.min(1, Math.max(0, p.u)));
      ctx.beginPath();
      ctx.arc(px, py, 1.6 + 1.1 * tier, 0, Math.PI * 2);
      ctx.fillStyle = sel ? `rgba(230,237,243,${0.9 * fade})`
        : colorMode === "topic" && p.t ? topicColor(p.t, 0.9 * fade)
        : col(0.9 * fade);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  for (const p of procs) drawFlower(p, t, false);
  if (showLaw) drawLawOverlay();

  // HUD
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  if (showLaw && !snap.law) {
    ctx.fillStyle = "#8b949e";
    ctx.textAlign = "center";
    ctx.font = "12px ui-monospace, monospace";
    ctx.fillText("no topology artifact — start fuse-hl with one to see law", innerWidth / 2, 30);
  }
  const live = procs.filter(p => p.state === "live");
  const totRec = live.reduce((a, p) => a + (rates.procs.get(p.pid) || 0), 0);
  const lawBits = snap.law
    ? ` · law: ${(snap.law.claims || []).filter(c => c.witnessed === "consistent").length}✓` +
      ` ${(snap.law.claims || []).filter(c => c.witnessed === "contradicted").length}✗` +
      ` ${(snap.law.claims || []).filter(c => c.witnessed === "not_exercised" || c.witnessed === "unwitnessed").length}·` +
      (showLaw ? "" : " [l]")
    : "";
  statsEl.textContent =
    `${live.length} processes · ${fmt(totRec)} records/s · ${ribbons.length} flows${lawBits}`;
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
      const dot = `<span style="color:${topicColor(tp.name, 0.9)}">●</span>`;
      let bg = "";
      let best = 0;
      for (const gl of afterglow.values())
        if (gl.topic === tp.name) best = Math.max(best, 1 - (now - gl.born) / GLOW_MS);
      if (best > 0) {
        const q = Math.ceil(best * 8) / 8; // quantize: rebuild ~1x/1.5s
        bg = ` style="background:hsla(${topicHue(tp.name)},60%,50%,${(0.22 * q).toFixed(2)})"`;
      }
      return `<div class="${cls}"${bg} data-t="${tp.name}">${dot} ${tp.name} ${rate} <span class="z">· ${fmt(Math.max(tp.pub, tp.dlv))}</span></div>`;
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
      `<span style="color:${topicColor(tp.name, 0.9)}">●</span> ${tp.name} — ${fmt(tp.rate)}/s · ${tp.mean.toFixed(0)}µs${tp.lost ? ` · <span style="color:rgb(${RED})">${fmt(tp.lost)} lost</span>` : ""}`
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
  if (e.key === "l" || e.key === "3") { showLaw = !showLaw; renderLawPanel(); }
  if (e.key === "4" || e.key === "d") { showDiff = !showDiff; renderDiffPanel(); }
  if (e.key === "m") { showMembrane = !showMembrane; renderMembranePanel(); }
  if (e.key === "5" || e.key === "o") { showOrganism = !showOrganism; renderOrganismPanel(); }
  if (e.key === "Escape") { pinned = null; pinnedTopic = null; selClaim = null; renderLawPanel(); }
});
