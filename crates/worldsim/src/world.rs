//! The agent world — port of `agents.svelte.ts`'s manager, chunk (a): the `ManagedAgent` entity, the `World`
//! container (agents + spatial-hash grid + clock + player), and the per-tick **flocking + step** core (the
//! Reynolds anti-overlap / comfort-spread / cohesion+alignment pass, then the steering integration).
//!
//! DETERMINISM CHANGE vs the JS: the JS interleaves flock+step (agent N reads some already-moved neighbours →
//! order-dependent). The Rust **double-buffers** — Phase 2 computes every agent's force from the PREVIOUS
//! positions, Phase 3 steps them all — so the result is invariant to agent order (the §6.8 thread-invariance
//! rule). It is therefore NOT bit-parity with the JS; verified by reproducibility + sanity (overlapping bodies
//! push apart). The food-chain (targeting / predation / combat / stamina-energy-sleep / mobbing / LOD) layers
//! onto this in the next chunks; the `ManagedAgent` already carries that state, seeded by `make_managed`.

use crate::clock::{SimClock, DT};
use crate::eco::{self, eco, prize, slash_max, Hunts, Kind};
use crate::spatialhash::SpatialHashGrid;
use crate::steering::{Agent, Behavior};

const NEIGHBOR_RADIUS: f64 = 4.0; // also the grid cell size (flocking only)
const DENSITY_THRESHOLD: f64 = 1.0; // a lone neighbour is cozy; spread ramps in from the 2nd
const SEP_WEIGHT: f64 = 1.6;
const ALI_WEIGHT: f64 = 0.4;

// food-chain targeting (chunk b)
const SEEK: f64 = 100.0; // a predator notices + stalks prey within this radius; also the seek-grid cell size
const SEEK2: f64 = SEEK * SEEK;
const DANGER2: f64 = 40.0 * 40.0; // prey bolts at 40 m (just outside the 34 m sprint trigger → a head start)
const COMPETE_W: f64 = 1.2; // a prey's appeal drops per hunter already on it → surplus predators fan out
const MAX_HUNTERS: u32 = 3; // a prey claimed by this many is "full" → extra predators peel off to search
const FIGHT_R2: f64 = 3.0 * 3.0; // two predators closer than this stay alert (and apex rivals track each other)
const MAX_CHASE2: f64 = 45.0 * 45.0; // give up a chase once this far from where it began
const GIVEUP_CD: f64 = 5.0; // seconds it won't re-acquire prey after abandoning a chase
const GIVEUP_ENERGY: f64 = 0.06; // ...or it abandons the chase early when this spent (stamina)

const ANIMAL_MENU: &[Behavior] = &[Behavior::Wander, Behavior::Pause, Behavior::LookAround, Behavior::Sit, Behavior::Groom];
const PERSON_MENU: &[Behavior] = &[Behavior::Wander, Behavior::Pause, Behavior::LookAround];

fn menu_for(kind: Kind) -> &'static [Behavior] {
    if matches!(kind, Kind::Person) {
        PERSON_MENU
    } else {
        ANIMAL_MENU
    }
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One ambient agent + its full food-chain state (most fields are consumed by the upcoming chunks; flocking
/// uses only `agent` / `kind` / `radius` / `dead` / `crowd`). Seeded by `make_managed`.
pub struct ManagedAgent {
    pub agent: Agent,
    pub kind: Kind,
    pub radius: f64,
    pub rank: u8,
    pub endurance: f64,
    pub aggressive: bool, // people only — hunts its own kind
    pub seed_id: i32,
    pub stamina: f64, // 0..1 sprint resource
    pub health: f64,  // 0..1; ≤0 = death
    pub meals: u32,
    pub spooked: f64,
    pub mobbed: bool,
    pub dead: bool,
    pub asleep: bool,
    pub sleep_timer: f64,
    pub chase_ox: f64, // chase origin (NaN = not chasing)
    pub chase_oz: f64,
    pub give_up_cd: f64,
    pub hungry: bool, // LATCHED hunger (hysteresis) — set in the food-chain chunk
    pub wake_cd: f64,
    pub slash_max: u32,
    pub slash_budget: u32,
    pub slash_cd: f64,
    pub rival_time: f64,
    pub crowd: u32, // flock neighbours this tick
}

/// Build a fully-seeded managed agent from its kind (so callers don't repeat the eco wiring).
pub fn make_managed(agent: Agent, kind: Kind, radius: f64, seed_id: i32) -> ManagedAgent {
    let e = eco(kind);
    let sm = slash_max(kind, seed_id);
    ManagedAgent {
        agent,
        kind,
        radius,
        rank: e.rank,
        endurance: e.endurance,
        aggressive: matches!(kind, Kind::Person) && eco::aggressive(seed_id),
        seed_id,
        stamina: if matches!(e.hunts, Hunts::Lower) { 0.45 } else { 1.0 }, // carnivores start a touch hungry
        health: 1.0,
        meals: 0,
        spooked: 0.0,
        mobbed: false,
        dead: false,
        asleep: false,
        sleep_timer: 0.0,
        chase_ox: f64::NAN,
        chase_oz: f64::NAN,
        give_up_cd: 0.0,
        hungry: matches!(e.hunts, Hunts::Lower),
        wake_cd: 0.0,
        slash_max: sm,
        slash_budget: sm,
        slash_cd: 0.0,
        rival_time: 0.0,
        crowd: 0,
    }
}

/// Per-tick transient targeting state, kept in a SEPARATE buffer from the agents so the targeting pass can
/// write `transient[j]` (mark prey j's threat / claim) while reading `agents[j]` — no aliasing. Reset each
/// tick. (The JS holds these as fields on ManagedAgent; here they're split out for Rust's borrow rules.)
#[derive(Clone, Copy)]
pub struct Transient {
    pub prey: Option<usize>,    // best prey this predator picked
    pub threat: Option<usize>,  // nearest hunter that picked THIS agent as prey
    prey_score: f64,            // best prey's score (prize / dist² / crowding) — pick the MAX
    threat_d: f64,              // nearest threat's dist² — pick the MIN (init = danger²)
    pub hunted_by: u32,         // predators that claimed this agent as prey this tick (competition tally)
    pub rival: Option<usize>,   // nearest same-rank apex predator crowding it
    rival_d2: f64,
    pub near_predator: bool,    // another hunter is close → stay alert (don't doze)
}

impl Transient {
    fn fresh(danger2: f64) -> Self {
        Transient { prey: None, threat: None, prey_score: 0.0, threat_d: danger2, hunted_by: 0, rival: None, rival_d2: f64::INFINITY, near_predator: false }
    }
}

/// Does `a` hunt `b` right now? (A sleeping/dead `a` doesn't hunt; a sleeping prey CAN be hunted.)
fn preys_on(a: &ManagedAgent, b: &ManagedAgent) -> bool {
    if a.dead || a.asleep || b.dead {
        return false;
    }
    match eco(a.kind).hunts {
        Hunts::Lower => b.rank < a.rank,                                       // cat/lion/dino → anything below
        Hunts::Humans => a.aggressive && matches!(b.kind, Kind::Person),       // aggressive person → people
        Hunts::None => false,
    }
}

pub struct World {
    pub agents: Vec<ManagedAgent>,
    pub clock: SimClock,
    pub transient: Vec<Transient>, // per-tick targeting (parallel to agents), read by the behaviour chunks
    grid: SpatialHashGrid,         // flocking grid (cell = NEIGHBOR_RADIUS)
    seek_grid: SpatialHashGrid,    // coarse food-chain grid (cell = SEEK)
    seek_neighbors: Vec<u32>,      // reused scratch for a seek query (mem::take'd in → no per-agent alloc)
    player: (f64, f64),
    night: f64,                    // 0 day … 1 night → prey jumpier (wider danger radius)
    forces: Vec<(f64, f64, u32)>,  // reused per-tick (fx, fz, crowd) buffer → no per-frame alloc
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        World {
            agents: Vec::new(),
            clock: SimClock::new(),
            transient: Vec::new(),
            grid: SpatialHashGrid::new(NEIGHBOR_RADIUS),
            seek_grid: SpatialHashGrid::new(SEEK),
            seek_neighbors: Vec::new(),
            player: (0.0, 0.0),
            night: 0.0,
            forces: Vec::new(),
        }
    }

    /// How nocturnal the world is (0 day … 1 night) — widens the prey's danger radius.
    pub fn set_night(&mut self, n: f64) {
        self.night = n.clamp(0.0, 1.0);
    }

    /// Spawn an agent; returns its index.
    pub fn spawn(&mut self, agent: Agent, kind: Kind, radius: f64, seed_id: i32) -> usize {
        self.agents.push(make_managed(agent, kind, radius, seed_id));
        self.agents.len() - 1
    }

    pub fn set_player(&mut self, x: f64, z: f64) {
        self.player = (x, z);
    }

    /// Drive the sim from real elapsed seconds (advances the clock; runs each emitted fixed-DT tick).
    pub fn step(&mut self, real_dt: f64) {
        let n = self.clock.advance(real_dt);
        for k in 0..n {
            let tick = self.clock.tick - (n - 1 - k) as i64;
            self.tick_once(tick as i32);
        }
    }

    /// One fixed-DT sim step at the given integer tick (for the addressed rng). grid → flock → step.
    pub fn tick_once(&mut self, tick: i32) {
        let (px, pz) = self.player;
        let n = self.agents.len();
        let danger2 = DANGER2 * (1.0 + 0.5 * self.night); // after dark prey flee from farther

        // 1. rebuild both grids from the PREVIOUS positions (flocking + coarse food-chain)
        self.grid.clear();
        self.seek_grid.clear();
        for (i, m) in self.agents.iter().enumerate() {
            if m.dead {
                continue;
            }
            self.grid.insert(m.agent.x, m.agent.z, i as u32);
            self.seek_grid.insert(m.agent.x, m.agent.z, i as u32);
        }

        // 2. food-chain targeting — reset the transient buffer, then each agent (as predator) picks its best
        // prey + marks its prey's threat, claims it (MAX_HUNTERS cap), and applies the chase-give-up threshold.
        self.transient.clear();
        self.transient.resize(n, Transient::fresh(danger2));
        for i in 0..n {
            if self.agents[i].dead {
                continue;
            }
            self.target(i, danger2);
        }

        // 3. compute every flock force from the previous positions (double-buffered → order-independent)
        self.forces.clear();
        self.forces.resize(n, (0.0, 0.0, 0));
        for i in 0..n {
            if self.agents[i].dead {
                continue;
            }
            let f = self.flock(i, px, pz);
            self.forces[i] = f;
        }

        // 4. step each agent (write the next positions)
        for i in 0..n {
            if self.agents[i].dead {
                continue;
            }
            let (fx, fz, crowd) = self.forces[i];
            self.agents[i].crowd = crowd;
            let menu = menu_for(self.agents[i].kind);
            self.agents[i].agent.update(tick, DT, menu, Some((fx, fz)), 1.0, false);
        }
    }

    /// Reynolds flocking force for agent `i` (anti-overlap + density-gated comfort-spread + cohesion +
    /// alignment), reading only the previous positions. Returns (fx, fz, crowd).
    fn flock(&self, i: usize, px: f64, pz: f64) -> (f64, f64, u32) {
        let m = &self.agents[i];
        let (ax, az, avx, avz, a_max) = (m.agent.x, m.agent.z, m.agent.vx, m.agent.vz, m.agent.max_speed);
        let is_person = matches!(m.kind, Kind::Person);
        let sep_r = m.radius + if is_person { 1.5 } else { 1.2 };
        let hard_r = m.radius + if is_person { 0.4 } else { 0.3 };
        let sep_r2 = sep_r * sep_r;
        let nr2 = NEIGHBOR_RADIUS * NEIGHBOR_RADIUS;

        let mut sep_x = 0.0;
        let mut sep_z = 0.0;
        let mut hard_x = 0.0;
        let mut hard_z = 0.0;
        let mut n_close: u32 = 0;
        let mut coh_x = 0.0;
        let mut coh_z = 0.0;
        let mut ali_x = 0.0;
        let mut ali_z = 0.0;
        let mut n_near: u32 = 0;

        // inline "repel" (the JS closure) — separation + hard anti-overlap.
        let repel = |dx: f64, dz: f64, d2: f64, sx: &mut f64, sz: &mut f64, hx: &mut f64, hz: &mut f64, nc: &mut u32| {
            let d = d2.sqrt().max(0.2);
            if d2 < sep_r2 {
                let w = (sep_r - d) / sep_r / d;
                *sx += dx * w;
                *sz += dz * w;
                *nc += 1;
            }
            if d < hard_r {
                let hw = (hard_r - d) / hard_r / d;
                *hx += dx * hw;
                *hz += dz * hw;
            }
        };

        let agents = &self.agents;
        self.grid.for_each_neighbor(ax, az, |j| {
            let j = j as usize;
            if j == i || agents[j].dead {
                return;
            }
            let o = &agents[j].agent;
            let dx = ax - o.x;
            let dz = az - o.z;
            let d2 = dx * dx + dz * dz;
            if d2 > nr2 {
                return; // out of range (or a hash-collision false neighbour)
            }
            coh_x += o.x;
            coh_z += o.z;
            ali_x += o.vx;
            ali_z += o.vz;
            n_near += 1;
            repel(dx, dz, d2, &mut sep_x, &mut sep_z, &mut hard_x, &mut hard_z, &mut n_close);
        });

        // the player is a separation-only neighbour → crowds part around you
        let pdx = ax - px;
        let pdz = az - pz;
        let pd2 = pdx * pdx + pdz * pdz;
        if pd2 < sep_r2 {
            repel(pdx, pdz, pd2, &mut sep_x, &mut sep_z, &mut hard_x, &mut hard_z, &mut n_close);
        }

        let mut fx = 0.0;
        let mut fz = 0.0;

        // ANTI-OVERLAP — always on so two agents never stand inside each other
        if hard_x != 0.0 || hard_z != 0.0 {
            let hl = {
                let h = hard_x.hypot(hard_z);
                if h == 0.0 {
                    1.0
                } else {
                    h
                }
            };
            let s = (a_max * 1.3) / hl;
            fx += hard_x * s;
            fz += hard_z * s;
        }

        // COMFORT-SPREAD — density-gated (smoothstep → no boundary jitter)
        let sep_gain = smoothstep(0.0, 2.0, n_close as f64 - DENSITY_THRESHOLD);
        if sep_gain > 0.0 && (sep_x != 0.0 || sep_z != 0.0) {
            let sl = {
                let h = sep_x.hypot(sep_z);
                if h == 0.0 {
                    1.0
                } else {
                    h
                }
            };
            let s = (a_max * SEP_WEIGHT * sep_gain) / sl;
            fx += sep_x * s;
            fz += sep_z * s;
        }

        // COHESION + ALIGNMENT — gentle (people barely cohere: 0.06 < animals' 0.1)
        if n_near > 0 {
            let nn = n_near as f64;
            let cdx = coh_x / nn - ax;
            let cdz = coh_z / nn - az;
            let cl = {
                let h = cdx.hypot(cdz);
                if h == 0.0 {
                    1.0
                } else {
                    h
                }
            };
            let coh_w = if is_person { 0.06 } else { 0.1 };
            let c = (a_max * coh_w) / cl;
            fx += cdx * c;
            fz += cdz * c;
            fx += (ali_x / nn - avx) * ALI_WEIGHT;
            fz += (ali_z / nn - avz) * ALI_WEIGHT;
        }

        (fx, fz, n_near)
    }

    /// Food-chain targeting for agent `i` (as the predator): scan its seek-grid neighbours, pick the best prey
    /// (prize / dist² / crowding), mark that prey's nearest threat, flag apex rivals, then CLAIM the prey
    /// (capped at MAX_HUNTERS so surplus predators fan out) and apply the chase give-up threshold. Reads only
    /// previous positions; writes the transient buffer (+ this agent's chase origin / give-up timer).
    fn target(&mut self, i: usize, danger2: f64) {
        let ax = self.agents[i].agent.x;
        let az = self.agents[i].agent.z;
        let a_rank = self.agents[i].rank;
        let a_hunts = matches!(eco(self.agents[i].kind).hunts, Hunts::Lower);
        // metabolism gate (HYSTERESIS — the latched `hungry`, the user's flip-flop fix; updated in the
        // stamina/energy chunk): a sated carnivore, or one cooling off from a given-up chase, doesn't hunt
        // (and so isn't a threat). Non-carnivores always "seek" (their prey check just never matches).
        let a_seeks = if a_hunts {
            self.agents[i].hungry && self.agents[i].give_up_cd <= 0.0
        } else {
            true
        };

        // collect the ~3×3 seek cells into the reused scratch (mem::take → the closure borrows only the buffer)
        let mut neighbors = std::mem::take(&mut self.seek_neighbors);
        neighbors.clear();
        self.seek_grid.for_each_neighbor(ax, az, |j| neighbors.push(j));

        for &ju in &neighbors {
            let j = ju as usize;
            if j == i {
                continue;
            }
            let dx = ax - self.agents[j].agent.x;
            let dz = az - self.agents[j].agent.z;
            let d2 = dx * dx + dz * dz;
            if d2 > SEEK2 {
                continue; // out of notice range (or a hash-collision false neighbour)
            }
            if a_seeks && preys_on(&self.agents[i], &self.agents[j]) {
                // size/proximity score, DISCOUNTED by how many hunters already claimed j → a crowded prey
                // looks worse, so the pack spreads instead of dogpiling one.
                let s = prize(self.agents[j].kind) / (d2.max(1.0) * (1.0 + COMPETE_W * self.transient[j].hunted_by as f64));
                if s > self.transient[i].prey_score {
                    self.transient[i].prey = Some(j);
                    self.transient[i].prey_score = s;
                }
                if d2 < danger2 && d2 < self.transient[j].threat_d {
                    self.transient[j].threat = Some(i); // j fears its nearest hunter
                    self.transient[j].threat_d = d2;
                }
            }
            // two peer predators close together stay ALERT (don't doze in a crowd); same-rank apex predators
            // track each other as rivals (they don't pack — prolonged crowding boils into a territorial fight).
            if a_hunts && d2 < FIGHT_R2 && matches!(eco(self.agents[j].kind).hunts, Hunts::Lower) {
                self.transient[i].near_predator = true;
                self.transient[j].near_predator = true;
                if a_rank >= 4 && a_rank == self.agents[j].rank && d2 < self.transient[i].rival_d2 {
                    self.transient[i].rival = Some(j);
                    self.transient[i].rival_d2 = d2;
                }
            }
        }
        self.seek_neighbors = neighbors; // give the scratch buffer back

        // CLAIM — if the chosen prey already has enough hunters, fan out (drop it); else tally a claim. Earlier
        // agents (stable Vec order) claim first, so the surplus consistently peels away. NOTE: this sequential
        // claim is order-dependent — fine single-threaded/deterministic; revisit for §6.8 multithreading.
        if let Some(p) = self.transient[i].prey {
            if self.transient[p].hunted_by >= MAX_HUNTERS {
                self.transient[i].prey = None;
                self.transient[i].prey_score = 0.0;
            } else {
                self.transient[p].hunted_by += 1;
            }
        }

        // CHASE THRESHOLD — abandon prey chased too far from where the chase began (or when too spent), then
        // cool off (give_up_cd) before hunting again → never pursues forever.
        if a_hunts {
            if self.transient[i].prey.is_some() {
                if self.agents[i].chase_ox.is_nan() {
                    self.agents[i].chase_ox = ax;
                    self.agents[i].chase_oz = az;
                }
                let far = (ax - self.agents[i].chase_ox).powi(2) + (az - self.agents[i].chase_oz).powi(2) > MAX_CHASE2;
                if far || self.agents[i].stamina < GIVEUP_ENERGY {
                    self.transient[i].prey = None;
                    self.transient[i].prey_score = 0.0;
                    self.agents[i].give_up_cd = GIVEUP_CD;
                    self.agents[i].chase_ox = f64::NAN;
                }
            } else {
                self.agents[i].chase_ox = f64::NAN; // not chasing → reset the origin for the next hunt
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steering::AgentOpts;

    fn animal(x: f64, z: f64, seed: i32) -> Agent {
        Agent::new(x, z, seed, &AgentOpts { max_speed: 3.0, home_radius: 30.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.3 })
    }

    #[test]
    fn spawns_and_steps_deterministically() {
        let run = || {
            let mut w = World::new();
            for k in 0..8 {
                let s = 100 + k;
                w.spawn(animal(k as f64 * 0.5, 0.0, s), Kind::Cat, 0.35, s);
            }
            for t in 1..=200 {
                w.tick_once(t);
            }
            (w.agents[0].agent.x, w.agents[7].agent.z, w.agents[3].agent.heading)
        };
        let a = run();
        let b = run();
        assert_eq!(a.0.to_bits(), b.0.to_bits()); // bit-identical → deterministic + order-independent
        assert_eq!(a.1.to_bits(), b.1.to_bits());
        assert_eq!(a.2.to_bits(), b.2.to_bits());
        assert!(a.0.is_finite() && a.1.is_finite());
    }

    #[test]
    fn overlapping_bodies_push_apart() {
        // two agents almost on top of each other → the anti-overlap force must separate them
        let mut w = World::new();
        w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        w.spawn(animal(0.05, 0.0, 2), Kind::Rabbit, 0.35, 2);
        let d0 = {
            let (a, b) = (&w.agents[0].agent, &w.agents[1].agent);
            (a.x - b.x).hypot(a.z - b.z)
        };
        for t in 1..=40 {
            w.tick_once(t);
        }
        let d1 = {
            let (a, b) = (&w.agents[0].agent, &w.agents[1].agent);
            (a.x - b.x).hypot(a.z - b.z)
        };
        assert!(d1 > d0 + 0.2, "expected separation: d0={d0}, d1={d1}");
        assert_eq!(w.agents[0].crowd, 1); // each sees exactly one neighbour
    }

    #[test]
    fn predator_targets_prey() {
        let mut w = World::new();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(5.0, 0.0, 11), Kind::Rabbit, 0.35, 11);
        w.tick_once(1);
        assert_eq!(w.transient[cat].prey, Some(rabbit)); // hungry cat picks the rabbit
        assert_eq!(w.transient[rabbit].threat, Some(cat)); // …and the rabbit fears it
        assert_eq!(w.transient[rabbit].hunted_by, 1);
        assert_eq!(w.transient[cat].threat, None); // a cat has no predator here
        assert_eq!(w.transient[rabbit].prey, None); // a rabbit hunts nothing
    }

    #[test]
    fn fed_predator_doesnt_seek() {
        let mut w = World::new();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(5.0, 0.0, 11), Kind::Rabbit, 0.35, 11);
        w.agents[cat].hungry = false; // sated → not hunting, and so not a threat
        w.tick_once(1);
        assert_eq!(w.transient[cat].prey, None);
        assert_eq!(w.transient[rabbit].threat, None);
    }

    #[test]
    fn max_hunters_caps_claim() {
        let mut w = World::new();
        let rabbit = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let cats: Vec<usize> = (0..5).map(|k| w.spawn(animal(2.0 + k as f64 * 0.4, 0.0, 100 + k), Kind::Cat, 0.35, 100 + k)).collect();
        w.tick_once(1);
        let claimers = cats.iter().filter(|&&c| w.transient[c].prey == Some(rabbit)).count();
        assert_eq!(claimers, MAX_HUNTERS as usize); // exactly 3 claim the one rabbit; the rest fan out
        assert_eq!(w.transient[rabbit].hunted_by, MAX_HUNTERS);
    }
}
