//! Fixed-timestep simulation clock — port of `src/lib/clock.ts` (the time-accounting half:
//! advance / step / seek / alpha). Listeners (`onTick`/`onSeek`) are a VIEW concern and stay in JS — the
//! Rust core's step loop calls the sim tick directly. Canonical time is the integer `tick` (seconds =
//! tick × DT). Pure deterministic f64 accounting → identical results across browser / worker / replay; the
//! same arithmetic as the JS source (verified in tests against a captured reference sequence).

pub const DT: f64 = 1.0 / 30.0; // seconds per sim tick (fixed timestep)
const MAX_CATCHUP: u32 = 6; // cap ticks per advance() so a long frame stall can't spiral the sim
const EPS: f64 = 1e-9; // float slack so a full tick's worth of dt isn't lost to rounding

#[derive(Clone, Copy, Debug)]
pub struct SimClock {
    pub tick: i64,    // integer sim step — the canonical clock position
    pub rate: f64,    // tick-speed multiplier (2 = double-time, 0 ≈ paused)
    pub playing: bool,
    acc: f64,         // leftover sub-tick real-time (seconds), carried between frames
}

impl Default for SimClock {
    fn default() -> Self {
        Self { tick: 0, rate: 1.0, playing: true, acc: 0.0 }
    }
}

impl SimClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulated seconds elapsed (tick × DT).
    pub fn time(&self) -> f64 {
        self.tick as f64 * DT
    }

    /// Not advancing right now (stopped or rate 0).
    pub fn paused(&self) -> bool {
        !self.playing || self.rate == 0.0
    }

    /// Sub-tick interpolation factor (0..1) — how far real time has advanced toward the next tick. The
    /// render side lerps between the previous and current sim step by this.
    pub fn alpha(&self) -> f64 {
        let a = self.acc / DT;
        if a < 0.0 {
            0.0
        } else if a > 1.0 {
            1.0
        } else {
            a
        }
    }

    /// Feed real elapsed seconds; returns how many whole, rate-scaled ticks advanced (the caller runs the
    /// sim that many steps). Capped at MAX_CATCHUP so a stall can't spiral; backlog beyond the cap is dropped
    /// (doesn't affect reproducibility AT a given tick, only how fast wall-clock maps to ticks during a stall).
    pub fn advance(&mut self, real_dt: f64) -> u32 {
        if self.paused() || !(real_dt > 0.0) {
            return 0;
        }
        self.acc += real_dt * self.rate;
        let mut n = 0u32;
        while self.acc >= DT - EPS && n < MAX_CATCHUP {
            self.acc -= DT;
            n += 1;
        }
        if self.acc >= DT - EPS {
            self.acc = 0.0;
        }
        self.tick += n as i64;
        n
    }

    /// Advance exactly n ticks now (manual single-step while paused, or scripted replay/fast-forward).
    pub fn step(&mut self, n: i64) {
        if n > 0 {
            self.tick += n;
        }
    }

    /// TIME TRAVEL: jump to an absolute tick (the caller reconstructs state for that moment).
    pub fn seek(&mut self, target_tick: i64) {
        self.tick = target_tick.max(0);
        self.acc = 0.0;
    }

    /// Hard reset to a tick (default 0) with no replay signal — for loading a fresh/shared world.
    pub fn reset(&mut self, tick: i64) {
        self.tick = tick.max(0);
        self.acc = 0.0;
    }

    /// Set the tick-speed multiplier (clamped ≥ 0).
    pub fn set_rate(&mut self, r: f64) {
        self.rate = r.max(0.0);
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }
    pub fn play(&mut self) {
        self.playing = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference sequence captured from src/lib/clock.ts — the (n_ticks, tick, alpha) at each step MUST match.
    #[test]
    fn advance_parity() {
        let mut c = SimClock::new();

        assert_eq!(c.advance(DT * 2.5), 2); // 2 whole ticks…
        assert_eq!(c.tick, 2);
        assert!((c.alpha() - 0.5).abs() < 1e-9); // …0.5 carried

        assert_eq!(c.advance(DT * 0.5), 1); // carried 0.5 + 0.5 = 1 tick
        assert_eq!(c.tick, 3);
        assert_eq!(c.alpha(), 0.0);

        assert_eq!(c.advance(DT * 10.0), 6); // a big stall → capped at MAX_CATCHUP, backlog dropped
        assert_eq!(c.tick, 9);
        assert_eq!(c.alpha(), 0.0);

        c.rate = 2.0;
        assert_eq!(c.advance(DT), 2); // double-time
        assert_eq!(c.tick, 11);

        c.playing = false;
        assert_eq!(c.advance(DT * 5.0), 0); // paused → no advance
        assert_eq!(c.tick, 11);
    }

    #[test]
    fn seek_reset_clamp() {
        let mut c = SimClock::new();
        c.advance(DT * 3.0);
        c.seek(100);
        assert_eq!(c.tick, 100);
        assert_eq!(c.alpha(), 0.0); // seek clears the sub-tick accumulator
        c.seek(-5);
        assert_eq!(c.tick, 0); // clamped ≥ 0
        c.reset(50);
        assert_eq!(c.tick, 50);
        c.set_rate(-3.0);
        assert_eq!(c.rate, 0.0); // clamped ≥ 0
    }
}
