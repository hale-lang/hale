// A record's seats, for lanes that send real commands (GH #1104 piece 5).
// The head gates every command on what the record says its socket peer
// holds, and the face's commands reach that socket as the head's own
// process: this maps that uid to a person (`dna.unix.member`) and appends
// the `holds` edges that seat them, before the head starts. The rows are
// the ones `ops::GraphEdge` writes, one journal commit each.
import { execFileSync } from 'node:child_process';

const SEATER = { GIT_AUTHOR_NAME: 'face-seats', GIT_AUTHOR_EMAIL: 'face-seats@example.invalid', GIT_COMMITTER_NAME: 'face-seats', GIT_COMMITTER_EMAIL: 'face-seats@example.invalid' };
function git(root, env, args, input) {
  return execFileSync('git', ['-C', root, ...args], { env: { ...env, ...SEATER }, input, encoding: 'utf8', timeout: 10_000 }).trim();
}

// One journal row, one commit, as the record's own writer appends it.
function appendRow(root, env, kind, entity, body) {
  const head = git(root, env, ['rev-parse', 'refs/dna/journal']);
  const text = git(root, env, ['show', `${head}:journal.jsonl`]);
  const seq = text.split('\n').filter(Boolean).length;
  const row = `{"seq": ${seq}, "kind": ${JSON.stringify(kind)}, "entity": ${JSON.stringify(entity)}, "body": ${JSON.stringify(body)}, "author": "dna"}`;
  const blob = git(root, env, ['hash-object', '-w', '--stdin'], text + '\n' + row + '\n');
  const tree = git(root, env, ['mktree'], `100644 blob ${blob}\tjournal.jsonl\n`);
  const commit = git(root, env, ['commit-tree', tree, '-p', head, '-m', `${kind} ${entity}`]);
  git(root, env, ['update-ref', 'refs/dna/journal', commit, head]);
}
const holds = (position, person) => `holds:position:${position}|${person}`;

// This process's uid — the head's too — is `person` and nobody else.
export function mapPeer(root, env, person) {
  git(root, env, ['config', '--local', '--replace-all', 'dna.unix.member', `uid:${process.getuid()}=${person}`]);
}

// `positions` are position names (`board`, `reviewer`, `api/dev`); `board`
// is the owner role, any seat is `position`.
export function seatRecord(root, env, person, positions) {
  mapPeer(root, env, person);
  for (const position of positions) {
    const node = 'position:' + position;
    // the position itself, as `ops::graph_node_body` states it, once
    if (!git(root, env, ['show', 'refs/dna/journal:journal.jsonl']).includes(`"kind": "graph.node", "entity": ${JSON.stringify(node)}`)) {
      appendRow(root, env, 'graph.node', node, JSON.stringify({ kind: 'position', name: position }));
    }
    const body = JSON.stringify({ kind: 'holds', members: [{ role: 'position', node }, { role: 'holder', node: person }] });
    appendRow(root, env, 'graph.edge', holds(position, person), body);
  }
}
// A governed decision takes the seats out again; the head re-reads the
// record within a second of it moving.
export function unseatRecord(root, env, person, positions) {
  for (const position of positions) appendRow(root, env, 'graph.retired', holds(position, person), '{}');
}
