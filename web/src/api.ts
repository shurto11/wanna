import type { Want, WantPatch } from "./model.ts";

/** 4xx。再送しても無駄なので outbox から捨てる */
export class Rejected extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

export class Api {
  constructor(private token: string) {}

  private async req<T>(method: string, path: string, body?: unknown): Promise<T> {
    const res = await fetch(path, {
      method,
      headers: {
        Authorization: `Bearer ${this.token}`,
        ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
      },
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (res.status >= 400 && res.status < 500) throw new Rejected(res.status, await res.text());
    if (!res.ok) throw new Error(`サーバーエラー ${res.status}`);
    return res.status === 204 ? (undefined as T) : res.json();
  }

  sync(since: number | null) {
    const q = since === null ? "" : `?since=${since}`;
    return this.req<{ rev: number; wants: Want[] }>("GET", `/api/sync${q}`);
  }
  create(w: Want) {
    return this.req<Want>("POST", "/api/wants", w);
  }
  patch(id: string, p: WantPatch) {
    return this.req<Want>("PATCH", `/api/wants/${encodeURIComponent(id)}`, p);
  }
  delete(id: string) {
    return this.req<void>("DELETE", `/api/wants/${encodeURIComponent(id)}`);
  }

  /** SSE。rev が進むたびに onRev。EventSource はヘッダを付けられないので token はクエリで渡す */
  events(onRev: (rev: number) => void, onState: (open: boolean) => void): EventSource {
    const es = new EventSource(`/api/events?token=${encodeURIComponent(this.token)}`);
    es.addEventListener("rev", (e) => onRev(Number((e as MessageEvent).data)));
    es.onopen = () => onState(true);
    es.onerror = () => onState(false);
    return es;
  }
}
