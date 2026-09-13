//! Baseline packing: best-fit-decreasing (BFD) over multiple stocks.
//!
//! Deterministic by construction: instances are expanded in input order and
//! stably sorted by decreasing length, ties keep input order, and every
//! choice picks the *first* optimum in a fixed scan order. Same input in,
//! same bars out — on every platform, every run.
//!
//! Multi-stock rules (free edition passes a single stock, so it behaves
//! exactly like classic BFD):
//! - a part may only go onto a bar of its own material,
//! - an open bar is chosen by smallest leftover after the cut (first bar
//!   wins on ties),
//! - a new bar is opened on the *smallest* stock length that can take the
//!   part (first index wins on ties) — the "smallest adequate stock" rule
//!   that keeps cheap offcuts in play,
//! - per-stock `max_bars` caps are respected while opening bars.

use crate::error::SolveError;
use crate::model::{Problem, Solution};

/// Which stock a new bar is opened on when several can take the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StockPreference {
    /// Smallest adequate stock: keeps cheap offcuts in play.
    Smallest,
    /// Largest stock: lets long pieces pair up on one bar.
    Largest,
}

/// Solves `problem` with deterministic best-fit-decreasing packing.
///
/// This is the free edition's engine: fast, predictable, and a strict upper
/// bound the improvement pass (Pro) can only improve on. Both stock
/// preferences are packed and the cheaper feasible one wins.
pub fn solve(problem: &Problem) -> Result<Solution, SolveError> {
    problem.validate()?;
    let order = decreasing_instances(problem);
    let bars = best_of_preferences(problem, &order)?;
    Ok(Solution::build(problem, bars))
}

/// Packs `order` under both stock preferences and returns the cheaper
/// packing that respects every `max_bars` cap.
pub(crate) fn best_of_preferences(
    problem: &Problem,
    order: &[usize],
) -> Result<Vec<(usize, Vec<usize>)>, SolveError> {
    let smallest = best_fit_pack(problem, order, StockPreference::Smallest);
    let largest = best_fit_pack(problem, order, StockPreference::Largest);
    match (smallest, largest) {
        (Ok(s), Ok(l)) => {
            if packing_cost(problem, &s) <= packing_cost(problem, &l) {
                Ok(s)
            } else {
                Ok(l)
            }
        }
        (Ok(s), Err(_)) => Ok(s),
        (Err(_), Ok(l)) => Ok(l),
        // Both infeasible: prefer the OutOfStock report.
        (Err(e), Err(_)) => Err(e),
    }
}

/// Total stock price of a packing — the objective being minimized.
pub(crate) fn packing_cost(problem: &Problem, bars: &[(usize, Vec<usize>)]) -> u64 {
    bars.iter().map(|(si, _)| problem.stocks[*si].price).sum()
}

/// Expands the problem's quantities into part indices in deterministic
/// best-fit-decreasing order: longest first, ties keep input order.
pub(crate) fn decreasing_instances(problem: &Problem) -> Vec<usize> {
    let mut instances = Vec::with_capacity(problem.instance_count() as usize);
    for (index, part) in problem.parts.iter().enumerate() {
        for _ in 0..part.qty {
            instances.push(index);
        }
    }
    // `sort_by` is stable, so equal-length parts keep their input order.
    instances.sort_by(|&a, &b| problem.parts[b].length.cmp(&problem.parts[a].length));
    instances
}

/// An open (partially filled) bar during packing. `stock_index` fixes the
/// bar's stock length, material and price.
#[derive(Debug, Clone)]
pub(crate) struct OpenBar {
    pub stock_index: usize,
    pub items: Vec<usize>,
    parts_len: u32,
    count: u32,
}

impl OpenBar {
    pub(crate) fn new(stock_index: usize) -> Self {
        OpenBar {
            stock_index,
            items: Vec::new(),
            parts_len: 0,
            count: 0,
        }
    }

    fn stock_length(&self, problem: &Problem) -> u32 {
        problem.stocks[self.stock_index].length
    }

    fn stock_material(&self, problem: &Problem) -> usize {
        problem.stocks[self.stock_index].material
    }

    /// Leftover stock after appending `length`, if it fits.
    /// `None` when the cut does not fit.
    pub(crate) fn leftover_after(&self, length: u32, problem: &Problem) -> Option<u32> {
        if self.can_fit(length, problem) {
            let consumed = self.parts_len + length + problem.kerf * self.count + problem.trim;
            Some(self.stock_length(problem) - consumed)
        } else {
            None
        }
    }

    pub(crate) fn can_fit(&self, length: u32, problem: &Problem) -> bool {
        self.parts_len + length + problem.kerf * self.count + problem.trim
            <= self.stock_length(problem)
    }

    pub(crate) fn add(&mut self, part: usize, length: u32) {
        self.items.push(part);
        self.parts_len += length;
        self.count += 1;
    }
}

/// Picks the stock index for a **new** bar according to `preference`, among
/// the part's material's stocks that can take `length` and have not hit
/// their `max_bars` cap. Errors with [`SolveError::OutOfStock`] when every
/// qualifying stock is capped out (post-validation, a fitting stock always
/// exists — the cap is the only way to end up here).
fn open_stock_for(
    problem: &Problem,
    length: u32,
    material: usize,
    bars: &[OpenBar],
    preference: StockPreference,
) -> Result<usize, SolveError> {
    let mut best: Option<(usize, u32)> = None;
    let mut capped: Option<(u32, u32)> = None; // (needed, available) of a capped stock
    for (index, stock) in problem.stocks.iter().enumerate() {
        if stock.material != material {
            continue;
        }
        if length + problem.trim > stock.length {
            continue;
        }
        if let Some(max) = stock.max_bars {
            let used = bars.iter().filter(|bar| bar.stock_index == index).count() as u32;
            if used >= max {
                capped = capped.or(Some((used + 1, max)));
                continue;
            }
        }
        let better = match best {
            None => true,
            Some((_, best_length)) => match preference {
                StockPreference::Smallest => stock.length < best_length,
                StockPreference::Largest => stock.length > best_length,
            },
        };
        if better {
            best = Some((index, stock.length));
        }
    }
    best.map(|(index, _)| index)
        .ok_or_else(|| match capped {
            Some((needed, available)) => SolveError::OutOfStock { needed, available },
            // Unreachable post-validation; keep a sane error regardless.
            None => SolveError::InvalidProblem(format!(
                "no stock of material {material} can take a {length} mm part"
            )),
        })
}

/// Packs the instance `order` into bars with best-fit selection: each cut
/// goes to the already-open bar (same material) with the smallest leftover
/// that can take it (first one wins on ties), otherwise a new bar is opened
/// according to `preference`. Errors when a stock's `max_bars` cap makes the
/// packing impossible.
pub(crate) fn best_fit_pack(
    problem: &Problem,
    order: &[usize],
    preference: StockPreference,
) -> Result<Vec<(usize, Vec<usize>)>, SolveError> {
    let mut bars: Vec<OpenBar> = Vec::new();
    for &part in order {
        let part_spec = &problem.parts[part];
        let length = part_spec.length;
        let material = part_spec.material;

        let mut best: Option<(usize, u32)> = None;
        for (bar_index, bar) in bars.iter().enumerate() {
            if bar.stock_material(problem) != material {
                continue;
            }
            if let Some(leftover) = bar.leftover_after(length, problem) {
                if best.map_or(true, |(_, best_leftover)| leftover < best_leftover) {
                    best = Some((bar_index, leftover));
                }
            }
        }

        match best {
            Some((bar_index, _)) => bars[bar_index].add(part, length),
            None => {
                let stock_index = open_stock_for(problem, length, material, &bars, preference)?;
                let mut bar = OpenBar::new(stock_index);
                bar.add(part, length);
                bars.push(bar);
            }
        }
    }
    Ok(bars
        .into_iter()
        .map(|bar| (bar.stock_index, bar.items))
        .collect())
}


