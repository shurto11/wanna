//! キー入力。vim に寄せる (カウント・`gg` `dd` などの前置キー・ビジュアルモード・`/` `:`)

use super::{editor_text, App, Edit, EditField, EditorReq, Mode, Screen, TABS};
use crate::input::{Insert, Normal, TextInput};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// ノーマル・ビジュアルモードで打ちかけのキー (vim の showcmd に出すもの)
#[derive(Default)]
pub struct Pending {
    /// 数字のカウント (`3j` の 3)
    pub count: Option<usize>,
    /// 前置キー (`gg` `gt` `dd` の1文字め)
    pub prefix: Option<char>,
}

impl Pending {
    pub fn show(&self) -> String {
        let count = self.count.map(|n| n.to_string()).unwrap_or_default();
        format!("{count}{}", self.prefix.map(String::from).unwrap_or_default())
    }
}

/// `?` で出すキー一覧 (見出し, [(キー, 動作)])
pub const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "どの画面でも",
        &[
            ("gt / gT", "次 / 前のタブ (3gt で3番目のタブ)。[ ] / Tab でも"),
            ("j / k", "下 / 上 (3j で3行)"),
            ("gg / G", "先頭 / 末尾 (5G で5番目)"),
            ("Ctrl+d / Ctrl+u", "半ページ下 / 上"),
            ("/  n / N", "名前で検索  次 / 前の一致"),
            ("u / Ctrl+r", "取り消す / やり直す"),
            ("v", "ビジュアルモード (範囲を選んでまとめて操作)"),
            ("dd", "削除 (3dd で3件)。p で貼れる"),
            ("m", "メモをエディタで開く"),
            ("r", "今すぐ同期"),
            (":", "コマンド (:q 終了  :w 同期  :5 5番目へ  :noh 検索の色を消す)"),
            ("q / Ctrl+c", "終了"),
        ],
    ),
    (
        "Want / Must",
        &[
            ("h / l", "左 / 右の区分へ。Must は学校 / その他の段へ"),
            ("o / O", "カーソルの下 / 上に追加"),
            ("c / Enter", "編集 (名前・区分・日時)"),
            ("s", "日時を編集 (Must のみ)"),
            ("t / a", "やった / しまう"),
            ("p / P", "dd で消したものをカーソルの下 / 上に貼る"),
            ("J / K", "下 / 上へ動かす。Want は区分の端で上下の区分へ、Must は段の中だけ"),
            ("H / L", "左 / 右の区分へ動かす。Must は学校 / その他へ (日時の位置に入る)"),
            ("X", "もう一方のリストへ移す (Must へはその他に入る)"),
        ],
    ),
    (
        "Done / Archive",
        &[
            ("p", "元のリストに戻す"),
            ("t", "やった (Archive のみ)"),
            ("Esc", "直前のリストへ戻る"),
        ],
    ),
    (
        "Memo",
        &[
            ("o", "名前を入れて作り、本文をエディタで開く"),
            ("Enter", "本文をエディタで開く"),
            ("c", "名前を変える"),
            ("p", "dd で消したメモを戻す"),
        ],
    ),
    (
        "ビジュアルモード",
        &[
            ("j / k / gg / G", "範囲を広げる・狭める"),
            ("o", "範囲の反対の端へ"),
            ("d", "削除"),
            ("t / a", "やった / しまう"),
            ("J / K", "まとめて下 / 上へ動かす"),
            ("H / L / X", "左右の区分 (Must は学校 / その他) / もう一方のリストへ"),
            ("p", "元のリストに戻す (Done / Archive)"),
            ("Esc / v", "やめる"),
        ],
    ),
    (
        "追加・編集の入力",
        &[
            ("Esc / jj", "追加なら確定。編集ならノーマルモードへ"),
            ("Ctrl+w / Ctrl+u", "1語 / 行頭まで消す"),
            ("Enter / Ctrl+s", "保存"),
            ("Ctrl+c", "やめる"),
        ],
    ),
    (
        "編集のノーマルモード",
        &[
            ("j / k", "欄の移動"),
            ("h / l / Space", "左 / 右 / 反転 (2択の欄)"),
            ("i a I A", "挿入モードへ"),
            ("h l w b e 0 $", "カーソル移動 (名前・日時の欄)"),
            ("x D dd dw cc cw C S", "消す / 書き換える"),
            ("Tab", "メモをエディタで編集"),
            ("Enter / :w / ZZ", "保存"),
            ("Esc / q / :q / ZQ", "やめる"),
        ],
    ),
];

/// キー一覧の行数 (見出しと空行を含む)
pub fn help_lines() -> usize {
    HELP.iter().map(|(_, keys)| keys.len() + 2).sum()
}

impl App {
    pub fn on_key(&mut self, key: KeyEvent) {
        self.begin_change();
        self.handle_key(key);
        self.end_change();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        // Ctrl+C はノーマルモードなら終了、それ以外ならそのモードをやめる
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if matches!(mode, Mode::Normal) {
                self.quit = true;
            }
            self.pending = Pending::default();
            return;
        }
        self.mode = match mode {
            Mode::Normal => {
                self.key_normal(key);
                return;
            }
            Mode::Visual { anchor } => self.key_visual(key, anchor),
            Mode::Add { input, at } => self.key_add(key, input, at),
            Mode::Edit(e) => self.key_edit(key, e),
            Mode::Search(input) => self.key_search(key, input),
            Mode::Command(input) => self.key_command(key, input),
            Mode::Help(scroll) => key_help(key, scroll),
        };
    }

    /// カウントと前置キーを読む。キーを使い切ったら None、まだなら打ちかけていた分を返して消す
    fn read_pending(&mut self, key: KeyEvent) -> Option<Pending> {
        if let (KeyCode::Char(c @ '0'..='9'), true, None) =
            (key.code, key.modifiers.is_empty(), self.pending.prefix)
        {
            // 0 はカウントの途中でだけ数字になる
            if c != '0' || self.pending.count.is_some() {
                let d = c as usize - '0' as usize;
                self.pending.count = Some((self.pending.count.unwrap_or(0) * 10 + d).min(9999));
                return None;
            }
        }
        let p = std::mem::take(&mut self.pending);
        // Esc は打ちかけを捨てるだけ
        if key.code == KeyCode::Esc && (p.count.is_some() || p.prefix.is_some()) {
            return None;
        }
        Some(p)
    }

    /// `gg` / `G` の行き先へ。`count` があればその行
    fn goto(&mut self, count: Option<usize>, last: bool) {
        let i = match count {
            Some(n) => n.saturating_sub(1),
            None if last => usize::MAX,
            None => 0,
        };
        self.view_select(i);
    }

    /// 半ページ下 (`down`) / 上へ
    fn half_page(&mut self, down: bool) {
        self.move_cursor(down, (self.view_rows / 2).max(1), false);
    }

    fn key_normal(&mut self, key: KeyEvent) {
        self.message = None;
        let Some(Pending { count, prefix }) = self.read_pending(key) else { return };
        let n = count.unwrap_or(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let c = match key.code {
            KeyCode::Char(c) => Some(c),
            _ => None,
        };
        if let Some(p) = prefix {
            // 続かないキーは打ちかけごと捨てる
            match (p, c) {
                ('g', Some('g')) => self.goto(count, false),
                ('g', Some('t')) => match count {
                    Some(i) => self.set_tab(i.clamp(1, TABS) - 1),
                    None => self.cycle_view(true),
                },
                ('g', Some('T')) => (0..n).for_each(|_| self.cycle_view(false)),
                ('d', Some('d')) => {
                    let sel = self.view().1;
                    let ids = self.view_ids(sel..sel + n);
                    self.delete(ids);
                }
                _ => {}
            }
            return;
        }
        if ctrl {
            match key.code {
                KeyCode::Char('d') => self.half_page(true),
                KeyCode::Char('u') => self.half_page(false),
                KeyCode::Char('r') => (0..n).for_each(|_| self.undo(true)),
                _ => {}
            }
            return;
        }
        if !self.key_common(key.code, count) {
            self.key_screen(key.code, n);
        }
    }

    /// どの画面でも同じキー。処理したら true
    fn key_common(&mut self, code: KeyCode, count: Option<usize>) -> bool {
        let n = count.unwrap_or(1);
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(']') | KeyCode::Tab => (0..n).for_each(|_| self.cycle_view(true)),
            KeyCode::Char('[') | KeyCode::BackTab => (0..n).for_each(|_| self.cycle_view(false)),
            KeyCode::Char(p @ ('g' | 'd')) => self.pending = Pending { count, prefix: Some(p) },
            KeyCode::Char('G') | KeyCode::End => self.goto(count, true),
            KeyCode::Home => self.goto(None, false),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(true, n, true),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(false, n, true),
            KeyCode::Char('u') => (0..n).for_each(|_| self.undo(false)),
            KeyCode::Char('/') => self.mode = Mode::Search(TextInput::default()),
            KeyCode::Char('n') => self.search_next(true, n),
            KeyCode::Char('N') => self.search_next(false, n),
            KeyCode::Char(':') => self.mode = Mode::Command(TextInput::default()),
            KeyCode::Char('?') => self.mode = Mode::Help(0),
            KeyCode::Char('v') => {
                if self.selected().is_some() {
                    self.mode = Mode::Visual { anchor: self.view().1 };
                }
            }
            KeyCode::Char('m') => self.edit_notes(),
            KeyCode::Char('r') => {
                self.message = Some("同期中…".into());
                self.request_sync();
            }
            _ => return false,
        }
        true
    }

    /// 画面ごとのキー
    fn key_screen(&mut self, code: KeyCode, n: usize) {
        let ids = self.selected().map(|w| vec![w.id.clone()]).unwrap_or_default();
        match self.screen {
            Screen::List => match code {
                KeyCode::Char('h') | KeyCode::Left => self.move_side(false),
                KeyCode::Char('l') | KeyCode::Right => self.move_side(true),
                KeyCode::Char('J') => self.shift_vertical(true, n),
                KeyCode::Char('K') => self.shift_vertical(false, n),
                KeyCode::Char('H') => self.shift_horizontal(&ids, false),
                KeyCode::Char('L') => self.shift_horizontal(&ids, true),
                KeyCode::Char('X') => self.move_to_other_list(&ids),
                KeyCode::Char('o') => {
                    let (len, sel) = self.view();
                    let at = if len == 0 { 0 } else { sel + 1 };
                    self.mode = Mode::Add { input: TextInput::default(), at };
                }
                KeyCode::Char('O') => self.mode = Mode::Add { input: TextInput::default(), at: self.sel() },
                KeyCode::Char('c') | KeyCode::Enter => self.open_edit(EditField::Title),
                KeyCode::Char('s') => self.open_edit(EditField::Due),
                KeyCode::Char('t') => self.mark_done(&ids),
                KeyCode::Char('a') => self.archive(&ids),
                KeyCode::Char('p') => self.paste(false),
                KeyCode::Char('P') => self.paste(true),
                _ => {}
            },
            Screen::Done | Screen::Archive => match code {
                // 直前に見ていたリストへ戻る
                KeyCode::Esc => self.screen = Screen::List,
                KeyCode::Char('p') => self.restore(&ids),
                // しまっていたものを結局やった
                KeyCode::Char('t') if self.screen == Screen::Archive => self.mark_done(&ids),
                _ => {}
            },
            Screen::Memo => match code {
                KeyCode::Esc => self.screen = Screen::List,
                KeyCode::Char('o' | 'O') => self.mode = Mode::Add { input: TextInput::default(), at: 0 },
                KeyCode::Enter => self.edit_notes(),
                KeyCode::Char('c') => self.open_edit(EditField::Title),
                KeyCode::Char('p' | 'P') => self.paste(false),
                _ => {}
            },
        }
    }

    fn key_visual(&mut self, key: KeyEvent, mut anchor: usize) -> Mode {
        self.message = None;
        let Some(Pending { count, prefix }) = self.read_pending(key) else {
            return Mode::Visual { anchor };
        };
        let n = count.unwrap_or(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let (len, sel) = self.view();
        let (start, end) = (anchor.min(sel), anchor.max(sel));
        let ids = self.view_ids(start..end + 1);
        let list = self.screen == Screen::List;
        match (prefix, key.code) {
            (Some('g'), KeyCode::Char('g')) => self.goto(count, false),
            (Some(_), _) => {}
            (None, KeyCode::Char('d')) if ctrl => self.half_page(true),
            (None, KeyCode::Char('u')) if ctrl => self.half_page(false),
            (None, _) if ctrl => {}
            (None, KeyCode::Esc | KeyCode::Char('v')) => return Mode::Normal,
            (None, KeyCode::Char('g')) => self.pending = Pending { count, prefix: Some('g') },
            (None, KeyCode::Char('G')) => self.goto(count, true),
            (None, KeyCode::Char('j') | KeyCode::Down) => self.move_cursor(true, n, false),
            (None, KeyCode::Char('k') | KeyCode::Up) => self.move_cursor(false, n, false),
            // 範囲の反対の端へ
            (None, KeyCode::Char('o')) => {
                self.view_select(anchor);
                anchor = sel;
            }
            (None, KeyCode::Char('d' | 'x')) => {
                self.delete(ids);
                return Mode::Normal;
            }
            // 範囲の下 / 上のものを反対側へ回して、範囲ごと動かす
            (None, KeyCode::Char('J')) if list => {
                for _ in 0..n {
                    let (len, sel) = self.view();
                    let (start, end) = (anchor.min(sel), anchor.max(sel));
                    if end + 1 >= len {
                        break;
                    }
                    let below = self.view_ids(end + 1..end + 2).remove(0);
                    self.place(&below, start);
                    anchor += 1;
                    self.select(sel + 1);
                }
            }
            (None, KeyCode::Char('K')) if list => {
                for _ in 0..n {
                    let sel = self.view().1;
                    let (start, end) = (anchor.min(sel), anchor.max(sel));
                    if start == 0 {
                        break;
                    }
                    let above = self.view_ids(start - 1..start).remove(0);
                    self.place(&above, end);
                    anchor -= 1;
                    self.select(sel - 1);
                }
            }
            (None, KeyCode::Char(c @ ('H' | 'L'))) if list => {
                self.shift_horizontal(&ids, c == 'L');
                return Mode::Normal;
            }
            (None, KeyCode::Char('X')) if list => {
                self.move_to_other_list(&ids);
                return Mode::Normal;
            }
            (None, KeyCode::Char('t')) if list || self.screen == Screen::Archive => {
                self.mark_done(&ids);
                return Mode::Normal;
            }
            (None, KeyCode::Char('a')) if list => {
                self.archive(&ids);
                return Mode::Normal;
            }
            (None, KeyCode::Char('p')) if matches!(self.screen, Screen::Done | Screen::Archive) => {
                self.restore(&ids);
                return Mode::Normal;
            }
            _ => {}
        }
        // 一覧が空になったら抜ける
        if len == 0 || self.selected().is_none() {
            return Mode::Normal;
        }
        Mode::Visual { anchor: anchor.min(self.view().0.saturating_sub(1)) }
    }

    /// `o` / `O` の名前の入力。vim の `o` と同じく Esc でも確定する
    fn key_add(&mut self, key: KeyEvent, mut input: TextInput, at: usize) -> Mode {
        let done = match key.code {
            KeyCode::Enter | KeyCode::Esc => true,
            _ => input.insert_key(key) == Insert::Escape,
        };
        if !done {
            return Mode::Add { input, at };
        }
        let title = input.text.trim();
        if !title.is_empty() {
            self.add(title, at);
        }
        Mode::Normal
    }

    /// 編集ポップアップ。名前と日時の欄は vim と同じく挿入モードとノーマルモードを持つ
    fn key_edit(&mut self, key: KeyEvent, mut e: Edit) -> Mode {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // ポップアップの中の `:` コマンド
        if let Some(mut cmd) = e.cmd.take() {
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Backspace if cmd.text.is_empty() => {}
                KeyCode::Enter => match cmd.text.trim() {
                    "w" | "wq" | "x" => return self.save_or_stay(e),
                    "q" | "q!" => return Mode::Normal,
                    "" => {}
                    other => e.error = Some(format!("知らないコマンドです: :{other}")),
                },
                _ => {
                    cmd.handle(key);
                    e.cmd = Some(cmd);
                }
            }
            return Mode::Edit(e);
        }

        if key.code == KeyCode::Enter || (ctrl && key.code == KeyCode::Char('s')) {
            return self.save_or_stay(e);
        }
        match key.code {
            KeyCode::Tab => {
                self.editor = Some(EditorReq {
                    id: e.id.clone(),
                    text: editor_text(e.kind, &e.notes),
                    from_popup: true,
                });
                return Mode::Edit(e);
            }
            // 矢印は挿入モードのままでも欄を移れる
            KeyCode::Up | KeyCode::Down => {
                e.move_field(key.code == KeyCode::Down);
                return Mode::Edit(e);
            }
            _ => {}
        }

        if e.inserting() {
            let Some(input) = e.input_mut() else { return Mode::Edit(e) };
            let res = match key.code {
                KeyCode::Esc => Insert::Escape,
                _ => input.insert_key(key),
            };
            match res {
                Insert::Escape => {
                    input.leave_insert();
                    e.insert = false;
                }
                Insert::Edited => e.error = None,
                Insert::Ignored => {}
            }
            return Mode::Edit(e);
        }

        // ここからポップアップの中のノーマルモード
        if std::mem::take(&mut e.zz) {
            return match key.code {
                KeyCode::Char('Z') => self.save_or_stay(e),
                KeyCode::Char('Q') => Mode::Normal,
                _ => Mode::Edit(e),
            };
        }
        if key.code == KeyCode::Esc {
            // 打ちかけの d / c があればそれだけ捨てる
            if e.input_mut().is_some_and(|i| i.cancel_op()) {
                return Mode::Edit(e);
            }
            return Mode::Normal;
        }
        if let Some(input) = e.input_mut() {
            match input.normal_key(key) {
                Normal::Insert => {
                    e.insert = true;
                    e.error = None;
                    return Mode::Edit(e);
                }
                Normal::Stay => return Mode::Edit(e),
                Normal::Ignored => {}
            }
        }
        match key.code {
            KeyCode::Char('j') => e.move_field(true),
            KeyCode::Char('k') => e.move_field(false),
            // 2択の切り替え (左が高 / 学校。空白は反転)
            KeyCode::Char('h' | 'l' | ' ') | KeyCode::Left | KeyCode::Right => {
                let v = match key.code {
                    KeyCode::Char(' ') => None,
                    KeyCode::Char('h') | KeyCode::Left => Some(true),
                    _ => Some(false),
                };
                match e.field {
                    EditField::Clau => e.clau = v.unwrap_or(!e.clau),
                    EditField::Axis => e.axis_hi = v.unwrap_or(!e.axis_hi),
                    _ => {}
                }
            }
            KeyCode::Char('q') => return Mode::Normal,
            KeyCode::Char(':') => e.cmd = Some(TextInput::default()),
            KeyCode::Char('Z') => e.zz = true,
            _ => {}
        }
        Mode::Edit(e)
    }

    /// 保存できたら閉じる。日時が読めなければ日時の欄に戻す
    fn save_or_stay(&mut self, mut e: Edit) -> Mode {
        match self.save_edit(&e) {
            Ok(()) => Mode::Normal,
            Err(msg) => {
                e.error = Some(msg);
                e.field = EditField::Due;
                e.insert = true;
                Mode::Edit(e)
            }
        }
    }

    fn key_search(&mut self, key: KeyEvent, mut input: TextInput) -> Mode {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Backspace if input.text.is_empty() => {}
            KeyCode::Enter => {
                // 空なら前回の検索語で探す
                if !input.text.is_empty() {
                    self.search = Some(input.text.clone());
                }
                self.search_next(true, 1);
            }
            _ => {
                input.handle(key);
                return Mode::Search(input);
            }
        }
        Mode::Normal
    }

    fn key_command(&mut self, key: KeyEvent, mut input: TextInput) -> Mode {
        match key.code {
            KeyCode::Esc => Mode::Normal,
            KeyCode::Backspace if input.text.is_empty() => Mode::Normal,
            KeyCode::Enter => self.run_command(input.text.trim()),
            _ => {
                input.handle(key);
                Mode::Command(input)
            }
        }
    }

    /// `:` のコマンドを実行して、次のモードを返す
    fn run_command(&mut self, cmd: &str) -> Mode {
        match cmd {
            "" => {}
            "q" | "q!" | "qa" | "qa!" | "qall" | "quit" | "wq" | "x" | "wqa" | "xa" => self.quit = true,
            "w" | "write" => {
                self.message = Some("同期中…".into());
                self.request_sync();
            }
            "noh" | "nohlsearch" => self.search = None,
            "h" | "help" => return Mode::Help(0),
            _ => match cmd.parse::<usize>() {
                Ok(n) => self.goto(Some(n.max(1)), false),
                Err(_) => self.message = Some(format!("知らないコマンドです: :{cmd}")),
            },
        }
        Mode::Normal
    }
}

fn key_help(key: KeyEvent, scroll: u16) -> Mode {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let max = help_lines() as u16;
    let scroll = match key.code {
        KeyCode::Char('q' | '?') | KeyCode::Esc | KeyCode::Enter => return Mode::Normal,
        KeyCode::Char('d') if ctrl => scroll.saturating_add(10),
        KeyCode::Char('u') if ctrl => scroll.saturating_sub(10),
        KeyCode::Char('j') | KeyCode::Down => scroll.saturating_add(1),
        KeyCode::Char('k') | KeyCode::Up => scroll.saturating_sub(1),
        KeyCode::Char('g') | KeyCode::Home => 0,
        KeyCode::Char('G') | KeyCode::End => max,
        _ => scroll,
    };
    Mode::Help(scroll.min(max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use wanna_core::{Kind, Quadrant};

    fn app() -> App {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        App::new(store, None).unwrap()
    }

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            let code = match c {
                '\n' => KeyCode::Enter,
                '\x1b' => KeyCode::Esc,
                c => KeyCode::Char(c),
            };
            app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }

    fn ctrl(app: &mut App, c: char) {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    /// Want の区分の名前を並び順で
    fn titles(app: &App, qi: usize) -> Vec<String> {
        app.list(Kind::Want, Quadrant::ALL[qi]).iter().map(|w| w.title.clone()).collect()
    }

    /// Must の段 (0 = 学校 / 2 = その他) の名前を並び順で
    fn must(app: &App, qi: usize) -> Vec<String> {
        app.list(Kind::Task, Quadrant::ALL[qi]).iter().map(|w| w.title.clone()).collect()
    }

    #[test]
    fn add_with_o_and_esc() {
        let mut a = app();
        press(&mut a, "oa\x1bob\noc\x1b");
        assert_eq!(titles(&a, 0), ["a", "b", "c"]);
        // O は上に入る。jj でも確定する
        press(&mut a, "ggOzjj");
        assert_eq!(titles(&a, 0), ["z", "a", "b", "c"]);
        // 空なら何も作らない
        press(&mut a, "o\x1b");
        assert_eq!(titles(&a, 0).len(), 4);
    }

    #[test]
    fn counts_and_gg() {
        let mut a = app();
        press(&mut a, "oa\x1bob\x1boc\x1bod\x1b");
        press(&mut a, "gg");
        assert_eq!(a.sel(), 0);
        press(&mut a, "2j");
        assert_eq!(a.sel(), 2);
        press(&mut a, "G");
        assert_eq!(a.sel(), 3);
        press(&mut a, "2G");
        assert_eq!(a.sel(), 1);
        press(&mut a, ":4\n");
        assert_eq!(a.sel(), 3);
        // g の後に続かないキーは捨てる
        press(&mut a, "gxk");
        assert_eq!(a.sel(), 2);
    }

    #[test]
    fn tabs() {
        let mut a = app();
        press(&mut a, "gt");
        assert_eq!(a.tab(), 1);
        press(&mut a, "4gt");
        assert_eq!(a.tab(), 3);
        press(&mut a, "gT");
        assert_eq!(a.tab(), 2);
        press(&mut a, "]]");
        assert_eq!(a.tab(), 4);
    }

    #[test]
    fn delete_paste_undo() {
        let mut a = app();
        press(&mut a, "oa\x1bob\x1boc\x1b");
        press(&mut a, "ggdd");
        assert_eq!(titles(&a, 0), ["b", "c"]);
        // 別の区分に貼る
        press(&mut a, "lp");
        assert_eq!(titles(&a, 1), ["a"]);
        press(&mut a, "u");
        assert!(titles(&a, 1).is_empty());
        press(&mut a, "u");
        assert_eq!(titles(&a, 0), ["a", "b", "c"]);
        ctrl(&mut a, 'r');
        assert_eq!(titles(&a, 0), ["b", "c"]);
        ctrl(&mut a, 'r');
        assert_eq!(titles(&a, 1), ["a"]);
        // 作ったのも取り消せる
        press(&mut a, "uuuu");
        assert_eq!(titles(&a, 0), ["a"]);
    }

    #[test]
    fn move_items() {
        let mut a = app();
        press(&mut a, "oa\x1bob\x1boc\x1b");
        press(&mut a, "ggJ");
        assert_eq!(titles(&a, 0), ["b", "a", "c"]);
        press(&mut a, "J");
        assert_eq!(titles(&a, 0), ["b", "c", "a"]);
        // 区分の端からは下の区分へ
        press(&mut a, "J");
        assert_eq!(titles(&a, 2), ["a"]);
        press(&mut a, "K");
        assert_eq!(titles(&a, 0), ["b", "c", "a"]);
        press(&mut a, "ggL");
        assert_eq!(titles(&a, 1), ["b"]);
        press(&mut a, "H");
        assert_eq!(titles(&a, 0), ["b", "c", "a"]);
    }

    #[test]
    fn visual() {
        let mut a = app();
        press(&mut a, "oa\x1bob\x1boc\x1bod\x1b");
        press(&mut a, "ggvjJ");
        assert_eq!(titles(&a, 0), ["c", "a", "b", "d"]);
        press(&mut a, "K\x1b");
        assert_eq!(titles(&a, 0), ["a", "b", "c", "d"]);
        press(&mut a, "ggjvjd");
        assert_eq!(titles(&a, 0), ["a", "d"]);
        press(&mut a, "P");
        assert_eq!(titles(&a, 0), ["a", "b", "c", "d"]);
        press(&mut a, "ggvjt");
        assert_eq!(titles(&a, 0), ["c", "d"]);
        assert_eq!(a.done().len(), 2);
        // Done から元のリストへ戻す
        press(&mut a, "3gtvjp");
        assert_eq!(a.done().len(), 0);
        assert_eq!(titles(&a, 0), ["c", "d", "a", "b"]);
    }

    #[test]
    fn search() {
        let mut a = app();
        press(&mut a, "oapple\x1bobanana\x1bocherry\x1bjomango\x1b");
        press(&mut a, "kgg/an\n");
        assert_eq!(titles(&a, 0)[a.sel()], "banana");
        press(&mut a, "n");
        assert_eq!((a.cur, titles(&a, 2)[a.sel()].as_str()), (2, "mango"));
        press(&mut a, "n");
        assert_eq!((a.cur, titles(&a, 0)[a.sel()].as_str()), (0, "banana"));
        press(&mut a, "N");
        assert_eq!(a.cur, 2);
        press(&mut a, ":noh\n");
        assert!(a.search.is_none());
    }

    #[test]
    fn edit_popup_modes() {
        let mut a = app();
        press(&mut a, "oa\x1b");
        // c で挿入モード。Esc でノーマル、もう一度 Esc でやめる
        press(&mut a, "cbc\x1b\x1b");
        assert_eq!(titles(&a, 0), ["a"]);
        press(&mut a, "cbc\x1b:w\n");
        assert_eq!(titles(&a, 0), ["abc"]);
        // ノーマルモードの編集と ZZ
        press(&mut a, "c\x1bccxyzjj");
        press(&mut a, "jlZZ");
        let w = a.selected().unwrap();
        assert_eq!((w.title.as_str(), w.axis_hi), ("xyz", false));
    }

    #[test]
    fn must_rows() {
        let mut a = app();
        press(&mut a, "gt");
        press(&mut a, "oa\x1bob\x1boc\x1b");
        assert_eq!(must(&a, 0), ["a", "b", "c"]);
        // 日時を入れると日時の位置へ。日時を持つものが他に無ければ先頭
        press(&mut a, "ggjs2030-01-02\n");
        assert_eq!(must(&a, 0), ["b", "a", "c"]);
        press(&mut a, "Gs2030-01-01\n");
        assert_eq!(must(&a, 0), ["c", "b", "a"]);
        // カーソルは日時を入れたものについていく
        assert_eq!(a.selected().unwrap().title, "c");
        // J / K は日時をまたいで段の中を動く。段の端からは出ない
        press(&mut a, "J");
        assert_eq!(must(&a, 0), ["b", "c", "a"]);
        press(&mut a, "JJ");
        assert_eq!(must(&a, 0), ["b", "a", "c"]);
        assert!(must(&a, 2).is_empty());
        press(&mut a, "K");
        assert_eq!(must(&a, 0), ["b", "c", "a"]);
        // L でその他へ、H で学校へ。行き先では日時の位置に入る
        press(&mut a, "L");
        assert_eq!((must(&a, 0), must(&a, 2)), (vec!["b".to_string(), "a".into()], vec!["c".to_string()]));
        assert_eq!((a.qi(), a.selected().unwrap().title.as_str()), (2, "c"));
        press(&mut a, "L");
        assert_eq!(must(&a, 2), ["c"]);
        press(&mut a, "H");
        assert_eq!(must(&a, 0), ["c", "b", "a"]);
        // h / l はカーソルを学校 / その他の段へ
        press(&mut a, "l");
        assert_eq!(a.qi(), 2);
        press(&mut a, "h");
        assert_eq!(a.qi(), 0);
        // ビジュアルモードでまとめてその他へ
        press(&mut a, "ggvjL");
        assert_eq!(must(&a, 2), ["c", "b"]);
        // Want から X で移すとその他の末尾に入り、Must に切り替わる
        press(&mut a, "gTozz\x1bX");
        assert_eq!(must(&a, 2), ["c", "b", "zz"]);
        assert_eq!(a.kind, Kind::Task);
        // 編集に clau度の欄は無く、j で日時の欄へ
        press(&mut a, "c\x1bjj");
        let Mode::Edit(e) = &a.mode else { panic!("編集ポップアップが開いていない") };
        assert!(e.field == EditField::Due);
    }

    #[test]
    fn esc_does_not_quit() {
        let mut a = app();
        press(&mut a, "\x1b");
        assert!(!a.quit);
        press(&mut a, ":q\n");
        assert!(a.quit);
    }
}
