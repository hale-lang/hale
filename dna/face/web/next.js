// The face: one organization at a time, three workspaces, the record as
// the head serves it. Reads rows, never decides. A hash is a place:
//   #/org[/<position id>]   #/board[/<review id or task id>]   #/knowledge[/<node id>]
"use strict";

const API = "/api/hale/v1/applications";
const HEAD = "/api/hale/v1/head";

// ---- wire -------------------------------------------------------------

class ReadError extends Error {
  constructor(status, code, message) { super(message || code); this.status = status; this.code = code; }
}

async function readJson(url, init) {
  const res = await fetch(url, Object.assign({ credentials: "same-origin", cache: "no-store" }, init || {}));
  if (res.status === 401) throw new ReadError(401, "unauthenticated", "sign in at /auth/login");
  let body = null;
  try { body = await res.json(); } catch (e) { throw new ReadError(res.status, "invalid_response", "the head did not answer JSON"); }
  if (body && body.error) throw new ReadError(res.status, body.error.code || "error", body.error.message || "");
  if (!res.ok) throw new ReadError(res.status, "http_" + res.status, "the head answered " + res.status);
  return body;
}

// A read of the record's API: `data` and `source` of the envelope.
async function read(app, path) {
  const body = await readJson(`${API}/${encodeURIComponent(app)}/${path}`);
  if (body.api_version !== "hale.v1" || !body.data) throw new ReadError(200, "invalid_response", "not a hale.v1 envelope");
  return body;
}

// A row of the head's surface, called as the signed-in person.
async function surface(member, payload) {
  return readJson(`${HEAD}/surface/call/Commands::${member}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Hale-Command": "1" },
    body: JSON.stringify(payload || {}),
  });
}

// A command of the record, relayed by the head to the surface's HTTP
// exposure; the reply is a CommandReply, refusals included.
async function command(app, call, payload) {
  return readJson(`${API}/${encodeURIComponent(app)}/commands`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Hale-Command": "1" },
    body: JSON.stringify({ call, payload }),
  });
}

async function describe(app) {
  try {
    const d = await readJson(`${API}/${encodeURIComponent(app)}/commands`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Hale-Command": "1" },
      body: JSON.stringify({ describe: true }),
    });
    return new Set((d.commands || []).map(c => c.name));
  } catch (e) { return new Set(); }
}

function requestId() {
  if (crypto.randomUUID) return "face-" + crypto.randomUUID();
  const b = new Uint8Array(16); crypto.getRandomValues(b);
  return "face-" + Array.from(b, x => x.toString(16).padStart(2, "0")).join("");
}

// ---- state -------------------------------------------------------------

const S = {
  app: null, record: null, principal: null, reads: {}, calls: new Set(),
  index: null, knowledge: null, reviews: null, practices: null, tasks: null,
  errors: {}, route: { tab: "org", id: "" },
};

function readRoute() {
  const h = location.hash.replace(/^#\/?/, "");
  const slash = h.indexOf("/");
  const tab = slash < 0 ? h : h.slice(0, slash);
  const id = slash < 0 ? "" : decodeURIComponent(h.slice(slash + 1));
  return { tab: ["org", "board", "knowledge"].includes(tab) ? tab : "org", id };
}
function href(tab, id) { return "#/" + tab + (id ? "/" + encodeURIComponent(id) : ""); }

// ---- dom ---------------------------------------------------------------

function el(tag, attrs, ...children) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") n.className = v;
    else if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
    else if (k === "text") n.textContent = v;
    else n.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    n.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return n;
}
const $ = id => document.getElementById(id);

function stateCard(cls, text) { return el("div", { class: "state " + cls, role: "status" }, text); }
function errorText(e) { return e instanceof ReadError ? `${e.code}: ${e.message}` : String(e && e.message || e); }

// ---- loading -------------------------------------------------------------

async function loadAll() {
  const apps = await readJson(`${API}`);
  const item = apps.data && apps.data.items && apps.data.items[0];
  if (!item) throw new ReadError(404, "no_application", "the head serves no record");
  S.app = item.id;
  S.record = apps.source;
  const caps = await read(S.app, "capabilities");
  S.principal = caps.data.principal || null;
  S.reads = caps.data.reads || {};
  S.calls = await describe(S.app);
  const jobs = [
    surface("index", { limit: 200 }).then(x => { S.index = x; }, e => { S.errors.index = e; }),
    // the org chart row: a head older than it answers 404, and the chart
    // falls back to the index's positions
    surface("org", { limit: 500 }).then(x => { if (x && x.ok) S.org = x; else S.errors.org = new ReadError(200, "org_refused", x && x.why || ""); }, e => { S.errors.org = e; }),
    readAllKnowledge().then(x => { S.knowledge = x; }, e => { S.errors.knowledge = e; }),
    read(S.app, "dna/reviews?limit=100").then(x => { S.reviews = x.data.items; }, e => { S.errors.reviews = e; }),
    read(S.app, "dna/practices?limit=100").then(x => { S.practices = x.data.items; }, e => { S.errors.practices = e; }),
    readTasks().then(x => { S.tasks = x; }, e => { S.errors.tasks = e; }),
  ];
  await Promise.all(jobs);
}

// Handed Tasks: the surface's row first (it answers wherever the head
// reaches the Ledger), the HTTP read for a head without it.
async function readTasks() {
  try {
    const t = await surface("tasks", { limit: 200 });
    if (t && t.ok) return parseList(t.tasks);
    if (t && t.code && t.code !== "forbidden") throw new ReadError(200, t.code, t.why || "");
  } catch (e) { if (!(e instanceof ReadError) || e.status !== 404) throw e; }
  const r = await read(S.app, "dna/tasks?limit=100");
  return r.data.items || [];
}

async function readAllKnowledge() {
  const items = [];
  let cursor = null, snapshot = null;
  for (let page = 0; page < 20; page++) {
    let q = "dna/knowledge/nodes?limit=100";
    if (cursor) q += `&cursor=${encodeURIComponent(cursor)}&snapshot=${encodeURIComponent(snapshot)}`;
    const r = await read(S.app, q);
    items.push(...(r.data.items || []));
    snapshot = r.data.page && r.data.page.snapshot;
    if (!r.data.page || !r.data.page.has_more) break;
    cursor = r.data.page.next_cursor;
  }
  return items;
}

async function readNodeRelations(id) {
  const q = `id=${encodeURIComponent(id)}&limit=100`;
  const [edges, bindings] = await Promise.all([
    read(S.app, `dna/knowledge/edges?${q}`).then(r => r.data.items || [], () => null),
    read(S.app, `dna/knowledge/bindings?${q}`).then(r => r.data.items || [], () => null),
  ]);
  return { edges, bindings };
}

// ---- the organization: positions from the index, mandates from knowledge ----

function parseList(s) { try { const v = JSON.parse(s || "[]"); return Array.isArray(v) ? v : []; } catch (e) { return []; } }

// Positions as a tree. A position `a/b` sits under the position `a` when
// the record states one, else under the part `a`; a bare name sits under the
// purpose. A grafted organism is a child organization. Until the head's org
// read lands, `under` is read from the name.
function organization() {
  const idx = S.index || {};
  let positions, parts = new Map();
  if (S.org) {
    // the graph's chart: what each position sits under is the record's
    positions = parseList(S.org.positions).map(p => ({
      id: p.id, name: p.name, kind: "position", text: p.text || "", under: p.under || "",
      holders: p.holders || [], reviews: p.reviews || [], roles: p.roles || [],
      state: p.state || "stated", review: p.review || "",
      mandate: { state: p.mandate === "" ? "none" : p.mandate, node: mandateOf(p.name).node },
    }));
    for (const part of parseList(S.org.parts)) parts.set(part.id, { id: part.id, name: part.name || part.id.slice(part.id.indexOf(":") + 1), kind: "part", nodeKind: part.kind, text: part.text || "", children: [] });
  } else {
    positions = parseList(idx.positions).map(p => ({
      id: "position:" + p.name, name: p.name, kind: "position", text: "", under: "",
      holders: String(p.holders || "").split(",").map(s => s.trim()).filter(Boolean), reviews: [], roles: [],
      state: "stated", review: "", mandate: mandateOf(p.name),
    }));
  }
  const byId = new Map(positions.map(p => [p.id, p]));
  const byName = new Map(positions.map(p => [p.name, p]));
  const roots = [];
  for (const p of positions) {
    p.children = p.children || [];
    // the recorded parent first; a name's prefix stands in where the
    // record states none
    let over = p.under && p.under !== p.id ? p.under : "";
    if (!over) {
      const cut = p.name.lastIndexOf("/");
      if (cut >= 0) over = byName.has(p.name.slice(0, cut)) ? "position:" + p.name.slice(0, cut) : "part:" + p.name.slice(0, cut);
    }
    if (!over || over.startsWith("purpose:")) { roots.push(p); continue; }
    if (byId.has(over)) { const o = byId.get(over); (o.children = o.children || []).push(p); continue; }
    if (!parts.has(over)) parts.set(over, { id: over, name: over.slice(over.indexOf(":") + 1), kind: "part", children: [] });
    const part = parts.get(over);
    if (!roots.includes(part)) roots.push(part);
    part.children.push(p);
  }
  for (const part of parts.values()) if (part.children.length && !roots.includes(part)) roots.push(part);
  for (const o of parseList(S.org ? S.org.organisms : idx.organisms)) {
    roots.push({ id: "organism:" + o.name, name: o.name, kind: "organism", face: o.face, head: o.head, state: o.state, pulse: o.pulse, purpose: o.purpose, children: [] });
  }
  return { roots, byId: new Map([...positions, ...parts.values()].map(n => [n.id, n])) };
}

function mandateOf(name) {
  const nodes = S.knowledge || [];
  const m = nodes.find(n => n.kind === "mandate" && (n.name === "mandate/" + name || n.name === "design/mandate-" + name));
  if (!m) return { state: "none", node: null };
  return { state: m.accepted ? "ratified" : (m.projection_state || "proposed"), node: m };
}

function positionNode(n, current) {
  const isCurrent = n.id === current;
  const attrs = { class: "pos " + n.kind, href: n.kind === "organism" ? n.face : href("org", n.id), "aria-current": isCurrent ? "true" : null };
  if (n.kind === "organism") attrs.target = "_blank";
  const head = el("a", attrs,
    el("span", { class: "name" }, n.name),
    el("span", { class: "kind" }, n.kind === "organism" ? (n.state || "organism") : n.kind === "part" ? "" : "position"),
  );
  if (n.kind === "position") {
    if (n.state === "proposed") head.classList.add("proposed");
    head.append(el("span", { class: "holders " + (n.holders.length ? "" : "vacant") }, n.state === "proposed" ? "proposed, awaiting the Board" : n.holders.length ? "held by " + n.holders.join(", ") : "vacant"));
    head.append(el("span", { class: "mandate " + n.mandate.state }, "mandate " + n.mandate.state));
  }
  if (n.kind === "organism") head.append(el("span", { class: "holders" }, n.purpose ? n.purpose.split(":")[0] + ": grafted organism" : "grafted organism"));
  const li = el("li", {}, head);
  if (n.children && n.children.length) li.append(el("ul", {}, n.children.map(c => positionNode(c, current))));
  return li;
}

function renderOrg(main) {
  if (S.errors.index) { main.append(stateCard("unavailable", "the record's index is unavailable: " + errorText(S.errors.index))); return; }
  const org = organization();
  const current = S.route.id;
  const canvas = el("section", { class: "panel", "aria-label": "Org chart" },
    el("header", {}, el("h2", {}, "Positions"), el("span", { class: "note" }, S.index && S.index.purpose ? S.index.purpose.split(".")[0] + "." : "")),
    org.roots.length ? el("ul", { class: "chart" }, el("li", {},
      el("div", { class: "pos part" }, el("span", { class: "name" }, "purpose")),
      el("ul", {}, org.roots.map(r => positionNode(r, current))))) : stateCard("", "the record states no positions yet"),
  );
  const insp = el("aside", { class: "panel insp", "aria-label": "Inspector" });
  const node = org.byId.get(current);
  if (!node) {
    insp.append(el("h2", {}, "Select a position"), el("p", { class: "note" }, "A position is a seat in the organization: who holds it, what its mandate says, what it signs."));
    if (S.calls.has("PositionOpen")) insp.append(openPositionForm(org));
  } else if (node.kind === "part") {
    insp.append(el("h2", {}, node.name), el("p", { class: "note" }, "A part of the organization its positions sit under."));
  } else {
    insp.append(positionInspector(node, org));
  }
  main.append(el("div", { class: "split" }, canvas, insp));
}

function positionInspector(n, org) {
  const wrap = el("div", {});
  wrap.append(el("h2", {}, n.name));
  const dl = el("dl", {},
    el("dt", {}, "id"), el("dd", { class: "mono" }, n.id),
    n.state === "proposed" ? el("dt", {}, "state") : "", n.state === "proposed" ? el("dd", {}, el("span", { class: "badge proposed" }, "proposed"), n.review ? el("span", {}, " · ", el("a", { href: href("board", n.review) }, "the Review")) : "") : "",
    el("dt", {}, "holders"), el("dd", { class: n.holders.length ? "" : "note" }, n.holders.length ? n.holders.join(", ") : "vacant"),
    el("dt", {}, "mandate"), el("dd", {}, el("span", { class: "badge " + n.mandate.state }, n.mandate.state)),
    n.reviews && n.reviews.length ? el("dt", {}, "signs") : "", n.reviews && n.reviews.length ? el("dd", {}, n.reviews.map(r => el("div", { class: "mono" }, r))) : "",
    n.roles && n.roles.length ? el("dt", {}, "roles") : "", n.roles && n.roles.length ? el("dd", {}, n.roles.join(" ")) : "",
  );
  wrap.append(dl);
  if (n.text) wrap.append(el("p", { class: "note" }, n.text));
  if (n.mandate.node) wrap.append(el("p", { class: "text" }, n.mandate.node.text), el("p", { class: "note mono" }, n.mandate.node.name, " · ", n.mandate.node.id.slice(0, 19)));
  const reviews = (S.reviews || []).filter(r => r.state === "pending" && (r.question || "").includes("`" + n.name + "`"));
  if (reviews.length) wrap.append(el("h3", {}, "awaiting the Board"), el("ul", {}, reviews.map(r => el("li", {}, el("a", { href: href("board", r.id) }, r.question)))));
  if (S.calls.has("HoldFill")) wrap.append(fillForm(n));
  if (S.calls.has("PositionOpen")) wrap.append(openPositionForm(org, n));
  return wrap;
}

function fillForm(n) {
  const holder = el("input", { name: "holder", placeholder: "a person the record knows", required: true });
  const out = el("div", {});
  const form = el("form", { class: "act", onsubmit: async ev => {
    ev.preventDefault();
    out.replaceChildren(el("div", { class: "receipt" }, "proposing…"));
    try {
      const reply = await command(S.app, "HoldFill", { request_id: requestId(), record_head: S.record.record_head, position: n.id, holder: holder.value.trim() });
      out.replaceChildren(receiptCard(reply, `proposed ${holder.value.trim()} holding ${n.name}; the Board ratifies it`));
    } catch (e) { out.replaceChildren(el("div", { class: "receipt refused" }, errorText(e))); }
  } },
    el("h3", {}, "fill this position"),
    el("label", {}, "holder", holder),
    el("div", { class: "actions" }, el("button", { class: "primary", type: "submit" }, "Propose the hold")),
    out,
  );
  return form;
}

function openPositionForm(org, under) {
  const name = el("input", { name: "position", placeholder: under ? under.name + "/…" : "team/role", required: true, value: under ? under.name + "/" : "" });
  const text = el("input", { name: "text", placeholder: "what the position keeps, one line" });
  const mandate = el("textarea", { name: "mandate", placeholder: "Decides: … May not: … Cites: … Escalates: …", required: true });
  const out = el("div", {});
  return el("form", { class: "act", onsubmit: async ev => {
    ev.preventDefault();
    out.replaceChildren(el("div", { class: "receipt" }, "proposing…"));
    try {
      const reply = await command(S.app, "PositionOpen", {
        request_id: requestId(), record_head: S.record.record_head,
        position: "position:" + name.value.trim(), text: text.value.trim(), mandate: mandate.value.trim(),
        under: under ? under.id : "",
      });
      out.replaceChildren(receiptCard(reply, `proposed opening ${name.value.trim()} with its mandate; the Board ratifies both`));
    } catch (e) { out.replaceChildren(el("div", { class: "receipt refused" }, errorText(e))); }
  } },
    el("h3", {}, under ? "open a position under " + under.name : "open a position"),
    el("label", {}, "name", name),
    el("label", {}, "keeps", text),
    el("label", {}, "mandate", mandate),
    el("div", { class: "actions" }, el("button", { class: "primary", type: "submit" }, "Propose the opening")),
    out,
  );
}

function receiptCard(reply, said) {
  if (!reply || reply.ok === false) return el("div", { class: "receipt refused" }, "refused: ", (reply && reply.code) || "unknown", " ", receiptWhy(reply));
  return el("div", { class: "receipt" }, said, el("br"), el("span", { class: "mono" }, "head ", String(reply.head || "").slice(0, 12), " · revision ", String(reply.revision || "")), receiptWhy(reply));
}
function receiptWhy(reply) {
  const r = reply && reply.receipt;
  const s = r && (r.structure || r);
  const why = s && (s.why || s.warning);
  return why ? el("div", { class: "note" }, why) : "";
}

// ---- the board: reviews and handed tasks as columns ---------------------

function renderBoard(main) {
  const reviews = S.reviews || [];
  const tasks = S.tasks || [];
  const columns = [
    { key: "handed", title: "Handed", items: tasks.filter(t => !["done", "delivered", "refused", "retired"].includes(t.state)).map(taskCard), unavailable: S.errors.tasks },
    { key: "verdict", title: "Needs a verdict", items: reviews.filter(r => r.state === "pending").map(reviewCard) },
    { key: "settled", title: "Settled", items: reviews.filter(r => r.state === "settled").slice().reverse().slice(0, 40).map(reviewCard) },
    { key: "done", title: "Done", items: tasks.filter(t => ["done", "delivered"].includes(t.state)).map(taskCard), unavailable: S.errors.tasks },
  ];
  const board = el("div", { class: "board", role: "list" }, columns.map(c => el("section", { class: "column", role: "listitem", "aria-label": c.title },
    el("header", {}, el("h2", {}, c.title), el("span", { class: "n" }, c.unavailable ? "" : String(c.items.length))),
    c.unavailable ? el("div", { class: "empty", title: errorText(c.unavailable) }, "tasks are not readable from this head yet (", c.unavailable.code, ")")
      : c.items.length ? el("ul", { class: "cards" }, c.items) : el("div", { class: "empty" }, "nothing here"),
  )));
  const current = S.route.id;
  const insp = el("aside", { class: "panel insp", "aria-label": "Inspector" });
  const review = reviews.find(r => r.id === current);
  const task = tasks.find(t => t.id === current);
  if (review) insp.append(reviewInspector(review));
  else if (task) insp.append(taskInspector(task));
  else insp.append(el("h2", {}, "Select a card"), el("p", { class: "note" }, "A card is a Review the Board decides or a Task a person holds. Nothing here is a leg's attempt; that is work in flight, read elsewhere."));
  main.append(el("div", { class: "split" }, board, insp));
}

// What a Review is about, when its row carries no question: the
// proposal it decides, by name.
function reviewTitle(r) {
  if (r.question) return r.question;
  const p = (S.practices || []).find(x => x.review_id === r.id || x.digest === r.knowledge_digest);
  if (p) return `ratify ${p.kind} ${p.name}`;
  const n = (S.knowledge || []).find(x => x.id === r.knowledge_digest);
  if (n) return `ratify ${n.kind} ${n.name || ""}`.trim();
  return r.id;
}

function reviewCard(r) {
  return el("li", {}, el("a", { class: "card", href: href("board", r.id), "aria-current": r.id === S.route.id ? "true" : null },
    el("span", { class: "title" }, reviewTitle(r)),
    el("span", { class: "meta" }, r.state === "pending" ? `needs ${r.required_authority}` : (r.settled || r.outcome), " · ", el("span", { class: "mono" }, r.id)),
  ));
}
function taskCard(t) {
  return el("li", {}, el("a", { class: "card", href: href("board", t.id), "aria-current": t.id === S.route.id ? "true" : null },
    el("span", { class: "title" }, t.outcome || t.id),
    el("span", { class: "meta" }, t.assignee ? "held by " + t.assignee : "unassigned", " · ", t.state, t.waiting ? " · waiting: " + t.waiting : ""),
  ));
}

function reviewInspector(r) {
  const wrap = el("div", {}, el("h2", {}, reviewTitle(r)));
  wrap.append(el("dl", {},
    el("dt", {}, "review"), el("dd", { class: "mono" }, r.id),
    el("dt", {}, "state"), el("dd", {}, el("span", { class: "badge " + r.state }, r.state), r.settled ? " " + r.settled : ""),
    el("dt", {}, "requires"), el("dd", {}, r.required_authority || "", r.approvers ? " · signers " + r.approvers : ""),
    el("dt", {}, "subject"), el("dd", { class: "mono" }, r.subject_digest),
  ));
  const practice = (S.practices || []).find(p => p.digest === r.knowledge_digest || p.review_id === r.id);
  if (practice) wrap.append(el("h3", {}, practice.kind), el("p", { class: "mono note" }, practice.name), practice.text_available ? el("p", { class: "text" }, practice.text) : el("p", { class: "note" }, "text " + practice.text_status));
  if (r.state === "pending" && S.calls.has("ReviewVerdict")) wrap.append(verdictForm(r));
  return wrap;
}

function verdictForm(r) {
  const choice = el("select", { name: "verdict" }, ["approve", "reject", "revise"].map(v => el("option", { value: v }, v)));
  const comment = el("input", { name: "comment", placeholder: "why, in a line" });
  const out = el("div", {});
  return el("form", { class: "act", onsubmit: async ev => {
    ev.preventDefault();
    out.replaceChildren(el("div", { class: "receipt" }, "deciding…"));
    try {
      const reply = await command(S.app, "ReviewVerdict", { request_id: requestId(), review_id: r.id, subject_digest: r.subject_digest, verdict: choice.value, comment: comment.value.trim() });
      out.replaceChildren(receiptCard(reply, `${choice.value} recorded on ${r.id}`));
    } catch (e) { out.replaceChildren(el("div", { class: "receipt refused" }, errorText(e))); }
  } },
    el("h3", {}, "decide"),
    el("label", {}, "verdict", choice),
    el("label", {}, "comment", comment),
    el("div", { class: "actions" }, el("button", { class: "primary", type: "submit" }, "Record the verdict")),
    out,
  );
}

function taskInspector(t) {
  const wrap = el("div", {}, el("h2", {}, t.outcome || t.id));
  wrap.append(el("dl", {},
    el("dt", {}, "task"), el("dd", { class: "mono" }, t.id),
    el("dt", {}, "state"), el("dd", {}, el("span", { class: "badge" }, t.state), t.waiting ? " waiting: " + t.waiting : ""),
    el("dt", {}, "assignee"), el("dd", {}, t.assignee || el("span", { class: "note" }, "unassigned")),
    el("dt", {}, "obligation"), el("dd", {}, t.obligation || "", t.evidence_required ? " · evidence required" : ""),
  ));
  const acts = el("div", { class: "actions" });
  if (S.calls.has("TaskTake") && !t.assignee) acts.append(el("button", { onclick: () => act(wrap, "TaskTake", { task_id: t.id }, "taken") }, "Take it"));
  if (S.calls.has("TaskDone") && (!t.assignee || t.assignee === (S.principal && S.principal.name))) {
    const text = el("input", { placeholder: "what was done, in a line" });
    acts.append(text, el("button", { class: "primary", onclick: () => act(wrap, "TaskDone", { task_id: t.id, text: text.value.trim() }, "reported done") }, "Report done"));
  }
  wrap.append(acts);
  return wrap;
}

async function act(wrap, member, payload, said) {
  const out = el("div", { class: "receipt" }, "…");
  wrap.append(out);
  try {
    const reply = await surface(member.replace(/^Task/, "").toLowerCase() === "take" ? "take_task" : "complete_task", payload);
    out.replaceChildren(reply.ok ? said + (reply.state ? " · " + reply.state : "") : "refused: " + (reply.code || "") + " " + (reply.why || ""));
    if (!reply.ok) out.classList.add("refused");
  } catch (e) { out.classList.add("refused"); out.replaceChildren(errorText(e)); }
}

// ---- knowledge: every node by kind, one node with its relations -----------

const KIND_ORDER = ["purpose", "mandate", "practice", "idea", "concept", "graph", "design", "document"];

function renderKnowledge(main) {
  if (S.errors.knowledge) { main.append(stateCard("unavailable", "knowledge is unavailable: " + errorText(S.errors.knowledge))); return; }
  const nodes = S.knowledge || [];
  const q = (S.filter || "").toLowerCase();
  const shown = nodes.filter(n => !q || (n.name || "").toLowerCase().includes(q) || (n.text || "").toLowerCase().includes(q));
  const kinds = [...new Set(shown.map(n => n.kind))].sort((a, b) => (KIND_ORDER.indexOf(a) + 99) % 99 - (KIND_ORDER.indexOf(b) + 99) % 99 || a.localeCompare(b));
  const search = el("input", { type: "search", placeholder: `search ${nodes.length} nodes`, value: S.filter || "", "aria-label": "Search knowledge", oninput: ev => { S.filter = ev.target.value; const list = $("kn-list"); if (list) list.replaceWith(knowledgeList(search)); } });
  const list = knowledgeList(search);
  const page = el("section", { class: "panel node", "aria-label": "Node" });
  const node = nodes.find(n => n.id === S.route.id);
  if (!node) {
    page.append(el("h2", {}, "Knowledge"), el("p", { class: "note" }, `${nodes.length} nodes the record holds: the purpose, the mandates of its positions, the practices in force. Select one.`));
    if (S.calls.has("KnowledgeNodePropose")) page.append(nodeForm(null));
  } else {
    page.append(nodePage(node));
    readNodeRelations(node.id).then(rel => page.append(relations(node, rel)));
    if (S.calls.has("KnowledgeNodeRevise") && node.accepted) page.append(nodeForm(node));
    if (S.calls.has("KnowledgeNodeRetire") && node.accepted) page.append(retireForm(node));
  }
  main.append(el("div", { class: "kn" }, list, page));

  function knowledgeList(searchEl) {
    return el("div", { class: "list", id: "kn-list" }, searchEl, kinds.map(k => el("div", { class: "group" },
      el("h3", {}, k, " ", el("span", { class: "mono" }, String(shown.filter(n => n.kind === k).length))),
      el("ul", { class: "items" }, shown.filter(n => n.kind === k).sort((a, b) => (a.name || "").localeCompare(b.name || "")).map(n =>
        el("li", {}, el("a", { href: href("knowledge", n.id), "aria-current": n.id === S.route.id ? "true" : null },
          n.name || n.id.slice(0, 19), el("span", { class: "sub" }, (n.text || "").slice(0, 90))))))),
    ));
  }
}

// Propose a node, or a revision of one: the record's Knowledge policy
// decides whether it is admitted, reviewed or refused.
function nodeForm(prev) {
  const kind = el("select", { name: "kind" }, ["practice", "idea", "concept"].map(k => el("option", { value: k, selected: prev && prev.kind === k ? "" : null }, k)));
  const name = el("input", { name: "name", placeholder: "family/name", required: true, value: prev ? prev.name || "" : "" });
  const text = el("textarea", { name: "text", required: true, placeholder: "the knowledge, as it should read" }, prev ? prev.text || "" : "");
  const target = el("input", { name: "target", placeholder: "org, or position:<name>", value: "org" });
  const rationale = el("input", { name: "rationale", placeholder: "why, in a line" });
  const out = el("div", {});
  return el("form", { class: "act", onsubmit: async ev => {
    ev.preventDefault();
    out.replaceChildren(el("div", { class: "receipt" }, "proposing…"));
    const payload = { request_id: requestId(), record_head: S.record.record_head, kind: kind.value, name: name.value.trim(), text: text.value, author: (S.principal && S.principal.name) || "", target: target.value.trim() || "org", rationale: rationale.value.trim() };
    if (prev) payload.supersedes = prev.id;
    try {
      const reply = await command(S.app, prev ? "KnowledgeNodeRevise" : "KnowledgeNodePropose", payload);
      out.replaceChildren(receiptCard(reply, prev ? `revision of ${prev.name} proposed` : `${kind.value} ${name.value.trim()} proposed`));
    } catch (e) { out.replaceChildren(el("div", { class: "receipt refused" }, errorText(e))); }
  } },
    el("h3", {}, prev ? "revise this node" : "propose a node"),
    prev ? "" : el("label", {}, "kind", kind),
    el("label", {}, "name", name),
    el("label", {}, "text", text),
    el("label", {}, "target", target),
    el("label", {}, "rationale", rationale),
    el("div", { class: "actions" }, el("button", { class: "primary", type: "submit" }, prev ? "Propose the revision" : "Propose the node")),
    out,
  );
}

function retireForm(node) {
  const rationale = el("input", { name: "rationale", placeholder: "why it goes, in a line", required: true });
  const out = el("div", {});
  return el("form", { class: "act", onsubmit: async ev => {
    ev.preventDefault();
    out.replaceChildren(el("div", { class: "receipt" }, "proposing…"));
    try {
      const reply = await command(S.app, "KnowledgeNodeRetire", { request_id: requestId(), record_head: S.record.record_head, id: node.id, rationale: rationale.value.trim() });
      out.replaceChildren(receiptCard(reply, `retirement of ${node.name} proposed`));
    } catch (e) { out.replaceChildren(el("div", { class: "receipt refused" }, errorText(e))); }
  } },
    el("h3", {}, "retire this node"),
    el("label", {}, "rationale", rationale),
    el("div", { class: "actions" }, el("button", { type: "submit" }, "Propose the retirement")),
    out,
  );
}

function nodePage(n) {
  const practice = (S.practices || []).find(p => p.digest === n.id);
  const wrap = el("div", {},
    el("header", {}, el("h2", {}, n.name || n.id), el("span", { class: "badge " + (n.accepted ? "ratified" : n.projection_state) }, n.kind, " · ", n.accepted ? "ratified" : n.projection_state)),
    el("p", { class: "text" }, n.text || ""),
    el("dl", {},
      el("dt", {}, "id"), el("dd", { class: "mono" }, n.id),
      el("dt", {}, "author"), el("dd", {}, n.author || ""),
      el("dt", {}, "revision"), el("dd", { class: "mono" }, n.revision || ""),
      n.supersedes ? el("dt", {}, "supersedes") : "", n.supersedes ? el("dd", {}, el("a", { href: href("knowledge", n.supersedes), class: "mono" }, n.supersedes.slice(0, 19))) : "",
      practice && practice.review_id ? el("dt", {}, "review") : "", practice && practice.review_id ? el("dd", {}, el("a", { href: href("board", practice.review_id), class: "mono" }, practice.review_id), practice.review_outcome ? " · " + practice.review_outcome : "") : "",
      practice && practice.rationale ? el("dt", {}, "rationale") : "", practice && practice.rationale ? el("dd", {}, practice.rationale) : "",
    ),
  );
  return wrap;
}

function relations(n, rel) {
  const wrap = el("div", {});
  wrap.append(el("h3", {}, "bound to"));
  if (rel.bindings === null) wrap.append(el("p", { class: "note" }, "bindings unavailable"));
  else if (!rel.bindings.length) wrap.append(el("p", { class: "note" }, "nothing: this node binds no target"));
  else wrap.append(el("ul", { class: "edges" }, rel.bindings.map(b => el("li", {}, el("span", { class: "rel" }, b.class || "binds"), targetLink(b.target), el("span", { class: "note" }, b.author ? "by " + b.author : "")))));
  wrap.append(el("h3", {}, "edges"));
  if (rel.edges === null) wrap.append(el("p", { class: "note" }, "edges unavailable"));
  else if (!rel.edges.length) wrap.append(el("p", { class: "note" }, "none"));
  else wrap.append(el("ul", { class: "edges" }, rel.edges.map(e => {
    const other = e.from_id === n.id ? e.to_id : e.from_id;
    const otherNode = (S.knowledge || []).find(k => k.id === other);
    return el("li", {}, el("span", { class: "rel" }, (e.from_id === n.id ? "→ " : "← ") + (e.rel || e.kind || "")), el("a", { href: href("knowledge", other) }, otherNode ? otherNode.name : other));
  })));
  return wrap;
}

function targetLink(target) {
  if (!target) return el("span", { class: "note" }, "org");
  if (target.startsWith("position:")) return el("a", { href: href("org", target) }, target.slice(9));
  return el("span", { class: "mono" }, target);
}

// ---- shell ---------------------------------------------------------------

function renderShell() {
  const idx = S.index || {};
  const horizon = $("horizon");
  horizon.replaceChildren();
  const name = idx.name || (S.record && "record " + S.record.record_id.slice(0, 12)) || "organization";
  horizon.append(el("span", { class: "org" }, name));
  if (idx.organisms && parseList(idx.organisms).length) horizon.append(el("span", { class: "note" }, `habitat · ${parseList(idx.organisms).length} grafted`));
  const who = $("who");
  who.replaceChildren(S.principal ? el("span", {}, el("b", {}, S.principal.name), " · ", S.principal.mode) : "");
  for (const a of document.querySelectorAll(".tabs a")) a.setAttribute("aria-current", a.dataset.tab === S.route.tab ? "page" : "false");
  const ev = $("evidence");
  ev.replaceChildren(
    el("span", {}, el("b", {}, "record "), S.record ? S.record.record_head.slice(0, 12) : "…"),
    el("span", {}, el("b", {}, "revision "), S.record ? S.record.record_revision : "…"),
    el("span", {}, el("b", {}, "reads "), Object.entries(S.reads).filter(([, v]) => v).map(([k]) => k).join(" ") || "…"),
  );
}

function render() {
  S.route = readRoute();
  renderShell();
  const main = $("main");
  main.replaceChildren();
  if (S.route.tab === "org") renderOrg(main);
  else if (S.route.tab === "board") renderBoard(main);
  else renderKnowledge(main);
}

async function boot() {
  const main = $("main");
  main.replaceChildren(stateCard("", "reading the record…"));
  try { await loadAll(); }
  catch (e) {
    if (e instanceof ReadError && e.status === 401) { location.href = "/auth/login"; return; }
    main.replaceChildren(stateCard("refused", errorText(e)));
    return;
  }
  render();
}

addEventListener("hashchange", render);
boot();
