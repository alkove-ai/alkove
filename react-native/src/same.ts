const plain = (v: object) => Array.isArray(v) || Object.getPrototypeOf(v) === Object.prototype;

// by content, toJSON() included; other objects by identity
export function same(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== 'object' || typeof b !== 'object' || !a || !b) return false;
  const ja = (a as { toJSON?: () => unknown }).toJSON;
  const jb = (b as { toJSON?: () => unknown }).toJSON;
  if (typeof ja === 'function' || typeof jb === 'function') return same(typeof ja === 'function' ? ja.call(a) : a, typeof jb === 'function' ? jb.call(b) : b);
  if (!plain(a) || !plain(b) || Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a).filter((k) => (a as Record<string, unknown>)[k] !== undefined);
  const kb = Object.keys(b).filter((k) => (b as Record<string, unknown>)[k] !== undefined);
  return ka.length === kb.length && ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}
