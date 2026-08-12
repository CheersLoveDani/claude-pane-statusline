//! Minimal JSON reader for the one object Claude Code pipes in on stdin.
//!
//! Deliberately not a general-purpose parser: it accepts the JSON subset that
//! appears in the hook payload and is lenient about the rest, because a
//! malformed field must degrade to a missing segment, never to a panic. The JS
//! this replaces wrapped every read in `try {} catch (_) {}` for the same reason.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    /// Field lookup that returns `Null` for anything missing, so callers can
    /// chain `.get("a").get("b")` without checking each hop.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(m) => m.get(key).unwrap_or(&Json::Null),
            _ => &Json::Null,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
}

pub fn parse(input: &str) -> Option<Json> {
    // PowerShell 5.1 pipes prepend a BOM; the JS stripped it so the script could
    // be exercised by hand from a PS prompt, and the same applies here.
    let s = input.strip_prefix('\u{feff}').unwrap_or(input);
    let b = s.as_bytes();
    let mut i = 0;
    let v = value(b, &mut i)?;
    Some(v)
}

fn ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn value(b: &[u8], i: &mut usize) -> Option<Json> {
    ws(b, i);
    match *b.get(*i)? {
        b'{' => object(b, i),
        b'[' => array(b, i),
        b'"' => string(b, i).map(Json::Str),
        b't' => lit(b, i, b"true", Json::Bool(true)),
        b'f' => lit(b, i, b"false", Json::Bool(false)),
        b'n' => lit(b, i, b"null", Json::Null),
        _ => number(b, i),
    }
}

fn lit(b: &[u8], i: &mut usize, want: &[u8], out: Json) -> Option<Json> {
    if b.len() >= *i + want.len() && &b[*i..*i + want.len()] == want {
        *i += want.len();
        Some(out)
    } else {
        None
    }
}

fn object(b: &[u8], i: &mut usize) -> Option<Json> {
    *i += 1; // '{'
    let mut m = BTreeMap::new();
    ws(b, i);
    if *b.get(*i)? == b'}' {
        *i += 1;
        return Some(Json::Obj(m));
    }
    loop {
        ws(b, i);
        let k = string(b, i)?;
        ws(b, i);
        if *b.get(*i)? != b':' {
            return None;
        }
        *i += 1;
        let v = value(b, i)?;
        m.insert(k, v);
        ws(b, i);
        match *b.get(*i)? {
            b',' => *i += 1,
            b'}' => {
                *i += 1;
                return Some(Json::Obj(m));
            }
            _ => return None,
        }
    }
}

fn array(b: &[u8], i: &mut usize) -> Option<Json> {
    *i += 1; // '['
    let mut v = Vec::new();
    ws(b, i);
    if *b.get(*i)? == b']' {
        *i += 1;
        return Some(Json::Arr(v));
    }
    loop {
        v.push(value(b, i)?);
        ws(b, i);
        match *b.get(*i)? {
            b',' => *i += 1,
            b']' => {
                *i += 1;
                return Some(Json::Arr(v));
            }
            _ => return None,
        }
    }
}

fn string(b: &[u8], i: &mut usize) -> Option<String> {
    if *b.get(*i)? != b'"' {
        return None;
    }
    *i += 1;
    let mut out = String::new();
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        match c {
            b'"' => return Some(out),
            b'\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                match e {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let cp = hex4(b, i)?;
                        // Surrogate pair: session titles and prompts are arbitrary
                        // user text, so astral-plane characters are reachable.
                        if (0xd800..0xdc00).contains(&cp)
                            && b.get(*i) == Some(&b'\\')
                            && b.get(*i + 1) == Some(&b'u')
                        {
                            *i += 2;
                            let lo = hex4(b, i)?;
                            if (0xdc00..0xe000).contains(&lo) {
                                let c = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                out.push(char::from_u32(c)?);
                            } else {
                                out.push('\u{fffd}');
                            }
                        } else {
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        }
                    }
                    _ => return None,
                }
            }
            // Multi-byte UTF-8 passes through a byte at a time; the accumulated
            // bytes are re-validated below.
            _ => {
                let start = *i - 1;
                let len = utf8_len(c);
                if len > 1 {
                    *i = start + len;
                    out.push_str(std::str::from_utf8(b.get(start..*i)?).ok()?);
                } else {
                    out.push(c as char);
                }
            }
        }
    }
}

fn utf8_len(c: u8) -> usize {
    if c >= 0xf0 {
        4
    } else if c >= 0xe0 {
        3
    } else if c >= 0xc0 {
        2
    } else {
        1
    }
}

fn hex4(b: &[u8], i: &mut usize) -> Option<u32> {
    let s = std::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
    *i += 4;
    u32::from_str_radix(s, 16).ok()
}

fn number(b: &[u8], i: &mut usize) -> Option<Json> {
    let start = *i;
    if *b.get(*i)? == b'-' {
        *i += 1;
    }
    while *i < b.len() && matches!(b[*i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
        *i += 1;
    }
    std::str::from_utf8(&b[start..*i])
        .ok()?
        .parse::<f64>()
        .ok()
        .map(Json::Num)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shape_claude_code_sends() {
        let v = parse(
            r#"{"session_id":"abc","workspace":{"current_dir":"E:\\dev\\x"},
                "model":{"display_name":"Opus 5"},"context_window":{"used_percentage":42},
                "rate_limits":{"five_hour":{"used_percentage":80.5}}}"#,
        )
        .unwrap();
        assert_eq!(v.get("session_id").as_str(), Some("abc"));
        assert_eq!(v.get("workspace").get("current_dir").as_str(), Some("E:\\dev\\x"));
        assert_eq!(v.get("model").get("display_name").as_str(), Some("Opus 5"));
        assert_eq!(v.get("context_window").get("used_percentage").as_f64(), Some(42.0));
        assert_eq!(
            v.get("rate_limits").get("five_hour").get("used_percentage").as_f64(),
            Some(80.5)
        );
    }

    #[test]
    fn missing_fields_chain_to_null_instead_of_panicking() {
        let v = parse("{}").unwrap();
        assert_eq!(v.get("model").get("display_name").as_str(), None);
        assert_eq!(v.get("context_window").get("used_percentage").as_f64(), None);
    }

    #[test]
    fn strips_the_powershell_bom() {
        let v = parse("\u{feff}{\"session_id\":\"x\"}").unwrap();
        assert_eq!(v.get("session_id").as_str(), Some("x"));
    }

    #[test]
    fn handles_escapes_and_non_ascii() {
        let v = parse(r#"{"a":"q\"uote\\ \u00e9 \ud83d\ude00 caf\u00e9 ✦"}"#).unwrap();
        assert_eq!(v.get("a").as_str(), Some("q\"uote\\ é 😀 café ✦"));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(parse("not json").is_none());
        assert!(parse("{\"a\":").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn arrays_and_nesting() {
        let v = parse(r#"{"a":[1,{"b":true},null,"s"]}"#).unwrap();
        match v.get("a") {
            Json::Arr(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[1].get("b"), &Json::Bool(true));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }
}
