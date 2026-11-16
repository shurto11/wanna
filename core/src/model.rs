use serde::{Deserialize, Deserializer, Serialize};

/// どちらのリストのものか。
///
/// - `Want` … やりたいこと。縦軸はエネルギー。日時は持たない
/// - `Task` … 次にやること。縦軸は「学校 / その他」の種類。clau度では分けず、日時を持てる
/// - `Memo` … メモ。区分・日時・やった/しまったを持たず、名前とメモ本文だけ使う
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Want,
    Task,
    Memo,
}

impl Kind {
    /// 区分で並べるリスト (画面上の並び)。メモは含まない
    pub const ALL: [Kind; 2] = [Kind::Want, Kind::Task];

    /// リストの名前
    pub fn label(self) -> &'static str {
        match self {
            Kind::Want => "Want",
            Kind::Task => "Must",
            Kind::Memo => "Memo",
        }
    }

    /// 縦軸の名前。Must は高低ではなく「学校 / その他」の種類で分ける
    pub fn axis(self) -> &'static str {
        match self {
            Kind::Want => "エネルギー",
            Kind::Task => "種類",
            Kind::Memo => "",
        }
    }

    /// 縦軸の 高 / 低 の呼び名
    pub fn axis_values(self) -> (&'static str, &'static str) {
        match self {
            Kind::Task => ("学校", "その他"),
            _ => ("高", "低"),
        }
    }

    /// clau度で区分を分けるか。Must は「学校 / その他」の2段だけで、clau度では分けない
    pub fn splits_clau(self) -> bool {
        self == Kind::Want
    }

    /// 日時を持てるか
    pub fn has_due(self) -> bool {
        self == Kind::Task
    }

    /// 区分で並べるリストか
    pub fn is_list(self) -> bool {
        self != Kind::Memo
    }

    /// もう一方のリスト。メモはメモのまま
    pub fn other(self) -> Kind {
        match self {
            Kind::Want => Kind::Task,
            Kind::Task => Kind::Want,
            Kind::Memo => Kind::Memo,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Kind::Want => 0,
            Kind::Task => 1,
            Kind::Memo => 2,
        }
    }

    /// DB / API での文字列表現
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Want => "want",
            Kind::Task => "task",
            Kind::Memo => "memo",
        }
    }

    /// 知らない値は「やりたいこと」に倒す
    pub fn from_str(s: &str) -> Kind {
        match s {
            "task" => Kind::Task,
            "memo" => Kind::Memo,
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
    /// 縦軸が高いか (やりたいこと = エネルギー高 / 次にやること = 学校)
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

    /// 区分の表示 (例: "エネルギー高・clau低" / "学校")。メモは区分を持たないので空
    pub fn quadrant_label(&self) -> String {
        if !self.kind.is_list() {
            return String::new();
        }
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
        if let Some(v) = p.deleted {
            self.deleted = v;
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

    /// `kind` のリストで画面に出す区分 (`ALL` の添字)。Must は clau度で分けないので左の列の2つ
    pub fn slots(kind: Kind) -> &'static [usize] {
        if kind.splits_clau() {
            &[0, 1, 2, 3]
        } else {
            &[0, 2]
        }
    }

    /// `w` がこの区分に入るか。clau度で分けないリスト (Must) では縦軸だけ見る
    pub fn holds(&self, w: &Want) -> bool {
        w.axis_hi == self.axis_hi && (!w.kind.splits_clau() || w.clau == self.clau)
    }

    /// 軸の値をそのまま書いた表示 (例: "エネルギー高・clau低")。Must は "学校" / "その他" だけ
    pub fn label(&self, kind: Kind) -> String {
        let (hi, lo) = kind.axis_values();
        let axis = if self.axis_hi { hi } else { lo };
        if !kind.splits_clau() {
            return axis.to_string();
        }
        format!("{}{axis}・clau{}", kind.axis(), if self.clau { "高" } else { "低" })
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
    /// `Some(false)` で消したものを戻す (取り消しと貼り付け用)。`Some(true)` は削除と同じ
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
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

/// 日時の順に並べる。日時なしは最後で、同じ日時どうし・日時なしどうしは `(pos, id)` の順。
/// Must の区分内の表示順 (Want は日時を持たないので `sort_by_pos` と同じになる)
pub fn sort_by_due(wants: &mut [&Want]) {
    wants.sort_by_cached_key(|w| {
        let at = w.due().and_then(crate::due::at);
        (at.is_none(), at, w.pos.clone(), w.id.clone())
    });
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
        assert_eq!(w.quadrant_label(), "学校");
        w.apply(&WantPatch { axis_hi: Some(false), ..Default::default() });
        assert_eq!(w.quadrant_label(), "その他");
    }

    /// Must は clau度で分けず、縦軸 (学校 / その他) だけで区分に入る
    #[test]
    fn must_ignores_clau() {
        let mut w = Want::new("a", Kind::Task, true, true, "V".into());
        assert!(Quadrant::ALL[0].holds(&w));
        assert!(!Quadrant::ALL[2].holds(&w));
        w.kind = Kind::Want;
        assert!(!Quadrant::ALL[0].holds(&w));
        assert!(Quadrant::ALL[1].holds(&w));
        assert_eq!(Quadrant::slots(Kind::Task), [0, 2]);
    }

    #[test]
    fn sort_by_due_puts_undated_last() {
        let mk = |t: &str, pos: &str, due: Option<&str>| {
            let mut w = Want::new(t, Kind::Task, true, false, pos.into());
            w.due_at = due.map(str::to_string);
            w
        };
        let ws = [
            mk("none-a", "A", None),
            mk("late", "B", Some("2026-12-01")),
            mk("none-0", "0", None),
            mk("timed", "C", Some("2026-11-25T12:00:00+09:00")),
            mk("day", "D", Some("2026-11-20")),
        ];
        let mut v: Vec<&Want> = ws.iter().collect();
        sort_by_due(&mut v);
        let titles: Vec<&str> = v.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["day", "timed", "late", "none-0", "none-a"]);
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
    fn undelete() {
        let mut w = Want::new("a", Kind::Want, true, false, "V".into());
        w.deleted = true;
        w.apply(&WantPatch { deleted: Some(false), ..Default::default() });
        assert!(w.is_active());
        let p: WantPatch = serde_json::from_str(r#"{"deleted": false}"#).unwrap();
        assert_eq!(p.deleted, Some(false));
        assert_eq!(serde_json::to_string(&WantPatch::default()).unwrap(), "{}");
    }

    #[test]
    fn memo_kind() {
        assert_eq!(Kind::from_str("memo"), Kind::Memo);
        assert_eq!(Kind::Memo.as_str(), "memo");
        let w: Want = serde_json::from_str(
            r#"{"id":"1","title":"t","kind":"memo","axis_hi":true,"clau":false,"created_at":"x"}"#,
        )
        .unwrap();
        assert_eq!(w.kind, Kind::Memo);
        assert_eq!(w.quadrant_label(), "");
        assert!(!Kind::ALL.contains(&Kind::Memo));
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
