//! Project files: the whole job (parts, stock, kerf, units) as a
//! `.cutlist.json` document the user can save to disk and reopen —
//! save-then-quote workflows, sharing a job folder, backups.
//!
//! The format is a flat object of string fields. It is read and written by
//! a tiny hand-rolled serializer instead of serde so the free build (which
//! compiles without serde) keeps its payload size.

/// The parsed contents of a project file. All values are the raw textarea
/// text — the regular parsers validate them when the job loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectJob {
    /// Unit the text was typed in (`"mm"` / `"in"`, see [`crate::parse::Units`]).
    pub units: String,
    /// Parts textarea content.
    pub parts: String,
    /// Stock textarea content (Pro edition; free uses a single length).
    pub stocks: String,
    /// Kerf width as typed.
    pub kerf: String,
}

const APP_ID: &str = "tpt-cutlist-optimizer";

/// Serializes a job as pretty-printed JSON.
pub fn write_project_json(job: &ProjectJob) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!("  \"app\": \"{}\",\n", APP_ID));
    out.push_str("  \"version\": 1,\n");
    out.push_str(&format!("  \"units\": {},\n", json_string(&job.units)));
    out.push_str(&format!("  \"parts\": {},\n", json_string(&job.parts)));
    out.push_str(&format!("  \"stocks\": {},\n", json_string(&job.stocks)));
    out.push_str(&format!("  \"kerf\": {}\n", json_string(&job.kerf)));
    out.push('}');
    out
}

/// Parses a project file. Lenient about extra fields and key order;
/// rejects anything that is not an object with the expected string fields.
pub fn read_project_json(text: &str) -> Option<ProjectJob> {
    let fields = parse_object(text)?;
    // A sanity marker is required so we don't try to open unrelated JSON.
    if fields.get("app").map(String::as_str) != Some(APP_ID) {
        return None;
    }
    Some(ProjectJob {
        units: fields.get("units").cloned().unwrap_or_else(|| "mm".into()),
        parts: fields.get("parts")?.clone(),
        stocks: fields.get("stocks").cloned().unwrap_or_default(),
        kerf: fields.get("kerf")?.clone(),
    })
}

/// Extracts `"key": "value"` string pairs from a flat JSON object. Numbers,
/// booleans and nested structures are skipped (the project schema only has
/// string fields plus the version number, which is not needed to load).
fn parse_object(text: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let bytes = text.as_bytes();
    let mut i = 0;
    skip_ws(bytes, &mut i);
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut fields = std::collections::BTreeMap::new();
    loop {
        skip_ws(bytes, &mut i);
        match bytes.get(i) {
            Some(b'}') => return Some(fields),
            Some(b',') => {
                i += 1;
            }
            Some(b'"') => {
                let key = scan_string(text, &mut i)?;
                skip_ws(bytes, &mut i);
                if bytes.get(i) != Some(&b':') {
                    return None;
                }
                i += 1;
                skip_ws(bytes, &mut i);
                if bytes.get(i) == Some(&b'"') {
                    let value = scan_string(text, &mut i)?;
                    fields.insert(key, value);
                } else {
                    // Non-string value (e.g. the version number): skip it.
                    skip_value(bytes, &mut i)?;
                }
            }
            _ => return None,
        }
    }
}

fn skip_ws(bytes: &[u8], i: &mut usize) {
    while matches!(bytes.get(*i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *i += 1;
    }
}

/// Reads a JSON string starting at `bytes[i] == '"'`, advancing `i` past the
/// closing quote. Returns `None` on malformed escapes or truncation.
fn scan_string(text: &str, i: &mut usize) -> Option<String> {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes.get(*i), Some(&b'"'));
    *i += 1;
    let mut out = String::new();
    loop {
        let b = *bytes.get(*i)?;
        match b {
            b'"' => {
                *i += 1;
                return Some(out);
            }
            b'\\' => {
                *i += 1;
                match bytes.get(*i)? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = text.get(*i + 1..*i + 5)?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        *i += 4;
                    }
                    _ => return None,
                }
                *i += 1;
            }
            _ => {
                // Copy the whole UTF-8 sequence (bytes are valid UTF-8 since
                // we always index into the original &str).
                let ch = text[*i..].chars().next()?;
                out.push(ch);
                *i += ch.len_utf8();
            }
        }
    }
}

/// Skips one non-string JSON value (number, true/false/null, nested object
/// or array) so unknown fields don't break the scan.
fn skip_value(bytes: &[u8], i: &mut usize) -> Option<()> {
    match bytes.get(*i)? {
        b'{' | b'[' => {
            let (open, close) = if bytes[*i] == b'{' {
                (b'{', b'}')
            } else {
                (b'[', b']')
            };
            let mut depth = 0usize;
            while let Some(&b) = bytes.get(*i) {
                match b {
                    b'"' => {
                        scan_string_core(bytes, i)?;
                        continue;
                    }
                    b if b == open => depth += 1,
                    b if b == close => {
                        depth -= 1;
                        if depth == 0 {
                            *i += 1;
                            return Some(());
                        }
                    }
                    _ => {}
                }
                *i += 1;
            }
            None
        }
        _ => {
            while let Some(&b) = bytes.get(*i) {
                if matches!(b, b',' | b'}' | b']' | b'\n' | b'\r' | b' ') {
                    break;
                }
                *i += 1;
            }
            Some(())
        }
    }
}

/// Byte-level string scanner (skips the contents); assumes the input is
/// valid UTF-8, which is safe here because escapes are ASCII and raw
/// multi-byte sequences never contain ASCII quotes.
fn scan_string_core(bytes: &[u8], i: &mut usize) -> Option<()> {
    *i += 1; // opening quote
    loop {
        match bytes.get(*i)? {
            b'"' => {
                *i += 1;
                return Some(());
            }
            b'\\' => *i += 2,
            _ => *i += 1,
        }
    }
}

/// JSON string literal with quotes, escaping `"`, `\`, control characters
/// and DEL. Unicode passes through unescaped (files are UTF-8).
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProjectJob {
        ProjectJob {
            units: "mm".into(),
            parts: "600 x 4 Shelf @ply !grain\n197 x 8 Door stile\n# note \"quoted\"".into(),
            stocks: "2400 x 4 @ply\n3000".into(),
            kerf: "3".into(),
        }
    }

    #[test]
    fn round_trips() {
        let json = write_project_json(&sample());
        assert_eq!(read_project_json(&json).unwrap(), sample());
    }

    #[test]
    fn round_trips_unicode_and_escapes() {
        let job = ProjectJob {
            units: "in".into(),
            parts: "600 x 2 B\u{fc}cherregal \u{1f600}\nline\nwith\ttabs \u{7f}".into(),
            stocks: String::new(),
            kerf: "0.25".into(),
        };
        let json = write_project_json(&job);
        assert_eq!(read_project_json(&json).unwrap(), job);
    }

    #[test]
    fn tolerates_extra_fields_and_key_order() {
        let json = concat!(
            "{\n  \"extra\": {\"nested\": [1, 2, \"three\"]},\n",
            "  \"kerf\": \"5\",\n  \"app\": \"tpt-cutlist-optimizer\",\n",
            "  \"version\": 1,\n  \"parts\": \"600 x 1\",\n",
            "  \"flag\": true,\n  \"units\": \"in\"\n}"
        );
        let job = read_project_json(json).unwrap();
        assert_eq!(job.parts, "600 x 1");
        assert_eq!(job.units, "in");
        assert_eq!(job.kerf, "5");
        assert_eq!(job.stocks, "");
    }

    #[test]
    fn rejects_foreign_and_malformed_files() {
        assert_eq!(read_project_json("{\"app\": \"other-tool\", \"parts\": \"1\"}"), None);
        assert_eq!(read_project_json("not json at all"), None);
        assert_eq!(read_project_json("{\"app\": \"tpt-cutlist-optimizer\"}"), None); // no parts
        assert_eq!(read_project_json(""), None);
    }
}
