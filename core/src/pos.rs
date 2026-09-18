//! 区分内の並び順キー (fractional index)。
//!
//! キーは base62 (`0-9A-Za-z`) の文字列で、辞書順がそのまま並び順になる。
//! 小数 `0.xxx` の小数部として解釈し、末尾が `0` のキーは作らない
//! （作ると、それより前に挟む余地が無くなるため）。

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

fn val(c: u8) -> usize {
    match c {
        b'0'..=b'9' => (c - b'0') as usize,
        b'A'..=b'Z' => (c - b'A') as usize + 10,
        b'a'..=b'z' => (c - b'a') as usize + 36,
        _ => panic!("pos キーに base62 以外の文字: {:?}", c as char),
    }
}

fn digit(v: usize) -> char {
    DIGITS[v] as char
}

/// `a` と `b` の間に入るキーを返す。`None` はそれぞれ先頭 / 末尾を表す。
///
/// `a < b` でなければならない。
pub fn between(a: Option<&str>, b: Option<&str>) -> String {
    if let (Some(a), Some(b)) = (a, b) {
        assert!(a < b, "pos::between: {a:?} < {b:?} ではない");
    }
    match (a, b) {
        // 末尾への追加は最頻出なので、キーが伸びにくい専用の方法を使う
        (Some(a), None) => after(a),
        (a, b) => midpoint(a.unwrap_or("").as_bytes(), b.map(str::as_bytes)),
    }
}

/// `a` より後ろのキー。最初の `z` でない桁を1つ増やし、全桁 `z` なら `1` を足す。
/// これで末尾への追加を繰り返しても 61 回に1文字しか伸びない。
pub fn after(a: &str) -> String {
    let bytes = a.as_bytes();
    match bytes.iter().position(|&c| c != b'z') {
        Some(i) => {
            let mut s = a[..i].to_string();
            s.push(digit(val(bytes[i]) + 1));
            s
        }
        None => format!("{a}1"),
    }
}

/// `a` (空 = 0) と `b` (None = 1) の中間。
fn midpoint(a: &[u8], b: Option<&[u8]>) -> String {
    if let Some(b) = b {
        // 共通接頭辞 (a は右を 0 で埋めて比較) を切り出す
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let prefix = std::str::from_utf8(&b[..n]).unwrap();
            let rest_a = if n < a.len() { &a[n..] } else { &[][..] };
            return format!("{prefix}{}", midpoint(rest_a, Some(&b[n..])));
        }
    }
    let da = a.first().map(|&c| val(c)).unwrap_or(0);
    let db = b.map(|b| val(b[0])).unwrap_or(BASE);
    if db - da > 1 {
        digit((da + db).div_ceil(2)).to_string()
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        // b の先頭桁だけ取れば a < b[0] < b になる
        digit(val(b[0])).to_string()
    } else {
        let rest_a = if a.len() > 1 { &a[1..] } else { &[][..] };
        format!("{}{}", digit(da), midpoint(rest_a, None))
    }
}

/// `a` と `b` の間に等間隔で `n` 個のキーを作る（一括移行用）。
pub fn n_between(a: Option<&str>, b: Option<&str>, n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let mid = midpoint(a.unwrap_or("").as_bytes(), b.map(str::as_bytes));
    let left = n / 2;
    let mut keys = n_between(a, Some(&mid), left);
    keys.push(mid.clone());
    keys.extend(n_between(Some(&mid), b, n - left - 1));
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(k: &str) -> bool {
        !k.is_empty() && !k.ends_with('0') && k.bytes().all(|c| c.is_ascii_alphanumeric())
    }

    #[test]
    fn first_key() {
        assert_eq!(between(None, None), "V");
    }

    #[test]
    fn append_is_short() {
        let mut k = between(None, None);
        for _ in 0..1000 {
            let next = between(Some(&k), None);
            assert!(next > k && valid(&next));
            k = next;
        }
        assert!(k.len() <= 20, "{k}");
    }

    #[test]
    fn prepend_repeatedly() {
        let mut k = between(None, None);
        for _ in 0..500 {
            let next = between(None, Some(&k));
            assert!(next < k && valid(&next), "{next} {k}");
            k = next;
        }
    }

    #[test]
    fn insert_between_repeatedly() {
        let a = "V".to_string();
        let mut b = "W".to_string();
        for _ in 0..500 {
            let m = between(Some(&a), Some(&b));
            assert!(a < m && m < b && valid(&m), "{a} {m} {b}");
            b = m;
        }
        let mut a = "V".to_string();
        let b = "W".to_string();
        for _ in 0..500 {
            let m = between(Some(&a), Some(&b));
            assert!(a < m && m < b && valid(&m), "{a} {m} {b}");
            a = m;
        }
    }

    #[test]
    fn edge_cases() {
        for (a, b) in [("1", "2"), ("z", "zz"), ("0V", "1"), ("A", "A1"), ("y", "z"), ("zz", "zzV")] {
            let m = between(Some(a), Some(b));
            assert!(a < m.as_str() && m.as_str() < b && valid(&m), "{a} {m} {b}");
        }
        let m = between(None, Some("01"));
        assert!(m.as_str() < "01" && valid(&m), "{m}");
    }

    /// web/src/pos.test.ts と同じ値。TS 実装と結果が一致すること
    #[test]
    fn same_as_typescript() {
        assert_eq!(between(Some("V"), None), "W");
        assert_eq!(between(Some("z"), None), "z1");
        assert_eq!(between(Some("1"), Some("2")), "1V");
        assert_eq!(between(None, Some("01")), "00V");
    }

    #[test]
    fn n_between_is_sorted() {
        let keys = n_between(None, None, 300);
        assert_eq!(keys.len(), 300);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| valid(k) && k.len() <= 3));
    }
}
