import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';

/**
 * The admin event stream: each event names a resource that changed, and
 * the queries that show it are refetched. A `reset` refetches everything.
 * The stream resumes by Last-Event-ID on its own (EventSource does that).
 */
const AFFECTS: Record<string, string[]> = {
  node: ['/admin/v1/nodes', '/admin/v1/routes'],
  seat: ['/admin/v1/nodes', '/admin/v1/routes', '/admin/v1/models'],
  account: ['/admin/v1/accounts', '/admin/v1/routes'],
  usage: ['/admin/v1/usage', '/admin/v1/usage/summary', '/admin/v1/projects'],
  change: ['/admin/v1/changes'],
};

export function useAdminEvents(): 'connecting' | 'open' | 'closed' {
  const qc = useQueryClient();
  const [state, setState] = useState<'connecting' | 'open' | 'closed'>('connecting');
  useEffect(() => {
    const es = new EventSource('/admin/v1/events');
    es.onopen = () => setState('open');
    es.onerror = () => setState(es.readyState === EventSource.CLOSED ? 'closed' : 'connecting');
    const on = (type: string) => () => {
      for (const key of AFFECTS[type] ?? []) void qc.invalidateQueries({ queryKey: [key] });
    };
    for (const type of Object.keys(AFFECTS)) es.addEventListener(type, on(type));
    es.addEventListener('reset', () => void qc.invalidateQueries());
    return () => es.close();
  }, [qc]);
  return state;
}
