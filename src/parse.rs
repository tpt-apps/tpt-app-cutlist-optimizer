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
    /// Material name from an `@name` tag (empty = the default material).
    pub material: String,
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
                // Merge duplicates (same length + label + material) by
                // summing qty so the cut report stays tidy.
                if let Some(existing) = parts.iter_mut().find(|p| {
                    p.length == part.length && p.label == part.label && p.material == part.material
                }) {
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
/// A `@name` token tags the part's material (Pro edition); it is pulled out
/// of the label. `line: 0` — the caller stamps the real 1-based line number
/// after merging.
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
        let (material, label) = split_material(tokens.collect());
        return Ok(ParsedPart {
            length,
            qty,
            label,
            material,
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
    let (material, label) = split_material(rest[label_start.min(rest.len())..].to_vec());
    Ok(ParsedPart {
        length,
        qty,
        label,
        material,
        line: 0,
    })
}

fn is_separator_token(token: &str) -> bool {
    matches!(token, "x" | "X" | "*")
}

/// Pulls the first `@name` token out of a label's tokens: it becomes the
/// material name, the remaining tokens re-join as the label.
fn split_material(tokens: Vec<&str>) -> (String, String) {
    let mut material = String::new();
    let mut label_tokens: Vec<&str> = Vec::new();
    for token in tokens {
        if material.is_empty() && token.len() > 1 && token.starts_with('@') {
            material = token[1..].to_string();
        } else {
            label_tokens.push(token);
        }
    }
    (material, label_tokens.join(" "))
}

/// One stock line of the Pro stock textarea.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "pro"), allow(dead_code))]
pub struct ParsedStock {
    pub length: u32,
    /// `x n` suffix: at most `n` bars of this stock ("I have 5 sheets").
    pub max_bars: Option<u32>,
    /// Material name from an `@name` tag (empty = the default material).
    pub material: String,
    /// 1-based source line.
    pub line: usize,
}

/// Parses the Pro stock textarea: one stock per line,
/// `length [x max-bars] [@material]`, e.g. `2400 x 4 @ply`.
/// Duplicate (length, material) lines merge, summing the bar counts.
#[cfg_attr(not(feature = "pro"), allow(dead_code))]
pub fn parse_stocks(text: &str) -> Result<Vec<ParsedStock>, Vec<Issue>> {
    const BAD_LENGTH: &str = "expected a whole number of millimetres, e.g. 2400";
    let mut issues = Vec::new();
    let mut stocks: Vec<ParsedStock> = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let head_end = line.find(char::is_whitespace).unwrap_or(line.len());
        let head = &line[..head_end];
        let mut rest = line[head_end..].split_whitespace().peekable();

        // Leading token: "2400", "2400x4" / "2400X4" / "2400*4".
        let (length, mut max_bars) = if let Some(position) = head.find(['x', 'X', '*']) {
            let length = head[..position].parse::<u32>().ok().filter(|v| *v > 0);
            match length.zip(parse_qty(&head[position + 1..]).ok()) {
                Some((length, max)) => (length, Some(max)),
                None => {
                    issues.push(Issue::at_line(line_number, BAD_LENGTH));
                    continue;
                }
            }
        } else {
            match head.parse::<u32>() {
                Ok(length) if length > 0 => (length, None),
                _ => {
                    issues.push(Issue::at_line(line_number, BAD_LENGTH));
                    continue;
                }
            }
        };

        // Spaced max-bars: "2400 x 4" or "2400 x4".
        if max_bars.is_none() {
            let sep_then_count = rest.peek().is_some_and(|t| is_separator_token(t));
            let glued = rest
                .peek()
                .is_some_and(|t| t.len() > 1 && t.starts_with(['x', 'X', '*']));
            if sep_then_count {
                let _ = rest.next();
                match rest.next().map(parse_qty) {
                    Some(Ok(max)) => max_bars = Some(max),
                    Some(Err(message)) => {
                        issues.push(Issue::at_line(line_number, message));
                        continue;
                    }
                    None => {
                        issues.push(Issue::at_line(
                            line_number,
                            "missing bar count after the separator (e.g. 2400 x 4)",
                        ));
                        continue;
                    }
                }
            } else if glued {
                let glued_token = rest.next().expect("checked by peek");
                match parse_qty(&glued_token[1..]) {
                    Ok(max) => max_bars = Some(max),
                    Err(message) => {
                        issues.push(Issue::at_line(line_number, message));
                        continue;
                    }
                }
            }
        }

        // Material: the first @token among the rest; anything else is junk.
        let mut material = String::new();
        let mut junk: Vec<&str> = Vec::new();
        for token in rest {
            if material.is_empty() && token.len() > 1 && token.starts_with('@') {
                material = token[1..].to_string();
            } else {
                junk.push(token);
            }
        }
        if !junk.is_empty() {
            issues.push(Issue::at_line(
                line_number,
                format!("unexpected '{}' on a stock line", junk.join(" ")),
            ));
            continue;
        }

        // Merge duplicates: same (length, material) sums the bar counts;
        // an unlimited entry dominates a limited one.
        if let Some(existing) = stocks
            .iter_mut()
            .find(|s| s.length == length && s.material == material)
        {
            existing.max_bars = match (existing.max_bars, max_bars) {
                (Some(a), Some(b)) => Some(a.saturating_add(b)),
                _ => None,
            };
        } else {
            stocks.push(ParsedStock {
                length,
                max_bars,
                material,
                line: line_number,
            });
        }
    }

    if issues.is_empty() && stocks.is_empty() {
        issues.push(Issue {
            line: None,
            message: "enter at least one stock length, one per line (e.g. 2400 x 4 @ply)"
                .to_string(),
        });
    }
    if issues.is_empty() {
        Ok(stocks)
    } else {
        Err(issues)
    }
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

    #[test]
    fn material_tag_is_extracted_from_the_label() {
        let parts = parse_parts("600 x 2 shelf @ply\n570 door @pine\n900 drawer").unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.length, p.label.as_str(), p.material.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (600, "shelf", "ply"),
                (570, "door", "pine"),
                (900, "drawer", "")
            ]
        );
    }

    #[test]
    fn material_duplicates_merge_within_the_same_material() {
        let parts = parse_parts("600 x 2 shelf @ply\n600x3 shelf @ply\n600 x 1 shelf @pine")
            .unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.qty, p.material.as_str()))
                .collect::<Vec<_>>(),
            vec![(5, "ply"), (1, "pine")]
        );
    }

    #[test]
    fn stock_lines_parse_with_counts_and_materials() {
        let stocks = parse_stocks("2400 x 4 @ply\n1800 @pine\n3000x2\n# comment\n\n3600 x 4 @ply")
            .unwrap();
        assert_eq!(
            stocks
                .iter()
                .map(|s| (s.length, s.max_bars, s.material.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (2400, Some(4), "ply"),
                (1800, None, "pine"),
                (3000, Some(2), ""),
                (3600, Some(4), "ply"),
            ]
        );
    }

    #[test]
    fn duplicate_stock_lines_merge_bar_counts() {
        let stocks = parse_stocks("2400 x 4 @ply\n2400 x 2 @ply").unwrap();
        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].max_bars, Some(6));
    }

    #[test]
    fn stock_line_errors_carry_line_numbers() {
        let issues = parse_stocks("2400 x 4 @ply\nabc\n2400 x 0\n2400 junk").unwrap_err();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].line, Some(2));
        assert_eq!(issues[1].line, Some(3));
        assert_eq!(issues[2].line, Some(4));
        assert!(issues[2].message.contains("unexpected 'junk'"));
    }

    #[test]
    fn empty_stock_input_reports_one_issue() {
        let issues = parse_stocks("  \n").unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, None);
    }
}


