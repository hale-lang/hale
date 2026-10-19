// A wire map (spec/api.md § Codecs) is a list of records here and an object on the wire, keyed by
// each record's key field: the key lifted out of each value on the way out, in the list's order,
// and put back on the way in. `Object.fromEntries` defines each key as its own property, so a key
// such as `__proto__` is an entry like any other. A map of values (`value` given) carries each
// record's `value` field alone.
function apiMapEncode<T>(list: __Array<T>, key: string, encode: (e: T) => unknown, value?: string): unknown {
  return globalThis.Object.fromEntries(
    list.map((e) => {
      const o = { ...(encode(e) as __Record<string, unknown>) };
      const k = String(o[key] ?? "");
      delete o[key];
      return [k, value === undefined ? o : o[value]];
    }),
  );
}

function apiMapDecode<T>(json: unknown, key: string, decode: (json: unknown) => T, value?: string): __Array<T> {
  if (json == null || typeof json !== "object" || globalThis.Array.isArray(json)) return [];
  return globalThis.Object.entries(json as __Record<string, unknown>).map(([k, v]) =>
    decode(value === undefined ? { ...((v ?? {}) as __Record<string, unknown>), [key]: k } : { [value]: v, [key]: k }),
  );
}
