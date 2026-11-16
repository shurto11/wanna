mod keys;

pub use keys::{help_lines, Pending, HELP};

use crate::input::TextInput;
use crate::store::{Op, Store};
use crate::sync::{FromSync, ToWorker};
use anyhow::Result;
use ratatui::widgets::ListState;
use std::ops::Range;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use wanna_core::{due, notes, now_rfc3339, pos, sort_by_due, sort_by_pos, Kind, Quadrant, Want, WantPatch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// やりたいこと / 次にやること (どちらかは `App::kind`)
    List,
    Done,
    /// やってはいないが、リストに置くほどでもなくなったもの
    Archive,
    /// 区分を持たないメモ。本文はエディタで書く
    Memo,
}

/// タブの数 (Want, Must, Done, Archive, Memo)
const TABS: usize = 5;

/// 取り消せる回数
const UNDO_MAX: usize = 100;

/// 外部エディタでのメモ編集の依頼。main ループが端末を明け渡して処理する
pub struct EditorReq {
    pub id: String,
    pub text: String,
    /// 編集ポップアップから開いたなら、結果をポップアップに戻す (保存はしない)
    pub from_popup: bool,
}

/// 編集ポップアップ内のカーソル位置
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Title,
    Axis,
    Clau,
    Due,
}

pub struct Edit {
    pub id: String,
    pub kind: Kind,
    pub title: TextInput,
    pub axis_hi: bool,
    pub clau: bool,
    /// 日時の入力欄 (`due` の入力形式)。次にやることだけ使う
    pub due: TextInput,
    pub notes: String,
    pub field: EditField,
    /// 日時が読めなかったなどの説明
    pub error: Option<String>,
    /// 入力欄に文字を打っている (vim の挿入モード)。false ならポップアップの中のノーマルモード
    pub insert: bool,
    /// ポップアップの中で打っている `:` コマンド
    pub cmd: Option<TextInput>,
    /// `ZZ` / `ZQ` の打ちかけ
    zz: bool,
}

impl Edit {
    pub fn new(w: &Want) -> Self {
        Self {
            id: w.id.clone(),
            kind: w.kind,
            title: TextInput::new(&w.title),
            axis_hi: w.axis_hi,
            clau: w.clau,
            due: TextInput::new(&w.due().map(due::to_input).unwrap_or_default()),
            notes: w.notes.clone(),
            field: EditField::Title,
            error: None,
            insert: true,
            cmd: None,
            zz: false,
        }
    }

    /// この1件で回れる欄。Must は clau度で分けないので clau度の欄を出さず、日時の欄を出す。メモは名前だけ
    pub fn fields(&self) -> &'static [EditField] {
        use EditField::*;
        match self.kind {
            Kind::Memo => &[Title],
            k if k.has_due() => &[Title, Axis, Due],
            _ => &[Title, Axis, Clau],
        }
    }

    /// 欄を移る。移った先ではノーマルモード
    fn move_field(&mut self, down: bool) {
        let fields = self.fields();
        let i = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = if down { (i + 1).min(fields.len() - 1) } else { i.saturating_sub(1) };
        self.field = fields[next];
        self.insert = false;
        if let Some(input) = self.input_mut() {
            input.clamp_normal();
        }
    }

    /// 文字を打つ欄か (高低を選ぶ欄では h/l などがキー操作になる)
    pub fn typing(&self) -> bool {
        matches!(self.field, EditField::Title | EditField::Due)
    }

    /// 今 vim の挿入モードか
    pub fn inserting(&self) -> bool {
        self.insert && self.typing()
    }

    fn input_mut(&mut self) -> Option<&mut TextInput> {
        match self.field {
            EditField::Title => Some(&mut self.title),
            EditField::Due => Some(&mut self.due),
            _ => None,
        }
    }
}

pub enum Mode {
    Normal,
    /// `o` / `O` で追加する名前の入力。`at` は区分の何番目に入れるか (メモでは使わない)
    Add { input: TextInput, at: usize },
    Edit(Edit),
    /// ビジュアルモード。`anchor` 行目から今のカーソルまでを選ぶ
    Visual { anchor: usize },
    /// `/` で打っている検索語
    Search(TextInput),
    /// `:` で打っているコマンド
    Command(TextInput),
    /// `?` のキー一覧。数字はスクロール位置
    Help(u16),
}

/// 取り消し1回ぶんの変更のうちの1件。変更前と変更後を持つ
struct Change {
    id: String,
    before: Want,
    after: Want,
}

pub struct App {
    store: Store,
    pub wants: Vec<Want>,
    pub screen: Screen,
    /// 表示しているリスト。やったこと画面から戻る先でもある
    pub kind: Kind,
    pub mode: Mode,
    /// カーソルのある区分 (`Quadrant::ALL` の添字)。リストをまたいで持ち越す。
    /// Must は左の列しか使わないので、読むときは `qi()` を通す
    pub cur: usize,
    /// リストごと・区分ごとの選択行
    pub lists: [[ListState; 4]; 2],
    pub done_list: ListState,
    pub archive_list: ListState,
    pub memo_list: ListState,
    pub message: Option<String>,
    pub quit: bool,
    pub editor: Option<EditorReq>,
    /// 打ちかけのカウントと前置キー
    pub pending: Pending,
    /// 最後に `/` で探した語。名前の中の一致を目立たせる
    pub search: Option<String>,
    /// カーソルのある一覧の見えている行数。描画のたびに更新し、半ページ移動に使う
    pub view_rows: usize,

    /// `dd` で消したもの。`p` で貼る
    register: Vec<String>,
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
    /// 今の操作で触ったものの変更前
    recording: Option<Vec<(String, Want)>>,

    worker: Option<Sender<ToWorker>>,
    syncing: bool,
    /// 同期中に次の同期が要求された
    dirty: bool,
    last_sync: Instant,
    pub online: Option<bool>,
    pub outbox_len: usize,
}

impl App {
    pub fn new(store: Store, worker: Option<Sender<ToWorker>>) -> Result<Self> {
        let wants = store.load_all()?;
        let outbox_len = store.outbox_len()?;
        let mut app = Self {
            store,
            wants,
            screen: Screen::List,
            kind: Kind::Want,
            mode: Mode::Normal,
            cur: 0,
            lists: Default::default(),
            done_list: ListState::default(),
            archive_list: ListState::default(),
            memo_list: ListState::default(),
            message: None,
            quit: false,
            editor: None,
            pending: Pending::default(),
            search: None,
            view_rows: 10,
            register: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            recording: None,
            worker,
            syncing: false,
            dirty: false,
            last_sync: Instant::now(),
            online: None,
            outbox_len,
        };
        for k in 0..2 {
            for i in 0..4 {
                app.lists[k][i].select(Some(0));
            }
        }
        app.done_list.select(Some(0));
        app.archive_list.select(Some(0));
        app.memo_list.select(Some(0));
        app.request_sync();
        Ok(app)
    }

    pub fn has_remote(&self) -> bool {
        self.worker.is_some()
    }

    // ───────── 表示用のリスト ─────────

    /// リスト・区分の中身。Want は `(pos, id)` 順、Must は日時の順 (日時なしは最後)。
    /// Must は clau度で分けないので、縦軸 (学校 / その他) だけで区分を見る
    pub fn list(&self, kind: Kind, q: Quadrant) -> Vec<&Want> {
        let mut v: Vec<&Want> =
            self.wants.iter().filter(|w| w.is_active() && w.kind == kind && q.holds(w)).collect();
        if kind.has_due() {
            sort_by_due(&mut v);
        } else {
            sort_by_pos(&mut v);
        }
        v
    }

    /// やったこと。達成日の降順 (どちらのリストのものも混ぜて並べる)
    pub fn done(&self) -> Vec<&Want> {
        let mut v: Vec<&Want> =
            self.wants.iter().filter(|w| !w.deleted && w.done_at.is_some()).collect();
        v.sort_by(|a, b| b.done_at.cmp(&a.done_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// 保管庫。しまった日の降順 (どちらのリストのものも混ぜて並べる)
    pub fn archived(&self) -> Vec<&Want> {
        let mut v: Vec<&Want> = self.wants.iter().filter(|w| w.is_archived()).collect();
        v.sort_by(|a, b| b.archived_at.cmp(&a.archived_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// メモ。作った日の降順
    pub fn memos(&self) -> Vec<&Want> {
        let mut v: Vec<&Want> =
            self.wants.iter().filter(|w| !w.deleted && w.kind == Kind::Memo).collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id)));
        v
    }

    /// やったこと / 保管庫 / メモ の中身と選択行。リストの画面では None
    fn past(&self) -> Option<(Vec<&Want>, usize)> {
        match self.screen {
            Screen::List => None,
            Screen::Done => Some((self.done(), self.done_list.selected().unwrap_or(0))),
            Screen::Archive => Some((self.archived(), self.archive_list.selected().unwrap_or(0))),
            Screen::Memo => Some((self.memos(), self.memo_list.selected().unwrap_or(0))),
        }
    }

    fn past_list_mut(&mut self) -> &mut ListState {
        match self.screen {
            Screen::Archive => &mut self.archive_list,
            Screen::Memo => &mut self.memo_list,
            _ => &mut self.done_list,
        }
    }

    /// カーソルのある区分の添字。Must は clau度で分けないので左の列 (学校 / その他) に寄せる
    pub fn qi(&self) -> usize {
        if self.kind.splits_clau() {
            self.cur
        } else {
            self.cur & !1
        }
    }

    pub fn cur_q(&self) -> Quadrant {
        Quadrant::ALL[self.qi()]
    }

    fn cur_list(&self) -> Vec<&Want> {
        self.list(self.kind, self.cur_q())
    }

    fn sel(&self) -> usize {
        self.lists[self.kind.index()][self.qi()].selected().unwrap_or(0)
    }

    fn select(&mut self, i: usize) {
        let (k, c) = (self.kind.index(), self.qi());
        self.lists[k][c].select(Some(i));
    }

    pub fn selected(&self) -> Option<&Want> {
        match self.past() {
            None => self.cur_list().get(self.sel()).copied(),
            Some((list, sel)) => list.get(sel).copied(),
        }
    }

    /// カーソルのある一覧 (リストの画面なら区分) の長さと選択行
    fn view(&self) -> (usize, usize) {
        match self.past() {
            None => (self.cur_list().len(), self.sel()),
            Some((list, sel)) => (list.len(), sel),
        }
    }

    /// カーソルのある一覧の `i` 行目を選ぶ (範囲に収める)
    fn view_select(&mut self, i: usize) {
        let i = i.min(self.view().0.saturating_sub(1));
        match self.screen {
            Screen::List => self.select(i),
            _ => self.past_list_mut().select(Some(i)),
        }
    }

    /// カーソルのある一覧の `range` 行目の ID
    fn view_ids(&self, range: Range<usize>) -> Vec<String> {
        let list = match self.past() {
            None => self.cur_list(),
            Some((list, _)) => list,
        };
        list.iter().skip(range.start).take(range.len()).map(|w| w.id.clone()).collect()
    }

    /// ビジュアルモードで選んでいる行 (カーソルのある一覧の中)
    pub fn visual_range(&self) -> Option<Range<usize>> {
        let Mode::Visual { anchor } = self.mode else { return None };
        let sel = self.view().1;
        Some(anchor.min(sel)..anchor.max(sel) + 1)
    }

    /// 選択位置をリストの範囲に収める
    fn clamp(&mut self) {
        for kind in Kind::ALL {
            for i in 0..4 {
                let len = self.list(kind, Quadrant::ALL[i]).len();
                let s = self.lists[kind.index()][i].selected().unwrap_or(0);
                self.lists[kind.index()][i].select(Some(s.min(len.saturating_sub(1))));
            }
        }
        let len = self.done().len();
        let s = self.done_list.selected().unwrap_or(0);
        self.done_list.select(Some(s.min(len.saturating_sub(1))));
        let len = self.archived().len();
        let s = self.archive_list.selected().unwrap_or(0);
        self.archive_list.select(Some(s.min(len.saturating_sub(1))));
        let len = self.memos().len();
        let s = self.memo_list.selected().unwrap_or(0);
        self.memo_list.select(Some(s.min(len.saturating_sub(1))));
    }

    /// `id` のあるリスト・区分・行へカーソルを合わせる
    fn focus(&mut self, id: &str) {
        for kind in Kind::ALL {
            for &qi in Quadrant::slots(kind) {
                if let Some(i) = self.list(kind, Quadrant::ALL[qi]).iter().position(|w| w.id == id) {
                    self.kind = kind;
                    self.cur = qi;
                    self.lists[kind.index()][qi].select(Some(i));
                    return;
                }
            }
        }
    }

    /// `id` が今の画面に出ていればカーソルを合わせる。リストの画面ならリストも切り替える
    fn reveal(&mut self, id: &str) {
        match self.past().and_then(|(list, _)| list.iter().position(|w| w.id == id)) {
            Some(i) => self.past_list_mut().select(Some(i)),
            None if self.screen == Screen::List => self.focus(id),
            None => {}
        }
    }

    /// カーソルを下 (`down`) / 上へ `n` 行動かす。`cross` ならリストの区分の端で上下の区分へ抜ける
    fn move_cursor(&mut self, down: bool, n: usize, cross: bool) {
        for _ in 0..n {
            let (len, sel) = self.view();
            let top = self.qi() < 2;
            if down && sel + 1 < len {
                self.view_select(sel + 1);
            } else if !down && sel > 0 {
                self.view_select(sel - 1);
            } else if cross && self.screen == Screen::List && down && top {
                self.cur += 2;
                self.select(0);
            } else if cross && self.screen == Screen::List && !down && !top {
                self.cur -= 2;
                let len = self.cur_list().len();
                self.select(len.saturating_sub(1));
            } else {
                break;
            }
        }
    }

    /// 今のタブの番号 (Want, Must, Done, Archive, Memo の順で 0 から)
    fn tab(&self) -> usize {
        match self.screen {
            Screen::List => self.kind.index(),
            Screen::Done => 2,
            Screen::Archive => 3,
            Screen::Memo => 4,
        }
    }

    fn set_tab(&mut self, i: usize) {
        match i {
            0 | 1 => {
                self.screen = Screen::List;
                self.kind = Kind::ALL[i];
            }
            2 => self.screen = Screen::Done,
            3 => self.screen = Screen::Archive,
            _ => self.screen = Screen::Memo,
        }
    }

    /// 表示を Want → Must → Done → Archive → Memo の順に回す
    fn cycle_view(&mut self, forward: bool) {
        self.set_tab((self.tab() + if forward { 1 } else { TABS - 1 }) % TABS);
    }

    // ───────── 変更 ─────────

    /// ローカルに適用して outbox に積み、同期を促す
    fn commit(&mut self, op: Op) {
        self.record(&op);
        match &op {
            Op::Create { want } => self.wants.push(want.clone()),
            Op::Patch { id, patch } => {
                if let Some(w) = self.wants.iter_mut().find(|w| &w.id == id) {
                    w.apply(patch);
                }
            }
            Op::Delete { id } => {
                if let Some(w) = self.wants.iter_mut().find(|w| &w.id == id) {
                    w.deleted = true;
                }
            }
        }
        let res = (|| -> Result<()> {
            if let Some(w) = self.wants.iter().find(|w| w.id == op.id()) {
                self.store.put(w)?;
            }
            // サーバー未設定でも積んでおき、設定したときにまとめて送る
            self.store.push_op(&op)?;
            self.outbox_len = self.store.outbox_len()?;
            Ok(())
        })();
        if let Err(e) = res {
            self.message = Some(format!("保存に失敗: {e:#}"));
        }
        self.clamp();
        self.request_sync();
    }

    fn patch(&mut self, id: &str, patch: WantPatch) {
        self.commit(Op::Patch { id: id.to_string(), patch });
    }

    // ───────── 取り消し ─────────

    /// キー1回 (エディタから戻った1回) を取り消しの単位にする
    fn begin_change(&mut self) {
        self.recording = Some(Vec::new());
    }

    fn end_change(&mut self) {
        let Some(rec) = self.recording.take() else { return };
        let changes: Vec<Change> = rec
            .into_iter()
            .filter_map(|(id, before)| {
                let after = self.wants.iter().find(|w| w.id == id)?.clone();
                (after != before).then_some(Change { id, before, after })
            })
            .collect();
        if !changes.is_empty() {
            self.undo.push(changes);
            if self.undo.len() > UNDO_MAX {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// 今の操作で初めて触るものなら、変更前を覚えておく
    fn record(&mut self, op: &Op) {
        let Some(rec) = &mut self.recording else { return };
        if rec.iter().any(|(id, _)| id == op.id()) {
            return;
        }
        let before = match op {
            // 作る前は「消してある」ものとして扱う
            Op::Create { want } => Want { deleted: true, ..want.clone() },
            _ => match self.wants.iter().find(|w| w.id == op.id()) {
                Some(w) => w.clone(),
                None => return,
            },
        };
        rec.push((op.id().to_string(), before));
    }

    /// 取り消す (`redo` ならやり直す)
    fn undo(&mut self, redo: bool) {
        // 取り消し自体は取り消しの履歴に積まない
        self.recording = None;
        let Some(changes) = (if redo { self.redo.pop() } else { self.undo.pop() }) else {
            self.message =
                Some(if redo { "これ以上やり直せません" } else { "これ以上取り消せません" }.into());
            return;
        };
        for c in &changes {
            let (to, from) = if redo { (&c.after, &c.before) } else { (&c.before, &c.after) };
            self.revert(&c.id, to, from);
        }
        let what = if redo { "やり直しました" } else { "取り消しました" };
        self.message = Some(match changes.as_slice() {
            [c] => format!("{what}: {}", c.after.title),
            cs => format!("{} 件を{what}", cs.len()),
        });
        self.reveal(&changes[0].id);
        if redo {
            self.undo.push(changes);
        } else {
            self.redo.push(changes);
        }
    }

    /// `id` を `to` の状態にする。`from` と違う欄 (その操作で変わった欄) だけを書き戻す
    fn revert(&mut self, id: &str, to: &Want, from: &Want) {
        let Some(cur) = self.wants.iter().find(|w| w.id == id) else { return };
        fn pick<T: PartialEq + Clone>(to: &T, from: &T, cur: &T) -> Option<T> {
            (to != from && to != cur).then(|| to.clone())
        }
        let mut p = WantPatch {
            title: pick(&to.title, &from.title, &cur.title),
            notes: pick(&to.notes, &from.notes, &cur.notes),
            kind: pick(&to.kind, &from.kind, &cur.kind),
            axis_hi: pick(&to.axis_hi, &from.axis_hi, &cur.axis_hi),
            clau: pick(&to.clau, &from.clau, &cur.clau),
            pos: pick(&to.pos, &from.pos, &cur.pos),
            due_at: pick(&to.due_at, &from.due_at, &cur.due_at),
            done_at: pick(&to.done_at, &from.done_at, &cur.done_at),
            archived_at: pick(&to.archived_at, &from.archived_at, &cur.archived_at),
            deleted: pick(&to.deleted, &from.deleted, &cur.deleted),
            ..Default::default()
        };
        // リストに戻す・区分を移すときに pos が無いと、サーバーが区分の末尾に付け直してしまう
        let moved = !cur.is_active() || p.kind.is_some() || p.axis_hi.is_some() || p.clau.is_some();
        if to.kind.is_list() && to.is_active() && moved && p.pos.is_none() {
            p.pos = Some(to.pos.clone());
        }
        if p.pos.as_deref() == Some("") {
            p.pos = None;
        }
        if p == WantPatch::default() {
            return;
        }
        // 消すだけなら削除として送る
        if p == (WantPatch { deleted: Some(true), ..Default::default() }) {
            self.commit(Op::Delete { id: id.to_string() });
        } else {
            self.patch(id, p);
        }
    }

    // ───────── 並び・移動 ─────────

    /// リスト・区分の末尾に付ける pos。Must は日時の順に並ぶので、表示の最後ではなく一番大きい pos の後ろ
    fn tail_pos(&self, kind: Kind, q: Quadrant) -> String {
        let list = self.list(kind, q);
        pos::between(list.iter().map(|w| w.pos.as_str()).max(), None)
    }

    /// 区分の `to` 番目に新しく入れる pos。前後の pos が重複していて挟めなければ None
    fn pos_at(&self, kind: Kind, q: Quadrant, to: usize) -> Option<String> {
        let list = self.list(kind, q);
        let to = to.min(list.len());
        let a = to.checked_sub(1).map(|i| list[i].pos.as_str());
        let b = list.get(to).map(|w| w.pos.as_str());
        a.zip(b).is_none_or(|(a, b)| a < b).then(|| pos::between(a, b))
    }

    /// `ids` をこの順で `kind` の区分 `q` の `to` 番目 (`ids` 自身を除いて数える) に置く。
    /// リストや区分が違えば移し、消した・やった・しまったものはリストに戻す。
    /// pos が重複していて挟めなければ区分全体を振り直す。
    /// Must は日時の順に並び、pos は同じ日時どうしの順にしか効かないので、`to` によらず pos の順で末尾に足す
    fn put(&mut self, ids: &[String], kind: Kind, q: Quadrant, to: usize) {
        let mut others: Vec<(String, String)> = self
            .list(kind, q)
            .into_iter()
            .filter(|w| !ids.contains(&w.id))
            .map(|w| (w.id.clone(), w.pos.clone()))
            .collect();
        let to = if kind.has_due() {
            others.sort_by(|a, b| a.1.cmp(&b.1));
            others.len()
        } else {
            to.min(others.len())
        };
        let a = to.checked_sub(1).map(|i| others[i].1.clone());
        let b = others.get(to).map(|o| o.1.clone());
        let (a, b) = (a.as_deref(), b.as_deref());
        let keys: Vec<(String, String)> = if a.zip(b).is_none_or(|(a, b)| a < b) {
            let keys = match ids.len() {
                // 末尾への追加はキーが伸びにくい方法で作る
                1 => vec![pos::between(a, b)],
                n => pos::n_between(a, b, n),
            };
            ids.iter().cloned().zip(keys).collect()
        } else {
            let mut order: Vec<String> = others.into_iter().map(|o| o.0).collect();
            order.splice(to..to, ids.iter().cloned());
            let keys = pos::n_between(None, None, order.len());
            order.into_iter().zip(keys).collect()
        };
        for (id, p) in keys {
            let Some(w) = self.wants.iter().find(|w| w.id == id) else { continue };
            let mut patch = WantPatch::default();
            if ids.contains(&id) {
                patch.kind = (w.kind != kind).then_some(kind);
                patch.axis_hi = (w.axis_hi != q.axis_hi).then_some(q.axis_hi);
                // Must は clau度で分けないので、持っている値をそのまま残す
                patch.clau = (kind.splits_clau() && w.clau != q.clau).then_some(q.clau);
                patch.deleted = w.deleted.then_some(false);
                patch.done_at = w.done_at.is_some().then_some(None);
                patch.archived_at = w.archived_at.is_some().then_some(None);
                // やりたいことは日時を持たない
                patch.due_at = (!kind.has_due() && w.due_at.is_some()).then_some(None);
            }
            // 移すときは pos を必ず送る (送らないとサーバーが末尾に付け直す)
            if w.pos != p || patch != WantPatch::default() {
                patch.pos = Some(p);
                self.patch(&id, patch);
            }
        }
        if let Some(id) = ids.first() {
            self.focus(id);
        }
    }

    /// 区分内で `id` を `to` 番目 (自分を除いて数える) に置く
    fn place(&mut self, id: &str, to: usize) {
        self.put(&[id.to_string()], self.kind, self.cur_q(), to);
    }

    /// 選んでいるものを区分の中で `n` 行下 (`down`) / 上へ。区分の端にいれば上下の区分へ移す。
    /// Must は日時の順に並ぶので、上下の段 (学校 / その他) へ移すだけ
    fn shift_vertical(&mut self, down: bool, n: usize) {
        let Some(id) = self.selected().map(|w| w.id.clone()) else { return };
        if self.kind.has_due() {
            self.shift_row(&[id], down);
            return;
        }
        let (len, sel) = self.view();
        let qi = self.qi();
        let top = qi < 2;
        if down && sel + 1 < len {
            self.place(&id, (sel + n).min(len - 1));
        } else if !down && sel > 0 {
            self.place(&id, sel.saturating_sub(n));
        } else if down && top {
            self.put(&[id], self.kind, Quadrant::ALL[qi + 2], 0);
        } else if !down && !top {
            let q = Quadrant::ALL[qi - 2];
            let len = self.list(self.kind, q).len();
            self.put(&[id], self.kind, q, len);
        }
    }

    /// Must で、学校の段のものを下 (その他) へ / その他の段のものを上 (学校) へ移す。段の中では日時の順に入る
    fn shift_row(&mut self, ids: &[String], down: bool) {
        let top = self.qi() < 2;
        if ids.is_empty() || top != down {
            return;
        }
        self.put(ids, self.kind, Quadrant::ALL[if down { 2 } else { 0 }], 0);
    }

    /// 選んでいるもの (`ids`) を左右の区分へ移す。同じ行の高さに入る
    fn shift_horizontal(&mut self, ids: &[String], right: bool) {
        if !self.kind.splits_clau() {
            self.message = Some("Must は左右に分けていません (J / K で学校 ⇄ その他)".into());
            return;
        }
        let target = if right { self.cur | 1 } else { self.cur & !1 };
        if target == self.cur || ids.is_empty() {
            return;
        }
        let at = self.view_ids(0..usize::MAX).iter().position(|id| *id == ids[0]).unwrap_or(0);
        self.put(ids, self.kind, Quadrant::ALL[target], at);
    }

    /// もう一方のリストへ移す。Must へは「その他」の段に (Want のものは学校の課題ではないので)、
    /// Want へは持っている縦軸・clau度の区分の末尾に入る
    fn move_to_other_list(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let to = self.kind.other();
        for id in ids {
            let Some(w) = self.wants.iter().find(|w| &w.id == id) else { continue };
            let q = if to.has_due() { Quadrant { axis_hi: false, clau: w.clau } } else { w.quadrant() };
            let len = self.list(to, q).len();
            self.put(std::slice::from_ref(id), to, q, len);
        }
        self.focus(&ids[0]);
        self.message = Some(match ids {
            [id] => format!("{}へ移しました: {}", to.label(), self.title(id)),
            _ => format!("{} 件を{}へ移しました", ids.len(), to.label()),
        });
    }

    fn title(&self, id: &str) -> String {
        self.wants.iter().find(|w| w.id == id).map(|w| w.title.clone()).unwrap_or_default()
    }

    /// やった (しまってあったものも、やったことにする)
    fn mark_done(&mut self, ids: &[String]) {
        let now = now_rfc3339();
        for id in ids {
            let archived = self.wants.iter().any(|w| &w.id == id && w.archived_at.is_some());
            let patch = WantPatch {
                done_at: Some(Some(now.clone())),
                archived_at: archived.then_some(None),
                ..Default::default()
            };
            self.patch(id, patch);
        }
        self.message = match ids {
            [] => None,
            [id] => Some(format!("やった！ {}", self.title(id))),
            _ => Some(format!("やった！ {} 件", ids.len())),
        };
    }

    fn archive(&mut self, ids: &[String]) {
        let now = now_rfc3339();
        for id in ids {
            self.patch(id, WantPatch { archived_at: Some(Some(now.clone())), ..Default::default() });
        }
        self.message = match ids {
            [] => None,
            [id] => Some(format!("保管庫にしまいました: {}", self.title(id))),
            _ => Some(format!("{} 件を保管庫にしまいました", ids.len())),
        };
    }

    /// やった / しまったものを元のリストに戻す。元の区分の末尾に入る
    fn restore(&mut self, ids: &[String]) {
        for id in ids {
            let Some(w) = self.wants.iter().find(|w| &w.id == id) else { continue };
            let (kind, q) = (w.kind, w.quadrant());
            let len = self.list(kind, q).len();
            self.put(std::slice::from_ref(id), kind, q, len);
        }
        self.message = match ids {
            [] => None,
            [id] => {
                let kind = self.wants.iter().find(|w| &w.id == id).map(|w| w.kind.label());
                Some(format!("{}に戻しました: {}", kind.unwrap_or(""), self.title(id)))
            }
            _ => Some(format!("{} 件をリストに戻しました", ids.len())),
        };
    }

    /// 消して、`p` で貼れるようにしておく
    fn delete(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            self.commit(Op::Delete { id: id.clone() });
        }
        self.message = Some(match ids.as_slice() {
            [id] => format!("削除しました: {}  (u で戻す)", self.title(id)),
            _ => format!("{} 件を削除しました  (u で戻す)", ids.len()),
        });
        self.register = ids;
    }

    /// `dd` で消したものをカーソルの下 (`above` なら上) に貼る
    fn paste(&mut self, above: bool) {
        let ids = self.register.clone();
        if ids.is_empty() {
            self.message = Some("貼るものがありません (dd で消したものを貼れます)".into());
            return;
        }
        let memo = self.screen == Screen::Memo;
        let fits = |id: &String| self.wants.iter().any(|w| &w.id == id && (w.kind == Kind::Memo) == memo);
        if !ids.iter().all(fits) {
            self.message =
                Some(if memo { "Memo には Memo しか貼れません" } else { "Memo はリストに貼れません" }.into());
            return;
        }
        // メモは作った日の順に並ぶので、戻すだけ
        if memo {
            for id in &ids {
                if self.wants.iter().any(|w| &w.id == id && w.deleted) {
                    self.patch(id, WantPatch { deleted: Some(false), ..Default::default() });
                }
            }
            self.reveal(&ids[0]);
            return;
        }
        // カーソルの行を基準に、貼るもの自身を除いて数えた位置
        let to = match self.selected().filter(|w| !ids.contains(&w.id)) {
            Some(cur) => {
                let list = self.cur_list();
                let i = list
                    .iter()
                    .filter(|w| !ids.contains(&w.id))
                    .position(|w| w.id == cur.id)
                    .unwrap_or(0);
                if above {
                    i
                } else {
                    i + 1
                }
            }
            None => self.sel(),
        };
        self.put(&ids, self.kind, self.cur_q(), to);
    }

    /// `o` / `O` の入力から1件作る
    fn add(&mut self, title: &str, at: usize) {
        if self.screen == Screen::Memo {
            // メモは作ったらそのまま本文をエディタで開く
            let w = Want::new(title, Kind::Memo, false, false, String::new());
            let id = w.id.clone();
            self.commit(Op::Create { want: w });
            if let Some(i) = self.memos().iter().position(|w| w.id == id) {
                self.memo_list.select(Some(i));
            }
            self.editor = Some(EditorReq { id, text: String::new(), from_popup: false });
            return;
        }
        let (kind, q) = (self.kind, self.cur_q());
        // Must は日時の順に並ぶので、カーソルの位置ではなく pos の順で末尾に足す
        let pos = if kind.has_due() { Some(self.tail_pos(kind, q)) } else { self.pos_at(kind, q, at) };
        let w = Want::new(title, kind, q.axis_hi, q.clau, pos.clone().unwrap_or_else(|| self.tail_pos(kind, q)));
        let id = w.id.clone();
        self.commit(Op::Create { want: w });
        if pos.is_none() {
            self.place(&id, at);
        }
        self.focus(&id);
    }

    // ───────── 検索 ─────────

    /// 検索語に合う次 (`forward`) / 前のものへ `n` 回。端まで来たら反対の端から続ける
    fn search_next(&mut self, forward: bool, n: usize) {
        let Some(query) = self.search.clone() else {
            self.message = Some("検索語がありません (/ で探せます)".into());
            return;
        };
        // 今の画面に並んでいる順の (区分, 行, 一致するか)
        let mut items: Vec<(usize, usize, bool)> = Vec::new();
        match self.past() {
            None => {
                for &qi in Quadrant::slots(self.kind) {
                    for (i, w) in self.list(self.kind, Quadrant::ALL[qi]).iter().enumerate() {
                        items.push((qi, i, !matches(&w.title, &query).is_empty()));
                    }
                }
            }
            Some((list, _)) => {
                for (i, w) in list.iter().enumerate() {
                    items.push((0, i, !matches(&w.title, &query).is_empty()));
                }
            }
        }
        let cur = match self.screen {
            Screen::List => (self.qi(), self.sel()),
            _ => (0, self.view().1),
        };
        let len = items.len();
        // カーソルの行、無ければ (空の区分なら) その次の行
        let mut start = items.iter().position(|&(q, i, _)| (q, i) >= cur).unwrap_or(len);
        let mut on = start < len && (items[start].0, items[start].1) == cur;
        let mut wrapped = false;
        let mut found = None;
        for _ in 0..n {
            let hit = if forward {
                let s = if on { start + 1 } else { start };
                let k = (s..len).chain(0..s.min(len)).find(|&k| items[k].2);
                wrapped |= k.is_some_and(|k| k < s);
                k
            } else {
                let s = start;
                let k = (0..s).rev().chain((s..len).rev()).find(|&k| items[k].2);
                wrapped |= k.is_some_and(|k| k >= s);
                k
            };
            let Some(k) = hit else { break };
            (start, on, found) = (k, true, Some(k));
        }
        let Some(k) = found else {
            self.message = Some(format!("見つかりません: {query}"));
            return;
        };
        let (qi, i, _) = items[k];
        if self.screen == Screen::List {
            self.cur = qi;
        }
        self.view_select(i);
        self.message = Some(match (wrapped, forward) {
            (false, _) => format!("/{query}"),
            (true, true) => "末尾まで探したので先頭から探しました".into(),
            (true, false) => "先頭まで探したので末尾から探しました".into(),
        });
    }

    // ───────── 同期 ─────────

    pub fn request_sync(&mut self) {
        let Some(worker) = &self.worker else { return };
        if self.syncing {
            self.dirty = true;
            return;
        }
        let since = match self.store.rev() {
            Ok(0) | Err(_) => None,
            Ok(r) => Some(r),
        };
        let ops = self.store.ops().unwrap_or_default();
        if worker.send(ToWorker::Sync { since, ops }).is_ok() {
            self.syncing = true;
            self.dirty = false;
            self.last_sync = Instant::now();
        }
    }

    /// 定期的に呼ぶ。オフライン時の再送と、SSE が無いときの取りこぼし対策
    pub fn tick(&mut self) {
        if !self.syncing && self.last_sync.elapsed() > Duration::from_secs(30) {
            self.request_sync();
        }
    }

    pub fn on_sync(&mut self, msg: FromSync) {
        match msg {
            FromSync::Remote(rev) => {
                // 再接続直後にも届くので、未送信やオフライン状態が残っていればここで送り直す
                let behind = rev > self.store.rev().unwrap_or(0);
                if behind || self.outbox_len > 0 || self.online == Some(false) {
                    self.request_sync();
                }
            }
            FromSync::Disconnected => {}
            FromSync::Done { acked, rejected, pulled, error } => {
                self.syncing = false;
                if let Err(e) = self.apply_sync(&acked, &rejected, pulled) {
                    self.message = Some(format!("同期結果の保存に失敗: {e:#}"));
                }
                self.online = Some(error.is_none());
                if !rejected.is_empty() {
                    self.message = Some(format!("{} 件の変更がサーバーに拒否されました", rejected.len()));
                    // 拒否されたものはローカルとサーバーがずれているので全件取り直す
                    let _ = self.store.set_rev(0);
                    self.dirty = true;
                }
                if self.dirty {
                    self.request_sync();
                }
            }
        }
    }

    fn apply_sync(
        &mut self,
        acked: &[i64],
        rejected: &[i64],
        pulled: Option<wanna_core::SyncResponse>,
    ) -> Result<()> {
        for seq in acked.iter().chain(rejected) {
            self.store.remove_op(*seq)?;
        }
        self.outbox_len = self.store.outbox_len()?;
        let Some(resp) = pulled else { return Ok(()) };
        // まだ送っていない変更があるものは、ローカルの状態を優先する（送った後の同期で揃う）
        let pending = self.store.pending_ids()?;
        let selected = self.selected().map(|w| w.id.clone());
        for w in resp.wants {
            if pending.contains(&w.id) {
                continue;
            }
            self.store.put(&w)?;
            match self.wants.iter_mut().find(|x| x.id == w.id) {
                Some(x) => *x = w,
                None => self.wants.push(w),
            }
        }
        self.store.set_rev(resp.rev)?;
        self.clamp();
        if let (Screen::List, Some(id)) = (self.screen, selected) {
            if self.wants.iter().any(|w| w.id == id && w.is_active()) {
                self.focus(&id);
            }
        }
        Ok(())
    }

    // ───────── 編集 ─────────

    /// 外部エディタから戻ったとき
    pub fn on_editor(&mut self, req: EditorReq, res: Result<String>) {
        self.begin_change();
        self.apply_editor(req, res);
        self.end_change();
    }

    fn apply_editor(&mut self, req: EditorReq, res: Result<String>) {
        let memo = self.wants.iter().any(|w| w.id == req.id && w.kind == Kind::Memo);
        let text = match res {
            // テンプレートのまま閉じたら何も書かなかったことにする (メモにはテンプレートが無い)
            Ok(t) if !memo && notes::is_blank(&t) => String::new(),
            Ok(t) => t,
            Err(e) => {
                self.message = Some(format!("メモを編集できませんでした: {e:#}"));
                return;
            }
        };
        if req.from_popup {
            if let Mode::Edit(e) = &mut self.mode {
                if e.id == req.id {
                    e.notes = text.trim_end().to_string();
                }
            }
            return;
        }
        let Some(w) = self.wants.iter().find(|w| w.id == req.id) else { return };
        let (title, axis_hi, clau, due) = (w.title.clone(), w.axis_hi, w.clau, w.due_at.clone());
        self.apply_edit(&req.id, &title, &text, axis_hi, clau, due);
    }

    /// 選択中のもののメモを外部エディタで開く
    fn edit_notes(&mut self) {
        if let Some(w) = self.selected() {
            self.editor = Some(EditorReq { id: w.id.clone(), text: editor_text(w.kind, &w.notes), from_popup: false });
        }
    }

    /// 編集ポップアップの内容を保存する。日時が読めなければ理由を返す
    fn save_edit(&mut self, e: &Edit) -> Result<(), String> {
        let due = if e.kind.has_due() {
            due::parse_input(&e.due.text, chrono::Local::now().naive_local())?
        } else {
            None
        };
        self.apply_edit(&e.id, &e.title.text, &e.notes, e.axis_hi, e.clau, due);
        Ok(())
    }

    fn apply_edit(
        &mut self,
        id: &str,
        title: &str,
        notes: &str,
        axis_hi: bool,
        clau: bool,
        due: Option<String>,
    ) {
        let Some(w) = self.wants.iter().find(|w| w.id == id) else { return };
        let title = title.trim();
        let notes = notes.trim_end();
        let q = Quadrant { axis_hi, clau };
        let cur = (w.kind, w.title.clone(), w.notes.clone(), w.axis_hi, w.clau, w.due_at.clone());
        // 区分が変わるなら移動先の末尾に置く
        let pos = (!q.holds(w)).then(|| self.tail_pos(cur.0, q));
        let patch = WantPatch {
            title: (!title.is_empty() && title != cur.1).then(|| title.to_string()),
            notes: (notes != cur.2).then(|| notes.to_string()),
            axis_hi: (axis_hi != cur.3).then_some(axis_hi),
            clau: (clau != cur.4).then_some(clau),
            pos,
            due_at: (due != cur.5).then_some(due),
            ..Default::default()
        };
        // 日時を変えれば Must の中で並びも変わるので、カーソルはついていく
        if patch != WantPatch::default() {
            self.patch(id, patch);
            self.focus(id);
        }
    }

    fn open_edit(&mut self, field: EditField) {
        let Some(w) = self.selected() else { return };
        let mut e = Edit::new(w);
        if e.fields().contains(&field) {
            e.field = field;
        } else {
            self.message = Some(format!("日時を持てるのは「{}」だけです", Kind::Task.label()));
        }
        self.mode = Mode::Edit(e);
    }
}

/// `query` が `text` に現れる範囲 (バイト位置)。
/// ~/.vimrc の `ignorecase` `smartcase` と同じく、大文字を含まなければ大文字小文字を区別しない
pub fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = !query.chars().any(char::is_uppercase);
    let norm = |c: char| if fold { c.to_lowercase().next().unwrap_or(c) } else { c };
    let q: Vec<char> = query.chars().map(norm).collect();
    let t: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (i, norm(c))).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= t.len() {
        if t[i..i + q.len()].iter().map(|x| x.1).eq(q.iter().copied()) {
            let end = t.get(i + q.len()).map(|x| x.0).unwrap_or(text.len());
            out.push(t[i].0..end);
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

/// エディタで開く中身。リストのものでメモが空ならテンプレートを入れておく (メモには入れない)
fn editor_text(kind: Kind, notes: &str) -> String {
    if notes.is_empty() && kind != Kind::Memo {
        notes::template()
    } else {
        notes.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smartcase() {
        assert_eq!(matches("Buy milk", "buy"), vec![0..3]);
        assert!(matches("Buy milk", "bUy").is_empty());
        assert_eq!(matches("牛乳と牛肉", "牛"), vec![0..3, 9..12]);
        assert!(matches("abc", "").is_empty());
    }
}
