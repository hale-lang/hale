import type { Node, SeatView, UsageRecord } from '../api/client';
import type { Fleet } from './network-model';

// Repository is caller metadata on demo requests, not a field on Voice's Project.
const PROJECTS = [
  ['research', 'hale-lang/hale'], ['product', 'hale-lang/voice'], ['automation', 'homelab'],
  ['compiler', 'hale-lang/hale'], ['admin', 'hale-lang/voice'], ['infrastructure', 'homelab'],
  ['evals', 'hale-lang/hale'],
] as const;
export const DEMO_TICK_MS = 1800;
export const demoProjectColor = (project: string) => ['#bbf879', '#45f4ff', '#f17cff', '#ffbb74', '#a99aff', '#78ffe0', '#ff8eac'][Math.max(0, PROJECTS.findIndex(([id]) => id === project))]!;

/** Explicitly synthetic, local-only fixture. Model names are illustrative catalog aliases. */
export function demoFleet(tick: number, now: number): Fleet {
  // Placement follows the homelab profile. Catalog entries are alternatives,
  // not a claim that all weights are loaded at once; q4 names denote demo quantizations.
  const definitions = [
    ['studio1', [['mlx', 'local', ['qwen-235b-q4', 'llama-70b']]]],
    ['h2', [['claude-cli', 'claude-personal', ['sonnet', 'haiku', 'opus', 'sonnet-thinking']], ['agy', 'agy', ['gemini-pro', 'gemini-flash', 'gemini-flash-lite', 'gemini-thinking']], ['vibe', 'vibe', ['devstral', 'codestral', 'mistral-large', 'mistral-small']], ['llama-cpp', 'local', ['llama-8b', 'qwen-7b']]]],
    ['m1', [['mlx', 'local', ['qwen-32b', 'qwen-coder-32b']]]],
    ['h1', [['claude-cli', 'claude-work', ['sonnet', 'haiku', 'opus', 'sonnet-thinking']], ['codex-cli', 'openai', ['gpt-codex', 'gpt-mini', 'gpt-reasoning', 'gpt-fast']], ['llama-cpp', 'local', ['llama-3b', 'qwen-7b']]]],
    ['studio2', [['mlx', 'local', ['deepseek-r1-q4', 'qwen-235b-q4']], ['mlx', 'local', ['llama-70b']]]],
    ['studio3', [['mlx', 'local', ['llama-70b', 'qwen-coder-32b']]]],
  ] as const;
  const names = { studio1: 'Studio 1 · 256 GB profile', studio2: 'Studio 2 · 512 GB profile', studio3: 'Studio 3 · 128 GB profile', m1: 'M1 Max · 64 GB profile', h1: 'H1 · small local models and CLIs', h2: 'H2 · small local models and CLIs' };
  const nodes: Node[] = definitions.map(([id, slots], index) => {
    const seats: SeatView[] = slots.map(([engine, account, models], j) => ({ id: `seat-${j + 1}`, ref: `${id}/seat-${j + 1}`, node: id, engine, account, data_classes: ['internal'], serves: models.map(model => ({ model })), status: index === 4 && j === 1 ? 'paused' : 'up', in_flight: 0, concurrency: 3, enabled: true, problems: [], reason: index === 4 && j === 1 ? 'Paused by the machine operator.' : null }));
    return { id, name: names[id], status: 'up', seats, in_flight: 0, effective_max_in_flight: 6, assignment_version: 7, applied_version: 7, last_heartbeat_at: new Date(now - 2000).toISOString(), created_at: new Date(now - 86400000).toISOString() };
  });
  const projects = PROJECTS.map(([id]) => ({ id, name: id, status: 'active' as const, owners: [], data_classes: ['internal' as const], created_at: new Date(now - 86400000).toISOString() }));
  const accounts = ['claude-personal', 'claude-work', 'openai', 'agy', 'vibe', 'local'].map((id, index) => ({ id, name: id, provider: id.startsWith('claude-') ? 'anthropic' : id === 'openai' ? 'openai' : null, billing: id === 'local' ? 'none' as const : 'subscription' as const, status: 'active' as const, seats: nodes.flatMap((n) => n.seats.filter((s) => s.account === id).map((s) => s.ref)), windows: id === 'local' ? [] : [{ name: 'five_hour', used_percent: [34, 62, 29, 47, 18][index]!, observed_at: new Date(now).toISOString() }], created_at: new Date(now - 86400000).toISOString() }));
  const fleet: Fleet = { nodes, accounts, projects, usage: [] };
  fleet.usage = Array.from({ length: 77 }, (_, i) => ({ ...demoRequest(fleet, i, now - (77 - i) * 1000), id: `demo-seed-${i}` }));
  for (const { record } of demoInFlight(fleet, tick, now)) {
    const node = nodes.find(n => n.id === record.served!.node)!;
    const seat = node.seats.find(s => s.ref === record.served!.seat)!;
    node.in_flight!++; seat.in_flight++;
  }
  return fleet;
}

export function demoRequest(fleet: Fleet, sequence: number, now: number): UsageRecord {
  const seats = fleet.nodes.flatMap(n => n.seats).filter(s => s.status === 'up');
  const seat = seats[(sequence * 7) % seats.length]!;
  const model = seat.serves[Math.floor(sequence / seats.length) % seat.serves.length]!.model;
  const project = fleet.projects[sequence % fleet.projects.length]!.id;
  const repository = PROJECTS.find(([id]) => id === project)?.[1];
  const failed = sequence > 0 && sequence % 13 === 0;
  const unserved = sequence > 0 && sequence % 17 === 0;
  return { id: `demo-${sequence}`, created_at: new Date(now).toISOString(), project, key: 'demo-key', requested_model: model, endpoint: 'responses', status: unserved ? 'no_capacity' : failed ? 'failed' : 'served', served: unserved ? null : { model, node: seat.node, seat: seat.ref, account: seat.account, engine: seat.engine }, input_tokens: unserved ? 0 : 420 + sequence % 17 * 100, output_tokens: unserved || failed ? 0 : 60 + sequence % 11 * 24, price_micros: unserved || seat.account === 'local' ? 0 : 3400, currency: 'USD', timing: { wall_ms: 920 + sequence % 9 * 140, queue_ms: sequence % 3 * 80, load_ms: 0 }, metadata: repository ? { repository } : {} };
}

// Three staggered workers, each with a gap between requests. No project occupies two workers.
function jobsAt(tick: number) {
  return [0, 1, 2].flatMap(lane => {
    const phase = tick - lane * 2;
    if (phase < 0) return [];
    const sequence = Math.floor(phase / 8) * 3 + lane;
    return [{ sequence, elapsed: phase % 8, duration: 5 + sequence % 2 }];
  });
}
export function demoInFlight(fleet: Fleet, tick: number, now: number) {
  return jobsAt(tick).filter(job => job.elapsed < job.duration).flatMap(job => {
    const record = demoRequest(fleet, job.sequence, now - job.elapsed * DEMO_TICK_MS);
    if (!record.served) return [];
    // This is a graph participant snapshot, never a completed usage row.
    return [{ ...job, record: { ...record, id: `demo-live-${job.sequence}`, status: 'served' as const, input_tokens: 0, output_tokens: 0, price_micros: 0 }, color: demoProjectColor(record.project) }];
  });
}
export function demoCompletions(fleet: Fleet, tick: number, now: number): UsageRecord[] {
  return jobsAt(tick).filter(job => job.elapsed === job.duration).map(job => {
    const record = demoRequest(fleet, job.sequence, now);
    return { ...record, id: `demo-live-${job.sequence}`, timing: { ...record.timing, wall_ms: job.duration * DEMO_TICK_MS } };
  });
}

/** Only used for the synthetic fleet; never interpreted as backend telemetry. */
export function demoContext(record: UsageRecord) {
  const sequence = Number(record.id.replace(/^demo-(?:seed-|live-)?/, ''));
  return { apiInstance: sequence % 2 ? 'api-h1' : 'api-studio3' };
}
