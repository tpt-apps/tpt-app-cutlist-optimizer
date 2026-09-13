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
//!
//! Materials and multiple stocks (Pro edition): parts and stocks carry a
//! `material` id; a part may only be cut from stock of the same id. A
//! `Problem` holds any number of stock specs (the free edition passes
//! exactly one). The packing objective is minimum total stock **price**
//! (`StockSpec::price`, which defaults to the stock's length — i.e.
//! "use as little material as possible").

use crate::error::SolveError;

/// A requested part: `qty` pieces of `length`, each worth a human label.
/// `material` restricts which stock the piece may be cut from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub length: u32,
    pub qty: u32,
    pub label: String,
    pub material: usize,
}

impl Part {
    /// A part of the default material (id 0).
    pub fn new(length: u32, qty: u32, label: impl Into<String>) -> Self {
        Part {
            length,
            qty,
            label: label.into(),
            material: 0,
        }
    }

    /// A part of a specific material.
    pub fn with_material(
        length: u32,
        qty: u32,
        label: impl Into<String>,
        material: usize,
    ) -> Self {
        Part {
            length,
            qty,
            label: label.into(),
            material,
        }
    }
}

/// One purchasable stock: `length` bars of one `material`, at `price` per
/// bar, optionally capped at `max_bars` bars. The default price is the
/// stock's length, so "cheapest packing" degenerates to "least material".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockSpec {
    pub length: u32,
    pub max_bars: Option<u32>,
    pub material: usize,
    pub price: u64,
}

impl StockSpec {
    /// Stock of the default material (id 0), priced by its length.
    pub fn new(length: u32) -> Self {
        StockSpec {
            length,
            max_bars: None,
            material: 0,
            price: u64::from(length),
        }
    }

    pub fn with_max_bars(length: u32, max_bars: u32) -> Self {
        StockSpec {
            length,
            max_bars: Some(max_bars),
            material: 0,
            price: u64::from(length),
        }
    }

    /// Builder: set the material id.
    pub fn material(mut self, material: usize) -> Self {
        self.material = material;
        self
    }

    /// Builder: set the per-bar price.
    pub fn price(mut self, price: u64) -> Self {
        self.price = price;
        self
    }

    /// Builder: cap the number of usable bars.
    pub fn max_bars(mut self, max_bars: u32) -> Self {
        self.max_bars = Some(max_bars);
        self
    }
}

/// A complete optimization problem over one or more stock specs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub parts: Vec<Part>,
    pub stocks: Vec<StockSpec>,
    /// Blade / saw kerf width removed between consecutive cuts.
    pub kerf: u32,
    /// Fixed end-of-bar trim loss charged once per used bar.
    pub trim: u32,
}

impl Problem {
    /// A multi-stock problem.
    pub fn new(parts: Vec<Part>, stocks: Vec<StockSpec>, kerf: u32, trim: u32) -> Self {
        Problem {
            parts,
            stocks,
            kerf,
            trim,
        }
    }

    /// Convenience for the single-stock case (free edition).
    pub fn single(parts: Vec<Part>, stock: StockSpec, kerf: u32, trim: u32) -> Self {
        Problem::new(parts, vec![stock], kerf, trim)
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

    /// Usable capacity of one bar of `stock_index` once the end trim is paid.
    pub fn usable_length(&self, stock_index: usize) -> u32 {
        self.stocks[stock_index].length.saturating_sub(self.trim)
    }

    /// Rejects malformed problems up front so every solver variant can
    /// assume well-formed input and the UI can show a precise message.
    pub fn validate(&self) -> Result<(), SolveError> {
        if self.stocks.is_empty() {
            return Err(SolveError::InvalidProblem(
                "the job has no stock to cut from".to_string(),
            ));
        }
        for (index, stock) in self.stocks.iter().enumerate() {
            if stock.length == 0 {
                return Err(SolveError::InvalidProblem(format!(
                    "stock {} has zero length",
                    index + 1
                )));
            }
            if self.trim >= stock.length {
                return Err(SolveError::InvalidProblem(
                    "trim must be smaller than the stock length".to_string(),
                ));
            }
            if stock.max_bars == Some(0) {
                return Err(SolveError::InvalidProblem(format!(
                    "stock {} allows zero bars; remove it instead",
                    index + 1
                )));
            }
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
            // The piece must fit at least one stock of its own material.
            let fits = self.stocks.iter().any(|stock| {
                stock.material == part.material && part.length + self.trim <= stock.length
            });
            if !fits {
                let longest = self
                    .stocks
                    .iter()
                    .filter(|stock| stock.material == part.material)
                    .map(|stock| stock.length)
                    .max()
                    .unwrap_or(0);
                return Err(SolveError::PartTooLong {
                    index,
                    label: part.label.clone(),
                    length: part.length,
                    stock_length: longest,
                    available: longest.saturating_sub(self.trim),
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

/// A single stock length with the cuts placed on it. `stock_index` points
/// into `Problem::stocks` — with multiple stocks, bars can differ in length
/// (and material).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bar {
    pub stock_index: usize,
    pub cuts: Vec<Cut>,
}

impl Bar {
    pub fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// Stock length of this bar (lookup through the problem).
    pub fn stock_length(&self, problem: &Problem) -> u32 {
        problem.stocks[self.stock_index].length
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
    /// Total offcut across all bars (`sum(stock lengths) - parts - kerf - trim`).
    pub waste: u32,
    /// Sum of the per-bar stock prices — the packing objective.
    pub cost: u64,
    /// `parts_length / sum(stock lengths)` as a fraction in `0.0..=1.0`.
    pub utilization: f64,
}

impl Solution {
    /// Assembles a [`Solution`] from per-bar `(stock_index, part indices)`
    /// groups, computing kerf-aware offsets, waste, cost and utilization.
    pub fn build(problem: &Problem, bar_groups: Vec<(usize, Vec<usize>)>) -> Solution {
        let mut bars = Vec::with_capacity(bar_groups.len());
        for (stock_index, parts_in_bar) in bar_groups {
            let mut bar = Bar {
                stock_index,
                cuts: Vec::new(),
            };
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

        let parts_total: u64 = bars.iter().map(|b| b.parts_length() as u64).sum();
        let mut stock_total: u64 = 0;
        let mut waste_total: u64 = 0;
        let mut cost_total: u64 = 0;
        for bar in &bars {
            let stock = &problem.stocks[bar.stock_index];
            stock_total += u64::from(stock.length);
            waste_total += u64::from(bar.waste(stock.length, problem.kerf, problem.trim));
            cost_total += stock.price;
        }
        let utilization = if stock_total == 0 {
            0.0
        } else {
            parts_total as f64 / stock_total as f64
        };
        Solution {
            bars,
            waste: waste_total.min(u32::MAX as u64) as u32,
            cost: cost_total,
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

