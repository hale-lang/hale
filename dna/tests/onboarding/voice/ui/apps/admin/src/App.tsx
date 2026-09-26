import { useIsFetching, useQueryClient } from '@tanstack/react-query';
import { Shell, RailHeader, RailNav, RailPrincipal, Evidence, ago } from '@hale/components';
import { useRoute, href } from './route';
import { useGet } from './api/client';
import { Overview } from './screens/Overview';
import { Nodes } from './screens/Nodes';
import { Usage } from './screens/Usage';
import { Routes } from './screens/Routes';
import { Changes } from './screens/Changes';
import { Accounts } from './screens/Accounts';
import { Catalog } from './screens/Catalog';
import { Projects } from './screens/Projects';
import { useAdminEvents } from './api/events';
import { useEffect, useState, type ReactNode } from 'react';

const NAV = [
  { href: href(), label: 'Overview', key: '' },
  { href: href('usage'), label: 'Usage', key: 'usage' },
  { href: href('routes'), label: 'Route table', key: 'routes' },
  { href: href('nodes'), label: 'Nodes', key: 'nodes' },
  { href: href('accounts'), label: 'Accounts', key: 'accounts' },
  { href: href('catalog'), label: 'Catalog', key: 'catalog' },
  { href: href('projects'), label: 'Projects', key: 'projects' },
  { href: href('changes'), label: 'Changes', key: 'changes' },
];

export function App() {
  const route = useRoute();
  const [inspector, setInspector] = useState<ReactNode>(null);
  const section = route.parts[0] ?? '';
  const screen = { '': Overview, nodes: Nodes, usage: Usage, routes: Routes, changes: Changes, accounts: Accounts, catalog: Catalog, projects: Projects }[section];
  const Screen = screen ?? Overview;

  // The inspector belongs to the screen; a new screen starts it empty.
  useEffect(() => setInspector(null), [section]);

  return (
    <Shell
      rail={
        <>
          <RailHeader app="voice" sub="admin" />
          <RailNav items={NAV.map((n) => ({ ...n, active: n.key === section }))} />
          <RailPrincipal>no principal · MVP</RailPrincipal>
        </>
      }
      inspector={inspector}
      evidence={<EvidenceStrip />}
    >
      <Screen route={route} setInspector={setInspector} />
    </Shell>
  );
}

export type ScreenProps = { route: ReturnType<typeof useRoute>; setInspector: (n: ReactNode) => void };

/** Source, revision and freshness: what the canvas is showing, from where. */
function EvidenceStrip() {
  const health = useGet('/readyz');
  const stream = useAdminEvents();
  const fetching = useIsFetching();
  const qc = useQueryClient();
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const newest = Math.max(0, ...qc.getQueryCache().getAll().map((q) => q.state.dataUpdatedAt));
  const version = health.data?.version ?? '–';
  const state = health.isError ? 'unreachable' : health.data?.status ?? 'connecting';
  return (
    <Evidence
      items={[
        { label: 'source', value: 'voice api, same origin' },
        { label: 'api', value: <span className={health.isError ? 'c-fail' : undefined}>{state} {version}</span> },
        { label: 'refreshed', value: newest ? `${ago(new Date(newest).toISOString(), now)} ago` : '–' },
        { label: 'events', value: <span className={stream === 'open' ? undefined : stream === 'closed' ? 'c-wait' : 'faint'}>{stream}</span> },
        { label: '', value: fetching ? <span className="c-active pulse">fetching</span> : <span className="faint">idle</span> },
      ]}
    />
  );
}
