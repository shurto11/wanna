import Sortable from "sortablejs";
import { after, between } from "./pos.ts";
import * as due from "./due.ts";
import {
  DONE_LABEL,
  KINDS,
  QUADRANTS,
  axisLabel,
  byPos,
  dueOf,
  hasDue,
  isActive,
  kindLabel,
  label,
  nowRfc3339,
  sameQuadrant,
  uuidv7,
  type Kind,
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
      <button role="tab" data-view="want" aria-selected="true">${kindLabel("want")}</button>
      <button role="tab" data-view="task" aria-selected="false">${kindLabel("task")}</button>
      <button role="tab" data-view="done" aria-selected="false">${DONE_LABEL}</button>
    </nav>
    <button class="conn" id="conn" title="同期の設定"></button>
  </header>

  <section id="view-done" class="view" hidden>
    <ol class="done-list" id="done-list"></ol>
  </section>

  <dialog id="editor">
    <form method="dialog" class="sheet">
      <label class="field"><span>名前</span><input name="title" required autocomplete="off" /></label>
      <label class="field" id="due-field">
        <span>日時</span>
        <input name="due" autocomplete="off" placeholder="${due.HINT}" />
      </label>
      <p class="error" id="due-error" hidden></p>
      <label class="field"><span>メモ</span><textarea name="notes" rows="5"></textarea></label>
      <div class="toggles">
        <fieldset class="seg"><legend>リスト</legend>
          <label><input type="radio" name="kind" value="want" />${kindLabel("want")}</label>
          <label><input type="radio" name="kind" value="task" />${kindLabel("task")}</label>
        </fieldset>
        <fieldset class="seg"><legend id="axis-legend">エネルギー</legend>
          <label><input type="radio" name="axis_hi" value="1" />高</label>
          <label><input type="radio" name="axis_hi" value="0" />低</label>
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

// ───────── 2×2 グリッド (リストごとに1枚) ─────────

/** 1枚の面を組み立てる。区分の位置で軸がわかるので、見出しは1列表示のときだけ出す */
function buildPlane(kind: Kind): HTMLOListElement[] {
  const grid = h("div", { class: "grid" });
  const section = h(
    "section",
    { id: `view-${kind}`, class: "view" },
    h(
      "div",
      { class: "plane" },
      h(
        "div",
        { class: "axis axis-y", "aria-hidden": "true" },
        h("span", { class: "hi" }, `${axisLabel(kind)}高`),
        h("span", { class: "lo" }, `${axisLabel(kind)}低`),
      ),
      h(
        "div",
        { class: "axis axis-x", "aria-hidden": "true" },
        h("span", { class: "lo" }, "clau低"),
        h("span", { class: "hi" }, "clau高"),
      ),
      grid,
    ),
  );
  $("#view-done").before(section);

  return QUADRANTS.map((q, qi) => {
    const list = h("ol", { class: "list", "data-q": String(qi) });
    const input = h("input", {
      placeholder: "＋ 追加",
      "aria-label": `${label(q, kind)} に追加`,
      enterkeyhint: "done",
    });
    const form = h("form", { class: "add" }, input);
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const title = input.value.trim();
      if (!title) return;
      input.value = "";
      add(title, kind, q);
    });
    grid.append(
      h(
        "section",
        { class: "quad", "data-q": String(qi) },
        h("h2", {}, h("span", { class: "label" }, label(q, kind)), h("span", { class: "count" })),
        list,
        form,
      ),
    );
    Sortable.create(list, {
      group: kind,
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
        place(e.item.dataset.id!, kind, QUADRANTS[Number(e.to.dataset.q)], e.newIndex ?? 0);
      },
    });
    return list;
  });
}

const planes = new Map<Kind, HTMLOListElement[]>(KINDS.map((k) => [k, buildPlane(k)]));

function listOf(kind: Kind, q: Quadrant, except?: string): Want[] {
  return [...store.wants.values()]
    .filter((w) => isActive(w) && w.kind === kind && sameQuadrant(w, q) && w.id !== except)
    .sort(byPos);
}

function tailPos(kind: Kind, q: Quadrant, except?: string): string {
  return between(listOf(kind, q, except).at(-1)?.pos ?? null, null);
}

function add(title: string, kind: Kind, q: Quadrant) {
  store.commit({
    op: "create",
    want: {
      id: uuidv7(),
      title,
      notes: "",
      kind,
      axis_hi: q.axis_hi,
      clau: q.clau,
      pos: tailPos(kind, q),
      due_at: null,
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

/** id をリスト kind の区分 q の index 番目に置く。pos が重複して挟めなければ区分全体を振り直す */
function place(id: string, kind: Kind, q: Quadrant, index: number) {
  const w = store.wants.get(id);
  if (!w) return;
  const others = listOf(kind, q, id);
  const a = others[index - 1]?.pos ?? null;
  const b = others[index]?.pos ?? null;
  const moveQ: WantPatch = sameQuadrant(w, q) ? {} : { axis_hi: q.axis_hi, clau: q.clau };
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

function renderPlane(kind: Kind) {
  const now = new Date();
  QUADRANTS.forEach((q, qi) => {
    const items = listOf(kind, q);
    planes.get(kind)![qi].replaceChildren(
      ...items.map((w, i) => {
        const check = h("button", { class: "check", title: "やった", "aria-label": `${w.title} をやった` });
        check.addEventListener("click", (e) => {
          e.stopPropagation();
          li.classList.add("leaving");
          setTimeout(() => markDone(w.id), 180);
        });
        const title = h("span", { class: "title" }, w.title);
        if (w.notes) title.append(h("span", { class: "has-notes", title: w.notes }, "…"));
        const li = h("li", { class: "item", "data-id": w.id, tabindex: "0" }, h("span", { class: "num" }, String(i + 1)));
        const d = dueOf(w);
        if (d) li.append(h("time", { class: "due", "data-state": due.state(d, now), datetime: d }, due.format(d, now)));
        li.append(title, check);
        li.addEventListener("click", () => openEditor(w.id));
        li.addEventListener("keydown", (e) => {
          if (e.key === "Enter") openEditor(w.id);
        });
        return li;
      }),
    );
    $<HTMLSpanElement>(`#view-${kind} .quad[data-q="${qi}"] .count`).textContent = String(items.length);
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
      undo.addEventListener("click", () => patch(w.id, { done_at: null, pos: tailPos(w.kind, w, w.id) }));
      const del = h("button", { class: "del", "aria-label": `${w.title} を削除` }, "削除");
      del.addEventListener("click", () => {
        if (confirm(`「${w.title}」を削除しますか？`)) store.commit({ op: "delete", id: w.id });
      });
      return h(
        "li",
        {},
        h("time", { datetime: w.done_at! }, date),
        h("span", { class: "title" }, w.title),
        h("span", { class: "tag" }, `${kindLabel(w.kind)} / ${label(w, w.kind)}`),
        undo,
        del,
      );
    }),
  );
}

// ───────── タブ ─────────

type View = Kind | "done";
const VIEWS: View[] = [...KINDS, "done"];

function showView(v: View) {
  for (const t of document.querySelectorAll<HTMLButtonElement>(".tabs button"))
    t.setAttribute("aria-selected", String(t.dataset.view === v));
  for (const x of VIEWS) $(`#view-${x}`).hidden = x !== v;
}

for (const tab of document.querySelectorAll<HTMLButtonElement>(".tabs button"))
  tab.addEventListener("click", () => showView(tab.dataset.view as View));

// 面はどちらも組み立て済みなので、最初にどれを出すかはここで決める
showView("want");

// ───────── 編集ダイアログ ─────────

const editor = $<HTMLDialogElement>("#editor");
const edForm = $<HTMLFormElement>("form", editor);
const dueField = $<HTMLLabelElement>("#due-field");
const dueError = $<HTMLParagraphElement>("#due-error");
const axisLegend = $<HTMLLegendElement>("#axis-legend");
let editing: string | null = null;

type Fields = Record<string, HTMLInputElement & RadioNodeList>;
const fields = () => edForm.elements as unknown as Fields;
const formKind = () => fields().kind.value as Kind;

/** リストによって縦軸の名前と日時欄が変わる */
function syncEditorKind() {
  const kind = formKind();
  axisLegend.textContent = axisLabel(kind);
  dueField.hidden = !hasDue(kind);
  if (dueField.hidden) dueError.hidden = true;
}

for (const r of edForm.querySelectorAll<HTMLInputElement>("input[name=kind]"))
  r.addEventListener("change", syncEditorKind);

function openEditor(id: string) {
  const w = store.wants.get(id);
  if (!w) return;
  editing = id;
  const f = fields();
  f.title.value = w.title;
  f.notes.value = w.notes;
  f.kind.value = w.kind;
  f.axis_hi.value = w.axis_hi ? "1" : "0";
  f.clau.value = w.clau ? "1" : "0";
  f.due.value = dueOf(w) ? due.toInput(dueOf(w)!) : "";
  dueError.hidden = true;
  syncEditorKind();
  editor.showModal();
}

edForm.addEventListener("submit", (e) => {
  if (((e as SubmitEvent).submitter as HTMLButtonElement | null)?.value !== "save") return;
  const w = editing === null ? undefined : store.wants.get(editing);
  if (!w) return;
  const f = fields();
  const kind = formKind();

  let dueAt: string | null = null;
  if (hasDue(kind)) {
    try {
      dueAt = due.parseInput(f.due.value);
    } catch (err) {
      // 読めない日時で閉じない。理由を出してそのまま直してもらう
      e.preventDefault();
      dueError.textContent = (err as Error).message;
      dueError.hidden = false;
      f.due.focus();
      return;
    }
  }

  const p: WantPatch = {};
  const title = f.title.value.trim();
  if (title && title !== w.title) p.title = title;
  const notes = f.notes.value.trimEnd();
  if (notes !== w.notes) p.notes = notes;
  const q = { axis_hi: f.axis_hi.value === "1", clau: f.clau.value === "1" };
  if (kind !== w.kind || !sameQuadrant(w, q))
    Object.assign(p, { kind, ...q, pos: tailPos(kind, q, w.id) });
  if (dueAt !== (w.due_at ?? null)) p.due_at = dueAt;
  if (Object.keys(p).length > 0) patch(w.id, p);
});

editor.addEventListener("close", () => {
  editing = null;
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
  if (!dragging) for (const kind of KINDS) renderPlane(kind);
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
