/**
 * A series as a line. Volume is logarithmic by default, since counts span
 * orders of magnitude: 1, 100 and 10 000 are three different things.
 */
export function Sparkline(props: { values: number[]; width?: number; height?: number; log?: boolean; tone?: 'slate' | 'cyan' | 'amber' | 'coral' }) {
  const w = props.width ?? 120;
  const h = props.height ?? 24;
  const log = props.log ?? true;
  const f = (v: number) => (log ? Math.log10(1 + Math.max(0, v)) : v);
  const ys = props.values.map(f);
  const max = Math.max(1e-9, ...ys);
  const n = ys.length;
  const pts = ys.map((y, i) => `${n === 1 ? w / 2 : (i / (n - 1)) * (w - 2) + 1},${h - 1 - (y / max) * (h - 2)}`).join(' ');
  const stroke = `var(--color-${props.tone ?? 'slate'})`;
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} role="img" aria-label={`${n} values, max ${Math.max(...props.values)}`}>
      <polyline points={pts} fill="none" stroke={stroke} strokeWidth={1.25} />
    </svg>
  );
}
