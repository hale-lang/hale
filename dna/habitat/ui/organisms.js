// The operator face, screen 4: organisms. The organisms the habitat has
// grafted (from the index) with the allocations placed for each, then the
// saplings the habitat pins and the nurseries it trusts. A sapling is
// grafted on a host the person picks (Commands::graft): the row answers at
// once with the graft's id, and the page follows the graft's log
// (Commands::graft_log) until it ends, then refreshes. An organism is
// removed plan-then-confirm: its plan (Commands::plan_remove_organism) is
// shown with a field for its name, which Commands::remove_organism checks.
// Every call is the generated client's (head-client.js).
import {
  commandsIndex,
  commandsAllocations,
  commandsSaplings,
  commandsNurseries,
  commandsHosts,
  commandsGraft,
  commandsGraftLog,
  commandsPlanRemoveOrganism,
  commandsRemoveOrganism,
} from "./head-client.js";

const endpoint = location.origin + "/api/hale/v1/head/surface";
const $ = (id) => document.getElementById(id);

function parse(list) {
  try { return JSON.parse(list || "[]"); } catch { return []; }
}

function cell(text, state) {
  const td = document.createElement("td");
  td.textContent = text;
  if (state) td.dataset.state = state;
  return td;
}

function outcome(text) {
  const el = $("organism-outcome");
  el.textContent = text;
  el.hidden = !text;
}

function graftOutcome(text) {
  const el = $("graft-outcome");
  el.textContent = text;
  el.hidden = !text;
}

const grafting = new Set();

// Follow a graft's log until it ends (Commands::graft_log), showing what it
// says, then refresh the lists: the organism is in the index by then.
async function follow(name) {
  grafting.add(name);
  let from = 0;
  let text = "";
  for (let tries = 0; tries < 2000; tries++) {
    let out;
    try { out = await commandsGraftLog({ endpoint }, { graft: name, from }); } catch { out = null; }
    if (out && out.kind === "result" && out.value.ok) {
      text += out.value.text;
      from = out.value.next;
      graftOutcome("grafting " + name + "\n" + text.split("\n").slice(-12).join("\n"));
      if (out.value.done) {
        grafting.delete(name);
        graftOutcome((out.value.exit === 0 ? "grafted " : "the graft of " + name + " ended with exit " + out.value.exit + "\n")
          + "\n" + text.split("\n").slice(-12).join("\n"));
        await refresh();
        return;
      }
    } else if (out && out.kind === "result") {
      grafting.delete(name);
      graftOutcome(out.value.code + ": " + out.value.why);
      return;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  grafting.delete(name);
}

async function graft(name, host) {
  const out = await commandsGraft({ endpoint }, { sapling: name, host, library: true });
  if (out.kind !== "result") {
    graftOutcome(out.refusal ? out.refusal.kind + ": " + out.refusal.reason : "the head did not answer");
    return;
  }
  if (!out.value.ok) { graftOutcome(out.value.code + ": " + out.value.why); return; }
  follow(out.value.graft || name);
}

function grafter(name, hosts) {
  const td = document.createElement("td");
  const sel = document.createElement("select");
  for (const h of ["local", ...hosts.filter((x) => x.state === "ready").map((x) => x.name)]) {
    const o = document.createElement("option");
    o.value = h;
    o.textContent = h;
    sel.append(o);
  }
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = "graft";
  b.disabled = grafting.has(name);
  b.addEventListener("click", () => { b.disabled = true; graft(name, sel.value); });
  td.append(sel, " ", b);
  return td;
}

function itemLine(i) {
  const measure = i.measure && i.measure !== "-" ? i.measure : "";
  const rec = i.recoverable && i.recoverable !== "-" ? "recoverable: " + i.recoverable : "";
  const tail = [measure, rec].filter(Boolean).join("; ");
  return i.what + (tail ? " (" + tail + ")" : "");
}

// An organism's removal planned, shown, and waiting for its name.
async function planRemove(name) {
  const out = await commandsPlanRemoveOrganism({ endpoint }, { name });
  if (out.kind !== "result") { outcome(out.refusal ? out.refusal.kind + ": " + out.refusal.reason : ""); return; }
  const p = out.value;
  if (!p.ok) { outcome(p.code + ": " + p.why); return; }
  const list = $("organism-plan");
  list.replaceChildren();
  for (const i of parse(p.plan)) {
    const li = document.createElement("li");
    li.textContent = itemLine(i);
    list.append(li);
  }
  $("organism-name").textContent = p.confirm;
  $("organism-typed").value = "";
  $("organism-confirm").dataset.name = name;
  // the plan shown: the row refuses when what it would do changed since
  $("organism-confirm").dataset.digest = p.digest;
  $("organism-confirm").hidden = false;
  outcome("");
}

function remover(name) {
  const td = document.createElement("td");
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = "remove";
  b.addEventListener("click", () => planRemove(name));
  td.append(b);
  return td;
}

function fill(id, rows) {
  const body = $(id);
  body.replaceChildren();
  for (const cells of rows) {
    const tr = document.createElement("tr");
    tr.append(...cells);
    body.append(tr);
  }
}

// An organism's allocations, each with the addresses its own services
// publish as links (its row's `links`: voice's api, whose `/` is its
// console, among them).
function allocationsCell(list) {
  const td = document.createElement("td");
  if (list.length === 0) { td.textContent = "none"; return td; }
  list.forEach((x, n) => {
    if (n > 0) td.append(" ");
    td.append(x.allocation);
    for (const l of x.links || []) {
      const a = document.createElement("a");
      a.href = l.url;
      a.textContent = l.service;
      a.title = l.url;
      a.target = "_blank";
      a.rel = "noopener";
      td.append(" ", a);
    }
  });
  return td;
}

export async function refresh() {
  let organisms, allocs, saplings, nurseries, hosts;
  try {
    const i = await commandsIndex({ endpoint }, { limit: 200 });
    const a = await commandsAllocations({ endpoint }, { limit: 1000 });
    const s = await commandsSaplings({ endpoint }, { limit: 200 });
    const n = await commandsNurseries({ endpoint }, { limit: 200 });
    const h = await commandsHosts({ endpoint }, { limit: 200 });
    const bad = [i, a, s, n, h].find((o) => o.kind !== "result");
    if (bad) {
      outcome(bad.refusal ? bad.refusal.kind + ": " + bad.refusal.reason : "the head did not answer");
      return;
    }
    organisms = parse(i.value.organisms);
    allocs = parse(a.value.allocations);
    saplings = parse(s.value.saplings);
    nurseries = parse(n.value.nurseries);
    hosts = parse(h.value.hosts);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  outcome("");
  fill("placed-rows", organisms.map((o) => [
    cell(o.name),
    cell(o.purpose || ""),
    cell(o.state, o.state === "up" ? "up" : ""),
    cell(o.pulse || ""),
    allocationsCell(allocs.filter((x) => (o.key && x.allocation.startsWith(o.key + "/")) || x.organism === o.name)),
    remover(o.name),
  ]));
  $("placed-empty").hidden = organisms.length > 0;
  fill("sapling-rows", saplings.map((x) => [
    cell(x.name),
    cell(x.purpose),
    cell(x.revision),
    cell(x.digest),
    cell(x.nursery),
    grafter(x.name, hosts),
  ]));
  $("saplings-empty").hidden = saplings.length > 0;
  fill("nursery-rows", nurseries.map((x) => [cell(x.source), cell(x.kind)]));
}

$("organism-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  // the row checks the name typed back, and refuses a mismatch
  const out = await commandsRemoveOrganism(
    { endpoint },
    {
      name: $("organism-confirm").dataset.name,
      confirm: $("organism-typed").value.trim(),
      digest: $("organism-confirm").dataset.digest,
    },
  );
  if (out.kind !== "result") { outcome(out.refusal ? out.refusal.kind + ": " + out.refusal.reason : ""); return; }
  const r = out.value;
  $("organism-confirm").hidden = true;
  const lines = parse(r.report).map((x) => x.state + ": " + x.what + (x.why ? " (" + x.why + ")" : ""));
  outcome((r.ok ? "" : r.code + ": " + r.why + "\n") + lines.join("\n"));
  await refresh();
});

$("organism-cancel").addEventListener("click", () => { $("organism-confirm").hidden = true; });
