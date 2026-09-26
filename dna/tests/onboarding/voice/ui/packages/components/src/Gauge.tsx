/**
 * A window as an arc: how much is used, and when it resets. Color is
 * earned: slate until pressure, amber under pressure, coral when spent.
 */
export function Gauge(props: {
  label: string;
  /** 0..1 used. */
  used: number;
  detail?: string;
  size?: number;
}) {
  const size = props.size ?? 88;
  const r = size / 2 - 6;
  const c = 2 * Math.PI * r;
  const used = Math.min(1, Math.max(0, props.used));
  const tone = used >= 1 ? 'var(--color-coral)' : used >= 0.8 ? 'var(--color-amber)' : 'var(--color-slate)';
  const pct = Math.round(used * 100);
  return (
    <figure style={{ margin: 0, display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={`${props.label}: ${pct}% used`}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--color-rule-strong)" strokeWidth={4} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={tone}
          strokeWidth={4}
          strokeDasharray={`${c * used} ${c}`}
          strokeLinecap="butt"
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
        <text x="50%" y="50%" dominantBaseline="central" textAnchor="middle" fill="var(--color-text)" fontSize={size / 5} fontFamily="var(--font-mono)">
          {pct}%
        </text>
      </svg>
      <figcaption>
        <div>{props.label}</div>
        {props.detail && <div className="muted" style={{ fontSize: 'var(--size-sm)' }}>{props.detail}</div>}
      </figcaption>
    </figure>
  );
}
