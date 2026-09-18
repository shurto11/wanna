// 区分内の並び順キー (fractional index)。core/src/pos.rs と同じアルゴリズム。
// キーは base62 の文字列で、辞書順 (コードユニット順) がそのまま並び順になる。

const DIGITS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE = 62;

const val = (c: string): number => {
  const v = DIGITS.indexOf(c);
  if (v < 0) throw new Error(`pos キーに base62 以外の文字: ${c}`);
  return v;
};

/** a と b の間に入るキー。null はそれぞれ先頭 / 末尾 */
export function between(a: string | null, b: string | null): string {
  if (a !== null && b !== null && !(a < b)) throw new Error(`pos.between: ${a} < ${b} ではない`);
  if (a !== null && b === null) return after(a);
  return midpoint(a ?? "", b);
}

/** a より後ろのキー。最初の z でない桁を1つ増やし、全桁 z なら 1 を足す */
export function after(a: string): string {
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== "z") return a.slice(0, i) + DIGITS[val(a[i]) + 1];
  }
  return a + "1";
}

function midpoint(a: string, b: string | null): string {
  if (b !== null) {
    let n = 0;
    while (n < b.length && (a[n] ?? "0") === b[n]) n++;
    if (n > 0) return b.slice(0, n) + midpoint(a.slice(n), b.slice(n));
  }
  const da = a.length > 0 ? val(a[0]) : 0;
  const db = b !== null ? val(b[0]) : BASE;
  if (db - da > 1) return DIGITS[Math.ceil((da + db) / 2)];
  if (b !== null && b.length > 1) return b[0];
  return DIGITS[da] + midpoint(a.slice(1), null);
}
