//! Baseline packing: best-fit-decreasing (BFD).
//!
//! Deterministic by construction: instances are expanded in input order and
//! stably sorted by decreasing length, ties keep input order, and the best-fit
//! scan picks the *first* bar with the minimal leftover. Same input in, same
//! bars out — on every platform, every run.

use crate::error::SolveError;
use crate::model::{Problem, Solution};

/// Solves `problem` with deterministic best-fit-decreasing packing.
///
/// This is the free edition's engine: fast, predictable, and a strict upper
/// bound the improvement pass (Pro) can only improve on.
pub fn solve(problem: &Problem) -> Result<Solution, SolveError> {
    problem.validate()?;
    let order = decreasing_instances(problem);
    let bars = best_fit_pack(problem, &order);
    check_max_bars(problem, bars.len() as u32)?;
    Ok(Solution::build(problem, bars))
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
    instances.sort_by(|&a, &b| {
        problem.parts[b]
            .length
            .cmp(&problem.parts[a].length)
    });
    instances
}

/// An open (partially filled) bar during packing.
#[derive(Debug, Clone)]
pub(crate) struct OpenBar {
    pub items: Vec<usize>,
    parts_len: u32,
    count: u32,
}

impl OpenBar {
    pub(crate) fn new() -> Self {
        OpenBar {
            items: Vec::new(),
            parts_len: 0,
            count: 0,
        }
    }

    /// Leftover stock after appending `length`, if it fits.
    /// `None` when the cut does not fit.
    pub(crate) fn leftover_after(&self, length: u32, problem: &Problem) -> Option<u32> {
        if self.can_fit(length, problem) {
            let consumed =
                self.parts_len + length + problem.kerf * self.count + problem.trim;
            Some(problem.stock.length - consumed)
        } else {
            None
        }
    }

    pub(crate) fn can_fit(&self, length: u32, problem: &Problem) -> bool {
        self.parts_len + length + problem.kerf * self.count + problem.trim
            <= problem.stock.length
    }

    pub(crate) fn add(&mut self, part: usize, length: u32) {
        self.items.push(part);
        self.parts_len += length;
        self.count += 1;
    }
}


/// Packs the instance `order` into bars with best-fit selection: each cut
/// goes to the already-open bar with the smallest leftover that can take it
/// (first one wins on ties), otherwise a new bar is opened.
pub(crate) fn best_fit_pack(problem: &Problem, order: &[usize]) -> Vec<Vec<usize>> {
    let mut bars: Vec<OpenBar> = Vec::new();
    for &part in order {
        let length = problem.parts[part].length;
        let mut best: Option<(usize, u32)> = None;
        for (bar_index, bar) in bars.iter().enumerate() {
            if let Some(leftover) = bar.leftover_after(length, problem) {
                if best.map_or(true, |(_, best_leftover)| leftover < best_leftover) {
                    best = Some((bar_index, leftover));
                }
            }
        }
        match best {
            Some((bar_index, _)) => bars[bar_index].add(part, length),
            None => {
                let mut bar = OpenBar::new();
                bar.add(part, length);
                bars.push(bar);
            }
        }
    }
    bars.into_iter().map(|bar| bar.items).collect()
}

/// Fails with [`SolveError::OutOfStock`] when the packing exceeds the
/// stock's `max_bars` cap.
pub(crate) fn check_max_bars(problem: &Problem, needed: u32) -> Result<(), SolveError> {
    match problem.stock.max_bars {
        Some(available) if needed > available => Err(SolveError::OutOfStock { needed, available }),
        _ => Ok(()),
    }
}
