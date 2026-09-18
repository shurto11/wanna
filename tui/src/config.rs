use serde::Deserialize;
use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wanna")
}

/// `~/.config/wanna/config.toml`。環境変数 `WANNA_SERVER` / `WANNA_TOKEN` が優先。
#[derive(Deserialize, Default)]
pub struct Config {
    pub server: Option<String>,
    pub token: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let mut cfg: Config = std::fs::read_to_string(config_dir().join("config.toml"))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        if let Ok(v) = std::env::var("WANNA_SERVER") {
            cfg.server = Some(v);
        }
        if let Ok(v) = std::env::var("WANNA_TOKEN") {
            cfg.token = Some(v);
        }
        cfg
    }

    /// サーバーと token が揃っていれば (server, token)
    pub fn remote(&self) -> Option<(&str, &str)> {
        Some((self.server.as_deref()?, self.token.as_deref()?))
    }
}
