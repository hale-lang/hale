// The operator face, screen 1: sign in, then the habitat's name and state.
// Screen 2 (hardware.js) is in the same page: the hash picks the screen.
//
// No build step: this module and the generated client of the head's rows
// (head-client.js, `hale api client --surface HeadCommands --lang js`, kept
// current by scripts/api-clients.sh) are all it loads.
// The page is served by the standard face under /app/, behind the head's
// gate: a visitor with no session is sent to the provider before this page
// is ever served. A read the head refuses for want of a session is shown as
// the crossing, and the link starts the same sign-in again.
import { commandsIndex } from "./head-client.js";
import { refresh as refreshHardware } from "./hardware.js";
import { refresh as refreshInfrastructure } from "./infrastructure.js";
import { refresh as refreshOrganisms } from "./organisms.js";

const endpoint = location.origin + "/api/hale/v1/head/surface";
const $ = (id) => document.getElementById(id);

function show(id, on) { $(id).hidden = !on; }

function setState(el, state) {
  el.dataset.state = state;
  el.textContent = state;
}

function parse(list) {
  try { return JSON.parse(list || "[]"); } catch { return []; }
}

function crossing(why) {
  show("crossing", true);
  show("habitat", false);
  show("organisms", false);
  $("crossing-why").textContent = why;
  setState($("state"), "down");
}

function render(r) {
  const organisms = parse(r.organisms);
  const name = (r.purpose || "").split("\n")[0].trim() || "habitat";
  show("crossing", false);
  show("habitat", true);
  $("name").textContent = name;
  $("habitat-name").textContent = name;
  // the habitat answers: up; a record that reports a reason is degraded
  const state = r.why ? "degraded" : "up";
  setState($("state"), state);
  setState($("habitat-state"), state);
  // the record carries no pulse of its own for the habitat: say so
  $("habitat-pulse").textContent = "none recorded";
  $("habitat-record").textContent = "revision " + r.revision;
  $("habitat-count").textContent = String(organisms.length);
  const rows = $("organism-rows");
  rows.replaceChildren();
  for (const o of organisms) {
    const tr = document.createElement("tr");
    const n = document.createElement("td");
    n.textContent = o.name;
    const s = document.createElement("td");
    s.textContent = o.state;
    if (o.state === "up") s.dataset.state = "up";
    const f = document.createElement("td");
    f.textContent = o.face;
    tr.append(n, s, f);
    rows.append(tr);
  }
  show("organisms", organisms.length > 0);
}

async function read() {
  try {
    const out = await commandsIndex({ endpoint }, { limit: 200 });
    if (out.kind === "result") return render(out.value);
    if (out.kind === "refusal") {
      const r = out.refusal;
      if (r.kind === "unauthenticated" || r.kind === "unauthorized") {
        return crossing(r.reason || "the head wants a session");
      }
      return crossing("the head refused the read: " + r.kind + " " + r.reason);
    }
    setState($("state"), "degraded");
  } catch (e) {
    setState($("state"), "down");
    show("habitat", false);
    show("organisms", false);
  }
}

read();
setInterval(read, 5000);

// One page, four screens: #habitat, #hardware, #infrastructure and #organisms.
function route() {
  const screen = location.hash === "#hardware" ? "hardware"
    : location.hash === "#infrastructure" ? "infrastructure"
    : location.hash === "#organisms" ? "organisms" : "habitat";
  for (const s of ["habitat", "hardware", "infrastructure", "organisms"]) {
    show("screen-" + s, s === screen);
    const link = $("nav-" + s);
    if (s === screen) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  if (screen === "hardware") refreshHardware();
  if (screen === "infrastructure") refreshInfrastructure();
  if (screen === "organisms") refreshOrganisms();
}
window.addEventListener("hashchange", route);
route();
setInterval(() => {
  if (location.hash === "#hardware") refreshHardware();
  if (location.hash === "#infrastructure") refreshInfrastructure();
  if (location.hash === "#organisms") refreshOrganisms();
}, 5000);
