//! TPT Cut List Optimizer — solver crate (the moat).
//!
//! Kerf-aware 1D cutting-stock packing in pure Rust with zero UI
//! dependencies: shared verbatim by the free WASM edition and the paid
//! desktop edition, and unit-testable on the host.
//!
//! Strategies:
//! - [`solve`] — deterministic best-fit-decreasing baseline (free edition).
//! - [`solve_optimized`] (feature `improvement`) — repeated randomized
//!   restarts plus a bar-emptying local search, time-boxable (Pro edition).
//! - [`solve_exact`] (feature `exact`) — branch-and-bound minimum-bar solver
//!   for small instances; benchmarks the heuristics against optimality.
//!
//! All strategies share the same exact integer accounting: every bar holds
//! `sum(parts) + (n-1) * kerf + trim <= stock_length`, cuts carry their
//! placement offsets, and a [`Solution`] is checked (in tests) to overfill
//! nothing and place every requested piece exactly once.

pub mod error;
pub mod model;

mod ffd;

#[cfg(feature = "improvement")]
pub mod improve;
#[cfg(feature = "exact")]
pub mod exact;

pub use error::SolveError;
pub use ffd::solve;
pub use model::{Bar, Cut, Part, Problem, Solution, StockSpec};

#[cfg(feature = "improvement")]
pub use improve::{solve_optimized, ImproveOptions};

#[cfg(feature = "exact")]
pub use exact::solve_exact;
