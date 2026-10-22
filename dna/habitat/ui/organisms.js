// The operator face, screen 4: organisms. The organisms the habitat has
// grafted (from the index) with the allocations placed for each, then the
// saplings the habitat pins and the nurseries it trusts. Reads only,
// through the generated client (head-client.js).
import {
  commandsIndex,
  commandsAllocations,
  commandsSaplings,
  commandsNurseries,
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
  let organisms, allocs, saplings, nurseries;
  try {
    const i = await commandsIndex({ endpoint }, { limit: 200 });
    const a = await commandsAllocations({ endpoint }, { limit: 1000 });
    const s = await commandsSaplings({ endpoint }, { limit: 200 });
    const n = await commandsNurseries({ endpoint }, { limit: 200 });
    const bad = [i, a, s, n].find((o) => o.kind !== "result");
    if (bad) {
      outcome(bad.refusal ? bad.refusal.kind + ": " + bad.refusal.reason : "the head did not answer");
      return;
    }
    organisms = parse(i.value.organisms);
    allocs = parse(a.value.allocations);
    saplings = parse(s.value.saplings);
    nurseries = parse(n.value.nurseries);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  outcome("");
  fill("placed-rows", organisms.map((o) => [
    cell(o.name),
    // the index row carries no purpose or pulse for an organism
    cell(o.purpose || "none recorded"),
    cell(o.state, o.state === "up" ? "up" : ""),
    cell(o.pulse || "none recorded"),
    cell(allocs.filter((x) => x.organism === o.name).map((x) => x.allocation).join(" ") || "none"),
  ]));
  $("placed-empty").hidden = organisms.length > 0;
  fill("sapling-rows", saplings.map((x) => [
    cell(x.name),
    cell(x.purpose),
    cell(x.revision),
    cell(x.digest),
    cell(x.nursery),
  ]));
  $("saplings-empty").hidden = saplings.length > 0;
  fill("nursery-rows", nurseries.map((x) => [cell(x.source), cell(x.kind)]));
}
