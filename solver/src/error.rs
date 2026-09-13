//! Solver error types shared by every strategy.

use std::fmt;

/// Why a problem could not be solved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
    /// Malformed input (zero lengths/quantities, empty job, trim >= stock).
    InvalidProblem(String),
    /// A part does not fit the stock at all — not even alone on an empty
    /// bar after the end trim is paid (`available` = stock - trim).
    PartTooLong {
        index: usize,
        label: String,
        length: u32,
        stock_length: u32,
        available: u32,
    },
    /// A `max_bars` cap on the stock was set and the packing needs more.
    OutOfStock { needed: u32, available: u32 },
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::InvalidProblem(msg) => write!(f, "invalid problem: {msg}"),
            SolveError::PartTooLong {
                label,
                length,
                stock_length,
                available,
                ..
            } => {
                if *available == *stock_length {
                    write!(
                        f,
                        "part '{label}' ({length} mm) is longer than the {stock_length} mm stock"
                    )
                } else {
                    write!(
                        f,
                        "part '{label}' ({length} mm) does not fit the {stock_length} mm stock \
                         ({available} mm usable after trim)"
                    )
                }
            }
            SolveError::OutOfStock { needed, available } => write!(
                f,
                "the job needs {needed} bars but only {available} are available"
            ),
        }
    }
}

impl std::error::Error for SolveError {}
