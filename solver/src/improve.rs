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
use crate::ffd::{
    best_fit_pack, best_of_preferences, decreasing_instances, packing_cost, OpenBar,
};
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
///
/// The objective is total stock **price** (`Problem.stocks[i].price`); with
/// default pricing this is total stock length used.
pub fn solve_optimized(
    problem: &Problem,
    options: &ImproveOptions,
) -> Result<Solution, SolveError> {
    use crate::ffd::StockPreference;

    problem.validate()?;

    let base_order = decreasing_instances(problem);
    let mut best = best_of_preferences(problem, &base_order)?;
    let mut best_cost = packing_cost(problem, &best);

    let mut rng = Rng::new(seed_problem(problem));
    for round in 0..options.rounds {
        if let Some(deadline) = options.deadline {
            if Instant::now() >= deadline {
                break;
            }
        }
        let order = perturbed_order(problem, &base_order, &mut rng, round);
        for preference in [StockPreference::Smallest, StockPreference::Largest] {
            // A candidate can be infeasible under a `max_bars` cap; skip it.
            let Ok(mut candidate) = best_fit_pack(problem, &order, preference) else {
                continue;
            };
            if packing_cost(problem, &candidate) < best_cost {
                empty_bar_moves(problem, &mut candidate);
                let candidate_cost = packing_cost(problem, &candidate);
                if candidate_cost < best_cost {
                    best = candidate;
                    best_cost = candidate_cost;
                }
            }
        }
    }

    Ok(Solution::build(problem, best))
}

/// First-improvement local search: repeatedly scan the bars (fewest cuts
/// first) and try to relocate every cut of one bar into the remaining bars
/// of the same material. Committing such a move removes a bar outright and
/// therefore strictly reduces total cost by that bar's price. Runs until a
/// full scan finds no improvement. Deterministic.
fn empty_bar_moves(problem: &Problem, bars: &mut Vec<(usize, Vec<usize>)>) {
    loop {
        let mut improved = false;
        // Try the most promising bars first: fewest cuts to relocate.
        let mut order: Vec<usize> = (0..bars.len()).collect();
        order.sort_by_key(|&i| bars[i].1.len());
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

/// Attempts to dissolve `bars[source]` into the other bars of the same
/// material. Returns the new bar list on success (source removed), `None`
/// when some cut does not fit anywhere else. Bar order is preserved.
fn try_empty_bar(
    problem: &Problem,
    bars: &[(usize, Vec<usize>)],
    source: usize,
) -> Option<Vec<(usize, Vec<usize>)>> {
    let source_material = problem.stocks[bars[source].0].material;

    let mut open: Vec<OpenBar> = Vec::new();
    let mut target_positions: Vec<usize> = Vec::new(); // indexes into `bars`
    for (index, (stock_index, items)) in bars.iter().enumerate() {
        if index == source {
            continue;
        }
        if problem.stocks[*stock_index].material != source_material {
            continue;
        }
        let mut bar = OpenBar::new(*stock_index);
        for &part in items {
            bar.add(part, problem.parts[part].length);
        }
        open.push(bar);
        target_positions.push(index);
    }

    // Relocate the source's cuts longest-first (kerf-aware best fit).
    let mut relocating: Vec<usize> = bars[source].1.clone();
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

    // Rebuild in the original order: same-material bars take their updated
    // item lists, everything else (and the dissolved source) is dropped.
    let mut open_iter = open.into_iter();
    Some(
        bars.iter()
            .enumerate()
            .filter(|(index, _)| *index != source)
            .map(|(index, (stock_index, items))| {
                if target_positions.contains(&index) {
                    let bar = open_iter.next().expect("one open bar per target");
                    (bar.stock_index, bar.items)
                } else {
                    (*stock_index, items.clone())
                }
            })
            .collect(),
    )
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

/// FNV-1a over the problem's numeric identity: lengths, quantities, stock
/// specs, kerf and trim. Same problem in, same seed out — deterministic
/// across runs and platforms.
fn seed_problem(problem: &Problem) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in &problem.parts {
        mix(&mut hash, &part.length.to_le_bytes());
        mix(&mut hash, &part.qty.to_le_bytes());
        mix(&mut hash, &part.material.to_le_bytes());
    }
    for stock in &problem.stocks {
        mix(&mut hash, &stock.length.to_le_bytes());
        mix(&mut hash, &stock.price.to_le_bytes());
        mix(&mut hash, &stock.material.to_le_bytes());
    }
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

