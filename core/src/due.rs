//! 「次にやること」の日時。
//!
//! 保存形式は2つある。時刻を持たないものは `YYYY-MM-DD`、持つものは RFC3339。
//! 「今日中にやる」と「14:00 にやる」は別物なので、時刻なしを 00:00 に潰さずそのまま残す。
//! 日付だけのものはその日のどこかなので、比較も表示もローカルの日付として扱う。

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};

/// 入力欄に添えるヒント
pub const HINT: &str = "例: 2026-09-25 / 09-25 14:00 / 明日 18:00 / 金 / 空で消す";

/// 日時の今との関係。表示の色分けに使う
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 過ぎている
    Over,
    /// 今日
    Today,
    /// まだ先
    Later,
}

/// 保存形式を (日付, 時刻) に開く。読めなければ None
pub fn parse(s: &str) -> Option<(NaiveDate, Option<NaiveTime>)> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some((d, None));
    }
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?.with_timezone(&Local).naive_local();
    Some((dt.date(), Some(dt.time())))
}

/// 保存形式として正しいか
pub fn valid(s: &str) -> bool {
    parse(s).is_some()
}

/// 並べ替え用のローカル日時。時刻なしはその日の 00:00 (同じ日なら時刻ありより先)
pub fn at(s: &str) -> Option<NaiveDateTime> {
    let (d, t) = parse(s)?;
    Some(d.and_time(t.unwrap_or_default()))
}

/// `now` から見た状態。時刻なしはその日のうちなら「今日」
pub fn state(s: &str, now: NaiveDateTime) -> State {
    let Some((d, t)) = parse(s) else { return State::Later };
    match d.cmp(&now.date()) {
        std::cmp::Ordering::Less => State::Over,
        std::cmp::Ordering::Greater => State::Later,
        std::cmp::Ordering::Equal => match t {
            Some(t) if t < now.time() => State::Over,
            _ => State::Today,
        },
    }
}

/// 一覧に出す短い表示 ("今日 14:00" / "09-25" / "2027-01-05 09:00")
pub fn format(s: &str, today: NaiveDate) -> String {
    let Some((d, t)) = parse(s) else { return s.to_string() };
    let day = match (d - today).num_days() {
        0 => "今日".to_string(),
        1 => "明日".to_string(),
        -1 => "昨日".to_string(),
        _ if d.year() == today.year() => d.format("%m-%d").to_string(),
        _ => d.format("%Y-%m-%d").to_string(),
    };
    match t {
        Some(t) => format!("{day} {}", t.format("%H:%M")),
        None => day,
    }
}

/// 編集欄に出す形。そのまま編集して `parse_input` に戻せる
pub fn to_input(s: &str) -> String {
    match parse(s) {
        Some((d, None)) => d.format("%Y-%m-%d").to_string(),
        Some((d, Some(t))) => format!("{} {}", d.format("%Y-%m-%d"), t.format("%H:%M")),
        None => s.to_string(),
    }
}

/// 入力を保存形式にする。空なら `Ok(None)`
pub fn parse_input(input: &str, now: NaiveDateTime) -> Result<Option<String>, String> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    let (date, time) = match parts.as_slice() {
        // 1語なら日付として読み、読めなければ時刻とみなして今日に付ける
        [one] => match parse_date(one, now.date()) {
            Some(d) => (d, None),
            None => (now.date(), Some(parse_time(one).ok_or_else(|| unreadable(s))?)),
        },
        [d, t] => (
            parse_date(d, now.date()).ok_or_else(|| unreadable(s))?,
            Some(parse_time(t).ok_or_else(|| unreadable(s))?),
        ),
        _ => return Err(unreadable(s)),
    };
    Ok(Some(store(date, time)))
}

fn unreadable(s: &str) -> String {
    format!("日時として読めません: {s}  ({HINT})")
}

fn store(d: NaiveDate, t: Option<NaiveTime>) -> String {
    let Some(t) = t else { return d.format("%Y-%m-%d").to_string() };
    let naive = d.and_time(t);
    // 存在しないローカル時刻 (DST の飛び) は起こらない想定だが、落とさず UTC として持つ
    let dt = Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive));
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// `2026-09-25` / `09-25` (今年) / `9月25日` / `明日` / `金` など
fn parse_date(s: &str, today: NaiveDate) -> Option<NaiveDate> {
    match s {
        "今日" | "きょう" | "本日" | "today" => return Some(today),
        "明日" | "あした" | "あす" | "tomorrow" => return Some(today + Duration::days(1)),
        "明後日" | "あさって" => return Some(today + Duration::days(2)),
        "昨日" | "きのう" | "yesterday" => return Some(today - Duration::days(1)),
        _ => {}
    }
    if let Some(w) = weekday(s) {
        // 今日は含めず、次のその曜日
        let mut d = today + Duration::days(1);
        while d.weekday() != w {
            d += Duration::days(1);
        }
        return Some(d);
    }
    let norm = s.replace(['/', '.', '年', '月'], "-");
    let norm = norm.trim_end_matches('日').trim_end_matches('-');
    let nums: Vec<u32> =
        norm.split('-').filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (y, m, d) = match nums.as_slice() {
        [y, m, d] => (*y as i32, *m, *d),
        // 月日だけなら今年
        [m, d] => (today.year(), *m, *d),
        _ => return None,
    };
    NaiveDate::from_ymd_opt(y, m, d)
}

fn weekday(s: &str) -> Option<Weekday> {
    let s = s.trim_end_matches("曜日").trim_end_matches('曜');
    Some(match s.to_ascii_lowercase().as_str() {
        "月" | "mon" | "monday" => Weekday::Mon,
        "火" | "tue" | "tuesday" => Weekday::Tue,
        "水" | "wed" | "wednesday" => Weekday::Wed,
        "木" | "thu" | "thursday" => Weekday::Thu,
        "金" | "fri" | "friday" => Weekday::Fri,
        "土" | "sat" | "saturday" => Weekday::Sat,
        "日" | "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

/// `14:00` / `14時30分` / `14` など
fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim_end_matches('分');
    let norm = s.replace(['時', '：'], ":");
    let norm = norm.trim_end_matches(':');
    let (h, m) = match norm.split_once(':') {
        Some((h, m)) => (h.parse().ok()?, m.parse().ok()?),
        None => (norm.parse().ok()?, 0),
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        // 2026-09-21 は月曜
        NaiveDate::from_ymd_opt(2026, 9, 21).unwrap().and_hms_opt(10, 0, 0).unwrap()
    }

    fn input(s: &str) -> String {
        parse_input(s, now()).unwrap().unwrap()
    }

    #[test]
    fn dates_without_time_stay_dates() {
        assert_eq!(input("2026-09-25"), "2026-09-25");
        assert_eq!(input("2026/9/25"), "2026-09-25");
        assert_eq!(input("9月25日"), "2026-09-25");
        assert_eq!(input("09-25"), "2026-09-25");
        assert_eq!(input("今日"), "2026-09-21");
        assert_eq!(input("明日"), "2026-09-22");
        assert_eq!(input("明後日"), "2026-09-23");
        // 今日と同じ曜日は来週
        assert_eq!(input("月"), "2026-09-28");
        assert_eq!(input("金曜"), "2026-09-25");
        assert_eq!(parse_input("  ", now()).unwrap(), None);
    }

    #[test]
    fn times_round_trip_through_local() {
        for (given, shown) in
            [("2026-09-25 14:00", "09-25 14:00"), ("明日 18時", "明日 18:00"), ("9:30", "今日 09:30")]
        {
            let stored = input(given);
            assert!(valid(&stored), "{stored}");
            assert_eq!(format(&stored, now().date()), shown);
        }
        assert_eq!(to_input(&input("明日 18時")), "2026-09-22 18:00");
        assert_eq!(to_input(&input("明日")), "2026-09-22");
    }

    #[test]
    fn unreadable_input_is_an_error() {
        assert!(parse_input("あとで", now()).is_err());
        assert!(parse_input("2026-13-40", now()).is_err());
        assert!(parse_input("25:00", now()).is_err());
        assert!(parse_input("明日 18:00 まで", now()).is_err());
    }

    #[test]
    fn state_and_order() {
        let n = now();
        assert_eq!(state(&input("昨日"), n), State::Over);
        assert_eq!(state(&input("今日"), n), State::Today);
        assert_eq!(state(&input("9:30"), n), State::Over);
        assert_eq!(state(&input("18:00"), n), State::Today);
        assert_eq!(state(&input("明日"), n), State::Later);
        // 同じ日なら時刻なしが先
        assert!(at(&input("今日")) < at(&input("9:30")));
    }
}
