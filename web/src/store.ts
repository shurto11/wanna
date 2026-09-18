// ローカルキャッシュ (IndexedDB) と outbox、サーバーとの同期。
// 流れは TUI と同じ: 編集はまずローカルに適用して outbox に積み、順に送ってから差分を取り直す。

import { openDB, type DBSchema, type IDBPDatabase } from "idb";
import { Api, Rejected } from "./api.ts";
import { opId, type Op, type Want } from "./model.ts";

interface Schema extends DBSchema {
  wants: { key: string; value: Want };
  outbox: { key: number; value: Op };
  meta: { key: string; value: number | string };
}

export type Conn = "local" | "connecting" | "online" | "offline" | "unauthorized";

export class Store {
  wants = new Map<string, Want>();
  conn: Conn = "local";
  outboxLen = 0;
  private api: Api | null = null;
  private es: EventSource | null = null;
  private rev = 0;
  private syncing = false;
  private dirty = false;
  private listeners = new Set<() => void>();

  private constructor(private db: IDBPDatabase<Schema>) {}

  static async open(): Promise<Store> {
    const db = await openDB<Schema>("wanna", 1, {
      upgrade(db) {
        db.createObjectStore("wants", { keyPath: "id" });
        db.createObjectStore("outbox", { autoIncrement: true });
        db.createObjectStore("meta");
      },
    });
    const s = new Store(db);
    for (const w of await db.getAll("wants")) s.wants.set(w.id, w);
    s.rev = Number((await db.get("meta", "rev")) ?? 0);
    s.outboxLen = await db.count("outbox");
    return s;
  }

  subscribe(fn: () => void) {
    this.listeners.add(fn);
  }

  private emit() {
    for (const fn of this.listeners) fn();
  }

  get token(): string | null {
    return localStorage.getItem("wanna.token");
  }

  /** token を設定して同期を始める */
  connect(token: string | null) {
    this.es?.close();
    this.es = null;
    if (token === null) {
      localStorage.removeItem("wanna.token");
      this.api = null;
      this.conn = "local";
      this.emit();
      return;
    }
    localStorage.setItem("wanna.token", token);
    this.api = new Api(token);
    this.conn = "connecting";
    this.es = this.api.events(
      (rev) => {
        if (rev > this.rev) this.sync();
      },
      (open) => {
        if (open) this.sync();
      },
    );
    this.emit();
    this.sync();
  }

  /** ローカルに適用して outbox に積み、同期を促す */
  async commit(op: Op) {
    const cur = this.wants.get(opId(op));
    let next: Want | undefined;
    if (op.op === "create") next = op.want;
    else if (cur && op.op === "patch") next = { ...cur, ...op.patch };
    else if (cur && op.op === "delete") next = { ...cur, deleted: true };
    if (next) this.wants.set(next.id, next);
    this.emit();

    const tx = this.db.transaction(["wants", "outbox"], "readwrite");
    if (next) await tx.objectStore("wants").put(next);
    // サーバー未設定でも積んでおき、token を設定したときにまとめて送る
    await tx.objectStore("outbox").add(op);
    await tx.done;
    this.outboxLen = await this.db.count("outbox");
    this.sync();
  }

  async sync(): Promise<void> {
    const api = this.api;
    if (!api) return;
    if (this.syncing) {
      this.dirty = true;
      return;
    }
    this.syncing = true;
    this.dirty = false;
    let rejected = false;
    try {
      // outbox を順に送る。到達不能なら例外で抜け、順序を守ったまま次回に再送する
      let cursor = await this.db.transaction("outbox").store.openCursor();
      const ops: [number, Op][] = [];
      while (cursor) {
        ops.push([cursor.key, cursor.value]);
        cursor = await cursor.continue();
      }
      for (const [key, op] of ops) {
        try {
          if (op.op === "create") await api.create(op.want);
          else if (op.op === "patch") await api.patch(op.id, op.patch);
          else await api.delete(op.id);
        } catch (e) {
          if (!(e instanceof Rejected)) throw e;
          if (e.status === 401) throw e;
          rejected = true;
        }
        await this.db.delete("outbox", key);
      }
      this.outboxLen = await this.db.count("outbox");

      const res = await api.sync(this.rev > 0 ? this.rev : null);
      // まだ送っていない変更があるものは、ローカルの状態を優先する（送った後の同期で揃う）
      const pending = new Set((await this.db.getAll("outbox")).map(opId));
      const tx = this.db.transaction(["wants", "meta"], "readwrite");
      for (const w of res.wants) {
        if (pending.has(w.id)) continue;
        this.wants.set(w.id, w);
        tx.objectStore("wants").put(w);
      }
      // 拒否された変更があったら、ずれを直すため次回は全件取り直す
      this.rev = rejected ? 0 : res.rev;
      tx.objectStore("meta").put(this.rev, "rev");
      await tx.done;
      this.conn = "online";
      if (rejected) this.dirty = true;
    } catch (e) {
      this.conn = e instanceof Rejected && e.status === 401 ? "unauthorized" : "offline";
    } finally {
      this.syncing = false;
      this.emit();
    }
    if (this.dirty) return this.sync();
  }
}
