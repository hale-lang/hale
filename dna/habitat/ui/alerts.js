// Screen 5: alerts. What the habitat's reflexes say now: each rule its
// senses' store evaluates (the habitat's defaults, and its manifest's
// [dna.reflex.*]) whose condition holds, from the store's own alerts
// (Commands::alerts). `pending` holds but not yet for its declared time;
// `firing` has. A read: nothing here acts, and nothing is recorded.
// @ts-check
import { commandsAlerts } from "./head-client.js";
import { hostLabel } from "./host-label.js";

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

// How long ago `iso` was, in the largest unit that says it.
function ago(iso) {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (s < 90) return `${s}s`;
  if (s < 5400) return `${Math.round(s / 60)}m`;
  if (s < 129600) return `${Math.round(s / 3600)}h`;
  return `${Math.round(s / 86400)}d`;
}

function outcome(text) {
  const el = $("alerts-outcome");
  el.textContent = text;
  el.hidden = !text;
}

export async function refresh() {
  let alerts;
  try {
    const r = await commandsAlerts({ endpoint }, { limit: 200 });
    if (r.kind !== "result") {
      outcome("the head did not answer");
      return;
    }
    if (!r.value.ok) {
      outcome(r.value.why || "the habitat's store did not answer");
      return;
    }
    alerts = parse(r.value.alerts);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  outcome("");
  // firing first, then pending; each oldest first
  alerts.sort((a, b) => (a.state === b.state ? String(a.activeAt).localeCompare(String(b.activeAt)) : a.state === "firing" ? -1 : 1));
  const rows = $("alert-rows");
  rows.replaceChildren();
  for (const a of alerts) {
    const l = a.labels || {};
    const tr = document.createElement("tr");
    const state = cell(a.state || "");
    // firing reads as down, pending as degraded: the screens' one color code
    state.dataset.state = a.state === "firing" ? "down" : "degraded";
    tr.append(
      state,
      cell(l.reflex || l.alertname || ""),
      cell(l.host ? hostLabel(l.host) : ""),
      cell((a.annotations && a.annotations.summary) || ""),
      cell(ago(a.activeAt)),
    );
    rows.append(tr);
  }
  $("alerts-empty").hidden = alerts.length > 0;
}
