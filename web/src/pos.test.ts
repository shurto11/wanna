import { test } from "node:test";
import assert from "node:assert/strict";
import { between } from "./pos.ts";

const valid = (k: string) => k.length > 0 && !k.endsWith("0") && /^[0-9A-Za-z]+$/.test(k);

// Rust 側 (core/src/pos.rs) と同じ結果になること
test("Rust 実装と一致", () => {
  assert.equal(between(null, null), "V");
  assert.equal(between("V", null), "W");
  assert.equal(between("z", null), "z1");
  assert.equal(between("1", "2"), "1V");
  assert.equal(between(null, "01"), "00V");
});

test("繰り返し挿入しても順序と形が保たれる", () => {
  let k = between(null, null);
  for (let i = 0; i < 1000; i++) {
    const n = between(k, null);
    assert.ok(n > k && valid(n));
    k = n;
  }
  let a = "V";
  const b = "W";
  for (let i = 0; i < 300; i++) {
    const m = between(a, b);
    assert.ok(a < m && m < b && valid(m), `${a} ${m} ${b}`);
    a = m;
  }
  let hi = "V";
  for (let i = 0; i < 300; i++) {
    const m = between(null, hi);
    assert.ok(m < hi && valid(m), `${m} ${hi}`);
    hi = m;
  }
});
