// The fit's negative tests: each file in negative/ changes one side, a
// copy of the bundled contract or of the exported surface, in a way the
// fit must refuse; this runs bundle.mjs on the copy and fails unless it
// exits non-zero and says what the file expects.
//   usage: node negative.mjs <bundled-contract.json> <Voice.openapi.json> <gaps.json>
import fs from 'fs';
import os from 'os';
import path from 'path';
import { spawnSync } from 'child_process';

const [contractPath, bundlePath, gapsPath] = process.argv.slice(2);
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'voice-negative-'));
let fails = 0;
const cases = fs.readdirSync('negative').filter(f => f.endsWith('.json')).sort();
for (const file of cases) {
  const t = JSON.parse(fs.readFileSync(path.join('negative', file), 'utf8'));
  const contract = JSON.parse(fs.readFileSync(contractPath, 'utf8'));
  const bundle = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));
  let at = t.side === 'contract' ? contract : bundle;
  for (const k of t.set.slice(0, -1)) at = at[k];
  at[t.set[t.set.length - 1]] = t.value;
  const c = path.join(dir, 'contract.json'), b = path.join(dir, 'bundle.json');
  fs.writeFileSync(c, JSON.stringify(contract));
  fs.writeFileSync(b, JSON.stringify(bundle));
  const r = spawnSync(process.execPath, ['bundle.mjs', c, b, gapsPath], { encoding: 'utf8' });
  const said = r.stdout + r.stderr;
  if (r.status === 0 || !said.includes(t.expect)) {
    fails++;
    console.log('FAIL', file, `exit ${r.status}`, `expected "${t.expect}"`);
  } else console.log('refused', file, ':', t.expect);
}
fs.rmSync(dir, { recursive: true, force: true });
console.log(`${cases.length} negative cases, ${fails} passed that should not`);
process.exit(fails ? 1 : 0);
