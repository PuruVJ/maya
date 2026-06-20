//! Agent steering — the deterministic port of `src/lib/steering.ts`'s `Agent` (action-selection FSM →
//! Reynolds-style wander → vehicle integration). This is the FIRST non-pure module: the live JS uses
//! `Math.random`, so there is no bit-parity with it — instead every random draw is now ADDRESSED by
//! `(seedId, tick, channel)` (the determinism migration the JS never did), making the sim reproducible and
//! thread-count-invariant (§6.8). The discrete state (behaviour picks + their TIMING) is bit-exact across
//! machines because it's driven only by the integer/bit-exact RNG and exact `elapsed = Σ dt`; only the
//! continuous positions differ in the last bits between platforms' `sin`/`cos`/`atan2` (≈1e-13), which is
//! immaterial. The parity test pins both: behaviour changes EXACTLY, positions to 1e-4, against a captured
//! JS reference that re-implements the same logic with this same addressed RNG. The `Spring` helper (lerp
//! secondary motion) is a VIEW concern and stays in JS.

use crate::simrng::{rand, range};

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

// RNG channels — one per draw SITE so two draws at the same (seedId, tick) don't correlate. Birth rolls key
// by (seedId, channel); per-tick rolls add the tick.
const CH_WANDER: i32 = 1;
const CH_PICK_RELOC: i32 = 2;
const CH_PICK_ANG: i32 = 3;
const CH_PICK_FAR: i32 = 4;
const CH_PICK_RELOC_DUR: i32 = 5;
const CH_PICK_CHOICE: i32 = 6;
const CH_PICK_CHOICE_DUR: i32 = 7;
const CH_HEADING: i32 = 10;
const CH_EXPLORER: i32 = 11;
const CH_PERSONALITY: i32 = 12;
const CH_DURATION: i32 = 13;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Behavior {
    Wander,
    Pause,
    LookAround,
    Groom,
    Sit,
    Pounce,
}

impl Behavior {
    fn weight(self) -> f64 {
        match self {
            Behavior::Wander => 5.0,
            Behavior::Pause => 2.0,
            Behavior::LookAround => 1.4,
            Behavior::Groom => 1.0,
            Behavior::Sit => 1.0,
            Behavior::Pounce => 0.8,
        }
    }
    #[inline]
    fn moving(self) -> bool {
        matches!(self, Behavior::Wander | Behavior::Pounce)
    }
    /// Stable u8 code for the JS render bridge (the renderers map it back to a pose). Order = enum order.
    pub fn code(self) -> u8 {
        match self {
            Behavior::Wander => 0,
            Behavior::Pause => 1,
            Behavior::LookAround => 2,
            Behavior::Groom => 3,
            Behavior::Sit => 4,
            Behavior::Pounce => 5,
        }
    }
}

pub struct AgentOpts {
    pub max_speed: f64,
    pub home_radius: f64,
    pub wander_rate: f64,
    pub accel: f64,
    pub turn_speed: f64,
    pub wanderlust: f64,
}

impl Default for AgentOpts {
    fn default() -> Self {
        Self { max_speed: 2.4, home_radius: 24.0, wander_rate: 2.4, accel: 7.0, turn_speed: 6.0, wanderlust: 0.14 }
    }
}

pub struct Agent {
    pub x: f64,
    pub z: f64,
    pub vx: f64,
    pub vz: f64,
    pub speed: f64,
    pub turn_rate: f64,
    pub heading: f64,
    pub wander_angle: f64,
    pub hx: f64, // home (leash centre)
    pub hz: f64,
    pub behavior: Behavior,
    pub elapsed: f64,
    pub duration: f64,
    pub seed_id: i32,
    pub explorer: bool,
    pub personality: f64,
    pub max_speed: f64, // public so the flock manager can scale forces to it
    home_radius: f64,
    wander_rate: f64,
    accel: f64,
    turn_speed: f64,
}

impl Agent {
    pub fn new(x: f64, z: f64, seed_id: i32, o: &AgentOpts) -> Self {
        let heading = range(0.0, TAU, &[seed_id, CH_HEADING]);
        Agent {
            x,
            z,
            vx: 0.0,
            vz: 0.0,
            speed: 0.0,
            turn_rate: 0.0,
            heading,
            wander_angle: heading,
            hx: x,
            hz: z,
            behavior: Behavior::Wander,
            elapsed: 0.0,
            duration: range(2.0, 5.0, &[seed_id, CH_DURATION]),
            seed_id,
            explorer: rand(&[seed_id, CH_EXPLORER]) < o.wanderlust,
            personality: range(0.3, 0.85, &[seed_id, CH_PERSONALITY]),
            max_speed: o.max_speed,
            home_radius: o.home_radius,
            wander_rate: o.wander_rate,
            accel: o.accel,
            turn_speed: o.turn_speed,
        }
    }

    /// Move the leash centre (e.g. keep a critter loosely near the player).
    pub fn set_home(&mut self, x: f64, z: f64) {
        self.hx = x;
        self.hz = z;
    }

    fn pick(&mut self, tick: i32, menu: &[Behavior]) {
        let s = self.seed_id;
        // explorers occasionally strike out for a far-off place — relocate the leash there.
        if self.explorer && rand(&[s, tick, CH_PICK_RELOC]) < 0.22 {
            let ang = range(0.0, TAU, &[s, tick, CH_PICK_ANG]);
            let far = range(70.0, 200.0, &[s, tick, CH_PICK_FAR]);
            self.hx = self.x + ang.sin() * far;
            self.hz = self.z + ang.cos() * far;
            self.behavior = Behavior::Wander;
            self.elapsed = 0.0;
            self.duration = range(4.0, 8.0, &[s, tick, CH_PICK_RELOC_DUR]);
            return;
        }
        // weighted: heavily favour wander, then pauses, then the flavour behaviours.
        let total: f64 = menu.iter().map(|b| b.weight()).sum();
        let mut r = range(0.0, total, &[s, tick, CH_PICK_CHOICE]);
        let mut chosen = menu[0];
        for &b in menu {
            r -= b.weight();
            if r <= 0.0 {
                chosen = b;
                break;
            }
        }
        self.behavior = chosen;
        self.elapsed = 0.0;
        self.duration = match chosen {
            Behavior::Wander => range(3.0, 7.0, &[s, tick, CH_PICK_CHOICE_DUR]),
            Behavior::Pounce => range(0.45, 0.7, &[s, tick, CH_PICK_CHOICE_DUR]),
            _ => range(1.6, 4.2, &[s, tick, CH_PICK_CHOICE_DUR]),
        };
    }

    /// One sim step. `flock` is the manager's Reynolds force (None in isolation); `boost` is the chase/flee
    /// speed multiplier; `force_move` keeps a chaser running instead of idling.
    pub fn update(&mut self, tick: i32, dt: f64, menu: &[Behavior], flock: Option<(f64, f64)>, boost: f64, force_move: bool) {
        self.elapsed += dt;
        if force_move {
            self.behavior = Behavior::Wander;
        } else if self.elapsed >= self.duration {
            self.pick(tick, menu);
        }

        let mut dvx = 0.0;
        let mut dvz = 0.0;
        let mut cap = self.max_speed;
        if self.behavior.moving() {
            // Reynolds wander: a target on a circle ahead, nudged a little each tick.
            self.wander_angle += range(-1.0, 1.0, &[self.seed_id, tick, CH_WANDER]) * self.wander_rate * dt;
            let cx = self.x + self.heading.sin() * 1.5;
            let cz = self.z + self.heading.cos() * 1.5;
            let mut tx = cx + self.wander_angle.sin();
            let mut tz = cz + self.wander_angle.cos();
            // containment — Arrive back toward home when past the leash.
            let home_dist = (self.x - self.hx).hypot(self.z - self.hz);
            if home_dist > self.home_radius {
                tx = self.hx;
                tz = self.hz;
                self.wander_angle = (self.hx - self.x).atan2(self.hz - self.z);
            }
            let tdx = tx - self.x;
            let tdz = tz - self.z;
            let td = {
                let h = tdx.hypot(tdz);
                if h == 0.0 {
                    1.0
                } else {
                    h
                }
            };
            let burst = if matches!(self.behavior, Behavior::Pounce) { 2.3 } else { 1.0 };
            cap = self.max_speed * burst;
            dvx = (tdx / td) * cap;
            dvz = (tdz / td) * cap;
        }

        cap *= boost;
        if let Some((fx, fz)) = flock {
            dvx += fx;
            dvz += fz;
            let dmag = dvx.hypot(dvz);
            if dmag > cap && dmag > 0.0 {
                dvx = (dvx / dmag) * cap;
                dvz = (dvz / dmag) * cap;
            }
        }

        // steering force = desired − current, applied as acceleration (the life-vs-mechanical fix).
        let k = (self.accel * dt).min(1.0);
        self.vx += (dvx - self.vx) * k;
        self.vz += (dvz - self.vz) * k;
        self.x += self.vx * dt;
        self.z += self.vz * dt;
        self.speed = self.vx.hypot(self.vz);

        // heading follows velocity; record signed turn rate for banking / tail lag.
        if self.speed > 0.06 {
            let desired = self.vx.atan2(self.vz);
            let mut dh = desired - self.heading;
            while dh > PI {
                dh -= TAU;
            }
            while dh < -PI {
                dh += TAU;
            }
            let turn = dh * (self.turn_speed * dt).min(1.0);
            self.heading += turn;
            self.turn_rate = turn / dt.max(1e-3);
        } else {
            self.turn_rate *= (1.0 - 4.0 * dt).max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: f64 = 1.0 / 30.0;

    fn close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-4, "expected ~{want}, got {got}");
    }

    // Reference captured from src/lib/steering.ts's logic re-run with this addressed RNG (the determinism
    // migration). Behaviour changes + their TIMING are bit-exact (rng/integer driven); positions match to
    // 1e-4 (platform sin/cos/atan2 last-bits + the reference's 6-dp rounding).
    #[test]
    fn steering_parity() {
        let menu = [Behavior::Wander, Behavior::Pause, Behavior::LookAround, Behavior::Groom, Behavior::Sit];
        let o = AgentOpts { max_speed: 3.0, home_radius: 40.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.55 };
        let mut a = Agent::new(0.0, 0.0, 12345, &o);

        // birth rolls
        close(a.heading, 5.11553);
        assert!(a.explorer);
        close(a.personality, 0.808471);
        close(a.duration, 3.191605);

        let mut changes: Vec<(i32, Behavior)> = Vec::new();
        let mut last = a.behavior;
        for t in 1..=250 {
            a.update(t, DT, &menu, None, 1.0, false);
            if a.behavior != last {
                changes.push((t, a.behavior));
                last = a.behavior;
            }
            match t {
                1 => {
                    close(a.x, -0.021331);
                    close(a.z, 0.009457);
                    close(a.heading, 5.117892);
                    assert_eq!(a.behavior, Behavior::Wander);
                }
                50 => {
                    close(a.x, -3.930816);
                    close(a.z, 2.507159);
                }
                120 => {
                    close(a.x, -7.778401);
                    close(a.z, 5.421086);
                    assert_eq!(a.behavior, Behavior::Pause);
                    close(a.duration, 3.735077);
                }
                250 => {
                    close(a.x, -10.75285);
                    close(a.z, 7.898893);
                    assert_eq!(a.behavior, Behavior::Wander);
                    close(a.duration, 4.821398);
                }
                _ => {}
            }
        }
        // bit-exact discrete events: pause at tick 96, back to wander at 209
        assert_eq!(changes, vec![(96, Behavior::Pause), (209, Behavior::Wander)]);
    }

    #[test]
    fn reproducible() {
        let menu = [Behavior::Wander, Behavior::Pause];
        let o = AgentOpts::default();
        let run = || {
            let mut a = Agent::new(3.0, -2.0, 999, &o);
            for t in 1..=300 {
                a.update(t, DT, &menu, None, 1.0, false);
            }
            (a.x, a.z, a.heading, a.behavior)
        };
        let r1 = run();
        let r2 = run();
        assert_eq!(r1.0.to_bits(), r2.0.to_bits()); // bit-identical → deterministic
        assert_eq!(r1.1.to_bits(), r2.1.to_bits());
        assert_eq!(r1.2.to_bits(), r2.2.to_bits());
        assert_eq!(r1.3, r2.3);
    }
}
