import { test } from "node:test";
import assert from "node:assert/strict";
import { byDue, holds, label, QUADRANTS, type Want } from "./model.ts";

const mk = (title: string, pos: string, due_at: string | null, kind: Want["kind"] = "task"): Want => ({
  id: title,
  title,
  notes: "",
  kind,
  axis_hi: true,
  clau: true,
  pos,
  due_at,
  done_at: null,
  archived_at: null,
  deleted: false,
  rev: 0,
  created_at: "2026-11-16T00:00:00Z",
});

test("Must は日時の順で、日時なしは最後 (core の sort_by_due と同じ)", () => {
  const ws = [
    mk("none-a", "A", null),
    mk("late", "B", "2026-12-01"),
    mk("none-0", "0", null),
    mk("timed", "C", "2026-11-25T12:00:00+09:00"),
    mk("day", "D", "2026-11-20"),
  ];
  assert.deepEqual(
    ws.sort(byDue).map((w) => w.title),
    ["day", "timed", "late", "none-0", "none-a"],
  );
});

test("Must は clau度で分けず、学校 / その他 で表示する", () => {
  const w = mk("a", "V", null);
  assert.ok(holds(QUADRANTS[0], "task", w));
  assert.ok(!holds(QUADRANTS[2], "task", w));
  assert.ok(!holds(QUADRANTS[0], "want", w));
  assert.equal(label(QUADRANTS[0], "task"), "学校");
  assert.equal(label(QUADRANTS[3], "task"), "その他");
  assert.equal(label(QUADRANTS[1], "want"), "エネルギー高・clau高");
});
