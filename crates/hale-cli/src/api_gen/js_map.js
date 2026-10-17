// A wire map (spec/api.md § Codecs) is a list of records here and an object on the wire, keyed by
// each record's key field: the key lifted out of each value on the way out, in the list's order,
// and put back on the way in. `Object.fromEntries` defines each key as its own property, so a key
// such as `__proto__` is an entry like any other.
/**
 * @template T
 * @param {globalThis.Array<T>} list
 * @param {string} key
 * @param {(e: T) => unknown} encode
 * @returns {unknown}
 */
function apiMapEncode(list, key, encode) {
  return globalThis.Object.fromEntries(
    list.map((e) => {
      const o = { .../** @type {globalThis.Record<string, unknown>} */ (encode(e)) };
      const k = String(o[key] ?? "");
      delete o[key];
      return [k, o];
    }),
  );
}

/**
 * @template T
 * @param {unknown} json
 * @param {string} key
 * @param {(json: unknown) => T} decode
 * @returns {globalThis.Array<T>}
 */
function apiMapDecode(json, key, decode) {
  if (json == null || typeof json !== "object" || globalThis.Array.isArray(json)) return [];
  return globalThis.Object.entries(/** @type {globalThis.Record<string, unknown>} */ (json)).map(([k, v]) =>
    decode({ .../** @type {globalThis.Record<string, unknown>} */ (v ?? {}), [key]: k }),
  );
}
