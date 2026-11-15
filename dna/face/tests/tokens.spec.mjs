import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

// The face is on the hale UI kit: every colour is one of the kit's tokens
// (voice ui/packages/tokens, copied into styles.css), so a colour literal
// outside the tokens' own blocks is a drift from the kit. The same guard
// voice keeps over its UI (tokens-used.test.mjs).
test('every colour in the face is a token of the kit', async () => {
  const css = await readFile(fileURLToPath(new URL('../web/styles.css', import.meta.url)), 'utf8');
  const after = css.slice(css.indexOf("/* The face's older names"));
  const literals = [...after.matchAll(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\boklch\(/g)].map(m => m[0]);
  expect(literals, 'colour literals outside the tokens').toEqual([]);
  // the kit's seven themes are all present, each defining the ground
  for (const theme of ['carbon', 'paper', 'contrast', 'contrast-light', 'blueprint', 'vfd', 'ledger']) {
    expect(css).toContain(`:root[data-theme="${theme}"]`);
  }
  expect((css.match(/--color-bg-0:/g) || []).length).toBeGreaterThanOrEqual(7);
});
