//! MimiEyes (VENDORED.md #16): slow-path timing for the pane view. Writes a
//! `hz_perf` line to the log only when something is slow, so a switch that
//! feels laggy leaves evidence of where the time went: whole frames, the
//! main thread's share of a transcript update, a transcript's row rebuild,
//! a pane mounting its conversation.

use std::time::{Duration, Instant};

/// Logs `what` (and `detail`) when dropped later than `budget` after it was
/// made — wrap a scope with it.
pub struct Timer {
    what: &'static str,
    detail: String,
    budget: Duration,
    started: Instant,
}

impl Timer {
    pub fn new(what: &'static str, detail: impl Into<String>, budget_ms: u64) -> Self {
        Self {
            what,
            detail: detail.into(),
            budget: Duration::from_millis(budget_ms),
            started: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        if took >= self.budget {
            tracing::info!(
                target: "hz_perf",
                what = self.what,
                detail = %self.detail,
                ms = took.as_millis() as u64,
                "slow"
            );
        }
    }
}

/// A whole frame: from this render to the start of the next frame (layout
/// and paint included). Logs frames over `budget_ms`.
pub fn time_frame(window: &mut gpui::Window, what: &'static str, budget_ms: u64) {
    let started = Instant::now();
    window.on_next_frame(move |_, _| {
        let took = started.elapsed();
        if took >= Duration::from_millis(budget_ms) {
            tracing::info!(target: "hz_perf", what, ms = took.as_millis() as u64, "slow frame");
        }
    });
}
