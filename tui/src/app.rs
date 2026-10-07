use crate::store::{Op, Store};
use crate::sync::{FromSync, ToWorker};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use wanna_core::{due, notes, now_rfc3339, pos, sort_by_pos, Kind, Quadrant, Want, WantPatch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// やりたいこと / 次にやること (どちらかは `App::kind`)
    List,
    Done,
    /// やってはいないが、リストに置くほどでもなくなったもの
    Archive,
}

/// 1行のテキスト入力
#[derive(Default, Clone)]
pub struct TextInput {
    pub text: String,
    /// 文字単位のカーソル位置
    pub cursor: usize,
}

impl TextInput {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.chars().count() }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(self.text.len())
    }

    pub fn before_cursor(&self) -> &str {
        &self.text[..self.byte_at(self.cursor)]
    }

    pub fn insert(&mut self, c: char) {
        let i = self.byte_at(self.cursor);
        self.text.insert(i, c);
        self.cursor += 1;
    }

    /// 編集キーなら処理して true
    fn handle(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.text.chars().count(),
            KeyCode::Char('u') if ctrl => {
                let i = self.byte_at(self.cursor);
                self.text.drain(..i);
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => self.insert(c),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let i = self.byte_at(self.cursor);
                self.text.remove(i);
            }
            KeyCode::Delete if self.cursor < self.text.chars().count() => {
                let i = self.byte_at(self.cursor);
                self.text.remove(i);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            _ => return false,
        }
        true
    }
}

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
    /// 日時が読めなかったときの説明
    pub error: Option<String>,
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
        }
    }

    /// この1件で回れる欄。日時はリストによって出ない
    pub fn fields(&self) -> &'static [EditField] {
        use EditField::*;
        if self.kind.has_due() {
            &[Title, Axis, Clau, Due]
        } else {
            &[Title, Axis, Clau]
        }
    }

    fn move_field(&mut self, down: bool) {
        let fields = self.fields();
        let i = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = if down { (i + 1).min(fields.len() - 1) } else { i.saturating_sub(1) };
        self.field = fields[next];
    }

    /// 文字キーを入力に回す欄か (高低を選ぶ欄では h/l などがキー操作になる)
    fn typing(&self) -> bool {
        matches!(self.field, EditField::Title | EditField::Due)
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
    Add(TextInput),
    Edit(Edit),
    ConfirmDelete { id: String, title: String },
}

pub struct App {
    store: Store,
    pub wants: Vec<Want>,
    pub screen: Screen,
    /// 表示しているリスト。やったこと画面から戻る先でもある
    pub kind: Kind,
    pub mode: Mode,
    /// カーソルのある区分 (`Quadrant::ALL` の添字)。リストをまたいで持ち越す
    pub cur: usize,
    /// リストごと・区分ごとの選択行
    pub lists: [[ListState; 4]; 2],
    pub done_list: ListState,
    pub archive_list: ListState,
    pub message: Option<String>,
    pub quit: bool,
    pub editor: Option<EditorReq>,

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
            message: None,
            quit: false,
            editor: None,
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
        app.request_sync();
        Ok(app)
    }

    pub fn has_remote(&self) -> bool {
        self.worker.is_some()
    }

    // ───────── 表示用のリスト ─────────

    /// リスト・区分の中身。`(pos, id)` 順
    pub fn list(&self, kind: Kind, q: Quadrant) -> Vec<&Want> {
        let mut v: Vec<&Want> = self
            .wants
            .iter()
            .filter(|w| w.is_active() && w.kind == kind && w.quadrant() == q)
            .collect();
        sort_by_pos(&mut v);
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

    /// やったこと / 保管庫 の中身と選択行。リストの画面では None
    fn past(&self) -> Option<(Vec<&Want>, usize)> {
        match self.screen {
            Screen::List => None,
            Screen::Done => Some((self.done(), self.done_list.selected().unwrap_or(0))),
            Screen::Archive => Some((self.archived(), self.archive_list.selected().unwrap_or(0))),
        }
    }

    fn past_list_mut(&mut self) -> &mut ListState {
        match self.screen {
            Screen::Archive => &mut self.archive_list,
            _ => &mut self.done_list,
        }
    }

    pub fn cur_q(&self) -> Quadrant {
        Quadrant::ALL[self.cur]
    }

    fn cur_list(&self) -> Vec<&Want> {
        self.list(self.kind, self.cur_q())
    }

    fn sel(&self) -> usize {
        self.lists[self.kind.index()][self.cur].selected().unwrap_or(0)
    }

    fn select(&mut self, i: usize) {
        let (k, c) = (self.kind.index(), self.cur);
        self.lists[k][c].select(Some(i));
    }

    pub fn selected(&self) -> Option<&Want> {
        match self.past() {
            None => self.cur_list().get(self.sel()).copied(),
            Some((list, sel)) => list.get(sel).copied(),
        }
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
    }

    /// `id` のあるリスト・区分・行へカーソルを合わせる
    fn focus(&mut self, id: &str) {
        for kind in Kind::ALL {
            for (qi, q) in Quadrant::ALL.iter().enumerate() {
                if let Some(i) = self.list(kind, *q).iter().position(|w| w.id == id) {
                    self.kind = kind;
                    self.cur = qi;
                    self.lists[kind.index()][qi].select(Some(i));
                    return;
                }
            }
        }
    }

    // ───────── 変更 ─────────

    /// ローカルに適用して outbox に積み、同期を促す
    fn commit(&mut self, op: Op) {
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

    /// リスト・区分の末尾に付ける pos
    fn tail_pos(&self, kind: Kind, q: Quadrant) -> String {
        let list = self.list(kind, q);
        pos::between(list.last().map(|w| w.pos.as_str()), None)
    }

    /// 区分内で `id` を `to` 番目に置く。pos が重複していて挟めなければ区分全体を振り直す
    fn place(&mut self, id: &str, to: usize) {
        let (kind, q) = (self.kind, self.cur_q());
        let others: Vec<(String, String)> = self
            .list(kind, q)
            .into_iter()
            .filter(|w| w.id != id)
            .map(|w| (w.id.clone(), w.pos.clone()))
            .collect();
        let to = to.min(others.len());
        let a = to.checked_sub(1).map(|i| others[i].1.as_str());
        let b = others.get(to).map(|o| o.1.as_str());
        if a.zip(b).is_none_or(|(a, b)| a < b) {
            let p = pos::between(a, b);
            self.patch(id, WantPatch { pos: Some(p), ..Default::default() });
        } else {
            let mut order: Vec<String> = others.into_iter().map(|o| o.0).collect();
            order.insert(to, id.to_string());
            let keys = pos::n_between(None, None, order.len());
            for (id, p) in order.iter().zip(keys) {
                self.patch(id, WantPatch { pos: Some(p), ..Default::default() });
            }
        }
        self.focus(id);
    }

    /// 選択中のものをもう一方のリストへ移す。行き先の同じ区分の末尾に入る
    fn move_to_other_list(&mut self) {
        let Some(w) = self.selected() else { return };
        let (id, title, q, to) = (w.id.clone(), w.title.clone(), w.quadrant(), w.kind.other());
        let pos = self.tail_pos(to, q);
        self.patch(
            &id,
            WantPatch {
                kind: Some(to),
                pos: Some(pos),
                // やりたいことは日時を持たない
                due_at: (!to.has_due()).then_some(None),
                ..Default::default()
            },
        );
        self.focus(&id);
        self.message = Some(format!("{}へ移しました: {title}", to.label()));
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

    // ───────── キー入力 ─────────

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = match mode {
            Mode::Normal => {
                self.message = None;
                match self.screen {
                    Screen::List => self.key_list(key),
                    Screen::Done | Screen::Archive => self.key_past(key),
                }
                return;
            }
            Mode::Add(mut input) => match key.code {
                KeyCode::Esc => Mode::Normal,
                KeyCode::Enter if !input.text.trim().is_empty() => {
                    let (kind, q) = (self.kind, self.cur_q());
                    let pos = self.tail_pos(kind, q);
                    let w = Want::new(input.text.trim(), kind, q.axis_hi, q.clau, pos);
                    let id = w.id.clone();
                    self.commit(Op::Create { want: w });
                    self.focus(&id);
                    Mode::Normal
                }
                _ => {
                    input.handle(key);
                    Mode::Add(input)
                }
            },
            Mode::Edit(mut e) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let save = key.code == KeyCode::Enter || (ctrl && key.code == KeyCode::Char('s'));
                // 高低を選ぶ欄では文字キーをキー操作として使う
                let choosing = !e.typing();
                let arrow = matches!(key.code, KeyCode::Up | KeyCode::Down);
                if save {
                    self.mode = match self.save_edit(&e) {
                        Ok(()) => Mode::Normal,
                        Err(msg) => {
                            e.error = Some(msg);
                            e.field = EditField::Due;
                            Mode::Edit(e)
                        }
                    };
                    return;
                }
                match key.code {
                    KeyCode::Esc => Mode::Normal,
                    KeyCode::Tab => {
                        self.editor = Some(EditorReq {
                            id: e.id.clone(),
                            text: draft(&e.notes),
                            from_popup: true,
                        });
                        Mode::Edit(e)
                    }

                    // 欄の移動
                    KeyCode::Down | KeyCode::Char('j') if choosing || arrow => {
                        e.move_field(true);
                        Mode::Edit(e)
                    }
                    KeyCode::Up | KeyCode::Char('k') if choosing || arrow => {
                        e.move_field(false);
                        Mode::Edit(e)
                    }

                    // 高低の切り替え (左が高。空白は反転)
                    KeyCode::Char('h' | 'l' | ' ') | KeyCode::Left | KeyCode::Right
                        if choosing =>
                    {
                        let v = match key.code {
                            KeyCode::Char(' ') => None,
                            KeyCode::Char('h') | KeyCode::Left => Some(true),
                            _ => Some(false),
                        };
                        match e.field {
                            EditField::Clau => e.clau = v.unwrap_or(!e.clau),
                            _ => e.axis_hi = v.unwrap_or(!e.axis_hi),
                        }
                        Mode::Edit(e)
                    }

                    _ => {
                        if let Some(input) = e.input_mut() {
                            if input.handle(key) {
                                e.error = None;
                            }
                        }
                        Mode::Edit(e)
                    }
                }
            }
            Mode::ConfirmDelete { id, title } => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.commit(Op::Delete { id });
                    self.message = Some(format!("削除しました: {title}"));
                    Mode::Normal
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => Mode::Normal,
                _ => Mode::ConfirmDelete { id, title },
            },
        };
    }

    /// 外部エディタから戻ったとき
    pub fn on_editor(&mut self, req: EditorReq, res: Result<String>) {
        let text = match res {
            // テンプレートのまま閉じたら何も書かなかったことにする
            Ok(t) if notes::is_blank(&t) => String::new(),
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
            self.editor = Some(EditorReq { id: w.id.clone(), text: draft(&w.notes), from_popup: false });
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
        let pos = (w.quadrant() != q).then(|| self.tail_pos(cur.0, q));
        let patch = WantPatch {
            title: (!title.is_empty() && title != cur.1).then(|| title.to_string()),
            notes: (notes != cur.2).then(|| notes.to_string()),
            axis_hi: (axis_hi != cur.3).then_some(axis_hi),
            clau: (clau != cur.4).then_some(clau),
            pos,
            due_at: (due != cur.5).then_some(due),
            ..Default::default()
        };
        let reorder = patch.due_at.as_ref().is_some_and(|d| d.is_some());
        if patch != WantPatch::default() {
            self.patch(id, patch);
            self.focus(id);
        }
        if reorder {
            self.place_by_due(id);
        }
    }

    /// 日時の順になる位置へ置く。自分より後の日時のものの直前、なければ日時を持つものの最後の後ろ。
    /// 日時を持たないものの位置は気にしない
    fn place_by_due(&mut self, id: &str) {
        let at = |w: &Want| w.due().and_then(due::at);
        let Some(t) = self.wants.iter().find(|w| w.id == id).and_then(at) else { return };
        self.focus(id);
        let others: Vec<&Want> = self.cur_list().into_iter().filter(|w| w.id != id).collect();
        let to = others
            .iter()
            .position(|w| at(w).is_some_and(|x| x > t))
            .or_else(|| others.iter().rposition(|w| at(w).is_some()).map(|i| i + 1));
        let cur = self.sel();
        if let Some(to) = to.filter(|&to| to != cur) {
            self.place(id, to);
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

    /// 表示を Want → Must → Done → Archive の順に回す
    fn cycle_view(&mut self, forward: bool) {
        let cur = match self.screen {
            Screen::List => self.kind.index(),
            Screen::Done => 2,
            Screen::Archive => 3,
        };
        match (cur + if forward { 1 } else { 3 }) % 4 {
            0 => {
                self.screen = Screen::List;
                self.kind = Kind::Want;
            }
            1 => {
                self.screen = Screen::List;
                self.kind = Kind::Task;
            }
            2 => self.screen = Screen::Done,
            _ => self.screen = Screen::Archive,
        }
    }

    fn key_list(&mut self, key: KeyEvent) {
        let len = self.cur_list().len();
        let sel = self.sel();
        let top = self.cur < 2;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            // 画面の切り替え
            KeyCode::Char(']') | KeyCode::Tab => self.cycle_view(true),
            KeyCode::Char('[') | KeyCode::BackTab => self.cycle_view(false),
            KeyCode::Char('n') => self.mode = Mode::Add(TextInput::default()),
            KeyCode::Char('r') => {
                self.message = Some("同期中…".into());
                self.request_sync();
            }

            // カーソル移動。j/k は区分内、端まで来たら上下の区分へ抜ける
            KeyCode::Char('j') | KeyCode::Down => {
                if sel + 1 < len {
                    self.select(sel + 1);
                } else if top {
                    self.cur += 2;
                    self.select(0);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if sel > 0 {
                    self.select(sel - 1);
                } else if !top {
                    self.cur -= 2;
                    let len = self.cur_list().len();
                    self.select(len.saturating_sub(1));
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.cur &= !1,
            KeyCode::Char('l') | KeyCode::Right => self.cur |= 1,
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(len.saturating_sub(1)),
            KeyCode::Char(c @ '0'..='9') => {
                let i = if c == '0' { 9 } else { c as usize - '1' as usize };
                if i < len {
                    self.select(i);
                }
            }

            // 並べ替え
            KeyCode::Char('K') if sel > 0 => {
                if let Some(id) = self.selected().map(|w| w.id.clone()) {
                    self.place(&id, sel - 1);
                }
            }
            KeyCode::Char('J') if sel + 1 < len => {
                if let Some(id) = self.selected().map(|w| w.id.clone()) {
                    self.place(&id, sel + 1);
                }
            }
            KeyCode::Char('X') => self.move_to_other_list(),

            KeyCode::Char('e') | KeyCode::Enter => self.open_edit(EditField::Title),
            KeyCode::Char('s') => self.open_edit(EditField::Due),
            KeyCode::Char('m') => self.edit_notes(),
            KeyCode::Char('t') => {
                if let Some(w) = self.selected() {
                    let (id, title) = (w.id.clone(), w.title.clone());
                    self.patch(&id, WantPatch { done_at: Some(Some(now_rfc3339())), ..Default::default() });
                    self.message = Some(format!("やった！ {title}"));
                }
            }
            KeyCode::Char('a') => {
                if let Some(w) = self.selected() {
                    let (id, title) = (w.id.clone(), w.title.clone());
                    self.patch(&id, WantPatch { archived_at: Some(Some(now_rfc3339())), ..Default::default() });
                    self.message = Some(format!("保管庫にしまいました: {title}"));
                }
            }
            KeyCode::Char('d') => {
                if let Some(w) = self.selected() {
                    self.mode = Mode::ConfirmDelete { id: w.id.clone(), title: w.title.clone() };
                }
            }
            _ => {}
        }
    }

    /// やったこと / 保管庫 の画面
    fn key_past(&mut self, key: KeyEvent) {
        let Some((list, sel)) = self.past() else { return };
        let len = list.len();
        let done = self.screen == Screen::Done;
        match key.code {
            KeyCode::Char(']') | KeyCode::Tab => self.cycle_view(true),
            KeyCode::Char('[') | KeyCode::BackTab => self.cycle_view(false),
            // 直前に見ていたリストへ戻る
            KeyCode::Esc => self.screen = Screen::List,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down if sel + 1 < len => {
                self.past_list_mut().select(Some(sel + 1))
            }
            KeyCode::Char('k') | KeyCode::Up if sel > 0 => self.past_list_mut().select(Some(sel - 1)),
            KeyCode::Char('g') | KeyCode::Home => self.past_list_mut().select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => {
                self.past_list_mut().select(Some(len.saturating_sub(1)))
            }
            // リストに戻す
            KeyCode::Char('u') => {
                if let Some(w) = self.selected() {
                    let (id, title, kind, q) =
                        (w.id.clone(), w.title.clone(), w.kind, w.quadrant());
                    let patch = WantPatch {
                        done_at: Some(None),
                        archived_at: Some(None),
                        pos: Some(self.tail_pos(kind, q)),
                        ..Default::default()
                    };
                    self.patch(&id, patch);
                    self.message = Some(format!("{}に戻しました: {title}", kind.label()));
                }
            }
            // しまっていたものを結局やった
            KeyCode::Char('t') if !done => {
                if let Some(w) = self.selected() {
                    let (id, title) = (w.id.clone(), w.title.clone());
                    let patch = WantPatch {
                        done_at: Some(Some(now_rfc3339())),
                        archived_at: Some(None),
                        ..Default::default()
                    };
                    self.patch(&id, patch);
                    self.message = Some(format!("やった！ {title}"));
                }
            }
            KeyCode::Char('m') => self.edit_notes(),
            KeyCode::Char('d') => {
                if let Some(w) = self.selected() {
                    self.mode = Mode::ConfirmDelete { id: w.id.clone(), title: w.title.clone() };
                }
            }
            _ => {}
        }
    }

}

/// エディタで開く中身。空ならテンプレートを入れておく
fn draft(notes: &str) -> String {
    if notes.is_empty() {
        notes::template()
    } else {
        notes.to_string()
    }
}
