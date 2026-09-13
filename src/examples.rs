//! Built-in example jobs and the share-link job codec.
//!
//! Kept free of DOM dependencies so the examples are integrity-tested on the
//! host (every example must parse AND solve, in both editions) and the
//! permalink codec is round-trip tested.

#[cfg(test)]
use crate::parse;

#[cfg(not(feature = "pro"))]
pub const SAMPLE_PARTS: &str = "730 x 4 Cabinet side\n560 x 6 Shelf\n396 x 4 Top rail\n197 x 8 Door stile\n640 x 2 Door panel";
#[cfg(not(feature = "pro"))]
pub const SAMPLE_STOCK: &str = "2400";
#[cfg(feature = "pro")]
pub const SAMPLE_PARTS: &str = "730 x 4 Cabinet side @bench\n560 x 6 Shelf @bench\n396 x 4 Top rail @bench\n197 x 8 Door stile @bench\n640 x 2 Door panel @bench";
#[cfg(feature = "pro")]
pub const SAMPLE_STOCKS: &str = "2400 x 4 @bench\n3000 x 2 @bench";
pub const SAMPLE_KERF: &str = "3";

/// Built-in example jobs: (button label, parts text, stock text, kerf mm).
/// The free edition's "stock text" is a single length; the Pro edition's is
/// the full stock textarea content (with materials and bar caps).
#[cfg(not(feature = "pro"))]
pub const EXAMPLES: &[(&str, &str, &str, &str)] = &[
    ("Cabinet job", SAMPLE_PARTS, SAMPLE_STOCK, SAMPLE_KERF),
    ("Shelving", "900 x 6 Shelf\n600 x 4 Side\n400 x 2 Back", "1800", "3"),
    ("Fence rails", "2900 x 2 Rail\n1500 x 12 Paling", "3200", "5"),
];
#[cfg(feature = "pro")]
pub const EXAMPLES: &[(&str, &str, &str, &str)] = &[
    ("Cabinet job", SAMPLE_PARTS, SAMPLE_STOCKS, SAMPLE_KERF),
    (
        "Two materials",
        "600 x 3 Shelf @ply\n400 x 2 Divider @pine\n1200 x 2 Top @ply",
        "2400 x 4 @ply\n1800 x 6 @pine",
        "3",
    ),
    (
        "Limited stock",
        "1500 x 8 Paling @treated\n2900 x 2 Rail @hardwood",
        "3200 x 3 @hardwood\n1600 x 12 @treated",
        "5",
    ),
];

/// Packs a job into a URL-safe permalink payload (`#/j=<payload>`).
/// `unit` is the UI unit the fields were typed in (`"mm"` / `"in"`).
pub fn encode_job(parts: &str, stock: &str, kerf: &str, unit: &str) -> String {
    let joined = format!("{parts}\u{1}{stock}\u{1}{kerf}\u{1}{unit}");
    base64_url_encode(joined.as_bytes())
}

/// Unpacks a permalink payload back into (parts, stock, kerf, unit).
/// Payloads from before the units toggle default to millimetres.
pub fn decode_job(encoded: &str) -> Option<(String, String, String, String)> {
    let joined = String::from_utf8(base64_url_decode(encoded.trim())?).ok()?;
    let mut fields = joined.split('\u{1}');
    let parts = fields.next()?.to_string();
    let stock = fields.next()?.to_string();
    let kerf = fields.next()?.to_string();
    let unit = fields.next().unwrap_or("mm").to_string();
    Some((parts, stock, kerf, unit))
}

const BASE64_TABLE: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64 without padding (`-`/`_` alphabet).
pub fn base64_url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_TABLE[(n >> 18) as usize & 63] as char);
        out.push(BASE64_TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(BASE64_TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(BASE64_TABLE[n as usize & 63] as char);
        }
    }
    out
}

/// Decodes URL-safe base64 (also tolerates the standard `+`/`/` alphabet and
/// `=` padding). Returns `None` on malformed input.
pub fn base64_url_decode(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    }
    let values: Vec<u32> = text
        .bytes()
        .filter(|&byte| byte != b'=')
        .map(value)
        .collect::<Option<Vec<_>>>()?;

    let mut out = Vec::with_capacity(values.len() * 3 / 4 + 1);
    for chunk in values.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut n = chunk[0] << 18;
        n |= chunk.get(1).copied().unwrap_or(0) << 12;
        if chunk.len() > 1 {
            out.push((n >> 16) as u8);
        }
        if chunk.len() > 2 {
            n |= chunk[2] << 6;
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            n |= chunk[3];
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_round_trips_jobs_with_unicode_labels() {
        let (parts, stock, kerf) = (
            "600 x 3 B\u{fc}cherregal @ply\n197 x 8 T\u{fc}r stile",
            "2400 x 4 @ply\n1800",
            "4",
        );
        let encoded = encode_job(parts, stock, kerf, "in");
        assert!(!encoded.contains('+') && !encoded.contains('/'));
        assert_eq!(
            decode_job(&encoded),
            Some((parts.into(), stock.into(), kerf.into(), "in".into()))
        );
    }

    #[test]
    fn decode_defaults_pre_units_payloads_to_millimetres() {
        // Links created before the units toggle carry only three fields.
        let legacy = base64_url_encode("600 x 1\u{1}2400\u{1}3".as_bytes());
        let (_, _, _, unit) = decode_job(&legacy).unwrap();
        assert_eq!(unit, "mm");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert_eq!(decode_job("%%%%"), None);
        assert_eq!(decode_job("a"), None); // single base64 char is malformed
    }

    /// Every built-in example must parse cleanly AND solve — this guards the
    /// adoption path (a broken example is the worst possible first impression).
    #[cfg(not(feature = "pro"))]
    #[test]
    fn every_example_parses_and_solves() {
        for (name, parts_text, stock_text, kerf_text) in EXAMPLES {
            let parsed = parse::parse_parts(parts_text, parse::Units::Millimetres)
                .unwrap_or_else(|issues| panic!("example {name}: {issues:?}"));
            let stock_length: u32 = stock_text
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("example {name}: bad stock length"));
            let problem = tpt_cutlist_solver::Problem::single(
                parsed
                    .iter()
                    .map(|p| tpt_cutlist_solver::Part::new(p.length, p.qty, p.label.clone()))
                    .collect(),
                tpt_cutlist_solver::StockSpec::new(stock_length),
                kerf_text.parse().unwrap(),
                0,
            );
            tpt_cutlist_solver::solve(&problem)
                .unwrap_or_else(|error| panic!("example {name}: {error}"));
        }
    }

    /// Pro edition: examples may mix materials and caps — parse the stock
    /// textarea, map material names to dense ids, and solve.
    #[cfg(feature = "pro")]
    #[test]
    fn every_example_parses_and_solves_pro() {
        for (name, parts_text, stocks_text, kerf_text) in EXAMPLES {
            let parsed = parse::parse_parts(parts_text, parse::Units::Millimetres)
                .unwrap_or_else(|issues| panic!("example {name}: {issues:?}"));
            let parsed_stocks = parse::parse_stocks(stocks_text, parse::Units::Millimetres)
                .unwrap_or_else(|issues| panic!("example {name}: {issues:?}"));

            let mut names: Vec<String> = vec![String::new()];
            for stock in &parsed_stocks {
                if !stock.material.is_empty() && !names.contains(&stock.material) {
                    names.push(stock.material.clone());
                }
            }
            for part in &parsed {
                if !part.material.is_empty() && !names.contains(&part.material) {
                    names.push(part.material.clone());
                }
            }
            let index_of = |material: &str| names.iter().position(|n| n == material).unwrap_or(0);

            let problem = tpt_cutlist_solver::Problem::new(
                parsed
                    .iter()
                    .map(|p| {
                        tpt_cutlist_solver::Part::with_material(
                            p.length,
                            p.qty,
                            p.label.clone(),
                            index_of(&p.material),
                        )
                    })
                    .collect(),
                parsed_stocks
                    .iter()
                    .map(|s| {
                        let spec = tpt_cutlist_solver::StockSpec::new(s.length)
                            .material(index_of(&s.material));
                        match s.max_bars {
                            Some(max) => spec.max_bars(max),
                            None => spec,
                        }
                    })
                    .collect(),
                kerf_text.parse().unwrap(),
                0,
            );
            tpt_cutlist_solver::solve(&problem)
                .unwrap_or_else(|error| panic!("example {name}: {error}"));
        }
    }
}


