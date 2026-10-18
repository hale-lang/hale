// A record's seats, for lanes that send real commands (GH #1104 piece 5).
// The head gates every command on what the record says its socket peer
// holds, and the face's commands reach that socket as the head's own
// process: this maps that uid to a person (`dna.unix.member`) and appends
// the `holds` edges that seat them, before the head starts. The rows are
// the ones `ops::GraphEdge` writes, each landed through memory's insert
// function (GH #1590: the record is memory's).
import { execFileSync } from 'node:child_process';
import { appendRecordRow, recordState } from './record-memory.mjs';

function git(root, env, args, input) {
  return execFileSync('git', ['-C', root, ...args], { env, input, encoding: 'utf8', timeout: 10_000 }).trim();
}

// One record row, as the record's own writer appends it; `author` is the
// row's author field, the record's writer unless given.
function appendRow(root, env, kind, entity, body, author = 'dna') {
  appendRecordRow(root, kind, entity, body, author);
}
const journalText = root => recordState(root).text;
const holds = (position, person) => `holds:position:${position}|${person}`;

// This process's uid — the head's too — is `person` and nobody else.
export function mapPeer(root, env, person) {
  git(root, env, ['config', '--local', '--replace-all', 'dna.unix.member', `uid:${process.getuid()}=${person}`]);
}

// `positions` are position names (`board`, `reviewer`, `api/dev`); `board`
// is the owner role, any seat is `position`.
export function seatRecord(root, env, person, positions) {
  mapPeer(root, env, person);
  // Memory projects a hold only for a person the record knows: an owner's
  // member, or the author of an earlier row in their own name (GH #1087
  // review). A fixture record names no members, and a hold memory refuses
  // is written back as a `hold.refused` row, which moves the record past
  // its projection and turns every Knowledge read into 503 until the next
  // tick. So the person writes one row of their own before their seat.
  const journal = journalText(root);
  if (!journal.includes(`"author": ${JSON.stringify(person)}`)) {
    appendRow(root, env, 'browser.fixture', 'person:' + person, `${person} is a person this record knows`, person);
  }
  for (const position of positions) {
    const node = 'position:' + position;
    // the position itself, as `ops::graph_node_body` states it, once
    if (!journalText(root).includes(`"kind": "graph.node", "entity": ${JSON.stringify(node)}`)) {
      appendRow(root, env, 'graph.node', node, JSON.stringify({ kind: 'position', name: position }));
    }
    const body = JSON.stringify({ kind: 'holds', members: [{ role: 'position', node }, { role: 'holder', node: person }] });
    appendRow(root, env, 'graph.edge', holds(position, person), body);
  }
}
// GH #1123: who owns what is the graph's — `<node> <holder>` lines, a
// firm holding a position (`position:org organization:acme`) or a person
// holding a firm (`organization:acme alice`). Each node an edge names is
// stated first, once, then the `holds` edge; the rows are the ones
// `ops::graph_seat` writes.
export function seatGraph(root, env, lines) {
  for (const line of lines.split('\n').map(l => l.trim()).filter(Boolean)) {
    const [node, holder] = line.split(' ');
    for (const id of [node, holder.startsWith('organization:') ? holder : null].filter(Boolean)) {
      if (journalText(root).includes(`"kind": "graph.node", "entity": ${JSON.stringify(id)}`)) continue;
      const colon = id.indexOf(':');
      appendRow(root, env, 'graph.node', id, JSON.stringify({ kind: id.slice(0, colon), name: id.slice(colon + 1) }));
    }
    const body = JSON.stringify({ kind: 'holds', members: [{ role: 'position', node }, { role: 'holder', node: holder }] });
    appendRow(root, env, 'graph.edge', `holds:${node}|${holder}`, body);
  }
}
// The `holds` edges of `lines` taken out again.
export function unseatGraph(root, env, lines) {
  for (const line of lines.split('\n').map(l => l.trim()).filter(Boolean)) {
    const [node, holder] = line.split(' ');
    appendRow(root, env, 'graph.retired', `holds:${node}|${holder}`, '{}');
  }
}
