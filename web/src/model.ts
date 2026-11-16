import { at } from "./due.ts";

/** どのリストのものか。memo は区分・日時を持たず、名前と本文だけ使う */
export type Kind = "want" | "task" | "memo";

/** 区分で並べるリスト (画面上の並び)。メモは含まない */
export const KINDS: Kind[] = ["want", "task"];

/** リストの名前 */
export const kindLabel = (k: Kind) => (k === "task" ? "Must" : k === "memo" ? MEMO_LABEL : "Want");

/** メモの名前 (5つめ) */
export const MEMO_LABEL = "Memo";

/** やったことリストの名前 (kindLabel と並ぶ3つめ) */
export const DONE_LABEL = "Done";

/** 保管庫の名前 (4つめ) */
export const ARCHIVE_LABEL = "Archive";

/** 縦軸の名前。Must は高低ではなく「学校 / その他」の種類で分ける */
export const axisLabel = (k: Kind) => (k === "task" ? "種類" : "エネルギー");

/** 縦軸の 高 / 低 の呼び名 */
export const axisValues = (k: Kind): [string, string] => (k === "task" ? ["学校", "その他"] : ["高", "低"]);

/** clau度で区分を分けるか。Must は「学校 / その他」の2段だけで、clau度では分けない */
export const splitsClau = (k: Kind) => k === "want";

/** 日時を持てるか */
export const hasDue = (k: Kind) => k === "task";

export const otherKind = (k: Kind): Kind => (k === "task" ? "want" : "task");

/** 1件。やりたいこと / 次にやること のどちらかで、kind が分ける */
export interface Want {
  id: string;
  title: string;
  notes: string;
  kind: Kind;
  /** true = 縦軸が高い (やりたいこと = エネルギー高 / 次にやること = 学校) */
  axis_hi: boolean;
  /** true = clau度高 */
  clau: boolean;
  pos: string;
  /** 日時 (due.ts の保存形式)。次にやることだけが持つ */
  due_at: string | null;
  done_at: string | null;
  /** しまった日時。やってはいないがリストに置くほどでもなくなったもの (古いキャッシュには無い) */
  archived_at?: string | null;
  deleted: boolean;
  rev: number;
  created_at: string;
}

export type WantPatch = Partial<
  Pick<Want, "title" | "notes" | "kind" | "axis_hi" | "clau" | "pos" | "due_at" | "done_at" | "archived_at">
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

/** リスト kind で画面に出す区分 (QUADRANTS の添字)。Must は clau度で分けないので左の列の2つ */
export const slotsOf = (k: Kind) => (splitsClau(k) ? [0, 1, 2, 3] : [0, 2]);

/** 軸の値をそのまま書いた表示 (例: "エネルギー高・clau低")。Must は "学校" / "その他" だけ */
export function label(q: Quadrant, kind: Kind): string {
  const [hi, lo] = axisValues(kind);
  const axis = q.axis_hi ? hi : lo;
  if (!splitsClau(kind)) return axis;
  return `${axisLabel(kind)}${axis}・clau${q.clau ? "高" : "低"}`;
}

export const sameQuadrant = (w: Quadrant, q: Quadrant) => w.axis_hi === q.axis_hi && w.clau === q.clau;

/** w がリスト kind の区分 q に入るか。clau度で分けないリスト (Must) では縦軸だけ見る */
export const holds = (q: Quadrant, kind: Kind, w: Want) =>
  w.axis_hi === q.axis_hi && (!splitsClau(kind) || w.clau === q.clau);

export const isActive = (w: Want) => !w.deleted && w.done_at === null && w.archived_at == null;

/** 保管庫に出るもの。やったことになっていれば、やったこと側に出す */
export const isArchived = (w: Want) => !w.deleted && w.done_at === null && w.archived_at != null;

/** 日時。やりたいことは持たないので常に null */
export const dueOf = (w: Want) => (hasDue(w.kind) ? w.due_at : null);

/** 区分内の表示順 (pos, id) */
export const byPos = (a: Want, b: Want) =>
  a.pos < b.pos ? -1 : a.pos > b.pos ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0;

/** 表示順の list (入れるもの自身は除く) のどこに日時 d のものを入れるか。
 *  自分より後の日時のものの直前、無ければ日時を持つものの最後の後ろ (core の due_index と同じ)。
 *  並びは手動なので、日時を入れたときと段を移したときにだけ使う */
export function dueIndex(list: Want[], d: string): number {
  const t = at(d);
  const dated = (w: Want) => dueOf(w) != null;
  const later = list.findIndex((w) => dated(w) && at(dueOf(w)!) > t);
  if (later >= 0) return later;
  let last = -1;
  list.forEach((w, i) => {
    if (dated(w)) last = i;
  });
  return last + 1;
}

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
