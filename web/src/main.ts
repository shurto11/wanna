import Sortable from "sortablejs";
import { after, between } from "./pos.ts";
import {
  QUADRANTS,
  byPos,
  isActive,
  label,
  nowRfc3339,
  sameQuadrant,
  uuidv7,
  type Quadrant,
  type Want,
  type WantPatch,
} from "./model.ts";
import { Store, type Conn } from "./store.ts";
import "./style.css";

const $ = <T extends HTMLElement>(sel: string, root: ParentNode = document) => root.querySelector(sel) as T;

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.append(...children);
  return el;
}

const CONN_LABEL: Record<Conn, string> = {
  local: "ローカルのみ",
  connecting: "接続中",
  online: "同期",
  offline: "オフライン",
  unauthorized: "token が違います",
};

document.querySelector("#app")!.innerHTML = `
  <header class="bar">
    <h1 class="brand">wanna</h1>
    <nav class="tabs" role="tablist">
      <button role="tab" data-view="wants" aria-selected="true">やりたいこと</button>
      <button role="tab" data-view="done" aria-selected="false">やったこと</button>
    </nav>
    <button class="conn" id="conn" title="同期の設定"></button>
  </header>

  <section id="view-wants" class="view">
    <div class="axis axis-top"><span>エネルギー高</span></div>
    <div class="grid" id="grid"></div>
    <div class="axis axis-bottom"><span>エネルギー低</span><span>clau低 → clau高</span></div>
  </section>

  <section id="view-done" class="view" hidden>
    <ol class="done-list" id="done-list"></ol>
  </section>

  <dialog id="editor">
    <form method="dialog" class="sheet">
      <label class="field"><span>名前</span><input name="title" required autocomplete="off" /></label>
      <label class="field"><span>メモ</span><textarea name="notes" rows="5"></textarea></label>
      <div class="toggles">
        <fieldset class="seg"><legend>エネルギー</legend>
          <label><input type="radio" name="energy" value="1" />高</label>
          <label><input type="radio" name="energy" value="0" />低</label>
        </fieldset>
        <fieldset class="seg"><legend>clau度</legend>
          <label><input type="radio" name="clau" value="1" />高</label>
          <label><input type="radio" name="clau" value="0" />低</label>
        </fieldset>
      </div>
      <div class="actions">
        <button type="button" class="danger" data-act="delete">削除</button>
        <span class="spacer"></span>
        <button type="button" data-act="done">やった</button>
        <button value="cancel" formnovalidate>閉じる</button>
        <button value="save" class="primary">保存</button>
      </div>
    </form>
  </dialog>

  <dialog id="settings">
    <form method="dialog" class="sheet">
      <p class="hint">wannad の設定ファイルに書いた token を入れてください。空にするとローカルのみで使います。</p>
      <label class="field"><span>token</span><input name="token" type="password" autocomplete="off" /></label>
      <div class="actions">
        <span class="spacer"></span>
        <button value="cancel" formnovalidate>閉じる</button>
        <button value="save" class="primary">保存</button>
      </div>
    </form>
  </dialog>
`;

const store = await Store.open();
let dragging = false;

// ───────── やりたいこと (2×2) ─────────

const grid = $<HTMLDivElement>("#grid");
const lists: HTMLOListElement[] = QUADRANTS.map((q, qi) => {
  const list = h("ol", { class: "list", "data-q": String(qi) });
  const input = h("input", { placeholder: "＋ 追加", "aria-label": `${label(q)} に追加`, enterkeyhint: "done" });
  const form = h("form", { class: "add" }, input);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const title = input.value.trim();
    if (!title) return;
    input.value = "";
    add(title, q);
  });
  grid.append(
    h(
      "section",
      { class: "quad", "data-q": String(qi) },
      h("h2", {}, label(q), h("span", { class: "count" })),
      list,
      form,
    ),
  );
  Sortable.create(list, {
    group: "wants",
    animation: 150,
    delay: 180,
    delayOnTouchOnly: true,
    ghostClass: "ghost",
    filter: ".check",
    preventOnFilter: false,
    onStart: () => (dragging = true),
    onEnd: (e) => {
      dragging = false;
      if (e.from === e.to && e.oldIndex === e.newIndex) return;
      const id = e.item.dataset.id!;
      const to = QUADRANTS[Number(e.to.dataset.q)];
      place(id, to, e.newIndex ?? 0);
    },
  });
  return list;
});

function listOf(q: Quadrant, except?: string): Want[] {
  return [...store.wants.values()].filter((w) => isActive(w) && sameQuadrant(w, q) && w.id !== except).sort(byPos);
}

function tailPos(q: Quadrant, except?: string): string {
  const l = listOf(q, except);
  return between(l.at(-1)?.pos ?? null, null);
}

function add(title: string, q: Quadrant) {
  store.commit({
    op: "create",
    want: {
      id: uuidv7(),
      title,
      notes: "",
      energy: q.energy,
      clau: q.clau,
      pos: tailPos(q),
      done_at: null,
      deleted: false,
      rev: 0,
      created_at: nowRfc3339(),
    },
  });
}

function patch(id: string, p: WantPatch) {
  store.commit({ op: "patch", id, patch: p });
}

/** id を区分 q の index 番目に置く。pos が重複して挟めなければ区分全体を振り直す */
function place(id: string, q: Quadrant, index: number) {
  const w = store.wants.get(id);
  if (!w) return;
  const others = listOf(q, id);
  const a = others[index - 1]?.pos ?? null;
  const b = others[index]?.pos ?? null;
  const moveQ: WantPatch = sameQuadrant(w, q) ? {} : { energy: q.energy, clau: q.clau };
  if (a === null || b === null || a < b) {
    patch(id, { ...moveQ, pos: between(a, b) });
    return;
  }
  const order = [...others];
  order.splice(index, 0, w);
  let key = between(null, null);
  for (const o of order) {
    patch(o.id, { ...(o.id === id ? moveQ : {}), pos: key });
    key = after(key);
  }
}

function markDone(id: string) {
  patch(id, { done_at: nowRfc3339() });
}

function renderWants() {
  if (dragging) return;
  QUADRANTS.forEach((q, qi) => {
    const items = listOf(q);
    const list = lists[qi];
    list.replaceChildren(
      ...items.map((w, i) => {
        const check = h("button", { class: "check", title: "やった", "aria-label": `${w.title} をやった` });
        check.addEventListener("click", (e) => {
          e.stopPropagation();
          li.classList.add("leaving");
          setTimeout(() => markDone(w.id), 180);
        });
        const title = h("span", { class: "title" }, w.title);
        if (w.notes) title.append(h("span", { class: "has-notes", title: w.notes }, "…"));
        const li = h("li", { class: "item", "data-id": w.id, tabindex: "0" }, h("span", { class: "num" }, String(i + 1)), title, check);
        li.addEventListener("click", () => openEditor(w.id));
        li.addEventListener("keydown", (e) => {
          if (e.key === "Enter") openEditor(w.id);
        });
        return li;
      }),
    );
    $<HTMLSpanElement>(`.quad[data-q="${qi}"] .count`).textContent = String(items.length);
  });
}

// ───────── やったこと ─────────

const doneList = $<HTMLOListElement>("#done-list");

function renderDone() {
  const done = [...store.wants.values()]
    .filter((w) => !w.deleted && w.done_at !== null)
    .sort((a, b) => (a.done_at! < b.done_at! ? 1 : a.done_at! > b.done_at! ? -1 : 0));
  if (done.length === 0) {
    doneList.replaceChildren(h("li", { class: "empty" }, "まだありません"));
    return;
  }
  doneList.replaceChildren(
    ...done.map((w) => {
      const d = new Date(w.done_at!);
      const date = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
      const undo = h("button", { class: "undo" }, "戻す");
      undo.addEventListener("click", () => patch(w.id, { done_at: null, pos: tailPos(w, w.id) }));
      return h(
        "li",
        {},
        h("time", { datetime: w.done_at! }, date),
        h("span", { class: "title" }, w.title),
        h("span", { class: "tag" }, label(w)),
        undo,
      );
    }),
  );
}

// ───────── タブ ─────────

let view: "wants" | "done" = "wants";
for (const tab of document.querySelectorAll<HTMLButtonElement>(".tabs button")) {
  tab.addEventListener("click", () => {
    view = tab.dataset.view as typeof view;
    for (const t of document.querySelectorAll<HTMLButtonElement>(".tabs button"))
      t.setAttribute("aria-selected", String(t === tab));
    $("#view-wants").hidden = view !== "wants";
    $("#view-done").hidden = view !== "done";
  });
}

// ───────── 編集ダイアログ ─────────

const editor = $<HTMLDialogElement>("#editor");
const edForm = $<HTMLFormElement>("form", editor);
let editing: string | null = null;

function openEditor(id: string) {
  const w = store.wants.get(id);
  if (!w) return;
  editing = id;
  const f = edForm.elements as unknown as Record<string, HTMLInputElement & RadioNodeList>;
  f.title.value = w.title;
  f.notes.value = w.notes;
  f.energy.value = w.energy ? "1" : "0";
  f.clau.value = w.clau ? "1" : "0";
  editor.showModal();
}

editor.addEventListener("close", () => {
  const id = editing;
  editing = null;
  if (editor.returnValue !== "save" || !id) return;
  const w = store.wants.get(id);
  if (!w) return;
  const f = edForm.elements as unknown as Record<string, HTMLInputElement & RadioNodeList>;
  const p: WantPatch = {};
  const title = f.title.value.trim();
  if (title && title !== w.title) p.title = title;
  const notes = f.notes.value.trimEnd();
  if (notes !== w.notes) p.notes = notes;
  const q = { energy: f.energy.value === "1", clau: f.clau.value === "1" };
  if (!sameQuadrant(w, q)) Object.assign(p, q, { pos: tailPos(q, id) });
  if (Object.keys(p).length > 0) patch(id, p);
});

editor.addEventListener("click", (e) => {
  const act = (e.target as HTMLElement).dataset.act;
  if (!act || !editing) return;
  const id = editing;
  const w = store.wants.get(id);
  if (act === "done") {
    markDone(id);
    editor.close();
  } else if (act === "delete" && w && confirm(`「${w.title}」を削除しますか？`)) {
    store.commit({ op: "delete", id });
    editor.close();
  }
});

// ───────── 同期の設定 ─────────

const settings = $<HTMLDialogElement>("#settings");
const tokenInput = $<HTMLInputElement>("input[name=token]", settings);
const connBtn = $<HTMLButtonElement>("#conn");
connBtn.addEventListener("click", () => {
  tokenInput.value = store.token ?? "";
  settings.showModal();
});
settings.addEventListener("close", () => {
  if (settings.returnValue !== "save") return;
  const t = tokenInput.value.trim();
  store.connect(t === "" ? null : t);
});

function renderConn() {
  connBtn.dataset.state = store.conn;
  connBtn.textContent = CONN_LABEL[store.conn] + (store.outboxLen > 0 ? ` · 未送信 ${store.outboxLen}` : "");
}

// ───────── 起動 ─────────

function render() {
  renderWants();
  renderDone();
  renderConn();
}

store.subscribe(render);
render();
store.connect(store.token);
if (store.token === null) settings.showModal();

// オフライン時の再送と、SSE の取りこぼし対策
setInterval(() => store.sync(), 30_000);
window.addEventListener("online", () => store.sync());

if ("serviceWorker" in navigator && import.meta.env.PROD) {
  navigator.serviceWorker.register("/sw.js");
}
