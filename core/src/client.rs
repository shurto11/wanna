//! wannad の API クライアント (blocking)。TUI と migrate が使う。

use crate::{SyncResponse, Want, WantPatch};
use anyhow::{bail, Result};
use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use std::io::{BufRead, BufReader};
use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    base: String,
    token: String,
    http: Http,
}

/// サーバーが 4xx を返した（再送しても無駄な）エラー
#[derive(Debug)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "サーバーが拒否しました: {}", self.0)
    }
}

impl std::error::Error for Rejected {}

impl Client {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: Http::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .build()?,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn send(&self, req: RequestBuilder) -> Result<Response> {
        let res = req.bearer_auth(&self.token).send()?;
        let status = res.status();
        if status.is_client_error() {
            let body = res.text().unwrap_or_default();
            return Err(Rejected(format!("{status} {body}")).into());
        }
        if !status.is_success() {
            bail!("サーバーエラー {status}");
        }
        Ok(res)
    }

    /// 差分取得。`since` が None なら全件
    pub fn sync(&self, since: Option<i64>) -> Result<SyncResponse> {
        let mut req = self.http.get(self.url("/api/sync"));
        if let Some(s) = since {
            req = req.query(&[("since", s)]);
        }
        Ok(self.send(req)?.json()?)
    }

    pub fn create(&self, w: &Want) -> Result<Want> {
        Ok(self.send(self.http.post(self.url("/api/wants")).json(w))?.json()?)
    }

    pub fn patch(&self, id: &str, p: &WantPatch) -> Result<Want> {
        Ok(self
            .send(self.http.patch(self.url(&format!("/api/wants/{id}"))).json(p))?
            .json()?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.send(self.http.delete(self.url(&format!("/api/wants/{id}"))))?;
        Ok(())
    }

    /// SSE を購読し、`rev` が進むたびに `on_rev` を呼ぶ。接続が切れたら返る。
    pub fn listen(&self, mut on_rev: impl FnMut(i64)) -> Result<()> {
        let http = Http::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(None)
            .build()?;
        let res = http
            .get(self.url("/api/events"))
            .bearer_auth(&self.token)
            .send()?;
        if !res.status().is_success() {
            bail!("SSE 接続失敗 {}", res.status());
        }
        for line in BufReader::new(res).lines() {
            let line = line?;
            if let Some(data) = line.strip_prefix("data:") {
                if let Ok(rev) = data.trim().parse() {
                    on_rev(rev);
                }
            }
        }
        Ok(())
    }
}
