//! Parsing and validation of the parts and stocks textareas, plus the
//! length-formatting helpers shared by every display surface.
//!
//! Part format — one part per line, `length x quantity` with an optional
//! label:
//!
//! ```text
//! 570 x 4 Side panel
//! 560x6 Shelf
//! 396          (quantity defaults to 1)
//! 197 * 8 Stile
//! 600, 4, Shelf     (pasted spreadsheet rows also work)
//! # comment lines are ignored
//! ```
//!
//! Tags: `@name` marks the part's material (Pro edition) and `!grain` notes
//! that grain runs along the part's length (informational; shown in the
//! placement table and print sheet).
//!
//! Lengths are entered in the UI's selected unit ([`Units`]) but the solver
//! always works in integer millimetres — every parsed length is converted
//! and rounded on the way in.
//!
//! Errors are collected per line (all reported at once, with line numbers)
//! so the UI can show a complete inline error list instead of one at a time.

/// Millimetres per inch (the international inch, 25.4 mm exactly).
pub const MM_PER_INCH: f64 = 25.4;

/// The UI's length unit. Input parsing and every display surface follow it;
/// the solver stays in integer millimetres regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Millimetres,
    Inches,
}

impl Units {
    /// Conversion factor from this unit to millimetres.
    fn factor(self) -> f64 {
        match self {
            Units::Millimetres => 1.0,
            Units::Inches => MM_PER_INCH,
        }
    }

    /// Converts a measured length to integer millimetres (rounded).
    pub fn to_mm(self, value: f64) -> u32 {
        if !value.is_finite() || value < 0.0 {
            return 0;
        }
        (value * self.factor()).round().min(f64::from(u32::MAX)) as u32
    }

    /// The canonical storage/URL name (`"mm"` / `"in"`).
    pub fn name(self) -> &'static str {
        match self {
            Units::Millimetres => "mm",
            Units::Inches => "in",
        }
    }

    /// Human-readable name for hints ("millimetres" / "inches").
    pub fn long_name(self) -> &'static str {
        match self {
            Units::Millimetres => "millimetres",
            Units::Inches => "inches",
        }
    }

    /// Parses a unit name (URL payload, project file, storage). Unknown
    /// names fall back to millimetres.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "in" | "inch" | "inches" | "imperial" => Units::Inches,
            _ => Units::Millimetres,
        }
    }
}

/// Formats a length for display: grouped millimetres ("1,234 mm") or inches
/// to two decimals ("48.58 in").
pub fn fmt_len(mm: u32, units: Units) -> String {
    match units {
        Units::Millimetres => format!("{} mm", grouped(u64::from(mm))),
        Units::Inches => format!("{:.2} in", f64::from(mm) / MM_PER_INCH),
    }
}

/// Formats a length for an input field or generated line: bare number in
/// the given unit, no grouping, no suffix.
pub fn fmt_len_plain(mm: u32, units: Units) -> String {
    match units {
        Units::Millimetres => mm.to_string(),
        Units::Inches => format!("{:.2}", f64::from(mm) / MM_PER_INCH),
    }
}

/// Thousands-separated integer, e.g. 19240 -> "19,240".
pub fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

/// Input guard: keeps a pasted 100k-line file from freezing the tab
/// (2,000 lines on the free edition, 10,000 on Pro).
#[cfg(not(feature = "pro"))]
pub const MAX_PART_LINES: usize = 2000;
#[cfg(feature = "pro")]
pub const MAX_PART_LINES: usize = 10000;

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
    /// Grain runs along the part's length (`!grain` tag).
    pub grain: bool,
    /// 1-based source line, used for "part longer than stock" messages.
    pub line: usize,
}

/// Parses the whole textarea. Returns all parts (duplicates merged by
/// length + label + material, quantities summed) or every issue found.
pub fn parse_parts(text: &str, units: Units) -> Result<Vec<ParsedPart>, Vec<Issue>> {
    let mut issues = Vec::new();
    let mut parts: Vec<ParsedPart> = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        if parts.len() >= MAX_PART_LINES {
            issues.push(Issue {
                line: None,
                message: format!(
                    "this edition handles up to {MAX_PART_LINES} part lines per job"
                ),
            });
            break;
        }
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line, units) {
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
/// `570 x 4 Side panel`, `570x4 Side panel`, `570*4`, `570 4`, `570`, `570 rail`,
/// and pasted spreadsheet rows (`570, 4, Side panel`). A `@name` token tags
/// the part's material, `!grain` notes grain along the length. `line: 0` —
/// the caller stamps the real 1-based line number after merging.
fn parse_line(line: &str, units: Units) -> Result<ParsedPart, String> {
    let bad_length = || match units {
        Units::Millimetres => "expected a whole number of millimetres, e.g. 570".to_string(),
        Units::Inches => "expected a length in inches, e.g. 23.5".to_string(),
    };
    // Pasted spreadsheet rows: "600, 4, Shelf" parses like "600 4 Shelf".
    let line: &str = &line.replace(',', " ");
    let mut tokens = line.split_whitespace();
    let head = tokens.next().ok_or_else(|| "empty line".to_string())?;

    // Glued separator: "570x4" / "570X4" / "570*4".
    if let Some(position) = head.find(['x', 'X', '*']) {
        let length = parse_length(&head[..position], units).map_err(|_| bad_length())?;
        if length == 0 {
            return Err("length must be at least 1 mm".to_string());
        }
        let qty = parse_qty(&head[position + 1..])?;
        let (material, grain, label) = split_tags(tokens.collect());
        return Ok(ParsedPart {
            length,
            qty,
            label,
            material,
            grain,
            line: 0,
        });
    }

    let length = parse_length(head, units).map_err(|_| bad_length())?;
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
    let (material, grain, label) = split_tags(rest[label_start.min(rest.len())..].to_vec());
    Ok(ParsedPart {
        length,
        qty,
        label,
        material,
        grain,
        line: 0,
    })
}

/// Parses a length token in the given unit, returning integer millimetres
/// (used by the app for kerf, stock fields and the calibration wizard).
pub fn parse_len(text: &str, units: Units) -> Option<u32> {
    parse_length(text, units).ok()
}

/// Parses a length token in the given unit, returning integer millimetres.
fn parse_length(text: &str, units: Units) -> Result<u32, String> {
    // Spreadsheet exports often carry a unit suffix ("600mm", "23.5in").
    let cleaned = text.trim().trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let value: f64 = cleaned.parse().map_err(|_| format!("'{text}'"))?;
    Ok(units.to_mm(value))
}

fn is_separator_token(token: &str) -> bool {
    matches!(token, "x" | "X" | "*")
}

/// Pulls the `@name` material tag and the `!grain` flag out of a label's
/// tokens; the remaining tokens re-join as the label.
fn split_tags(tokens: Vec<&str>) -> (String, bool, String) {
    let mut material = String::new();
    let mut grain = false;
    let mut label_tokens: Vec<&str> = Vec::new();
    for token in tokens {
        if material.is_empty() && token.len() > 1 && token.starts_with('@') {
            material = token[1..].to_string();
        } else if !grain && token == "!grain" {
            grain = true;
        } else {
            label_tokens.push(token);
        }
    }
    (material, grain, label_tokens.join(" "))
}

/// Re-emits parsed parts as canonical textarea lines in the given unit
/// (used by the units toggle, example loading and CSV import).
pub fn parts_to_lines(parts: &[ParsedPart], units: Units) -> String {
    parts
        .iter()
        .map(|p| {
            let mut line = format!("{} x {}", fmt_len_plain(p.length, units), p.qty);
            if !p.label.is_empty() {
                line.push(' ');
                line.push_str(&p.label);
            }
            if !p.material.is_empty() {
                line.push_str(" @");
                line.push_str(&p.material);
            }
            if p.grain {
                line.push_str(" !grain");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
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
pub fn parse_stocks(text: &str, units: Units) -> Result<Vec<ParsedStock>, Vec<Issue>> {
    let bad_length = || match units {
        Units::Millimetres => "expected a whole number of millimetres, e.g. 2400".to_string(),
        Units::Inches => "expected a length in inches, e.g. 94.5".to_string(),
    };
    let mut issues = Vec::new();
    let mut stocks: Vec<ParsedStock> = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.replace(',', " ");
        let line = line.trim();

        let head_end = line.find(char::is_whitespace).unwrap_or(line.len());
        let head = &line[..head_end];
        let mut rest = line[head_end..].split_whitespace().peekable();

        // Leading token: "2400", "2400x4" / "2400X4" / "2400*4".
        let (length, mut max_bars) = if let Some(position) = head.find(['x', 'X', '*']) {
            let length = parse_length(&head[..position], units).ok().filter(|v| *v > 0);
            match length.zip(parse_qty(&head[position + 1..]).ok()) {
                Some((length, max)) => (length, Some(max)),
                None => {
                    issues.push(Issue::at_line(line_number, bad_length()));
                    continue;
                }
            }
        } else {
            match parse_length(head, units) {
                Ok(length) if length > 0 => (length, None),
                _ => {
                    issues.push(Issue::at_line(line_number, bad_length()));
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

/// Re-emits parsed stocks as canonical textarea lines in the given unit.
#[cfg_attr(not(feature = "pro"), allow(dead_code))]
pub fn stocks_to_lines(stocks: &[ParsedStock], units: Units) -> String {
    stocks
        .iter()
        .map(|s| {
            let mut line = fmt_len_plain(s.length, units);
            if let Some(max) = s.max_bars {
                line.push_str(&format!(" x {max}"));
            }
            if !s.material.is_empty() {
                line.push_str(" @");
                line.push_str(&s.material);
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses CSV text into rows of cells (RFC 4180-style: double quotes escape
/// commas and newlines, `""` is a literal quote). Blank trailing lines are
/// skipped. Kept tiny and dependency-free — the inputs are hand-typed or
/// spreadsheet exports, not adversarial data.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    #[allow(clippy::while_let_on_iterator)]
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        cell.push('"');
                        let _ = chars.next();
                    } else {
                        in_quotes = false;
                    }
                }
                _ => cell.push(c),
            }
        } else {
            match c {
                '"' if cell.is_empty() => in_quotes = true,
                ',' => {
                    row.push(std::mem::take(&mut cell));
                }
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut cell));
                    if row.iter().any(|c| !c.trim().is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                _ => cell.push(c),
            }
        }
    }
    // Final cell/row without a trailing newline.
    row.push(cell);
    if row.iter().any(|c| !c.trim().is_empty()) {
        rows.push(row);
    }
    rows
}

/// Converts pasted/uploaded CSV into canonical part lines. Columns are
/// `length, qty, label[, material]`; a non-numeric first row is treated as
/// a header and skipped. Lengths are interpreted in `units`. `#`-prefixed
/// rows are comments.
pub fn csv_to_parts_lines(
    text: &str,
    units: Units,
    materials: bool,
) -> Result<String, Vec<Issue>> {
    let rows = parse_csv(text);
    let mut issues = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        let first = row.first().map(String::as_str).unwrap_or_default().trim();
        if first.is_empty() || first.starts_with('#') {
            continue;
        }
        let length_cell = first.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let Ok(length_value) = length_cell.parse::<f64>() else {
            // Header row (first row only) or junk — report junk, skip headers.
            if index == 0 {
                continue;
            }
            issues.push(Issue::at_line(
                row_number,
                format!("'{first}' is not a length"),
            ));
            continue;
        };
        let length = units.to_mm(length_value);
        if length == 0 {
            issues.push(Issue::at_line(row_number, "length must be at least 1 mm"));
            continue;
        }

        let qty = match row.get(1).map(String::as_str).unwrap_or_default().trim() {
            "" => 1,
            qty_text => match qty_text.parse::<u32>() {
                Ok(q) if q > 0 => q,
                _ => {
                    issues.push(Issue::at_line(
                        row_number,
                        format!("'{qty_text}' is not a quantity (whole number)"),
                    ));
                    continue;
                }
            },
        };
        let label = row.get(2).map(String::as_str).unwrap_or_default().trim();
        let material = if materials {
            row.get(3)
                .map(String::as_str)
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-")
        } else {
            String::new()
        };

        let mut line = format!("{} x {}", fmt_len_plain(length, units), qty);
        if !label.is_empty() {
            line.push(' ');
            line.push_str(label);
        }
        if !material.is_empty() {
            line.push_str(" @");
            line.push_str(&material);
        }
        lines.push(line);
    }

    if issues.is_empty() && lines.is_empty() {
        issues.push(Issue {
            line: None,
            message: "no usable CSV rows found — expected columns length, qty, label".to_string(),
        });
    }
    if issues.is_empty() {
        Ok(lines.join("\n"))
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

    fn mm(text: &str) -> Result<Vec<ParsedPart>, Vec<Issue>> {
        parse_parts(text, Units::Millimetres)
    }

    fn labels(parts: &[ParsedPart]) -> Vec<(u32, u32, String)> {
        parts
            .iter()
            .map(|p| (p.length, p.qty, p.label.clone()))
            .collect()
    }

    #[test]
    fn parses_length_x_qty_with_label() {
        let parts = mm("570 x 4 Side panel\n560x6 Shelf\n396 * 2 Rail").unwrap();
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
        let parts = mm("396\n600 rail").unwrap();
        assert_eq!(
            labels(&parts),
            vec![(396, 1, String::new()), (600, 1, "rail".to_string())]
        );
    }

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        let parts = mm("\n# a comment\n  \n600 x 2 shelf\n# another").unwrap();
        assert_eq!(labels(&parts), vec![(600, 2, "shelf".to_string())]);
    }

    #[test]
    fn duplicate_lines_merge() {
        let parts = mm("600 x 2 shelf\n600x3 shelf").unwrap();
        assert_eq!(labels(&parts), vec![(600, 5, "shelf".to_string())]);
    }

    #[test]
    fn collects_all_line_errors_with_numbers() {
        let issues = mm("600 x 2 shelf\nabc\n700 x 0 oops\n0 x 1 nope").unwrap_err();
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
        let issues = mm("  \n# only comments\n").unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, None);
    }

    #[test]
    fn free_tier_line_cap_is_enforced() {
        let text = (0..MAX_PART_LINES + 10)
            .map(|i| format!("{} x 1 part{i}", 100 + i % 400))
            .collect::<Vec<_>>()
            .join("\n");
        let issues = mm(&text).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.line.is_none() && issue.message.contains("up to")));
    }

    #[test]
    fn lowercase_x_and_star_separators_work_glued() {
        let parts = mm("570X4 panel\n600*2 rail").unwrap();
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
        let parts = mm("600 x 2 shelf @ply\n570 door @pine\n900 drawer").unwrap();
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
        let parts = mm("600 x 2 shelf @ply\n600x3 shelf @ply\n600 x 1 shelf @pine").unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.qty, p.material.as_str()))
                .collect::<Vec<_>>(),
            vec![(5, "ply"), (1, "pine")]
        );
    }

    #[test]
    fn grain_tag_is_extracted() {
        let parts = mm("600 x 2 shelf !grain\n400 rail\n240 x 1 Top !grain @ply").unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.label.as_str(), p.grain, p.material.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("shelf", true, ""),
                ("rail", false, ""),
                ("Top", true, "ply")
            ]
        );
    }

    #[test]
    fn comma_separated_rows_parse_like_spaced_ones() {
        let parts = mm("600, 4, Shelf\n570,2,Side panel\n396,1,@ply").unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.length, p.qty, p.label.as_str(), p.material.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (600, 4, "Shelf", ""),
                (570, 2, "Side panel", ""),
                (396, 1, "", "ply")
            ]
        );
    }

    #[test]
    fn inches_are_converted_to_millimetres() {
        let parts = parse_parts("23.5 x 2 Shelf\n1 in\n96 x 1 Rail", Units::Inches).unwrap();
        assert_eq!(parts[0].length, 597); // 23.5 * 25.4 = 596.9 -> 597
        assert_eq!(parts[1].length, 25); // 25.4 -> 25
        assert_eq!(parts[2].length, 2438); // 96 * 25.4 = 2438.4 -> 2438
    }

    #[test]
    fn unit_suffixes_are_stripped() {
        // The suffix is stripped, not honoured — the UI's selected unit
        // governs the conversion.
        let parts = mm("600mm x 2 shelf").unwrap();
        assert_eq!(parts[0].length, 600);
        let parts = parse_parts("23.5in rail", Units::Inches).unwrap();
        assert_eq!(parts[0].length, 597);
    }

    #[test]
    fn inch_error_messages_mention_inches() {
        let issues = parse_parts("abc x 2", Units::Inches).unwrap_err();
        assert!(issues[0].message.contains("inches"));
    }

    #[test]
    fn parts_round_trip_through_lines() {
        let text = "600 x 4 Shelf @ply !grain\n197 x 8 Door stile";
        let parts = mm(text).unwrap();
        assert_eq!(parts_to_lines(&parts, Units::Millimetres), text);
        let inches = parts_to_lines(&parts, Units::Inches);
        assert_eq!(parts_to_lines(&parse_parts(&inches, Units::Inches).unwrap(), Units::Inches), inches);
        assert!(inches.starts_with("23.62 x 4"));
    }

    #[test]
    fn stock_lines_parse_with_counts_and_materials() {
        let stocks = parse_stocks(
            "2400 x 4 @ply\n1800 @pine\n3000x2\n# comment\n\n3600 x 4 @ply",
            Units::Millimetres,
        )
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
        let stocks =
            parse_stocks("2400 x 4 @ply\n2400 x 2 @ply", Units::Millimetres).unwrap();
        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].max_bars, Some(6));
    }

    #[test]
    fn stock_line_errors_carry_line_numbers() {
        let issues = parse_stocks(
            "2400 x 4 @ply\nabc\n2400 x 0\n2400 junk",
            Units::Millimetres,
        )
        .unwrap_err();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].line, Some(2));
        assert_eq!(issues[1].line, Some(3));
        assert_eq!(issues[2].line, Some(4));
        assert!(issues[2].message.contains("unexpected 'junk'"));
    }

    #[test]
    fn empty_stock_input_reports_one_issue() {
        let issues = parse_stocks("  \n", Units::Millimetres).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, None);
    }

    #[test]
    fn stocks_parse_in_inches_and_round_trip() {
        let stocks = parse_stocks("94.5 x 4 @ply", Units::Inches).unwrap();
        assert_eq!(stocks[0].length, 2400); // 94.5 * 25.4 = 2400.3 -> 2400
        let lines = stocks_to_lines(&stocks, Units::Inches);
        assert_eq!(lines, "94.49 x 4 @ply"); // re-emitted from the rounded mm
    }

    #[test]
    fn csv_rows_become_part_lines() {
        let csv = "Length,Qty,Label,Material\n600,4,Shelf,ply\n570,2,Side\n300\n# note,1,x";
        let lines = csv_to_parts_lines(csv, Units::Millimetres, true).unwrap();
        assert_eq!(
            lines,
            "600 x 4 Shelf @ply\n570 x 2 Side\n300 x 1"
        );
    }

    #[test]
    fn csv_quoted_labels_and_bad_rows_report() {
        let csv = "600,2,\"Shelf, oak\"\njunk,1\n400,x,Bad qty";
        let issues = csv_to_parts_lines(csv, Units::Millimetres, false).unwrap_err();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].line, Some(2));
        assert_eq!(issues[1].line, Some(3));
    }

    #[test]
    fn csv_empty_input_reports_one_issue() {
        let issues = csv_to_parts_lines("Length,Qty\n", Units::Millimetres, false).unwrap_err();
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn fmt_helpers() {
        assert_eq!(fmt_len(1234, Units::Millimetres), "1,234 mm");
        assert_eq!(fmt_len(2400, Units::Inches), "94.49 in");
        assert_eq!(fmt_len_plain(2400, Units::Millimetres), "2400");
        assert_eq!(fmt_len_plain(597, Units::Inches), "23.50");
        assert_eq!(grouped(19240), "19,240");
    }

    #[test]
    fn units_names_round_trip() {
        assert_eq!(Units::from_name("in"), Units::Inches);
        assert_eq!(Units::from_name("Inches"), Units::Inches);
        assert_eq!(Units::from_name("mm"), Units::Millimetres);
        assert_eq!(Units::from_name("nonsense"), Units::Millimetres);
        assert_eq!(Units::Inches.name(), "in");
    }
}
