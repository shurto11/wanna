//! 1行の入力欄。挿入モードの編集と、編集ポップアップで使う vim のノーマルモードの編集。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 1行のテキスト入力
#[derive(Default, Clone)]
pub struct TextInput {
    pub text: String,
    /// 文字単位のカーソル位置
    pub cursor: usize,
    /// 直前のキーで `j` を挿入した (`jj` で挿入モードを抜ける)
    after_j: bool,
    /// ノーマルモードで打ちかけの `d` / `c`
    op: Option<char>,
}

/// 挿入モードでキーを処理した結果
#[derive(PartialEq, Eq)]
pub enum Insert {
    Edited,
    /// `jj` を打った。~/.vimrc の `inoremap jj <Esc>` と同じく Esc として扱う
    Escape,
    /// 編集のキーではない
    Ignored,
}

/// ノーマルモードでキーを処理した結果
#[derive(PartialEq, Eq)]
pub enum Normal {
    /// 移動・削除した、または `d` `c` の続きを待っている
    Stay,
    /// 挿入モードへ (`i` `a` `cw` など)
    Insert,
    /// 入力欄の編集のキーではない
    Ignored,
}

impl TextInput {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.chars().count(), ..Default::default() }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
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

    /// `from..to` 文字目を消してカーソルを `from` に置く
    fn remove(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte_at(from), self.byte_at(to));
        self.text.drain(a..b);
        self.cursor = from;
    }

    /// 編集キーなら処理して true (`:` や `/` の入力欄はこれだけを使う)
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.len(),
            KeyCode::Char('u') if ctrl => self.remove(0, self.cursor),
            KeyCode::Char('w') if ctrl => self.remove(self.word_back(), self.cursor),
            KeyCode::Char('h') if ctrl && self.cursor > 0 => self.remove(self.cursor - 1, self.cursor),
            KeyCode::Char(c) if !ctrl => self.insert(c),
            KeyCode::Backspace if self.cursor > 0 => self.remove(self.cursor - 1, self.cursor),
            KeyCode::Delete if self.cursor < self.len() => self.remove(self.cursor, self.cursor + 1),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }

    /// 挿入モードのキー。`jj` は1つめの `j` を消して Esc として返す
    pub fn insert_key(&mut self, key: KeyEvent) -> Insert {
        let j = key.code == KeyCode::Char('j') && key.modifiers.is_empty();
        if j && self.after_j && self.cursor > 0 {
            self.after_j = false;
            self.remove(self.cursor - 1, self.cursor);
            return Insert::Escape;
        }
        let edited = self.handle(key);
        self.after_j = j && edited;
        if edited {
            Insert::Edited
        } else {
            Insert::Ignored
        }
    }

    /// 挿入モードを抜ける。vim と同じくカーソルを1つ左へ
    pub fn leave_insert(&mut self) {
        self.after_j = false;
        self.cursor = self.cursor.saturating_sub(1);
        self.clamp_normal();
    }

    /// ノーマルモードのカーソルは文字の上に置く (末尾の後ろには置かない)
    pub fn clamp_normal(&mut self) {
        self.cursor = self.cursor.min(self.len().saturating_sub(1));
    }

    /// 打ちかけの `d` / `c` があれば捨てて true
    pub fn cancel_op(&mut self) -> bool {
        self.op.take().is_some()
    }

    /// ノーマルモードのキー。`h` `l` `0` `$` `w` `b` `e` で動き、
    /// `i` `a` `I` `A` `s` `S` `C` `cc` `c{motion}` で挿入モードへ、`x` `X` `D` `dd` `d{motion}` で消す
    pub fn normal_key(&mut self, key: KeyEvent) -> Normal {
        let c = match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => c,
            KeyCode::Left => 'h',
            KeyCode::Right => 'l',
            KeyCode::Home => '0',
            KeyCode::End => '$',
            _ => return Normal::Ignored,
        };
        let (cur, len) = (self.cursor, self.len());
        if let Some(op) = self.op.take() {
            let range = match c {
                _ if c == op => Some((0, len)),
                // vim の `cw` は語の後ろの空白を残す
                'w' if op == 'c' => Some((cur, self.word_here_end())),
                'w' => Some((cur, self.word_next())),
                'e' => Some((cur, self.word_end() + 1)),
                'b' => Some((self.word_back(), cur)),
                '$' => Some((cur, len)),
                '0' | '^' => Some((0, cur)),
                _ => None,
            };
            if let Some((from, to)) = range {
                self.remove(from, to.min(len));
                if op == 'c' {
                    return Normal::Insert;
                }
                self.clamp_normal();
            }
            return Normal::Stay;
        }
        match c {
            'h' => self.cursor = cur.saturating_sub(1),
            'l' => self.cursor = (cur + 1).min(len.saturating_sub(1)),
            '0' | '^' => self.cursor = 0,
            '$' => self.cursor = len.saturating_sub(1),
            'w' => self.cursor = self.word_next().min(len.saturating_sub(1)),
            'e' => self.cursor = self.word_end(),
            'b' => self.cursor = self.word_back(),
            'i' => return Normal::Insert,
            'a' => {
                self.cursor = (cur + 1).min(len);
                return Normal::Insert;
            }
            'I' => {
                self.cursor = 0;
                return Normal::Insert;
            }
            'A' => {
                self.cursor = len;
                return Normal::Insert;
            }
            'x' if len > 0 => {
                self.remove(cur, cur + 1);
                self.clamp_normal();
            }
            'X' if cur > 0 => self.remove(cur - 1, cur),
            's' => {
                self.remove(cur, (cur + 1).min(len));
                return Normal::Insert;
            }
            'S' => {
                self.remove(0, len);
                return Normal::Insert;
            }
            'C' => {
                self.remove(cur, len);
                return Normal::Insert;
            }
            'D' => {
                self.remove(cur, len);
                self.clamp_normal();
            }
            'd' | 'c' => self.op = Some(c),
            'x' | 'X' => {}
            _ => return Normal::Ignored,
        }
        Normal::Stay
    }

    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    /// 次の語の頭 (`w`)。無ければ末尾の後ろ
    fn word_next(&self) -> usize {
        let cs = self.chars();
        let mut i = self.cursor;
        if let Some(&c) = cs.get(i) {
            let k = class(c);
            while i < cs.len() && class(cs[i]) == k && k != Class::Space {
                i += 1;
            }
        }
        while i < cs.len() && class(cs[i]) == Class::Space {
            i += 1;
        }
        i
    }

    /// カーソルのある語の終わりの次 (`cw` の範囲)。空白の上なら `w` と同じ
    fn word_here_end(&self) -> usize {
        let cs = self.chars();
        let Some(&c) = cs.get(self.cursor) else { return self.cursor };
        if class(c) == Class::Space {
            return self.word_next();
        }
        let mut i = self.cursor;
        while i < cs.len() && class(cs[i]) == class(c) {
            i += 1;
        }
        i
    }

    /// 今か次の語の終わり (`e`)
    fn word_end(&self) -> usize {
        let cs = self.chars();
        if cs.is_empty() {
            return 0;
        }
        let mut i = self.cursor + 1;
        while i < cs.len() && class(cs[i]) == Class::Space {
            i += 1;
        }
        if i >= cs.len() {
            return cs.len() - 1;
        }
        let k = class(cs[i]);
        while i + 1 < cs.len() && class(cs[i + 1]) == k {
            i += 1;
        }
        i
    }

    /// 今か前の語の頭 (`b`。挿入モードの Ctrl+W もここまで消す)
    fn word_back(&self) -> usize {
        let cs = self.chars();
        let mut i = self.cursor.min(cs.len());
        while i > 0 && class(cs[i - 1]) == Class::Space {
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let k = class(cs[i - 1]);
        while i > 0 && class(cs[i - 1]) == k {
            i -= 1;
        }
        i
    }
}

/// 語の区切りに使う文字の種類。vim と同じく、ひらがな・カタカナ・漢字はそれぞれ別の語になる
#[derive(PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
    Hiragana,
    Katakana,
    Kanji,
    Other,
}

fn class(c: char) -> Class {
    match c {
        _ if c.is_whitespace() => Class::Space,
        _ if c.is_ascii_alphanumeric() || c == '_' => Class::Word,
        _ if c.is_ascii() => Class::Punct,
        '\u{3000}'..='\u{303f}' | '\u{ff01}'..='\u{ff0f}' | '\u{ff1a}'..='\u{ff20}' => Class::Punct,
        '\u{3040}'..='\u{309f}' => Class::Hiragana,
        '\u{30a0}'..='\u{30ff}' => Class::Katakana,
        '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => Class::Kanji,
        _ => Class::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn at(text: &str, cursor: usize) -> TextInput {
        TextInput { text: text.into(), cursor, ..Default::default() }
    }

    #[test]
    fn jj_leaves_insert() {
        let mut t = TextInput::new("ab");
        assert!(t.insert_key(key('j')) == Insert::Edited);
        assert_eq!(t.text, "abj");
        assert!(t.insert_key(key('j')) == Insert::Escape);
        assert_eq!(t.text, "ab");
        // 間に別のキーを挟めばただの j
        let mut t = TextInput::new("");
        for c in "jaj".chars() {
            t.insert_key(key(c));
        }
        assert_eq!(t.text, "jaj");
    }

    #[test]
    fn ctrl_w_deletes_a_word() {
        let mut t = TextInput::new("buy milk");
        t.handle(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(t.text, "buy ");
        let mut t = TextInput::new("牛乳を買う");
        t.handle(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(t.text, "牛乳を買");
    }

    #[test]
    fn motions() {
        let mut t = at("foo bar.baz", 0);
        t.normal_key(key('w'));
        assert_eq!(t.cursor, 4);
        t.normal_key(key('e'));
        assert_eq!(t.cursor, 6);
        t.normal_key(key('w'));
        assert_eq!(t.cursor, 7);
        t.normal_key(key('b'));
        assert_eq!(t.cursor, 4);
        t.normal_key(key('$'));
        assert_eq!(t.cursor, 10);
        t.normal_key(key('l'));
        assert_eq!(t.cursor, 10);
        t.normal_key(key('0'));
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn operators() {
        let mut t = at("foo bar baz", 4);
        assert!(t.normal_key(key('d')) == Normal::Stay);
        t.normal_key(key('w'));
        assert_eq!(t.text, "foo baz");
        assert!(t.normal_key(key('c')) == Normal::Stay);
        assert!(t.normal_key(key('w')) == Normal::Insert);
        assert_eq!(t.text, "foo ");
        // 語の最後の文字で cw してもその文字だけ
        let mut t = at("foo bar", 2);
        t.normal_key(key('c'));
        t.normal_key(key('w'));
        assert_eq!(t.text, "fo bar");
        let mut t = at("foo bar", 2);
        t.normal_key(key('D'));
        assert_eq!((t.text.as_str(), t.cursor), ("fo", 1));
        t.normal_key(key('d'));
        t.normal_key(key('d'));
        assert_eq!(t.text, "");
        let mut t = at("abc", 1);
        t.normal_key(key('x'));
        assert_eq!(t.text, "ac");
        assert!(t.normal_key(key('A')) == Normal::Insert);
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn leave_insert_steps_back() {
        let mut t = TextInput::new("abc");
        t.leave_insert();
        assert_eq!(t.cursor, 2);
        let mut t = TextInput::new("");
        t.leave_insert();
        assert_eq!(t.cursor, 0);
    }
}
