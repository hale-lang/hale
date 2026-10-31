// The operator face, screen 1: sign in, then the habitat's name and state,
// and the web UIs across its organisms (Commands::uis): each organism's face
// and each UI its manifest declares ([dna.ui.<name>]), with its link and how
// to sign in. A UI whose service is published only on its host's loopback
// is named, not linked; one signed in with a declared secret says which
// command reads it, never its value.
// Screen 2 (hardware.js) is in the same page: the hash picks the screen.
//
// No build step: this module and the generated client of the head's rows
// (head-client.js, `hale api client --surface HeadCommands --lang js`, kept
// current by scripts/api-clients.sh) are all it loads.
// The page is served by the standard face under /app/, behind the head's
// gate: a visitor with no session is sent to the provider before this page
// is ever served. A read the head refuses for want of a session is shown as
// the crossing, and the link starts the same sign-in again.
import { commandsIndex, commandsUis } from "./head-client.js";
import { refresh as refreshHardware } from "./hardware.js";
import { refresh as refreshInfrastructure } from "./infrastructure.js";
import { refresh as refreshOrganisms } from "./organisms.js";
import { refresh as refreshAlerts } from "./alerts.js";

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
  show("uis", false);
  $("crossing-why").textContent = why;
  setState($("state"), "down");
}

function render(r) {
  const organisms = parse(r.organisms);
  // the habitat's name, as its record holds it (`hale habitat up --name`)
  const name = r.name || (r.purpose || "").split("\n")[0].trim() || "habitat";
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
}

// A word as a shell takes it: quoted when it is not plain.
function shellWord(w) {
  return /^[A-Za-z0-9._\/-]+$/.test(w) ? w : "'" + w.replace(/'/g, "'\\''") + "'";
}

function td(text) {
  const c = document.createElement("td");
  c.textContent = text;
  return c;
}

// Where the UI is: a link, its host when only that host's loopback serves
// it, else why it has no address.
function linkCell(u) {
  const c = document.createElement("td");
  if (u.url) {
    const a = document.createElement("a");
    a.href = u.url;
    a.textContent = u.url;
    a.target = "_blank";
    a.rel = "noopener";
    c.append(a);
  } else if (u.loopback_host) {
    c.textContent = "loopback only on " + u.loopback_host;
    c.className = "dim";
  } else {
    c.textContent = u.note || "no address";
    c.className = "dim";
  }
  return c;
}

// How to sign in: an organism's face through the habitat's sign-in; a UI
// that declares a login with the command that reads its secret, run on the
// person's machine through the pointer.
function signInCell(u) {
  const c = document.createElement("td");
  if (u.kind === "face") {
    c.textContent = "the habitat's sign-in";
    c.className = "dim";
  } else if (u.login) {
    const code = document.createElement("code");
    code.textContent = "hale habitat organism " + shellWord(u.organism) + " dna secret show " + u.login;
    c.append("sign in with ", code);
  }
  return c;
}

function renderUis(uis) {
  const rows = $("ui-rows");
  rows.replaceChildren();
  for (const u of uis) {
    const tr = document.createElement("tr");
    tr.append(td(u.label), td(u.organism), linkCell(u), signInCell(u));
    rows.append(tr);
  }
  show("uis", true);
  show("uis-empty", uis.length === 0);
}

async function readUis() {
  const el = $("uis-outcome");
  try {
    const out = await commandsUis({ endpoint }, { limit: 500 });
    if (out.kind === "result" && out.value.ok) {
      el.hidden = true;
      return renderUis(parse(out.value.uis));
    }
    el.textContent = out.kind === "result" ? "the head did not list the UIs: " + out.value.why
      : out.refusal ? out.refusal.kind + ": " + out.refusal.reason : "the head did not answer";
    el.hidden = false;
  } catch (e) {
    el.textContent = "the head did not answer";
    el.hidden = false;
  }
}

async function read() {
  try {
    const out = await commandsIndex({ endpoint }, { limit: 200 });
    if (out.kind === "result") {
      render(out.value);
      return readUis();
    }
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
    show("uis", false);
  }
}

read();
setInterval(read, 5000);

// One page, five screens: #habitat, #hardware, #infrastructure, #organisms
// and #alerts.
function route() {
  const screen = location.hash === "#hardware" ? "hardware"
    : location.hash === "#infrastructure" ? "infrastructure"
    : location.hash === "#organisms" ? "organisms"
    : location.hash === "#alerts" ? "alerts" : "habitat";
  for (const s of ["habitat", "hardware", "infrastructure", "organisms", "alerts"]) {
    show("screen-" + s, s === screen);
    const link = $("nav-" + s);
    if (s === screen) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  if (screen === "hardware") refreshHardware();
  if (screen === "infrastructure") refreshInfrastructure();
  if (screen === "organisms") refreshOrganisms();
  if (screen === "alerts") refreshAlerts();
}
window.addEventListener("hashchange", route);
route();
setInterval(() => {
  // a picker open in a select is not rebuilt under the person's hand
  if (document.activeElement && document.activeElement.tagName === "SELECT") return;
  if (location.hash === "#hardware") refreshHardware();
  if (location.hash === "#infrastructure") refreshInfrastructure();
  if (location.hash === "#organisms") refreshOrganisms();
  if (location.hash === "#alerts") refreshAlerts();
}, 5000);
