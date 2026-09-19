//! 外部エディタ ($VISUAL / $EDITOR、無ければ vim) でテキストを編集する

use anyhow::{bail, Context, Result};
use std::process::Command;

/// `text` を一時ファイルに書いてエディタで開き、保存後の内容を返す
pub fn edit(text: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("wanna-notes-{}.md", std::process::id()));
    std::fs::write(&path, text).context("一時ファイルを書けません")?;
    let res = (|| {
        let cmd = ["VISUAL", "EDITOR"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "vim".into());
        let mut parts = cmd.split_whitespace();
        let prog = parts.next().unwrap_or("vim");
        let status = Command::new(prog)
            .args(parts)
            .arg(&path)
            .status()
            .with_context(|| format!("{prog} を起動できません"))?;
        if !status.success() {
            bail!("{prog} が異常終了しました ({status})");
        }
        std::fs::read_to_string(&path).context("一時ファイルを読めません")
    })();
    let _ = std::fs::remove_file(&path);
    res
}
