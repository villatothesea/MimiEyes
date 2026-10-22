//! MimiEyes (VENDORED.md #18): a stopwatch for engine assembly. Each step
//! slower than 150 ms leaves an `hz_perf` line, so a slow start shows which
//! step holds up the window.

use std::time::Instant;

pub struct Lap {
    last: Instant,
}

impl Lap {
    pub fn new() -> Self {
        Self {
            last: Instant::now(),
        }
    }

    /// The step since the previous mark took this long; log it when slow.
    pub fn mark(&mut self, step: &'static str) {
        let now = Instant::now();
        let ms = now.duration_since(self.last).as_millis() as u64;
        if ms >= 150 {
            tracing::info!(target: "hz_perf", step, ms, "slow engine step");
        }
        self.last = now;
    }
}

/// An engine request slower than a second leaves an `hz_perf` line when
/// it finishes (streams are timed to their first reply only).
pub struct RpcTimer {
    method: String,
    started: Instant,
}

impl RpcTimer {
    pub fn new(method: &str) -> Self {
        Self {
            method: method.to_owned(),
            started: Instant::now(),
        }
    }
}

impl Drop for RpcTimer {
    fn drop(&mut self) {
        let ms = self.started.elapsed().as_millis() as u64;
        if ms >= 1000 {
            tracing::info!(target: "hz_perf", method = %self.method, ms, "slow engine request");
        }
    }
}

impl Default for Lap {
    fn default() -> Self {
        Self::new()
    }
}
