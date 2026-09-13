//! Parsing and validation of the parts textarea.
//!
//! Format — one part per line, `length x quantity` with an optional label:
//!
//! ```text
//! 570 x 4 Side panel
//! 560x6 Shelf
//! 396          (quantity defaults to 1)
//! 197 * 8 Stile
//! # comment lines are ignored
//! ```
//!
//! Errors are collected per line (all reported at once, with line numbers)
//! so the UI can show a complete inline error list instead of one at a time.

/// Free-tier guard: keeps a pasted 100k-line file from freezing the tab.
pub const MAX_PART_LINES: usize = 2000;

/// One user-facing validation issue, tied to a line of the textarea
/// (`line` is 1-based; `None` for whole-form issues).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub line: Option<usize>,
    pub message: String,
}

impl Issue {
    fn at_line(line: usize, message: impl Into<String>) -> Self {
        Issue {
            line: Some(line),
            message: message.into(),
        }
    }
}

/// A successfully parsed part line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPart {
    pub length: u32,
    pub qty: u32,
    pub label: String,
    /// 1-based source line, used for "part longer than stock" messages.
    pub line: usize,
}

/// Parses the whole textarea. Returns all parts (duplicates merged by
/// length + label, quantities summed) or every issue found.
pub fn parse_parts(text: &str) -> Result<Vec<ParsedPart>, Vec<Issue>> {
    let mut issues = Vec::new();
    let mut parts: Vec<ParsedPart> = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        if parts.len() >= MAX_PART_LINES {
            issues.push(Issue {
                line: None,
                message: format!("free edition handles up to {MAX_PART_LINES} part lines per job"),
            });
            break;
        }
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Ok(part) => {
                // Merge duplicates (same length + label) by summing qty so
                // the cut report stays tidy.
                if let Some(existing) = parts
                    .iter_mut()
                    .find(|p| p.length == part.length && p.label == part.label)
                {
                    existing.qty += part.qty;
                } else {
                    parts.push(part);
                }
            }
            Err(message) => issues.push(Issue::at_line(line_number, message)),
        }
    }

    if issues.is_empty() && parts.is_empty() {
        issues.push(Issue {
            line: None,
            message: "enter at least one part, one per line (e.g. 570 x 4 Side panel)".to_string(),
        });
    }

    if issues.is_empty() {
        Ok(parts)
    } else {
        Err(issues)
    }
}

/// Parses one `length x qty [label]` line, accepting the natural spellings:
/// `570 x 4 Side panel`, `570x4 Side panel`, `570*4`, `570 4`, `570`, `570 rail`.
/// `line: 0` — the caller stamps the real 1-based line number after merging.
fn parse_line(line: &str) -> Result<ParsedPart, String> {
    let bad_length = || "expected a whole number of millimetres, e.g. 570".to_string();
    let mut tokens = line.split_whitespace();
    let head = tokens.next().ok_or_else(|| "empty line".to_string())?;

    // Glued separator: "570x4" / "570X4" / "570*4".
    if let Some(position) = head.find(['x', 'X', '*']) {
        let length: u32 = head[..position].parse().map_err(|_| bad_length())?;
        if length == 0 {
            return Err("length must be at least 1 mm".to_string());
        }
        let qty = parse_qty(&head[position + 1..])?;
        let label = tokens.collect::<Vec<_>>().join(" ");
        return Ok(ParsedPart {
            length,
            qty,
            label,
            line: 0,
        });
    }

    let length: u32 = head.parse().map_err(|_| bad_length())?;
    if length == 0 {
        return Err("length must be at least 1 mm".to_string());
    }

    // Spaced forms: "570 x 4", "570 * 4", "570 x4", "570 4", "570 rail", "570".
    let rest: Vec<&str> = tokens.collect();
    let (qty, label_start): (u32, usize) = match rest.first() {
        Some(first) if is_separator_token(first) => {
            let qty_text = rest
                .get(1)
                .copied()
                .ok_or_else(|| "missing quantity after the separator (e.g. 570 x 4)".to_string())?;
            (parse_qty(qty_text)?, 2)
        }
        Some(first) if first.len() > 1 && first.starts_with(['x', 'X', '*']) => {
            (parse_qty(&first[1..])?, 1)
        }
        Some(first) if first.parse::<u32>().is_ok() => (parse_qty(first)?, 1),
        Some(_) => (1, 0),
        None => (1, 0),
    };
    let label = rest[label_start.min(rest.len())..].join(" ");
    Ok(ParsedPart {
        length,
        qty,
        label,
        line: 0,
    })
}

fn is_separator_token(token: &str) -> bool {
    matches!(token, "x" | "X" | "*")
}

fn parse_qty(text: &str) -> Result<u32, String> {
    let qty: u32 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a quantity (whole number, e.g. 4)"))?;
    if qty == 0 {
        return Err("quantity must be at least 1".to_string());
    }
    Ok(qty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(parts: &[ParsedPart]) -> Vec<(u32, u32, String)> {
        parts
            .iter()
            .map(|p| (p.length, p.qty, p.label.clone()))
            .collect()
    }

    #[test]
    fn parses_length_x_qty_with_label() {
        let parts = parse_parts("570 x 4 Side panel\n560x6 Shelf\n396 * 2 Rail").unwrap();
        assert_eq!(
            labels(&parts),
            vec![
                (570, 4, "Side panel".to_string()),
                (560, 6, "Shelf".to_string()),
                (396, 2, "Rail".to_string())
            ]
        );
    }

    #[test]
    fn bare_length_defaults_to_one() {
        let parts = parse_parts("396\n600 rail").unwrap();
        assert_eq!(
            labels(&parts),
            vec![(396, 1, String::new()), (600, 1, "rail".to_string())]
        );
    }

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        let parts = parse_parts("\n# a comment\n  \n600 x 2 shelf\n# another").unwrap();
        assert_eq!(labels(&parts), vec![(600, 2, "shelf".to_string())]);
    }

    #[test]
    fn duplicate_lines_merge() {
        let parts = parse_parts("600 x 2 shelf\n600x3 shelf").unwrap();
        assert_eq!(labels(&parts), vec![(600, 5, "shelf".to_string())]);
    }

    #[test]
    fn collects_all_line_errors_with_numbers() {
        let issues = parse_parts("600 x 2 shelf\nabc\n700 x 0 oops\n0 x 1 nope").unwrap_err();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].line, Some(2));
        assert_eq!(issues[1].line, Some(3));
        assert_eq!(issues[2].line, Some(4));
        assert!(issues[0].message.contains("whole number"));
        assert!(issues[1].message.contains("at least 1"));
        assert!(issues[2].message.contains("at least 1 mm"));
    }

    #[test]
    fn empty_input_reports_one_issue() {
        let issues = parse_parts("  \n# only comments\n").unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, None);
    }

    #[test]
    fn free_tier_line_cap_is_enforced() {
        let text = (0..MAX_PART_LINES + 10)
            .map(|i| format!("{} x 1 part{i}", 100 + i % 400))
            .collect::<Vec<_>>()
            .join("\n");
        let issues = parse_parts(&text).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.line.is_none() && issue.message.contains("up to")));
    }

    #[test]
    fn lowercase_x_and_star_separators_work_glued() {
        let parts = parse_parts("570X4 panel\n600*2 rail").unwrap();
        assert_eq!(
            labels(&parts),
            vec![
                (570, 4, "panel".to_string()),
                (600, 2, "rail".to_string())
            ]
        );
    }
}


