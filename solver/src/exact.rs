//! Exact minimum-bar solver (cargo feature `exact`).
//!
//! Finds the provably optimal bar count for a single stock length by
//! searching the decision problem "can all parts fit in `k` bars?" for
//! `k = lower_bound ..= ffd_upper_bound` with a pruned backtracking search:
//!
//! - parts are placed longest-first (the classic bin-packing dominance rule),
//! - bins with identical remaining capacity are tried once per item
//!   (symmetry breaking),
//! - the search stops as soon as a `k` admits a packing, and the FFD result
//!   caps the search from above.
//!
//! Practical for small instances (the spec's "≤ ~25 parts, single stock
//! length"); used to benchmark the heuristics, not for the free-tier hot path.

use crate::error::SolveError;
use crate::ffd::{best_fit_pack, check_max_bars, decreasing_instances};
use crate::model::{Problem, Solution};

/// Solves `problem` to proven-optimal bar count (single stock length).
pub fn solve_exact(problem: &Problem) -> Result<Solution, SolveError> {
    problem.validate()?;

    let order = decreasing_instances(problem);
    let lengths: Vec<u32> = order.iter().map(|&part| problem.parts[part].length).collect();
    let total: u64 = lengths.iter().map(|&l| u64::from(l)).sum();

    // Upper bound straight from the baseline heuristic.
    let ffd_bars = best_fit_pack(problem, &order);
    let upper = ffd_bars.len();

    // Lower bound ignoring kerf (a relaxation, so it is admissible).
    let usable = u64::from(problem.usable_length());
    let lower = ((total + usable - 1) / usable).max(1) as usize;

    let mut k = lower.min(upper);
    let mut packing: Option<Vec<usize>> = None;
    while k <= upper {
        if let Some(assign) = feasible(problem, &lengths, k) {
            packing = Some(assign);
            break;
        }
        k += 1;
    }
    let k = k.min(upper);

    let bar_parts: Vec<Vec<usize>> = match packing {
        Some(assign) => {
            let mut bars: Vec<Vec<usize>> = vec![Vec::new(); k];
            for (instance, &part) in order.iter().enumerate() {
                bars[assign[instance]].push(part);
            }
            bars
        }
        // `k == upper` and FFD itself is a witness packing for that k.
        None => ffd_bars,
    };

    check_max_bars(problem, bar_parts.len() as u32)?;
    Ok(Solution::build(problem, bar_parts))
}

/// Backtracking decision search: can all `lengths` (already sorted
/// decreasing) fit into `k` bars? Returns the bar index per instance on
/// success. Prunes bin-load symmetry and rejects items that fit nowhere.
fn feasible(problem: &Problem, lengths: &[u32], k: usize) -> Option<Vec<usize>> {
    let mut remaining = vec![problem.stock.length; k];
    let mut counts = vec![0u32; k];
    let mut assignment = vec![usize::MAX; lengths.len()];

    if dfs(problem, 0, lengths, &mut remaining, &mut counts, &mut assignment) {
        Some(assignment)
    } else {
        None
    }
}

fn dfs(
    problem: &Problem,
    index: usize,
    lengths: &[u32],
    remaining: &mut [u32],
    counts: &mut [u32],
    assignment: &mut [usize],
) -> bool {
    if index == lengths.len() {
        return true;
    }
    let length = lengths[index];
    // Symmetry breaking: never try two bins with the same remaining capacity
    // for the same item — they are interchangeable.
    let mut tried: Vec<u32> = Vec::new();
    for bin_index in 0..remaining.len() {
        let available = remaining[bin_index];
        if available < length {
            continue;
        }
        // A first cut pays the end trim; later cuts pay the kerf first.
        let extra = if counts[bin_index] > 0 {
            problem.kerf
        } else {
            problem.trim
        };
        if extra > available - length {
            continue;
        }
        if tried.contains(&available) {
            continue;
        }
        tried.push(available);

        remaining[bin_index] = available - length - extra;
        counts[bin_index] += 1;
        assignment[index] = bin_index;
        if dfs(problem, index + 1, lengths, remaining, counts, assignment) {
            return true;
        }
        counts[bin_index] -= 1;
        remaining[bin_index] = available;
        assignment[index] = usize::MAX;
    }
    false
}

