// Compiles tokens.json to tokens.css: every leaf becomes a custom property
// on :root (the first theme) and, for each further theme, on
// [data-theme="<name>"]. Framework-free by construction.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const tokens = JSON.parse(readFileSync(join(here, 'tokens.json'), 'utf8'));

const lines = ['/* generated from tokens.json; do not edit */'];
const themes = Object.entries(tokens.themes);
themes.forEach(([name, colors], i) => {
  lines.push(i === 0 ? ':root {' : `:root[data-theme="${name}"] {`);
  for (const [k, v] of Object.entries(colors)) lines.push(`  --color-${k}: ${v};`);
  lines.push('}');
});
lines.push(':root {');
for (const group of ['font', 'size', 'space', 'radius', 'cut', 'line', 'motion', 'layer']) {
  for (const [k, v] of Object.entries(tokens[group])) lines.push(`  --${group}-${k}: ${v};`);
}
lines.push('}');
lines.push('@media (prefers-reduced-motion: reduce) { :root { --motion-fast: 0ms; --motion-normal: 0ms; --motion-afterglow: 0ms; } }');
writeFileSync(join(here, 'tokens.css'), lines.join('\n') + '\n');
console.log(`tokens.css: ${themes.length} theme(s)`);
