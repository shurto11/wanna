// 「次にやること」の日時。core/src/due.rs と同じ形式・同じ入力を受ける。
// 保存形式は2つ: 時刻を持たないものは YYYY-MM-DD、持つものは RFC3339。

/** 入力欄に添えるヒント */
export const HINT = "例: 2026-09-25 / 09-25 14:00 / 明日 18:00 / 金 / 空で消す";

export type State = "over" | "today" | "later";

export interface Due {
  /** ローカルのその日の 0:00 */
  date: Date;
  /** [時, 分]。時刻なしは null */
  time: [number, number] | null;
}

const DATE_ONLY = /^(\d{4})-(\d{2})-(\d{2})$/;

const p2 = (n: number) => String(n).padStart(2, "0");
const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const dayDiff = (a: Date, b: Date) => Math.round((midnight(a).getTime() - midnight(b).getTime()) / 86400000);

/** 保存形式を開く。読めなければ null */
export function parse(s: string): Due | null {
  const m = DATE_ONLY.exec(s);
  if (m) return { date: new Date(+m[1], +m[2] - 1, +m[3]), time: null };
  if (!s.includes("T")) return null;
  const t = new Date(s);
  if (Number.isNaN(t.getTime())) return null;
  return { date: midnight(t), time: [t.getHours(), t.getMinutes()] };
}

/** 保存形式として正しいか */
export const valid = (s: string) => parse(s) !== null;

/** 並べ替え用。時刻なしはその日の 0:00 */
export function at(s: string): number {
  const d = parse(s);
  if (!d) return 0;
  const [h, min] = d.time ?? [0, 0];
  return new Date(d.date.getFullYear(), d.date.getMonth(), d.date.getDate(), h, min).getTime();
}

/** now から見た状態。時刻なしはその日のうちなら "today" */
export function state(s: string, now = new Date()): State {
  const d = parse(s);
  if (!d) return "later";
  const diff = dayDiff(d.date, now);
  if (diff < 0) return "over";
  if (diff > 0) return "later";
  if (d.time === null) return "today";
  const [h, min] = d.time;
  return h * 60 + min < now.getHours() * 60 + now.getMinutes() ? "over" : "today";
}

/** 一覧に出す短い表示 ("今日 14:00" / "09-25" / "2027-01-05") */
export function format(s: string, now = new Date()): string {
  const d = parse(s);
  if (!d) return s;
  const diff = dayDiff(d.date, now);
  const day =
    diff === 0
      ? "今日"
      : diff === 1
        ? "明日"
        : diff === -1
          ? "昨日"
          : d.date.getFullYear() === now.getFullYear()
            ? `${p2(d.date.getMonth() + 1)}-${p2(d.date.getDate())}`
            : `${d.date.getFullYear()}-${p2(d.date.getMonth() + 1)}-${p2(d.date.getDate())}`;
  return d.time === null ? day : `${day} ${p2(d.time[0])}:${p2(d.time[1])}`;
}

/** 編集欄に出す形。そのまま編集して parseInput に戻せる */
export function toInput(s: string): string {
  const d = parse(s);
  if (!d) return s;
  const ymd = `${d.date.getFullYear()}-${p2(d.date.getMonth() + 1)}-${p2(d.date.getDate())}`;
  return d.time === null ? ymd : `${ymd} ${p2(d.time[0])}:${p2(d.time[1])}`;
}

/** 読めなかった入力。理由を持たせて投げる */
export class Unreadable extends Error {
  constructor(input: string) {
    super(`日時として読めません: ${input}  (${HINT})`);
  }
}

/** 入力を保存形式にする。空なら null */
export function parseInput(input: string, now = new Date()): string | null {
  const s = input.trim();
  if (s === "") return null;
  const parts = s.split(/\s+/);
  if (parts.length === 1) {
    // 1語なら日付として読み、読めなければ時刻とみなして今日に付ける
    const date = parseDate(parts[0], now);
    if (date !== null) return store(date, null);
    const time = parseTime(parts[0]);
    if (time !== null) return store(midnight(now), time);
    throw new Unreadable(s);
  }
  if (parts.length === 2) {
    const date = parseDate(parts[0], now);
    const time = parseTime(parts[1]);
    if (date === null || time === null) throw new Unreadable(s);
    return store(date, time);
  }
  throw new Unreadable(s);
}

function store(d: Date, time: [number, number] | null): string {
  const ymd = `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}`;
  if (time === null) return ymd;
  const local = new Date(d.getFullYear(), d.getMonth(), d.getDate(), time[0], time[1]);
  const off = -local.getTimezoneOffset();
  const sign = off < 0 ? "-" : "+";
  const abs = Math.abs(off);
  return `${ymd}T${p2(time[0])}:${p2(time[1])}:00${sign}${p2(Math.floor(abs / 60))}:${p2(abs % 60)}`;
}

const WEEKDAYS: Record<string, number> = {
  日: 0, sun: 0, sunday: 0,
  月: 1, mon: 1, monday: 1,
  火: 2, tue: 2, tuesday: 2,
  水: 3, wed: 3, wednesday: 3,
  木: 4, thu: 4, thursday: 4,
  金: 5, fri: 5, friday: 5,
  土: 6, sat: 6, saturday: 6,
};

/** 2026-09-25 / 09-25 (今年) / 9月25日 / 明日 / 金 など */
function parseDate(s: string, now: Date): Date | null {
  const today = midnight(now);
  const named: Record<string, number> = {
    今日: 0, きょう: 0, 本日: 0, today: 0,
    明日: 1, あした: 1, あす: 1, tomorrow: 1,
    明後日: 2, あさって: 2,
    昨日: -1, きのう: -1, yesterday: -1,
  };
  if (s in named) return addDays(today, named[s]);

  const w = WEEKDAYS[s.replace(/曜日?$/, "").toLowerCase()];
  if (w !== undefined) {
    // 今日は含めず、次のその曜日
    let d = addDays(today, 1);
    while (d.getDay() !== w) d = addDays(d, 1);
    return d;
  }

  const nums = s
    .replace(/[\/.年月]/g, "-")
    .replace(/日$/, "")
    .replace(/-$/, "")
    .split("-")
    .filter((x) => x !== "");
  if (!nums.every((x) => /^\d+$/.test(x))) return null;
  let y: number, m: number, d: number;
  if (nums.length === 3) [y, m, d] = nums.map(Number);
  // 月日だけなら今年
  else if (nums.length === 2) [y, [m, d]] = [today.getFullYear(), nums.map(Number) as [number, number]];
  else return null;
  const date = new Date(y, m - 1, d);
  // 2026-13-40 のような日付をカレンダー側で繰り上げさせない
  if (date.getFullYear() !== y || date.getMonth() !== m - 1 || date.getDate() !== d) return null;
  return date;
}

/** 14:00 / 14時30分 / 14 など */
function parseTime(s: string): [number, number] | null {
  const norm = s.replace(/分$/, "").replace(/[時：]/g, ":").replace(/:$/, "");
  const parts = norm.split(":");
  if (parts.length > 2 || !parts.every((x) => /^\d+$/.test(x))) return null;
  const h = Number(parts[0]);
  const m = parts.length === 2 ? Number(parts[1]) : 0;
  if (h > 23 || m > 59) return null;
  return [h, m];
}
