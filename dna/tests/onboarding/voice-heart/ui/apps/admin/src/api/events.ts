import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { Schemas } from './client';

type AdminEvent = Schemas['AdminEvent'];
const listeners = new Set<(event: AdminEvent) => void>();

/** Share the existing stream; screens do not open a second connection. */
export function subscribeAdminEvents(listener: (event: AdminEvent) => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

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
    const seen = new Set<string>();
    const receive = (message: MessageEvent) => {
      try {
        const event = JSON.parse(message.data) as AdminEvent;
        if (!event || typeof event.id !== 'string' || typeof event.at !== 'string' || !event.data || seen.has(event.id)) return;
        if (event.type === 'usage' && (typeof event.data.id !== 'string' || typeof event.data.project !== 'string' || typeof event.data.requested_model !== 'string' || !event.data.timing || typeof event.data.input_tokens !== 'number' || typeof event.data.output_tokens !== 'number')) return;
        seen.add(event.id);
        if (seen.size > 512) seen.delete(seen.values().next().value!);
        for (const listener of listeners) listener(event);
      } catch { /* A malformed event does not stop subsequent valid events. */ }
    };
    const on = (type: string) => (message: MessageEvent) => {
      receive(message);
      for (const key of AFFECTS[type] ?? []) void qc.invalidateQueries({ queryKey: [key] });
    };
    for (const type of Object.keys(AFFECTS)) es.addEventListener(type, on(type));
    es.addEventListener('reset', (message) => { seen.clear(); receive(message); void qc.invalidateQueries(); });
    return () => es.close();
  }, [qc]);
  return state;
}
