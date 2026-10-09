// The one rule the page keeps about the stream's `seq`: the hub numbers the
// events it delivers one after another, and a subscriber that falls behind a
// full queue loses the oldest, so a number that is not the next one means
// events were shed. `last` is the `seq` of the event applied before (null
// before the first), `next` the one that just arrived.
export function needs_resync(last, next) {
  return last !== null && next !== last + 1;
}
