import { test } from "node:test";
import assert from "node:assert/strict";
import { dueIndex, holds, label, QUADRANTS, type Want } from "./model.ts";

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

test("日時を入れる位置 (core の due_index と同じ)", () => {
  // 手で並べた結果、日時の順になっていないところもある
  const ws = [
    mk("a", "A", "2026-11-20"),
    mk("b", "B", null),
    mk("c", "C", "2026-11-25T12:00:00+09:00"),
    mk("d", "D", "2026-12-01"),
    mk("e", "E", null),
  ];
  assert.equal(dueIndex(ws, "2026-11-01"), 0);
  assert.equal(dueIndex(ws, "2026-11-21"), 2);
  assert.equal(dueIndex(ws, "2026-11-26"), 3);
  assert.equal(dueIndex(ws, "2027-01-01"), 4);
  assert.equal(dueIndex([mk("x", "A", null), mk("y", "B", null)], "2026-11-20"), 0);
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
