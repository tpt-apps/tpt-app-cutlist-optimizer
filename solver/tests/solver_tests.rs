//! Solver correctness suite: known-optimal instances, kerf accounting,
//! infeasibility, determinism, and property-style invariants (no bar is ever
//! overfilled; every requested part is placed exactly once).

use std::collections::BTreeMap;

use tpt_cutlist_solver::{Part, Problem, SolveError, Solution, StockSpec};

fn problem(parts: Vec<(&str, u32, u32)>, stock: u32, kerf: u32, trim: u32) -> Problem {
    Problem::new(
        parts
            .into_iter()
            .map(|(label, length, qty)| Part::new(length, qty, label))
            .collect(),
        StockSpec::new(stock),
        kerf,
        trim,
    )
}

/// Core invariant check shared by all property tests: no bar overfilled,
/// offsets consistent (kerf-aware), every requested part placed exactly once.
fn assert_valid(problem: &Problem, solution: &Solution) {
    // 1. No bar is overfilled (kerf + trim aware) and none is empty.
    for (index, bar) in solution.bars.iter().enumerate() {
        let consumed = bar.consumed(problem.kerf, problem.trim);
        assert!(
            consumed <= problem.stock.length,
            "bar {index} overfilled: {consumed} > {} (cuts: {:?})",
            problem.stock.length,
            bar.cuts
        );
        assert!(!bar.cuts.is_empty(), "solution contains an empty bar");
        for pair in bar.cuts.windows(2) {
            assert_eq!(
                pair[1].offset,
                pair[0].offset + pair[0].length + problem.kerf,
                "inconsistent offset on bar {index}"
            );
        }
        assert_eq!(bar.cuts[0].offset, 0, "first cut must start at 0");
    }

    // 2. Every requested part placed exactly once, cut lengths matching.
    let mut placed: BTreeMap<usize, u32> = BTreeMap::new();
    for bar in &solution.bars {
        for cut in &bar.cuts {
            *placed.entry(cut.part).or_insert(0) += 1;
            assert_eq!(
                problem.parts[cut.part].length,
                cut.length,
                "cut length does not match its part"
            );
        }
    }
    for (index, part) in problem.parts.iter().enumerate() {
        let count = placed.get(&index).copied().unwrap_or(0);
        assert_eq!(
            count, part.qty,
            "part {index} ({}) placed {count} times, requested {}",
            part.label, part.qty
        );
    }

    // 3. Accounting fields agree with the bars.
    let stock_total = solution.bar_count() as u64 * problem.stock.length as u64;
    let parts_total = solution.parts_length() as u64;
    let kerf_total: u64 = solution
        .bars
        .iter()
        .map(|b| u64::from(b.kerf_length(problem.kerf)))
        .sum();
    let trim_total = solution.bar_count() as u64 * u64::from(problem.trim);
    assert_eq!(
        u64::from(solution.waste),
        stock_total - parts_total - kerf_total - trim_total
    );
    let expected_utilization = parts_total as f64 / stock_total as f64;
    assert!((solution.utilization - expected_utilization).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Known-optimal instances
// ---------------------------------------------------------------------------

#[test]
fn known_optimal_fits_in_one_bar() {
    // 3 x 600 + 1 x 450 = 2250 <= 2400 -> a single bar is optimal.
    let job = problem(vec![("shelf", 600, 3), ("rail", 450, 1)], 2400, 0, 0);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 1);
    assert_eq!(solution.waste, 150);
    assert!((solution.utilization - 2250.0 / 2400.0).abs() < 1e-9);
}

#[test]
fn thirty_six_hundred_mm_needs_two_bars() {
    // 4 x 900 = 3600 in 2400 stock -> two bars, 1200 mm total offcut.
    let job = problem(vec![("rail", 900, 4)], 2400, 0, 0);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 2);
    assert_eq!(solution.waste, 1200);
}

// ---------------------------------------------------------------------------
// Kerf correctness
// ---------------------------------------------------------------------------

#[test]
fn kerf_five_lets_two_1195_parts_share_a_bar() {
    // 1195 + 5 (kerf) + 1195 = 2395 <= 2400 -> one bar.
    let job = problem(vec![("stile", 1195, 2)], 2400, 5, 0);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 1);
    let cuts = &solution.bars[0].cuts;
    assert_eq!(cuts.len(), 2);
    assert_eq!(cuts[1].offset, 1195 + 5, "the kerf separates the two cuts");
}

#[test]
fn kerf_eleven_forces_a_second_bar() {
    // 1195 + 11 + 1195 = 2401 > 2400 -> two bars.
    let job = problem(vec![("stile", 1195, 2)], 2400, 11, 0);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 2);
}

#[test]
fn trim_is_charged_once_per_bar() {
    // Without trim, 4 x 600 fits one 2400 bar exactly.
    let job = problem(vec![("shelf", 600, 4)], 2400, 0, 0);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 1);

    // With 25 mm end trim each bar only takes 3 pieces
    // (600x3 + 2 kerfs + trim = 1825; a 4th piece + trim = 2425 > 2400),
    // so the job now needs a second bar — trim charged per used bar.
    let job = problem(vec![("shelf", 600, 4)], 2400, 0, 25);
    let solution = tpt_cutlist_solver::solve(&job).expect("solves");
    assert_valid(&job, &solution);
    assert_eq!(solution.bar_count(), 2);

    // A part that cannot pay the trim even alone on a bar is rejected.
    let job = problem(vec![("top", 2380, 1)], 2400, 0, 25);
    let error = tpt_cutlist_solver::solve(&job).unwrap_err();
    assert!(
        matches!(error, SolveError::PartTooLong { available: 2375, .. }),
        "expected PartTooLong, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// Infeasibility and validation
// ---------------------------------------------------------------------------

#[test]
fn part_longer_than_stock_is_a_clear_error() {
    let job = problem(vec![("bench top", 2500, 1)], 2400, 0, 0);
    let error = tpt_cutlist_solver::solve(&job).unwrap_err();
    match error {
        SolveError::PartTooLong {
            label,
            length,
            stock_length,
            ..
        } => {
            assert_eq!(label, "bench top");
            assert_eq!(length, 2500);
            assert_eq!(stock_length, 2400);
        }
        other => panic!("expected PartTooLong, got {other:?}"),
    }
}

#[test]
fn empty_job_is_rejected() {
    let job = problem(vec![], 2400, 0, 0);
    assert!(matches!(
        tpt_cutlist_solver::solve(&job),
        Err(SolveError::InvalidProblem(_))
    ));
}

#[test]
fn zero_length_and_zero_qty_are_rejected() {
    let zero_length = problem(vec![("oops", 0, 1)], 2400, 0, 0);
    assert!(matches!(
        tpt_cutlist_solver::solve(&zero_length),
        Err(SolveError::InvalidProblem(_))
    ));
    let zero_qty = problem(vec![("oops", 600, 0)], 2400, 0, 0);
    assert!(matches!(
        tpt_cutlist_solver::solve(&zero_qty),
        Err(SolveError::InvalidProblem(_))
    ));
}

#[test]
fn max_bars_cap_is_enforced() {
    // 2 x 1300 needs 2 bars; cap it at 1 -> OutOfStock.
    let mut job = problem(vec![("shelf", 1300, 2)], 2400, 0, 0);
    job.stock.max_bars = Some(1);
    assert!(matches!(
        tpt_cutlist_solver::solve(&job),
        Err(SolveError::OutOfStock {
            needed: 2,
            available: 1
        })
    ));
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn baseline_is_deterministic() {
    let job = problem(
        vec![
            ("side", 730, 4),
            ("shelf", 560, 6),
            ("rail", 396, 4),
            ("stile", 197, 8),
        ],
        2400,
        3,
        0,
    );
    let a = format!("{:?}", tpt_cutlist_solver::solve(&job).unwrap());
    let b = format!("{:?}", tpt_cutlist_solver::solve(&job).unwrap());
    assert_eq!(a, b, "same input must produce the same bars, byte for byte");
}

// ---------------------------------------------------------------------------
// Improvement pass (Pro, feature `improvement`)
// ---------------------------------------------------------------------------

#[cfg(feature = "improvement")]
mod improvement {
    use super::*;
    use std::time::{Duration, Instant};
    use tpt_cutlist_solver::{solve_optimized, ImproveOptions};

    #[test]
    fn never_worse_than_baseline() {
        let job = problem(
            vec![
                ("side", 730, 4),
                ("shelf", 560, 6),
                ("rail", 396, 4),
                ("stile", 197, 8),
                ("door", 640, 2),
            ],
            2400,
            3,
            0,
        );
        let baseline = tpt_cutlist_solver::solve(&job).unwrap();
        let improved = solve_optimized(&job, &ImproveOptions::default()).unwrap();
        assert_valid(&job, &improved);
        assert!(
            improved.bar_count() <= baseline.bar_count(),
            "improvement ({}) must not use more bars than baseline ({})",
            improved.bar_count(),
            baseline.bar_count()
        );
    }

    #[test]
    fn improvement_is_deterministic_without_deadline() {
        let job = problem(vec![("a", 990, 3), ("b", 640, 4), ("c", 501, 5)], 2400, 4, 0);
        let options = ImproveOptions::default();
        let a = format!("{:?}", solve_optimized(&job, &options).unwrap());
        let b = format!("{:?}", solve_optimized(&job, &options).unwrap());
        assert_eq!(a, b);
    }

    #[test]
    fn deadline_is_respected() {
        let job = problem(vec![("a", 990, 30), ("b", 641, 30)], 2400, 4, 0);
        let options = ImproveOptions {
            rounds: 100_000,
            deadline: Some(Instant::now() + Duration::from_millis(50)),
        };
        let started = Instant::now();
        let solution = solve_optimized(&job, &options).expect("solves");
        assert_valid(&job, &solution);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "deadline should stop the search quickly"
        );
    }
}

// ---------------------------------------------------------------------------
// Exact solver (feature `exact`)
// ---------------------------------------------------------------------------

#[cfg(feature = "exact")]
mod exact_mode {
    use super::*;
    use tpt_cutlist_solver::solve_exact;

    #[test]
    fn matches_optimum_on_known_instances() {
        // 3 x 600 + 1 x 450 in 2400: optimum is 1 bar.
        let job = problem(vec![("shelf", 600, 3), ("rail", 450, 1)], 2400, 0, 0);
        let solution = solve_exact(&job).expect("solves");
        assert_valid(&job, &solution);
        assert_eq!(solution.bar_count(), 1);

        // 5 x 700 + 3 x 350 in 2400, kerf 4: the optimum is 3 bars. With
        // 2 bars the 700s must split 3/2; the 3-heavy bar then holds
        // 3 x 700 + 2 kerfs = 2108 (292 free — no room for a 350 + kerf)
        // and the 2-heavy bar holds 1404 + 4 + 350 + 4 + 350 = 2112
        // (288 free) — the third 350 has no home. FFD also lands on 3.
        let job = problem(vec![("panel", 700, 5), ("strip", 350, 3)], 2400, 4, 0);
        let solution = solve_exact(&job).expect("solves");
        assert_valid(&job, &solution);
        assert_eq!(solution.bar_count(), 3);
    }

    #[test]
    fn never_worse_than_baseline_on_small_instances() {
        // Tiny deterministic pseudo-random jobs (LCG), small enough for the
        // exact search to stay fast.
        let mut state: u64 = 0x5EED_1234_ABCD_0001;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        let names = ["a", "b", "c", "d", "e", "f"];
        for seed in 0..40u32 {
            let _ = seed;
            let stock = 300 + next() % 2100;
            let count = 1 + next() % 6;
            let parts: Vec<(&str, u32, u32)> = (0..count)
                .map(|i| (names[i as usize], 1 + next() % stock, 1 + next() % 3))
                .collect();
            let job = problem(parts, stock, next() % 5, 0);

            let baseline = tpt_cutlist_solver::solve(&job).unwrap();
            let optimal = solve_exact(&job).unwrap();
            assert_valid(&job, &optimal);
            assert!(
                optimal.bar_count() <= baseline.bar_count(),
                "exact ({}) worse than baseline ({})",
                optimal.bar_count(),
                baseline.bar_count()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property tests (proptest): the two safety invariants over random jobs
// ---------------------------------------------------------------------------

#[cfg(test)]
mod property {
    use super::*;
    use proptest::prelude::*;

    fn build_job(
        stock: u32,
        kerf: u32,
        lengths: Vec<u32>,
        qtys: Vec<u32>,
    ) -> Problem {
        let stock = stock.max(40);
        let parts: Vec<Part> = lengths
            .into_iter()
            .zip(qtys)
            .map(|(length, qty)| Part::new(length % stock.max(2) + 1, qty % 4 + 1, "part"))
            .collect();
        Problem::new(parts, StockSpec::new(stock), kerf, 0)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn no_bar_overfilled_and_every_part_placed(
            stock in 1u32..5000,
            kerf in 0u32..10,
            lengths in proptest::collection::vec(any::<u32>(), 1usize..8),
            qtys in proptest::collection::vec(any::<u32>(), 1usize..8),
        ) {
            let job = build_job(stock, kerf, lengths, qtys);
            let solution = tpt_cutlist_solver::solve(&job).expect("solves");
            assert_valid(&job, &solution);
        }

        #[test]
        fn utilization_stays_between_zero_and_one(
            stock in 1u32..5000,
            lengths in proptest::collection::vec(any::<u32>(), 1usize..6),
            qtys in proptest::collection::vec(any::<u32>(), 1usize..6),
        ) {
            let job = build_job(stock, 0, lengths, qtys);
            let solution = tpt_cutlist_solver::solve(&job).expect("solves");
            prop_assert!((0.0..=1.0).contains(&solution.utilization));
        }
    }
}




