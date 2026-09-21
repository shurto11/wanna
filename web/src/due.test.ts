import { test } from "node:test";
import assert from "node:assert/strict";
import { at, format, parseInput, state, toInput, valid } from "./due.ts";

// 2026-09-21 は月曜
const now = new Date(2026, 8, 21, 10, 0, 0);
const input = (s: string) => parseInput(s, now)!;

// Rust 側 (core/src/due.rs) と同じ結果になること
test("時刻のない日付は日付のまま持つ", () => {
  assert.equal(input("2026-09-25"), "2026-09-25");
  assert.equal(input("2026/9/25"), "2026-09-25");
  assert.equal(input("9月25日"), "2026-09-25");
  assert.equal(input("09-25"), "2026-09-25");
  assert.equal(input("今日"), "2026-09-21");
  assert.equal(input("明日"), "2026-09-22");
  assert.equal(input("明後日"), "2026-09-23");
  // 今日と同じ曜日は来週
  assert.equal(input("月"), "2026-09-28");
  assert.equal(input("金曜"), "2026-09-25");
  assert.equal(parseInput("  ", now), null);
});

test("時刻はローカルのまま往復する", () => {
  for (const [given, shown] of [
    ["2026-09-25 14:00", "09-25 14:00"],
    ["明日 18時", "明日 18:00"],
    ["9:30", "今日 09:30"],
  ]) {
    const stored = input(given);
    assert.ok(valid(stored), stored);
    assert.equal(format(stored, now), shown);
  }
  assert.equal(toInput(input("明日 18時")), "2026-09-22 18:00");
  assert.equal(toInput(input("明日")), "2026-09-22");
});

test("読めない入力はエラー", () => {
  for (const s of ["あとで", "2026-13-40", "25:00", "明日 18:00 まで"]) {
    assert.throws(() => parseInput(s, now), undefined, s);
  }
});

test("状態と並び順", () => {
  assert.equal(state(input("昨日"), now), "over");
  assert.equal(state(input("今日"), now), "today");
  assert.equal(state(input("9:30"), now), "over");
  assert.equal(state(input("18:00"), now), "today");
  assert.equal(state(input("明日"), now), "later");
  // 同じ日なら時刻なしが先
  assert.ok(at(input("今日")) < at(input("9:30")));
});
