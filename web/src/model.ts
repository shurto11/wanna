export interface Want {
  id: string;
  title: string;
  notes: string;
  /** true = エネルギー高 */
  energy: boolean;
  /** true = clau度高 */
  clau: boolean;
  pos: string;
  done_at: string | null;
  deleted: boolean;
  rev: number;
  created_at: string;
}

export type WantPatch = Partial<Pick<Want, "title" | "notes" | "energy" | "clau" | "pos" | "done_at">>;

export interface Quadrant {
  energy: boolean;
  clau: boolean;
}

/** 画面上の並び (左上, 右上, 左下, 右下) */
export const QUADRANTS: Quadrant[] = [
  { energy: true, clau: false },
  { energy: true, clau: true },
  { energy: false, clau: false },
  { energy: false, clau: true },
];

/** 軸の値をそのまま書いた表示 (例: "エネルギー高・clau低") */
export function label(q: Quadrant): string {
  return `エネルギー${q.energy ? "高" : "低"}・clau${q.clau ? "高" : "低"}`;
}

export const sameQuadrant = (w: Quadrant, q: Quadrant) => w.energy === q.energy && w.clau === q.clau;

export const isActive = (w: Want) => !w.deleted && w.done_at === null;

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
