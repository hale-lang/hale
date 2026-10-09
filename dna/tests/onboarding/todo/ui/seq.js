// The one rule the page keeps about the stream's `seq`: the hub numbers the
// events it delivers one after another, and a subscriber that falls behind a
// full queue loses the oldest, so a number that is not the next one means
// events were shed. `last` is the `seq` of the event applied before (null
// before the first), `next` the one that just arrived.
export function needs_resync(last, next) {
  return last !== null && next !== last + 1;
}

// The page's bookkeeping across a load: `state` is `{ last, loading,
// gap_seen }`. An event that arrives while the list is on its way is only
// queued, but the list may have been read before the events that were shed,
// so a gap seen then is kept in `gap_seen` instead of being forgotten when
// the replay moves `last` on. Returns the state after the event `next` and
// `act`: "queue" (a load is under way), "relist" (a gap with no load under
// way: queue the event and load) or "apply".
export function on_event(state, next) {
  const gap = needs_resync(state.last, next);
  return {
    state: { last: next, loading: state.loading, gap_seen: state.gap_seen || (state.loading && gap) },
    act: state.loading ? "queue" : gap ? "relist" : "apply",
  };
}

// The load finished and its queue was replayed: `relist` says the page has
// to read the list once more (a gap was seen while it loaded), and `state`
// is the state with the load over and the flag cleared.
export function load_done(state) {
  return { relist: state.gap_seen, state: { ...state, loading: false, gap_seen: false } };
}
