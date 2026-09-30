//! メモのテンプレートと見出し (`## 名前`) 単位の読み書き
//!
//! メモは Markdown の自由文のまま持つ。見出しは目安で、無くても壊れない。

/// 進捗ログの見出し
pub const LOG: &str = "Log";

/// 見出し。やりたいことと次にやることで同じものを使う
pub const SECTIONS: [&str; 5] = ["Why", "Done when", "Next steps", LOG, "Open questions"];

/// 空のメモを開いたときに入れておく下書き
pub fn template() -> String {
    SECTIONS.iter().map(|s| format!("## {s}\n")).collect::<Vec<_>>().join("\n")
}

/// 見出しと空行しか無ければ空とみなす（テンプレートを開いて何も書かずに閉じたとき）
pub fn is_blank(text: &str) -> bool {
    text.lines().all(|l| heading(l).is_some() || l.trim().is_empty())
}

fn heading(line: &str) -> Option<&str> {
    line.strip_prefix("## ").map(str::trim)
}

/// `name` の見出しの本文の範囲 (行番号, 見出し行は含まない)
fn range(lines: &[&str], name: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| heading(l) == Some(name))? + 1;
    let end = lines[start..]
        .iter()
        .position(|l| heading(l).is_some())
        .map_or(lines.len(), |i| start + i);
    Some((start, end))
}

/// 見出しの本文。前後の空行は落とす
pub fn section(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (s, e) = range(&lines, name)?;
    Some(lines[s..e].join("\n").trim_matches('\n').trim_end().to_string())
}

/// 見出しの本文を `body` に置き換える。見出しが無ければ末尾に足す
pub fn set_section(text: &str, name: &str, body: &str) -> String {
    let body = body.trim_matches('\n').trim_end();
    let lines: Vec<&str> = text.lines().collect();
    let Some((s, e)) = range(&lines, name) else {
        return push_section(text, name, body);
    };
    let mut out: Vec<&str> = lines[..s].to_vec();
    if !body.is_empty() {
        out.extend(body.lines());
    }
    if e < lines.len() {
        out.push("");
    }
    out.extend(&lines[e..]);
    out.join("\n").trim_end().to_string()
}

/// 見出しの本文の末尾に1行足す。見出しが無ければ末尾に足す
pub fn append(text: &str, name: &str, line: &str) -> String {
    match section(text, name) {
        Some(cur) if !cur.is_empty() => set_section(text, name, &format!("{cur}\n{line}")),
        _ => set_section(text, name, line),
    }
}

fn push_section(text: &str, name: &str, body: &str) -> String {
    let text = text.trim_end();
    let mut out = String::new();
    if !text.is_empty() {
        out.push_str(text);
        out.push_str("\n\n");
    }
    out.push_str(&format!("## {name}\n{body}"));
    out.trim_end().to_string()
}

/// 進捗ログの1行。`by_claude` なら `[c]` を付ける
pub fn log_line(date: &str, text: &str, by_claude: bool) -> String {
    let mark = if by_claude { " [c]" } else { "" };
    format!("- {date}{mark} {}", text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_blank() {
        assert!(is_blank(&template()));
        assert!(is_blank(""));
        assert!(!is_blank("## Why\n楽しいから"));
    }

    #[test]
    fn read_section() {
        let t = "## Why\n\n楽しい\n\n## Next steps\nA を読む\n";
        assert_eq!(section(t, "Why").as_deref(), Some("楽しい"));
        assert_eq!(section(t, "Next steps").as_deref(), Some("A を読む"));
        assert_eq!(section(t, "無い"), None);
    }

    #[test]
    fn replace_section_keeps_others() {
        let t = template();
        let t = set_section(&t, "Next steps", "API を読む");
        assert_eq!(section(&t, "Next steps").as_deref(), Some("API を読む"));
        assert!(t.contains("## Done when\n\n## Next steps\nAPI を読む\n\n## Log"));
        let t = set_section(&t, "Next steps", "テストを書く");
        assert_eq!(section(&t, "Next steps").as_deref(), Some("テストを書く"));
        assert_eq!(t.matches("## Next steps").count(), 1);
    }

    #[test]
    fn append_log() {
        let t = template();
        let t = append(&t, LOG, "- 1");
        let t = append(&t, LOG, "- 2");
        assert_eq!(section(&t, LOG).as_deref(), Some("- 1\n- 2"));
        assert!(t.contains("- 2\n\n## Open questions"));
    }

    #[test]
    fn missing_section_goes_to_end() {
        let t = append("自由に書いたメモ", LOG, "- 1");
        assert_eq!(t, "自由に書いたメモ\n\n## Log\n- 1");
        assert_eq!(append("", LOG, "- 1"), "## Log\n- 1");
    }

    #[test]
    fn log_mark() {
        assert_eq!(log_line("2026-09-30", "API を足した", true), "- 2026-09-30 [c] API を足した");
        assert_eq!(log_line("2026-09-30", "読んだ", false), "- 2026-09-30 読んだ");
    }
}
