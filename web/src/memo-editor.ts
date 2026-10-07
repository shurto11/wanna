// メモ本文のエディタ。CodeMirror 6 に vim のキー操作を載せる
import { EditorView, basicSetup } from "codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { Compartment, EditorState } from "@codemirror/state";
import { Vim, vim } from "@replit/codemirror-vim";

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

export function createMemoEditor(
  parent: HTMLElement,
  doc: string,
  opts: { vim: boolean; onSave: () => void; onClose: () => void; onChange: () => void },
): MemoEditor {
  const vimSlot = new Compartment();
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        // vim のキーは他のキー割り当てより先に取る
        vimSlot.of(opts.vim ? vim() : []),
        basicSetup,
        markdown(),
        EditorView.lineWrapping,
        EditorView.updateListener.of((u) => {
          if (u.docChanged) opts.onChange();
        }),
      ],
    }),
  });
  // vim はコマンドの処理を終えてからエディタに触るので、壊すのはその後にする
  current = { save: opts.onSave, close: () => setTimeout(opts.onClose) };
  return {
    view,
    setVim(on) {
      view.dispatch({ effects: vimSlot.reconfigure(on ? vim() : []) });
    },
    destroy() {
      current = null;
      view.destroy();
    },
  };
}
