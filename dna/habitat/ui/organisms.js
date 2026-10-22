// The operator face, screen 4: organisms. The organisms the habitat has
// grafted (from the index) with the allocations placed for each, then the
// saplings the habitat pins and the nurseries it trusts. A sapling is
// grafted on a host the person picks (Commands::graft): the row answers at
// once with where the graft writes, and the page follows the graft by what
// ends it, the organism in the index. Every call is the generated client's
// (head-client.js).
import {
  commandsIndex,
  commandsAllocations,
  commandsSaplings,
  commandsNurseries,
  commandsHosts,
  commandsGraft,
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

// The organisms the index lists, by name.
async function placedNames() {
  const i = await commandsIndex({ endpoint }, { limit: 200 });
  return i.kind === "result" ? parse(i.value.organisms).map((o) => o.name) : [];
}

// Follow a graft until its organism is in the index (the end of a graft is
// the record's `habitat.grafted` row), then refresh the lists.
async function follow(name, log) {
  grafting.add(name);
  graftOutcome("grafting " + name + "; it writes " + log);
  for (let tries = 0; tries < 360; tries++) {
    await new Promise((r) => setTimeout(r, 5000));
    let names = [];
    try { names = await placedNames(); } catch { continue; }
    if (names.includes(name)) {
      grafting.delete(name);
      graftOutcome("grafted " + name);
      await refresh();
      return;
    }
  }
  grafting.delete(name);
  graftOutcome("grafting " + name + " has not ended; it writes " + log);
}

async function graft(name, host) {
  const out = await commandsGraft({ endpoint }, { sapling: name, host, library: true });
  if (out.kind !== "result") {
    graftOutcome(out.refusal ? out.refusal.kind + ": " + out.refusal.reason : "the head did not answer");
    return;
  }
  if (!out.value.ok) { graftOutcome(out.value.code + ": " + out.value.why); return; }
  follow(name, out.value.log);
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

function fill(id, rows) {
  const body = $(id);
  body.replaceChildren();
  for (const cells of rows) {
    const tr = document.createElement("tr");
    tr.append(...cells);
    body.append(tr);
  }
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
    cell(allocs.filter((x) => x.organism === o.name).map((x) => x.allocation).join(" ") || "none"),
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
