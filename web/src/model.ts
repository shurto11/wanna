/** どちらのリストのものか */
export type Kind = "want" | "task";

/** 画面上の並び */
export const KINDS: Kind[] = ["want", "task"];

/** リストの名前 */
export const kindLabel = (k: Kind) => (k === "task" ? "Must" : "Want");

/** やったことリストの名前 (kindLabel と並ぶ3つめ) */
export const DONE_LABEL = "Done";

/** 縦軸の名前 */
export const axisLabel = (k: Kind) => (k === "task" ? "重要度" : "エネルギー");

/** 日時を持てるか */
export const hasDue = (k: Kind) => k === "task";

export const otherKind = (k: Kind): Kind => (k === "task" ? "want" : "task");

/** 1件。やりたいこと / 次にやること のどちらかで、kind が分ける */
export interface Want {
  id: string;
  title: string;
  notes: string;
  kind: Kind;
  /** true = 縦軸が高い (やりたいこと = エネルギー高 / 次にやること = 重要度高) */
  axis_hi: boolean;
  /** true = clau度高 */
  clau: boolean;
  pos: string;
  /** 日時 (due.ts の保存形式)。次にやることだけが持つ */
  due_at: string | null;
  done_at: string | null;
  deleted: boolean;
  rev: number;
  created_at: string;
}

export type WantPatch = Partial<
  Pick<Want, "title" | "notes" | "kind" | "axis_hi" | "clau" | "pos" | "due_at" | "done_at">
>;

export interface Quadrant {
  axis_hi: boolean;
  clau: boolean;
}

/** 画面上の並び (左上, 右上, 左下, 右下) */
export const QUADRANTS: Quadrant[] = [
  { axis_hi: true, clau: false },
  { axis_hi: true, clau: true },
  { axis_hi: false, clau: false },
  { axis_hi: false, clau: true },
];

/** 軸の値をそのまま書いた表示 (例: "重要度高・clau低") */
export function label(q: Quadrant, kind: Kind): string {
  const hi = (b: boolean) => (b ? "高" : "低");
  return `${axisLabel(kind)}${hi(q.axis_hi)}・clau${hi(q.clau)}`;
}

export const sameQuadrant = (w: Quadrant, q: Quadrant) => w.axis_hi === q.axis_hi && w.clau === q.clau;

export const isActive = (w: Want) => !w.deleted && w.done_at === null;

/** 日時。やりたいことは持たないので常に null */
export const dueOf = (w: Want) => (hasDue(w.kind) ? w.due_at : null);

/** 区分内の表示順 (pos, id) */
export const byPos = (a: Want, b: Want) =>
  a.pos < b.pos ? -1 : a.pos > b.pos ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0;

export const nowRfc3339 = () => new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

/** UUIDv7 (先頭 48bit がミリ秒タイムスタンプ) */
export function uuidv7(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  let ts = Date.now();
  for (let i = 5; i >= 0; i--) {
    b[i] = ts & 0xff;
    ts = Math.floor(ts / 256);
  }
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

export type Op =
  | { op: "create"; want: Want }
  | { op: "patch"; id: string; patch: WantPatch }
  | { op: "delete"; id: string };

export const opId = (o: Op) => (o.op === "create" ? o.want.id : o.id);
