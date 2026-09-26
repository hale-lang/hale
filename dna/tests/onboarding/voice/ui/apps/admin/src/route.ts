import { useEffect, useState } from 'react';

/**
 * Hash routes, so deep links survive any host: `#/nodes/laptop`. The
 * parts are the route; the query, if any, is opaque state.
 */
export type Route = { parts: string[]; query: URLSearchParams };

function parse(): Route {
  const raw = location.hash.replace(/^#\/?/, '');
  const [path = '', q = ''] = raw.split('?');
  return { parts: path.split('/').filter(Boolean), query: new URLSearchParams(q) };
}

export function useRoute(): Route {
  const [route, setRoute] = useState(parse);
  useEffect(() => {
    const on = () => setRoute(parse());
    addEventListener('hashchange', on);
    return () => removeEventListener('hashchange', on);
  }, []);
  return route;
}

export function href(...parts: string[]): string {
  return '#/' + parts.map(encodeURIComponent).join('/');
}

export function go(...parts: string[]) {
  location.hash = href(...parts);
}
