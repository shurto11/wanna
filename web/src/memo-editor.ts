// メモ本文のエディタ。CodeMirror 6 に vim のキー操作を載せる。
// キー操作と見た目は ~/.vimrc (habamax + vim-airline) に寄せている
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { bracketMatching, indentUnit, syntaxHighlighting } from "@codemirror/language";
import { searchKeymap } from "@codemirror/search";
import { Compartment, EditorSelection, EditorState, Prec, type StateCommand } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  type Panel,
  ViewPlugin,
  type ViewUpdate,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  showPanel,
} from "@codemirror/view";
import { Tag, styleTags, tagHighlighter, tags as t } from "@lezer/highlight";
import { Vim, getCM, vim } from "@replit/codemirror-vim";

/** vim を使うかの保存先 (端末ごとの好み) */
const VIM_KEY = "wanna.vim";

/** 既定はタッチ端末なら切、それ以外は入 */
export function vimPreferred(): boolean {
  try {
    const v = localStorage.getItem(VIM_KEY);
    if (v !== null) return v === "1";
  } catch {
    // 保存できない環境では既定のまま
  }
  return !matchMedia("(pointer: coarse)").matches;
}

export function setVimPreferred(on: boolean) {
  try {
    localStorage.setItem(VIM_KEY, on ? "1" : "0");
  } catch {
    // 保存できなくても今回の画面では効く
  }
}

export interface MemoEditor {
  view: EditorView;
  setVim(on: boolean): void;
  /** airline に出すファイル名と変更の有無 */
  setFile(name: string, modified: boolean): void;
  destroy(): void;
}

/** 開いている1つのエディタ。:w / :q はこれに向けて呼ぶ */
let current: { save: () => void; close: () => void } | null = null;

// :w で保存、:q で閉じる、:wq / :x で両方
Vim.defineEx("write", "w", () => current?.save());
Vim.defineEx("quit", "q", () => current?.close());
Vim.defineEx("wq", "wq", () => {
  current?.save();
  current?.close();
});
Vim.defineEx("xit", "x", () => {
  current?.save();
  current?.close();
});

// ───────── .vimrc ─────────

/** set tabstop=4 shiftwidth=4 */
const TAB = 4;

// inoremap jj <Esc>。待つ長さは vim の timeoutlen の既定 (1秒) に合わせる
Vim.noremap("jj", "<Esc>", "insert");
Vim.setOption("insertModeEscKeysTimeout", 1000);

// UseEasyRegname: 無名レジスタへの y / d / c を "y "d "c にも写す
{
  const regs = Vim.getRegisterController();
  const pushText = regs.pushText.bind(regs);
  const easy: Record<string, string> = { yank: "y", delete: "d", change: "c" };
  regs.pushText = (name, op, text, linewise, blockwise) => {
    pushText(name, op, text, linewise, blockwise);
    if (name || !easy[op]) return;
    const u = regs.unnamedRegister;
    regs.getRegister(easy[op]).setText(u.toString(), u.linewise, u.blockwise);
  };
}

/** set expandtab: 挿入モードの Tab を次のタブ位置までの空白にする */
const expandTab: StateCommand = ({ state, dispatch }) => {
  dispatch(
    state.update(
      state.changeByRange((r) => {
        const line = state.doc.lineAt(r.from);
        const n = TAB - (displayWidth(line.text.slice(0, r.from - line.from)) % TAB);
        return {
          changes: { from: r.from, to: r.to, insert: " ".repeat(n) },
          range: EditorSelection.cursor(r.from + n),
        };
      }),
      { scrollIntoView: true, userEvent: "input" },
    ),
  );
  return true;
};

/** ノーマルモードの Tab は vim に任せ、挿入モードのときだけ空白を入れる */
const vimKeys = Prec.high(
  keymap.of([
    {
      key: "Tab",
      run: (view) => !!getCM(view)?.state.vim?.insertMode && expandTab(view),
    },
  ]),
);

// ───────── habamax ─────────

/** Markdown の記号を vim の syntax/markdown.vim と同じ粒度で塗り分けるためのタグ */
const md = {
  headerMark: Tag.define(),
  setextMark: Tag.define(),
  listMark: Tag.define(),
  quoteMark: Tag.define(),
  codeMark: Tag.define(),
  linkMark: Tag.define(),
};

const markdownTags = styleTags({
  HeaderMark: md.headerMark,
  "SetextHeading1/HeaderMark SetextHeading2/HeaderMark": md.setextMark,
  ListMark: md.listMark,
  QuoteMark: md.quoteMark,
  "CodeMark CodeInfo": md.codeMark,
  LinkMark: md.linkMark,
});

/** 色は style.css の .hb-* に置く。vim の highlight group 名を併記する */
const habamax = tagHighlighter([
  { tag: t.heading, class: "hb-title" }, // markdownH1 → Title
  { tag: md.headerMark, class: "hb-special" }, // markdownHeadingDelimiter → Delimiter
  { tag: [md.setextMark, t.contentSeparator], class: "hb-preproc" }, // markdownRule → PreProc
  { tag: md.listMark, class: "hb-statement" }, // markdownListMarker → htmlTagName
  { tag: [md.quoteMark, t.comment], class: "hb-comment" }, // markdownBlockquote → Comment
  { tag: md.codeMark, class: "hb-special" }, // markdownCodeDelimiter → Delimiter
  { tag: [t.escape, t.character], class: "hb-special" }, // markdownEscape → Special
  { tag: t.emphasis, class: "hb-italic" },
  { tag: t.strong, class: "hb-bold" },
  { tag: t.link, class: "hb-link" }, // markdownLinkText → Underlined
  { tag: md.linkMark, class: "hb-linkmark" },
  { tag: [t.url, t.string], class: "hb-string" }, // markdownUrl → String
  { tag: t.labelName, class: "hb-type" }, // markdownId → Type
]);

/** 選択中の文字を Visual の文字色にする (選択の背景は文字の下に描かれるため) */
const visualText = Decoration.mark({ class: "cm-visualText" });
const visualTextPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = this.build(view);
    }
    update(u: ViewUpdate) {
      if (u.selectionSet || u.docChanged) this.decorations = this.build(u.view);
    }
    build(view: EditorView) {
      const ranges = view.state.selection.ranges.filter((r) => !r.empty);
      return Decoration.set(ranges.map((r) => visualText.range(r.from, r.to)));
    }
  },
  { decorations: (p) => p.decorations },
);

// ───────── vim-airline ─────────

type Mode = "normal" | "insert" | "replace" | "visual" | "vline" | "vblock" | "command";

/** airline の mode 表示と、showmode が下の行に出す文言 */
const MODES: Record<Mode, { name: string; msg: string }> = {
  normal: { name: "NORMAL", msg: "" },
  insert: { name: "INSERT", msg: "-- INSERT --" },
  replace: { name: "REPLACE", msg: "-- REPLACE --" },
  visual: { name: "VISUAL", msg: "-- VISUAL --" },
  vline: { name: "V-LINE", msg: "-- VISUAL LINE --" },
  vblock: { name: "V-BLOCK", msg: "-- VISUAL BLOCK --" },
  command: { name: "COMMAND", msg: "" },
};

function modeOf(view: EditorView): Mode | null {
  const cm = getCM(view);
  const v = cm?.state.vim;
  if (!cm || !v) return null;
  // : や / の入力欄が開いているとき。通知 ("3 lines yanked" など) は入力欄を持たない
  if (cm.state.dialog?.querySelector("input")) return "command";
  if (v.insertMode) return cm.state.overwrite ? "replace" : "insert";
  if (v.visualMode) return v.visualBlock ? "vblock" : v.visualLine ? "vline" : "visual";
  return "normal";
}

/**
 * vim の通知を vim に寄せる。codemirror-vim はエラーも "3 lines yanked" も同じ赤字で出すので、
 * ヤンクの知らせは Normal の色にし、report=2 (既定) にならって2行以下なら出さない
 */
function tidyMessage(cm: NonNullable<ReturnType<typeof getCM>>) {
  const dialog = cm.state.dialog;
  const msg = dialog?.querySelector(".cm-vim-message");
  const m = msg && /^(\d+) lines yanked/.exec(msg.textContent ?? "");
  if (!dialog || !msg || !m) return;
  // 通知はクリックで閉じる
  if (Number(m[1]) <= 2) dialog.click();
  else msg.classList.add("cm-vim-info");
}

/** 端末での表示幅。全角は2、タブは次のタブ位置まで */
function displayWidth(s: string): number {
  let w = 0;
  for (const ch of s) {
    if (ch === "\t") w += TAB - (w % TAB);
    else w += WIDE.test(ch) ? 2 : 1;
  }
  return w;
}
const WIDE =
  /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦\u{1f300}-\u{1f64f}\u{1f900}-\u{1f9ff}\u{20000}-\u{3fffd}]/u;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text = "") {
  const e = document.createElement(tag);
  e.className = cls;
  e.textContent = text;
  return e;
}

export function createMemoEditor(
  parent: HTMLElement,
  doc: string,
  opts: { vim: boolean; onSave: () => void; onClose: () => void; onChange: () => void },
): MemoEditor {
  const vimSlot = new Compartment();
  const file = { name: "", modified: false };

  // airline の statusline。a(mode) c(ファイル名) x(filetype) y(ffenc) z(位置)
  const status = el("div", "cm-airline");
  const secA = el("span", "al-a");
  const secC = el("span", "al-c");
  const secZ = el("span", "al-z");
  const pos = el("b", "");
  status.append(secA, secC, el("span", "al-x", "markdown"), el("span", "al-y", "utf-8[unix]"), secZ);
  // statusline の下の行。showmode の文言を出す。: の入力中は vim の入力欄に譲る
  const cmdline = el("div", "cm-cmdline");

  const render = () => {
    const mode = modeOf(view);
    status.dataset.mode = mode ?? "none";
    status.classList.toggle("al-modified", file.modified);
    secA.hidden = mode === null;
    secA.textContent = mode ? MODES[mode].name : "";
    secC.textContent = file.name + (file.modified ? "[+]" : "");

    const { state } = view;
    const sel = state.selection.main;
    // ビジュアルモードの選択は先頭側に1文字広がっているので、カーソルの位置に戻す
    const head = mode?.startsWith("v") && sel.anchor < sel.head ? sel.head - 1 : sel.head;
    const line = state.doc.lineAt(head);
    const col = displayWidth(line.text.slice(0, head - line.from)) + 1;
    const pct = Math.floor((line.number * 100) / state.doc.lines);
    // %p%% ㏑:%2l/%L≡ ℅:%v。位置の部分は太字
    pos.textContent = `㏑:${String(line.number).padStart(2)}/${state.doc.lines}≡ ℅:${col}`;
    secZ.replaceChildren(`${pct}% `, pos);

    cmdline.hidden = mode === null || !!getCM(view)?.state.dialog;
    cmdline.textContent = mode ? MODES[mode].msg : "";
  };

  /** vim のモードはトランザクションを伴わずに変わるので、イベントで拾う */
  const modeWatcher = ViewPlugin.define((v) => {
    const cm = getCM(v);
    const onDialog = () => {
      if (cm) tidyMessage(cm);
      render();
    };
    cm?.on("vim-mode-change", render);
    cm?.on("dialog", onDialog);
    queueMicrotask(render);
    return {
      destroy() {
        cm?.off("vim-mode-change", render);
        cm?.off("dialog", onDialog);
        queueMicrotask(render);
      },
    };
  });

  const statusPanel = (): Panel => ({ dom: status, update: render });
  const cmdlinePanel = (): Panel => ({ dom: cmdline });

  const vimExt = (on: boolean) => (on ? [vim(), modeWatcher, vimKeys] : []);

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        // vim のキーは他のキー割り当てより先に取る
        vimSlot.of(vimExt(opts.vim)),
        lineNumbers(), // set number
        highlightActiveLine(), // set cursorline
        highlightActiveLineGutter(),
        highlightSpecialChars(),
        history(),
        drawSelection(),
        EditorState.allowMultipleSelections.of(true),
        bracketMatching(), // set showmatch
        closeBrackets(), // カッコ補完
        EditorState.languageData.of(() => [{ closeBrackets: { brackets: ["(", "[", "{", '"'] } }]),
        indentUnit.of(" ".repeat(TAB)), // set shiftwidth=4 expandtab
        EditorState.tabSize.of(TAB), // set tabstop=4
        EditorView.lineWrapping, // set wrap linebreak
        // vim の markdown は formatoptions-=r なので、Enter でリストの記号を継がない
        markdown({ addKeymap: false, extensions: { props: [markdownTags] } }),
        syntaxHighlighting(habamax),
        EditorView.darkTheme.of(true), // habamax は background=dark
        visualTextPlugin,
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap]),
        // statusline は vim のコマンド行より上に置く
        Prec.highest(showPanel.of(statusPanel)),
        showPanel.of(cmdlinePanel),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) opts.onChange();
        }),
      ],
    }),
  });
  render();
  // vim はコマンドの処理を終えてからエディタに触るので、壊すのはその後にする
  current = { save: opts.onSave, close: () => setTimeout(opts.onClose) };
  return {
    view,
    setVim(on) {
      view.dispatch({ effects: vimSlot.reconfigure(vimExt(on)) });
    },
    setFile(name, modified) {
      file.name = name;
      file.modified = modified;
      render();
    },
    destroy() {
      current = null;
      view.destroy();
    },
  };
}
