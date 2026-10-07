use serde::{Deserialize, Deserializer, Serialize};

/// どちらのリストのものか。
///
/// - `Want` … やりたいこと。縦軸はエネルギー。日時は持たない
/// - `Task` … 次にやること。縦軸は重要度。日時を持てる
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Want,
    Task,
}

impl Kind {
    /// 画面上の並び
    pub const ALL: [Kind; 2] = [Kind::Want, Kind::Task];

    /// リストの名前
    pub fn label(self) -> &'static str {
        match self {
            Kind::Want => "Want",
            Kind::Task => "Must",
        }
    }

    /// 縦軸の名前
    pub fn axis(self) -> &'static str {
        match self {
            Kind::Want => "エネルギー",
            Kind::Task => "重要度",
        }
    }

    /// 日時を持てるか
    pub fn has_due(self) -> bool {
        self == Kind::Task
    }

    pub fn other(self) -> Kind {
        match self {
            Kind::Want => Kind::Task,
            Kind::Task => Kind::Want,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Kind::Want => 0,
            Kind::Task => 1,
        }
    }

    /// DB / API での文字列表現
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Want => "want",
            Kind::Task => "task",
        }
    }

    /// 知らない値は「やりたいこと」に倒す
    pub fn from_str(s: &str) -> Kind {
        match s {
            "task" => Kind::Task,
            _ => Kind::Want,
        }
    }
}

/// 1件。やりたいこと / 次にやること のどちらかで、`kind` が分ける。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Want {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub kind: Kind,
    /// 縦軸が高いか (やりたいこと = エネルギー高 / 次にやること = 重要度高)
    #[serde(alias = "energy")]
    pub axis_hi: bool,
    /// true = clau度高
    pub clau: bool,
    /// 区分内の並び順キー (`pos` モジュール)
    #[serde(default)]
    pub pos: String,
    /// 日時 (`due` モジュールの保存形式)。次にやることだけが持つ
    #[serde(default)]
    pub due_at: Option<String>,
    /// やった日時 (RFC3339)。入っていれば「やったこと」側
    #[serde(default)]
    pub done_at: Option<String>,
    /// しまった日時 (RFC3339)。やってはいないがリストに置くほどでもなくなったもの
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// サーバー採番のリビジョン。クライアント未送信のものは 0
    #[serde(default)]
    pub rev: i64,
    pub created_at: String,
}

impl Want {
    /// 新しい1件を作る。ID は UUIDv7。
    pub fn new(title: impl Into<String>, kind: Kind, axis_hi: bool, clau: bool, pos: String) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            title: title.into(),
            notes: String::new(),
            kind,
            axis_hi,
            clau,
            pos,
            due_at: None,
            done_at: None,
            archived_at: None,
            deleted: false,
            rev: 0,
            created_at: now_rfc3339(),
        }
    }

    pub fn quadrant(&self) -> Quadrant {
        Quadrant { axis_hi: self.axis_hi, clau: self.clau }
    }

    /// 区分の表示 (例: "エネルギー高・clau低")
    pub fn quadrant_label(&self) -> String {
        self.quadrant().label(self.kind)
    }

    /// リストに出るもの（やってない・しまってない・消してない）
    pub fn is_active(&self) -> bool {
        !self.deleted && self.done_at.is_none() && self.archived_at.is_none()
    }

    /// 保管庫に出るもの。やったことになっていれば、やったこと側に出す
    pub fn is_archived(&self) -> bool {
        !self.deleted && self.done_at.is_none() && self.archived_at.is_some()
    }

    /// 日時。やりたいことは持たないので常に None
    pub fn due(&self) -> Option<&str> {
        self.kind.has_due().then(|| self.due_at.as_deref()).flatten()
    }

    /// 部分更新を適用する
    pub fn apply(&mut self, p: &WantPatch) {
        if let Some(v) = &p.title {
            self.title = v.clone();
        }
        if let Some(v) = &p.notes {
            self.notes = v.clone();
        }
        if let Some(v) = p.kind {
            self.kind = v;
        }
        if let Some(v) = p.axis_hi {
            self.axis_hi = v;
        }
        if let Some(v) = p.clau {
            self.clau = v;
        }
        if let Some(v) = &p.pos {
            self.pos = v.clone();
        }
        if let Some(v) = &p.due_at {
            self.due_at = v.clone();
        }
        if let Some(v) = &p.done_at {
            self.done_at = v.clone();
        }
        if let Some(v) = &p.archived_at {
            self.archived_at = v.clone();
        }
    }
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 区分。縦軸の高低 × clau度の高低。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quadrant {
    pub axis_hi: bool,
    pub clau: bool,
}

impl Quadrant {
    /// 画面上の並び (左上, 右上, 左下, 右下)
    pub const ALL: [Quadrant; 4] = [
        Quadrant { axis_hi: true, clau: false },
        Quadrant { axis_hi: true, clau: true },
        Quadrant { axis_hi: false, clau: false },
        Quadrant { axis_hi: false, clau: true },
    ];

    /// 軸の値をそのまま書いた表示 (例: "重要度高・clau低")
    pub fn label(&self, kind: Kind) -> String {
        let hi = |b: bool| if b { "高" } else { "低" };
        format!("{}{}・clau{}", kind.axis(), hi(self.axis_hi), hi(self.clau))
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
    pub kind: Option<Kind>,
    #[serde(default, alias = "energy", skip_serializing_if = "Option::is_none")]
    pub axis_hi: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clau: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<String>,
    /// `None` = 変更なし、`Some(None)` = 日時なし (JSON の null)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub due_at: Option<Option<String>>,
    /// `None` = 変更なし、`Some(None)` = 取り消し (JSON の null)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub done_at: Option<Option<String>>,
    /// `None` = 変更なし、`Some(None)` = 取り出す (JSON の null)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub archived_at: Option<Option<String>>,
    /// 書き込み時点の rev がこれでなければサーバーは 409 を返す (読んで書き戻す人向け)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_rev: Option<i64>,
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
        assert_eq!(p.due_at, None);
        let p: WantPatch = serde_json::from_str(r#"{"due_at": null}"#).unwrap();
        assert_eq!(p.due_at, Some(None));
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
        let mut w = Want::new("a", Kind::Want, true, false, "V".into());
        w.apply(&WantPatch { done_at: Some(Some("t".into())), ..Default::default() });
        assert!(!w.is_active());
        w.apply(&WantPatch { done_at: Some(None), clau: Some(true), ..Default::default() });
        assert!(w.is_active());
        assert_eq!(w.quadrant_label(), "エネルギー高・clau高");
        w.apply(&WantPatch { kind: Some(Kind::Task), ..Default::default() });
        assert_eq!(w.quadrant_label(), "重要度高・clau高");
    }

    #[test]
    fn archive() {
        let mut w = Want::new("a", Kind::Want, true, false, "V".into());
        w.apply(&WantPatch { archived_at: Some(Some("t".into())), ..Default::default() });
        assert!(!w.is_active());
        assert!(w.is_archived());
        // やったことにすれば保管庫からは消える
        w.apply(&WantPatch { done_at: Some(Some("t".into())), ..Default::default() });
        assert!(!w.is_archived());
        w.apply(&WantPatch { done_at: Some(None), archived_at: Some(None), ..Default::default() });
        assert!(w.is_active());
        let p: WantPatch = serde_json::from_str(r#"{"archived_at": null}"#).unwrap();
        assert_eq!(p.archived_at, Some(None));
    }

    #[test]
    fn due_only_for_tasks() {
        let mut w = Want::new("a", Kind::Want, true, false, "V".into());
        w.due_at = Some("2026-09-25".into());
        assert_eq!(w.due(), None);
        w.kind = Kind::Task;
        assert_eq!(w.due(), Some("2026-09-25"));
    }

    /// 旧 `energy` の JSON もそのまま読めること (TUI のキャッシュと未送信キュー)
    #[test]
    fn reads_legacy_energy_field() {
        let w: Want = serde_json::from_str(
            r#"{"id":"1","title":"t","energy":true,"clau":false,"created_at":"x"}"#,
        )
        .unwrap();
        assert!(w.axis_hi);
        assert_eq!(w.kind, Kind::Want);
        let p: WantPatch = serde_json::from_str(r#"{"energy": false}"#).unwrap();
        assert_eq!(p.axis_hi, Some(false));
    }
}
