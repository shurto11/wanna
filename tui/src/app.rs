use crate::store::{Op, Store};
use crate::sync::{FromSync, ToWorker};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use wanna_core::{now_rfc3339, pos, sort_by_pos, Quadrant, Want, WantPatch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Wants,
    Done,
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
    Energy,
    Clau,
}

pub struct Edit {
    pub id: String,
    pub title: TextInput,
    pub energy: bool,
    pub clau: bool,
    pub notes: String,
    pub field: EditField,
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
    pub mode: Mode,
    /// カーソルのある区分 (`Quadrant::ALL` の添字)
    pub cur: usize,
    pub lists: [ListState; 4],
    pub done_list: ListState,
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
            screen: Screen::Wants,
            mode: Mode::Normal,
            cur: 0,
            lists: Default::default(),
            done_list: ListState::default(),
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
        for i in 0..4 {
            app.lists[i].select(Some(0));
        }
        app.done_list.select(Some(0));
        app.request_sync();
        Ok(app)
    }

    pub fn has_remote(&self) -> bool {
        self.worker.is_some()
    }

    // ───────── 表示用のリスト ─────────

    /// 区分内のやりたいこと。`(pos, id)` 順
    pub fn list(&self, q: Quadrant) -> Vec<&Want> {
        let mut v: Vec<&Want> =
            self.wants.iter().filter(|w| w.is_active() && w.quadrant() == q).collect();
        sort_by_pos(&mut v);
        v
    }

    /// やったこと。達成日の降順
    pub fn done(&self) -> Vec<&Want> {
        let mut v: Vec<&Want> =
            self.wants.iter().filter(|w| !w.deleted && w.done_at.is_some()).collect();
        v.sort_by(|a, b| b.done_at.cmp(&a.done_at).then_with(|| a.id.cmp(&b.id)));
        v
    }

    pub fn cur_q(&self) -> Quadrant {
        Quadrant::ALL[self.cur]
    }

    fn sel(&self) -> usize {
        self.lists[self.cur].selected().unwrap_or(0)
    }

    pub fn selected(&self) -> Option<&Want> {
        match self.screen {
            Screen::Wants => self.list(self.cur_q()).get(self.sel()).copied(),
            Screen::Done => self.done().get(self.done_list.selected().unwrap_or(0)).copied(),
        }
    }

    /// 選択位置をリストの範囲に収める
    fn clamp(&mut self) {
        for i in 0..4 {
            let len = self.list(Quadrant::ALL[i]).len();
            let s = self.lists[i].selected().unwrap_or(0);
            self.lists[i].select(Some(s.min(len.saturating_sub(1))));
        }
        let len = self.done().len();
        let s = self.done_list.selected().unwrap_or(0);
        self.done_list.select(Some(s.min(len.saturating_sub(1))));
    }

    /// `id` のある区分・行へカーソルを合わせる
    fn focus(&mut self, id: &str) {
        for (qi, q) in Quadrant::ALL.iter().enumerate() {
            if let Some(i) = self.list(*q).iter().position(|w| w.id == id) {
                self.cur = qi;
                self.lists[qi].select(Some(i));
                return;
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

    /// 区分の末尾に付ける pos
    fn tail_pos(&self, q: Quadrant) -> String {
        let list = self.list(q);
        pos::between(list.last().map(|w| w.pos.as_str()), None)
    }

    /// 区分内で `id` を `to` 番目に置く。pos が重複していて挟めなければ区分全体を振り直す
    fn place(&mut self, id: &str, to: usize) {
        let q = self.cur_q();
        let others: Vec<(String, String)> = self
            .list(q)
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
        if let (Screen::Wants, Some(id)) = (self.screen, selected) {
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
                    Screen::Wants => self.key_wants(key),
                    Screen::Done => self.key_done(key),
                }
                return;
            }
            Mode::Add(mut input) => match key.code {
                KeyCode::Esc => Mode::Normal,
                KeyCode::Enter if !input.text.trim().is_empty() => {
                    let q = self.cur_q();
                    let w = Want::new(input.text.trim(), q.energy, q.clau, self.tail_pos(q));
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
                // 名前欄では文字キーは入力にまわすので、欄の移動と切り替えは他の欄だけ
                let choosing = e.field != EditField::Title;
                match key.code {
                    KeyCode::Esc => Mode::Normal,
                    KeyCode::Enter => {
                        self.save_edit(&e.id, &e.title.text, &e.notes, e.energy, e.clau);
                        Mode::Normal
                    }
                    KeyCode::Char('s') if ctrl => {
                        self.save_edit(&e.id, &e.title.text, &e.notes, e.energy, e.clau);
                        Mode::Normal
                    }
                    KeyCode::Tab => {
                        self.editor = Some(EditorReq {
                            id: e.id.clone(),
                            text: e.notes.clone(),
                            from_popup: true,
                        });
                        Mode::Edit(e)
                    }

                    // 欄の移動
                    KeyCode::Down | KeyCode::Char('j') if choosing || key.code == KeyCode::Down => {
                        e.field = match e.field {
                            EditField::Title => EditField::Energy,
                            _ => EditField::Clau,
                        };
                        Mode::Edit(e)
                    }
                    KeyCode::Up | KeyCode::Char('k') if choosing || key.code == KeyCode::Up => {
                        e.field = match e.field {
                            EditField::Clau => EditField::Energy,
                            _ => EditField::Title,
                        };
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
                            EditField::Energy => e.energy = v.unwrap_or(!e.energy),
                            _ => e.clau = v.unwrap_or(!e.clau),
                        }
                        Mode::Edit(e)
                    }

                    _ => {
                        if !choosing {
                            e.title.handle(key);
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
        let (title, energy, clau) = (w.title.clone(), w.energy, w.clau);
        self.save_edit(&req.id, &title, &text, energy, clau);
    }

    /// 選択中のもののメモを外部エディタで開く
    fn edit_notes(&mut self) {
        if let Some(w) = self.selected() {
            self.editor = Some(EditorReq { id: w.id.clone(), text: w.notes.clone(), from_popup: false });
        }
    }

    fn save_edit(&mut self, id: &str, title: &str, notes: &str, energy: bool, clau: bool) {
        let Some(w) = self.wants.iter().find(|w| w.id == id) else { return };
        let title = title.trim();
        let notes = notes.trim_end();
        let q = Quadrant { energy, clau };
        // 区分が変わるなら移動先の末尾に置く
        let pos = (w.quadrant() != q).then(|| self.tail_pos(q));
        let Some(w) = self.wants.iter().find(|w| w.id == id) else { return };
        let patch = WantPatch {
            title: (!title.is_empty() && title != w.title).then(|| title.to_string()),
            notes: (notes != w.notes).then(|| notes.to_string()),
            energy: (energy != w.energy).then_some(energy),
            clau: (clau != w.clau).then_some(clau),
            pos,
            ..Default::default()
        };
        if patch != WantPatch::default() {
            self.patch(id, patch);
            self.focus(id);
        }
    }

    fn key_wants(&mut self, key: KeyEvent) {
        let len = self.list(self.cur_q()).len();
        let sel = self.sel();
        let top = self.cur < 2;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('a') => self.screen = Screen::Done,
            KeyCode::Char('n') => self.mode = Mode::Add(TextInput::default()),
            KeyCode::Char('r') => {
                self.message = Some("同期中…".into());
                self.request_sync();
            }

            // カーソル移動。j/k は区分内、端まで来たら上下の区分へ抜ける
            KeyCode::Char('j') | KeyCode::Down => {
                if sel + 1 < len {
                    self.lists[self.cur].select(Some(sel + 1));
                } else if top {
                    self.cur += 2;
                    self.lists[self.cur].select(Some(0));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if sel > 0 {
                    self.lists[self.cur].select(Some(sel - 1));
                } else if !top {
                    self.cur -= 2;
                    let len = self.list(self.cur_q()).len();
                    self.lists[self.cur].select(Some(len.saturating_sub(1)));
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.cur &= !1,
            KeyCode::Char('l') | KeyCode::Right => self.cur |= 1,
            KeyCode::Char('g') | KeyCode::Home => self.lists[self.cur].select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => {
                self.lists[self.cur].select(Some(len.saturating_sub(1)))
            }
            KeyCode::Char(c @ '0'..='9') => {
                let i = if c == '0' { 9 } else { c as usize - '1' as usize };
                if i < len {
                    self.lists[self.cur].select(Some(i));
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

            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(w) = self.selected() {
                    self.mode = Mode::Edit(Edit {
                        id: w.id.clone(),
                        title: TextInput::new(&w.title),
                        energy: w.energy,
                        clau: w.clau,
                        notes: w.notes.clone(),
                        field: EditField::Title,
                    });
                }
            }
            KeyCode::Char('m') => self.edit_notes(),
            KeyCode::Char('t') => {
                if let Some(w) = self.selected() {
                    let (id, title) = (w.id.clone(), w.title.clone());
                    self.patch(&id, WantPatch { done_at: Some(Some(now_rfc3339())), ..Default::default() });
                    self.message = Some(format!("やった！ {title}"));
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

    fn key_done(&mut self, key: KeyEvent) {
        let len = self.done().len();
        let sel = self.done_list.selected().unwrap_or(0);
        match key.code {
            KeyCode::Char('a') | KeyCode::Esc => self.screen = Screen::Wants,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down if sel + 1 < len => {
                self.done_list.select(Some(sel + 1))
            }
            KeyCode::Char('k') | KeyCode::Up if sel > 0 => self.done_list.select(Some(sel - 1)),
            KeyCode::Char('g') | KeyCode::Home => self.done_list.select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => {
                self.done_list.select(Some(len.saturating_sub(1)))
            }
            KeyCode::Char('u') => {
                if let Some(w) = self.selected() {
                    let (id, title, q) = (w.id.clone(), w.title.clone(), w.quadrant());
                    let patch = WantPatch {
                        done_at: Some(None),
                        pos: Some(self.tail_pos(q)),
                        ..Default::default()
                    };
                    self.patch(&id, patch);
                    self.message = Some(format!("やりたいことに戻しました: {title}"));
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
