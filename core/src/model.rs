use serde::{Deserialize, Deserializer, Serialize};

/// やりたいこと。このアプリが扱う唯一のもの。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Want {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    /// true = エネルギー高
    pub energy: bool,
    /// true = clau度高
    pub clau: bool,
    /// 区分内の並び順キー (`pos` モジュール)
    #[serde(default)]
    pub pos: String,
    /// やった日時 (RFC3339)。入っていれば「やったこと」側
    #[serde(default)]
    pub done_at: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// サーバー採番のリビジョン。クライアント未送信のものは 0
    #[serde(default)]
    pub rev: i64,
    pub created_at: String,
}

impl Want {
    /// 新しい「やりたいこと」を作る。ID は UUIDv7。
    pub fn new(title: impl Into<String>, energy: bool, clau: bool, pos: String) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            title: title.into(),
            notes: String::new(),
            energy,
            clau,
            pos,
            done_at: None,
            deleted: false,
            rev: 0,
            created_at: now_rfc3339(),
        }
    }

    pub fn quadrant(&self) -> Quadrant {
        Quadrant { energy: self.energy, clau: self.clau }
    }

    /// やりたいことリストに出るもの（やってない・消してない）
    pub fn is_active(&self) -> bool {
        !self.deleted && self.done_at.is_none()
    }

    /// 部分更新を適用する
    pub fn apply(&mut self, p: &WantPatch) {
        if let Some(v) = &p.title {
            self.title = v.clone();
        }
        if let Some(v) = &p.notes {
            self.notes = v.clone();
        }
        if let Some(v) = p.energy {
            self.energy = v;
        }
        if let Some(v) = p.clau {
            self.clau = v;
        }
        if let Some(v) = &p.pos {
            self.pos = v.clone();
        }
        if let Some(v) = &p.done_at {
            self.done_at = v.clone();
        }
    }
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 区分。エネルギー高低 × clau度高低。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quadrant {
    pub energy: bool,
    pub clau: bool,
}

impl Quadrant {
    /// 画面上の並び (左上, 右上, 左下, 右下)
    pub const ALL: [Quadrant; 4] = [
        Quadrant { energy: true, clau: false },
        Quadrant { energy: true, clau: true },
        Quadrant { energy: false, clau: false },
        Quadrant { energy: false, clau: true },
    ];

    /// 軸の値をそのまま書いた表示 (例: "エネルギー高・clau低")
    pub fn label(&self) -> &'static str {
        match (self.energy, self.clau) {
            (true, false) => "エネルギー高・clau低",
            (true, true) => "エネルギー高・clau高",
            (false, false) => "エネルギー低・clau低",
            (false, true) => "エネルギー低・clau高",
        }
    }
}

/// 部分更新。送ったフィールドだけ上書きする。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WantPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clau: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<String>,
    /// `None` = 変更なし、`Some(None)` = 取り消し (JSON の null)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub done_at: Option<Option<String>>,
}

fn double_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// `GET /api/sync` の応答
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub rev: i64,
    pub wants: Vec<Want>,
}

/// 区分内の表示順 `(pos, id)` で並べる
pub fn sort_by_pos(wants: &mut [&Want]) {
    wants.sort_by(|a, b| (&a.pos, &a.id).cmp(&(&b.pos, &b.id)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_null_vs_missing() {
        let p: WantPatch = serde_json::from_str(r#"{"done_at": null}"#).unwrap();
        assert_eq!(p.done_at, Some(None));
        let p: WantPatch = serde_json::from_str(r#"{"title": "x"}"#).unwrap();
        assert_eq!(p.done_at, None);
        let p: WantPatch = serde_json::from_str(r#"{"done_at": "2026-09-18T00:00:00Z"}"#).unwrap();
        assert_eq!(p.done_at, Some(Some("2026-09-18T00:00:00Z".into())));
    }

    #[test]
    fn patch_serializes_null() {
        let p = WantPatch { done_at: Some(None), ..Default::default() };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"done_at":null}"#);
        assert_eq!(serde_json::to_string(&WantPatch::default()).unwrap(), "{}");
    }

    #[test]
    fn apply_patch() {
        let mut w = Want::new("a", true, false, "V".into());
        w.apply(&WantPatch { done_at: Some(Some("t".into())), ..Default::default() });
        assert!(!w.is_active());
        w.apply(&WantPatch { done_at: Some(None), clau: Some(true), ..Default::default() });
        assert!(w.is_active());
        assert_eq!(w.quadrant().label(), "エネルギー高・clau高");
    }
}
