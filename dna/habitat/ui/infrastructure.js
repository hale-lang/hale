// The operator face, screen 3: infrastructure. What the habitat has
// allocated (role, instance, provider, host, and the state its provider
// observes) and the policy that says which provider a role is allocated by.
// Reads only, through the generated client (head-client.js).
import { commandsAllocations, commandsPolicy } from "./head-client.js";

const endpoint = location.origin + "/api/hale/v1/head/surface";
const $ = (id) => document.getElementById(id);

function parse(list) {
  try { return JSON.parse(list || "[]"); } catch { return []; }
}

function cell(text) {
  const td = document.createElement("td");
  td.textContent = text;
  return td;
}

function stateCell(a) {
  const td = cell(a.state || "not observed");
  if (a.detail) td.title = a.detail;
  if (a.state === "up" || a.state === "degraded" || a.state === "down") td.dataset.state = a.state;
  return td;
}

function outcome(text) {
  const el = $("allocation-outcome");
  el.textContent = text;
  el.hidden = !text;
}

export async function refresh() {
  let allocs, policy;
  try {
    const a = await commandsAllocations({ endpoint }, { limit: 1000 });
    const p = await commandsPolicy({ endpoint }, { limit: 200 });
    if (a.kind !== "result" || p.kind !== "result") {
      const r = (a.kind !== "result" ? a : p).refusal;
      outcome(r ? r.kind + ": " + r.reason : "the head did not answer");
      return;
    }
    allocs = parse(a.value.allocations);
    policy = parse(p.value.policy);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  outcome("");
  const rows = $("allocation-rows");
  rows.replaceChildren();
  for (const a of allocs) {
    const tr = document.createElement("tr");
    tr.append(cell(a.role), cell(a.instance), cell(a.provider), cell(a.host || "none"), stateCell(a));
    rows.append(tr);
  }
  $("allocations-empty").hidden = allocs.length > 0;
  const prows = $("policy-rows");
  prows.replaceChildren();
  for (const r of policy) {
    const tr = document.createElement("tr");
    tr.append(cell(r.role), cell(r.provider), cell(r.profile));
    prows.append(tr);
  }
}
