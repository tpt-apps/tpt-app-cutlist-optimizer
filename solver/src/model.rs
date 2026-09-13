//! Domain model for the 1D cut list optimizer.
//!
//! All lengths are integer stock units (millimetres in the UI) so the
//! accounting is exact — no floating point anywhere in the packing math.
//!
//! Kerf model (matches how a saw actually behaves):
//! - each cut consumes the part's length,
//! - the blade removes `kerf` between two consecutive cuts on the same bar,
//! - `trim` is a fixed loss at the end of every used bar (clean-up cut),
//! - everything left over is `waste` (the offcut).
//!
//! A bar with `n` cuts therefore consumes
//! `sum(part lengths) + (n-1) * kerf + trim` and its waste is
//! `stock_length - consumed`.

use crate::error::SolveError;

/// A requested part: `qty` pieces of `length`, each worth a human label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub length: u32,
    pub qty: u32,
    pub label: String,
}

impl Part {
    pub fn new(length: u32, qty: u32, label: impl Into<String>) -> Self {
        Part {
            length,
            qty,
            label: label.into(),
        }
    }
}

/// The stock the parts are cut from. `max_bars` optionally caps how many
/// bars may be used (e.g. "I only have 5 sheets of this").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockSpec {
    pub length: u32,
    pub max_bars: Option<u32>,
}

impl StockSpec {
    pub fn new(length: u32) -> Self {
        StockSpec {
            length,
            max_bars: None,
        }
    }

    pub fn with_max_bars(length: u32, max_bars: u32) -> Self {
        StockSpec {
            length,
            max_bars: Some(max_bars),
        }
    }
}

/// A complete optimization problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub parts: Vec<Part>,
    pub stock: StockSpec,
    /// Blade / saw kerf width removed between consecutive cuts.
    pub kerf: u32,
    /// Fixed end-of-bar trim loss charged once per used bar.
    pub trim: u32,
}

impl Problem {
    pub fn new(parts: Vec<Part>, stock: StockSpec, kerf: u32, trim: u32) -> Self {
        Problem {
            parts,
            stock,
            kerf,
            trim,
        }
    }

    /// Total number of part instances (sum of quantities).
    pub fn instance_count(&self) -> u32 {
        self.parts.iter().map(|p| p.qty).sum()
    }

    /// Total length of material requested (sum of `length * qty`).
    pub fn parts_length(&self) -> u32 {
        self.parts
            .iter()
            .map(|p| p.length.saturating_mul(p.qty))
            .sum()
    }

    /// Usable capacity of one bar once the end trim is paid.
    pub fn usable_length(&self) -> u32 {
        self.stock.length.saturating_sub(self.trim)
    }

    /// Rejects malformed problems up front so every solver variant can
    /// assume well-formed input and the UI can show a precise message.
    pub fn validate(&self) -> Result<(), SolveError> {
        if self.stock.length == 0 {
            return Err(SolveError::InvalidProblem(
                "stock length must be greater than zero".to_string(),
            ));
        }
        if self.trim >= self.stock.length {
            return Err(SolveError::InvalidProblem(
                "trim must be smaller than the stock length".to_string(),
            ));
        }
        if self.parts.is_empty() {
            return Err(SolveError::InvalidProblem(
                "the job has no parts".to_string(),
            ));
        }
        for (index, part) in self.parts.iter().enumerate() {
            if part.length == 0 {
                return Err(SolveError::InvalidProblem(format!(
                    "part {} ({}) has zero length",
                    index + 1,
                    part.label
                )));
            }
            if part.qty == 0 {
                return Err(SolveError::InvalidProblem(format!(
                    "part {} ({}) requests zero pieces",
                    index + 1,
                    part.label
                )));
            }
            if part.length + self.trim > self.stock.length {
                return Err(SolveError::PartTooLong {
                    index,
                    label: part.label.clone(),
                    length: part.length,
                    stock_length: self.stock.length,
                    available: self.stock.length - self.trim,
                });
            }
        }
        Ok(())
    }
}

/// One placed piece on a bar: which problem part (index into
/// `Problem::parts`), how long the cut is, and how far along the bar it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    pub part: usize,
    pub length: u32,
    /// Distance from the start of the bar to the start of this cut.
    pub offset: u32,
}

/// A single stock length with the cuts placed on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bar {
    pub cuts: Vec<Cut>,
}

impl Bar {
    pub fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// Sum of the placed part lengths (excluding kerf and trim).
    pub fn parts_length(&self) -> u32 {
        self.cuts.iter().map(|c| c.length).sum()
    }

    /// Kerf consumed between consecutive cuts (`(n-1) * kerf`).
    pub fn kerf_length(&self, kerf: u32) -> u32 {
        kerf.saturating_mul(self.cuts.len().saturating_sub(1) as u32)
    }

    /// Total stock consumed by this bar: parts + kerf + the end trim.
    /// Zero for an empty bar (an uncut bar is not charged trim).
    pub fn consumed(&self, kerf: u32, trim: u32) -> u32 {
        if self.cuts.is_empty() {
            0
        } else {
            self.parts_length() + self.kerf_length(kerf) + trim
        }
    }

    /// Unused remainder of the bar (the offcut).
    pub fn waste(&self, stock_length: u32, kerf: u32, trim: u32) -> u32 {
        stock_length.saturating_sub(self.consumed(kerf, trim))
    }

    /// Kerf-aware feasibility check for appending one more cut of `length`.
    ///
    /// After the addition the bar would hold `n+1` cuts, i.e. `n` inter-cut
    /// kerfs plus the end trim:
    /// `parts + length + n * kerf + trim <= stock` (the `n = 0` case has no
    /// kerf and is handled naturally by the same expression).
    pub fn can_fit(&self, length: u32, stock_length: u32, kerf: u32, trim: u32) -> bool {
        let n = self.cuts.len() as u32;
        self.parts_length() + length + kerf * n + trim <= stock_length
    }
}

/// A finished packing: every requested piece placed exactly once.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    pub bars: Vec<Bar>,
    /// Total offcut across all bars (`bars * stock - parts - kerf - trim`).
    pub waste: u32,
    /// `parts_length / (bars * stock_length)` as a fraction in `0.0..=1.0`.
    pub utilization: f64,
}

impl Solution {
    /// Assembles a [`Solution`] from per-bar lists of part indices,
    /// computing kerf-aware offsets, total waste and utilization.
    pub fn build(problem: &Problem, bar_parts: Vec<Vec<usize>>) -> Solution {
        let mut bars = Vec::with_capacity(bar_parts.len());
        for parts_in_bar in bar_parts {
            let mut bar = Bar::default();
            let mut offset = 0u32;
            for (i, part_index) in parts_in_bar.into_iter().enumerate() {
                let length = problem.parts[part_index].length;
                // Kerf sits between cuts: before the first cut there is none.
                if i > 0 {
                    offset += problem.kerf;
                }
                bar.cuts.push(Cut {
                    part: part_index,
                    length,
                    offset,
                });
                offset += length;
            }
            bars.push(bar);
        }

        let stock = problem.stock.length;
        let total_stock = (bars.len() as u64) * (stock as u64);
        let parts_total: u64 = bars.iter().map(|b| b.parts_length() as u64).sum();
        let waste_total: u64 = bars
            .iter()
            .map(|b| b.waste(stock, problem.kerf, problem.trim) as u64)
            .sum();
        let utilization = if total_stock == 0 {
            0.0
        } else {
            parts_total as f64 / total_stock as f64
        };
        Solution {
            bars,
            waste: waste_total.min(u32::MAX as u64) as u32,
            utilization,
        }
    }

    pub fn parts_length(&self) -> u32 {
        self.bars.iter().map(|b| b.parts_length()).sum()
    }

    pub fn bar_count(&self) -> usize {
        self.bars.len()
    }
}

