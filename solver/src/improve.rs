//! Improvement pass (Pro edition, cargo feature `improvement`).
//!
//! Two ingredients over the baseline:
//! 1. repeated BFD restarts with randomized / rotated orderings — the
//!    randomization is seeded from the problem itself, so runs are
//!    reproducible when no wall-clock deadline is set;
//! 2. a deterministic first-improvement local search that tries to empty
//!    whole bars by relocating their cuts elsewhere (the only move that
//!    strictly reduces the bar count — total waste is
//!    `bars * stock - parts`, so fewer bars always dominates).
//!
//! Both are kerf-aware through the shared `OpenBar` accounting.

use std::time::Instant;

use crate::error::SolveError;
use crate::ffd::{best_fit_pack, check_max_bars, decreasing_instances, OpenBar};
use crate::model::{Problem, Solution};

/// Budget knobs for [`solve_optimized`].
#[derive(Debug, Clone)]
pub struct ImproveOptions {
    /// Maximum number of randomized-restart rounds. The local search runs
    /// once up front and after every round that beats the incumbent.
    pub rounds: u32,
    /// Optional wall-clock deadline. Checked between rounds, so one round is
    /// the smallest granularity. `None` makes the whole run deterministic.
    pub deadline: Option<Instant>,
}

impl Default for ImproveOptions {
    fn default() -> Self {
        ImproveOptions {
            rounds: 4096,
            deadline: None,
        }
    }
}

/// Solves `problem` with the baseline plus the improvement pass.
pub fn solve_optimized(
    problem: &Problem,
    options: &ImproveOptions,
) -> Result<Solution, SolveError> {
    problem.validate()?;

    let base_order = decreasing_instances(problem);
    let mut best = best_fit_pack(problem, &base_order);
    empty_bar_moves(problem, &mut best);

    let mut rng = Rng::new(seed_problem(problem));
    for round in 0..options.rounds {
        if let Some(deadline) = options.deadline {
            if Instant::now() >= deadline {
                break;
            }
        }
        let order = perturbed_order(problem, &base_order, &mut rng, round);
        let mut candidate = best_fit_pack(problem, &order);
        if candidate.len() < best.len() {
            empty_bar_moves(problem, &mut candidate);
            if candidate.len() < best.len() {
                best = candidate;
            }
        }
    }

    check_max_bars(problem, best.len() as u32)?;
    Ok(Solution::build(problem, best))
}

/// First-improvement local search: repeatedly scan the bars (fewest cuts
/// first) and try to relocate every cut of one bar into the remaining bars.
/// Committing such a move removes a bar outright. Runs until a full scan
/// finds no improvement. Deterministic.
fn empty_bar_moves(problem: &Problem, bars: &mut Vec<Vec<usize>>) {
    loop {
        let mut improved = false;
        // Try the most promising bars first: fewest cuts to relocate.
        let mut order: Vec<usize> = (0..bars.len()).collect();
        order.sort_by_key(|&i| bars[i].len());
        for &source in &order {
            if bars.len() <= 1 {
                break;
            }
            if let Some(merged) = try_empty_bar(problem, bars, source) {
                *bars = merged;
                improved = true;
                break;
            }
        }
        if !improved {
            break;
        }
    }
}

/// Attempts to dissolve `bars[source]` into the other bars. Returns the new
/// bar lists on success (source removed), `None` when some cut does not fit
/// anywhere else.
fn try_empty_bar(
    problem: &Problem,
    bars: &[Vec<usize>],
    source: usize,
) -> Option<Vec<Vec<usize>>> {
    let mut open: Vec<OpenBar> = Vec::with_capacity(bars.len() - 1);
    for (index, items) in bars.iter().enumerate() {
        if index == source {
            continue;
        }
        let mut bar = OpenBar::new();
        for &part in items {
            bar.add(part, problem.parts[part].length);
        }
        open.push(bar);
    }

    // Relocate the source's cuts longest-first (kerf-aware best fit).
    let mut relocating: Vec<usize> = bars[source].clone();
    relocating.sort_by(|&a, &b| problem.parts[b].length.cmp(&problem.parts[a].length));
    for part in relocating {
        let length = problem.parts[part].length;
        let mut best: Option<(usize, u32)> = None;
        for (bar_index, bar) in open.iter().enumerate() {
            if let Some(leftover) = bar.leftover_after(length, problem) {
                if best.map_or(true, |(_, current)| leftover < current) {
                    best = Some((bar_index, leftover));
                }
            }
        }
        let (bar_index, _) = best?;
        open[bar_index].add(part, length);
    }

    Some(open.into_iter().map(|bar| bar.items).collect())
}

/// Builds a randomized ordering for a restart round. Even rounds rotate the
/// base order (so equal-length ties resolve differently) and jitter only the
/// sort key — near-BFD variants. Odd rounds do a seeded Fisher-Yates shuffle
/// for wide exploration. Both are deterministic given the seed.
fn perturbed_order(
    problem: &Problem,
    base_order: &[usize],
    rng: &mut Rng,
    round: u32,
) -> Vec<usize> {
    let n = base_order.len();
    if n < 2 {
        return base_order.to_vec();
    }

    if round % 2 == 0 {
        let rotate = (rng.next_u32() as usize) % n;
        let rotated: Vec<usize> = (0..n).map(|i| base_order[(i + rotate) % n]).collect();
        let mut keyed: Vec<(u64, usize)> = rotated
            .into_iter()
            .map(|part| {
                let length = u64::from(problem.parts[part].length);
                let jitter = u64::from(rng.next_u32() % 4);
                (length * 4 + jitter, part)
            })
            .collect();
        keyed.sort_by(|a, b| b.0.cmp(&a.0));
        keyed.into_iter().map(|(_, part)| part).collect()
    } else {
        let mut shuffled = base_order.to_vec();
        for i in (1..shuffled.len()).rev() {
            let j = (rng.next_u32() as usize) % (i + 1);
            shuffled.swap(i, j);
        }
        shuffled
    }
}

/// Deterministic xorshift64* PRNG (no external dependencies, reproducible).
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        // A zero state would stick at zero; any nonzero state works.
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub(crate) fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
}

/// FNV-1a over the problem's numeric identity: lengths, quantities, stock,
/// kerf and trim. Same problem in, same seed out — deterministic across runs
/// and platforms.
fn seed_problem(problem: &Problem) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in &problem.parts {
        mix(&mut hash, &part.length.to_le_bytes());
        mix(&mut hash, &part.qty.to_le_bytes());
    }
    mix(&mut hash, &problem.stock.length.to_le_bytes());
    mix(&mut hash, &problem.kerf.to_le_bytes());
    mix(&mut hash, &problem.trim.to_le_bytes());
    hash
}

fn mix(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

