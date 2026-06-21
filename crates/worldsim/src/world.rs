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
use crate::eco::{self, eco, prize, sleep_secs, slash_max, Hunts, Kind};
use crate::spatialhash::SpatialHashGrid;
use crate::steering::{Agent, AgentOpts, Behavior};

const NEIGHBOR_RADIUS: f64 = 4.0; // also the grid cell size (flocking only)
const DENSITY_THRESHOLD: f64 = 0.4; // spread ramps in almost immediately (was 1.0 → small clumps lingered)
const SEP_WEIGHT: f64 = 1.8; // push apart a touch harder → divergent fan-out
const ALI_WEIGHT: f64 = 0.05; // was 0.4 → agents matched velocities + moved as ONE direction; near-off so they FAN OUT

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

// mobbing (chunk e) — when prey heavily outnumber one hunter, the herd turns and swarms it
const MOB_MIN: u32 = 4; // this many prey fleeing ONE hunter flips them flee → swarm
const MOB_RELEASE: u32 = 3; // hysteresis: a mobbed hunter stays mobbed until the swarm thins BELOW this
const MOB_W: f64 = 2.2; // converge force as the mob charges the predator
const MOB_KILL_DPS: f64 = 0.03; // health/s a hunter loses PER attacker pressed against it (size+health combo)
const SLASH_CD: f64 = 1.2; // seconds between a cornered hunter's retaliatory slashes (each kills one attacker)
const HURT_AT: f64 = 0.45; // below this health an animal is injured → limps (HURT_SPEED) and flees
const HURT_SPEED: f64 = 0.6; // injured locomotion multiplier (so a healthy hunter can run it down)
const RIVAL_PATIENCE: f64 = 5.0; // seconds two apex predators tolerate crowding before they turn and fight
const RIVAL_DPS: f64 = 0.35; // health/s each loses in a territorial scrap → one breaks off wounded (or down)

// predation/combat behaviour (chunk c)
const HUNT2: f64 = 34.0 * 34.0; // a predator breaks into a sprint for the kill once this close
const FLEE_W: f64 = 2.6; // strong — overrides wander when running for your life
const CHASE_W: f64 = 2.0;
const AVOID_W: f64 = 1.8; // gentle personal space — every animal steers AROUND the player's body
const FLEE_BOOST: f64 = 1.7; // panic-run speed multiplier
const CHASE_BOOST: f64 = 1.45;
const CONTACT_PAD: f64 = 0.4; // extra reach that counts as a catch
const CAN_SPRINT: f64 = 0.03; // stamina above this can still sprint
const LURE_R: f64 = 11.0; // an idle cat within this range of a lake fish pads to the bank after it (never catches)
const OBSTACLE_CELL: f64 = 12.0; // obstacle grid cell — must exceed the biggest footprint+body radius (port of the JS)

/// A solid the agents can't walk through — a CIRCLE (props/ponds) or an ORIENTED BOX (buildings, so animals hug
/// walls / use streets like the player). Port of the JS `Obstacle`; `is_box` picks the resolve path.
#[derive(Clone, Copy)]
struct Obstacle {
    x: f64,
    z: f64,
    r: f64,         // bounding radius (circle radius, or the box's corner radius so the grid still finds it)
    hx: f64,        // box half-extent along local X (used when is_box)
    hz: f64,        // box half-extent along local Z
    cos: f64,       // cos/sin of the box's Y-rotation
    sin: f64,
    is_box: bool,
}

// stamina / energy / metabolism (chunk d)
const EXERT_DRAIN: f64 = 0.22; // /s while sprinting, divided by endurance
const RECOVER: f64 = 0.16; // /s at rest (prey/people only), × endurance
const BASAL_DRAIN: f64 = 0.02; // /s a carnivore's energy always ebbs → it must eat to sustain (no idle recover)
const EAT_GAIN: f64 = 0.6; // a kill refuels this much energy
const HEAL: f64 = 0.04; // health/s regained while unharmed
// HYSTERESIS hunger latch (the user's flip-flop fix): a carnivore commits to hunting below LO and won't stop
// (rest) until eating lifts it past HI — without the gap, energy at one threshold flipped hunting on/off every
// tick, so its prey flip-flopped flee↔doze. Latched in `hungry`.
const HUNGRY_LO: f64 = 0.5;
const HUNGRY_HI: f64 = 0.72;

// sleep (chunk d2)
const SLEEP_MULT: f64 = 2.4; // recover this much faster while asleep
const WAKE_REST: f64 = 3.0; // after waking, stay up at least this long before dozing again (anti sleep/wake flip)
const WAKE_BASE: f64 = 1.5; // a tiptoeing player can get this close to a sleeper; sprinting startles it from
const WAKE_MAX: f64 = 7.0; // …farther (scaled by player speed) — the sneak mechanic

const ANIMAL_MENU: &[Behavior] = &[Behavior::Wander, Behavior::Pause, Behavior::LookAround, Behavior::Sit, Behavior::Groom, Behavior::Pounce];
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
    pub rival_time: f64,      // seconds crowded by a rival → ≥RIVAL_PATIENCE boils into a territorial fight
    pub bully: Option<usize>, // who last wounded it in a rival fight → it flees this one while spooked
    pub companion: bool,      // the player's pet → its leash tracks the player (follows) and it doesn't fear you
    pub hunting: bool,        // this apex is actively charging the PLAYER this tick → the view glares its eyes
    pub crowd: u32,           // flock neighbours this tick
}

/// Build a fully-seeded managed agent from its kind (so callers don't repeat the eco wiring).
/// Default steering opts for a kind — mirrors the JS `Critter`/`Npc` Agent configs so the bridge can spawn an
/// agent from just `(kind, seedId)`. People roam a wide leash and EXPLORE (high wanderlust → they disperse,
/// don't clump); animals keep a tighter leash + loose flocks. `max_speed` is the per-individual eco roll.
pub fn opts_for(kind: Kind, seed_id: i32) -> AgentOpts {
    let max_speed = eco::speed_for(kind, seed_id);
    // higher wanderlust → more agents are far-roaming EXPLORERS that relocate their leash + journey the map,
    // so they DISPERSE instead of orbiting overlapping home spawns and clumping. Wider leash too (roam further).
    if kind == Kind::Person {
        AgentOpts { max_speed, home_radius: 55.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.72 }
    } else {
        AgentOpts { max_speed, home_radius: 42.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.52 }
    }
}

/// A flat struct-of-arrays read-back the JS renderer consumes by index (no per-agent JS↔WASM calls). Filled
/// from the AoS agent Vec after each tick; the wasm wrapper hands JS typed-array VIEWS over these buffers.
#[derive(Default)]
pub struct Snapshot {
    pub xs: Vec<f32>,       // world X per agent
    pub zs: Vec<f32>,       // world Z per agent
    pub headings: Vec<f32>, // facing (radians)
    pub healths: Vec<f32>,    // 0..1 (drives injury/blood/limp on the view side)
    pub flags: Vec<u32>,      // bit0 = dead, bit1 = asleep, bit2 = moving (speed past a walk), bit3 = hunting-player
    pub behaviors: Vec<u8>,   // current idle-FSM behaviour code (0 wander·1 pause·2 lookAround·3 groom·4 sit·5 pounce)
    pub progress: Vec<f32>,   // 0..1 through the current behaviour → drives groom cycles / pounce arcs on the view
}

impl Snapshot {
    /// Resize + fill every buffer from the world's current agents (stable order = spawn order = JS index).
    pub fn fill(&mut self, world: &World) {
        let n = world.agents.len();
        self.xs.resize(n, 0.0);
        self.zs.resize(n, 0.0);
        self.headings.resize(n, 0.0);
        self.healths.resize(n, 0.0);
        self.flags.resize(n, 0);
        self.behaviors.resize(n, 0);
        self.progress.resize(n, 0.0);
        for (i, m) in world.agents.iter().enumerate() {
            self.xs[i] = m.agent.x as f32;
            self.zs[i] = m.agent.z as f32;
            self.headings[i] = m.agent.heading as f32;
            self.healths[i] = m.health as f32;
            let mut f = 0u32;
            if m.dead {
                f |= 1;
            }
            if m.asleep {
                f |= 2;
            }
            if m.agent.speed > 1.0 {
                f |= 4;
            }
            if m.hunting {
                f |= 8;
            }
            self.flags[i] = f;
            self.behaviors[i] = m.agent.behavior.code();
            self.progress[i] = (m.agent.elapsed / m.agent.duration).min(1.0) as f32;
        }
    }
}

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
        bully: None,
        companion: false,
        hunting: false,
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
    pub mob_count: u32,         // prey currently fleeing THIS agent → ≥MOB_MIN swarms it
    mob_x: f64,                 // running sum of those mobbers' positions → their centroid (for break-away)
    mob_z: f64,
    pub attackers: u32,         // mobbers actually pressed into CONTACT → they wound it; it slashes them
}

impl Transient {
    fn fresh(danger2: f64) -> Self {
        Transient {
            prey: None,
            threat: None,
            prey_score: 0.0,
            threat_d: danger2,
            hunted_by: 0,
            rival: None,
            rival_d2: f64::INFINITY,
            near_predator: false,
            mob_count: 0,
            mob_x: 0.0,
            mob_z: 0.0,
            attackers: 0,
        }
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
    pub danger: f64,               // 0..1 — how imminent a player-hunting predator is (eased → the UI vignette)
    pub transient: Vec<Transient>, // per-tick targeting (parallel to agents), read by the behaviour chunks
    grid: SpatialHashGrid,         // flocking grid (cell = NEIGHBOR_RADIUS)
    seek_grid: SpatialHashGrid,    // coarse food-chain grid (cell = SEEK)
    seek_neighbors: Vec<u32>,      // reused scratch for a seek query (mem::take'd in → no per-agent alloc)
    player: (f64, f64),
    last_player: (f64, f64),       // previous tick's player pos → its speed (a running player scares wildlife)
    night: f64,                    // 0 day … 1 night → prey jumpier (wider danger radius)
    forces: Vec<(f64, f64, u32)>,  // reused per-tick (fx, fz, crowd) flock buffer → no per-frame alloc
    behave: Vec<(f64, bool)>,      // reused per-tick (boost, pursuing) from the behaviour pass
    kills: Vec<usize>,             // prey caught this tick → turned to corpses after the behaviour pass
    slept: Vec<bool>,              // was asleep AT TICK START → handled by the sleep pass, skipped elsewhere
    fish: Vec<(f64, f64)>,         // lake-fish lure points (fed from the JS view; cats pad to the bank after them)
    obstacles: Vec<Obstacle>,      // solid props/buildings/ponds → agents are pushed out (no tunnelling)
    ob_grid: SpatialHashGrid,      // obstacle lookup grid (cell = OBSTACLE_CELL), rebuilt on set_obstacles
    has_obstacles: bool,
    ob_scratch: Vec<u32>,          // reused obstacle-query scratch (mem::take'd in → no per-agent alloc)
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
            danger: 0.0,
            transient: Vec::new(),
            grid: SpatialHashGrid::new(NEIGHBOR_RADIUS),
            seek_grid: SpatialHashGrid::new(SEEK),
            seek_neighbors: Vec::new(),
            player: (0.0, 0.0),
            last_player: (f64::NAN, f64::NAN),
            night: 0.0,
            forces: Vec::new(),
            behave: Vec::new(),
            kills: Vec::new(),
            slept: Vec::new(),
            fish: Vec::new(),
            obstacles: Vec::new(),
            ob_grid: SpatialHashGrid::new(OBSTACLE_CELL),
            has_obstacles: false,
            ob_scratch: Vec::new(),
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

    /// Mark agent `i` as the player's pet — its wander leash will track the player (so it follows you around)
    /// and it won't flee/berth you. (Idempotent; pass a valid spawned index.)
    pub fn set_companion(&mut self, i: usize) {
        if let Some(m) = self.agents.get_mut(i) {
            m.companion = true;
        }
    }

    /// Remove agent `i` from the simulation (its world-object was deleted / the world cleared). Marks it dead,
    /// which every pass already skips (grid insert, targeting, flock, behaviour, step) → it goes fully inert and
    /// stops affecting the food chain, instead of lingering as an invisible ghost. The slot is not reused (the
    /// read-back is index-stable); the JS adapter stops tracking it so it's never rendered.
    pub fn despawn(&mut self, i: usize) {
        if let Some(m) = self.agents.get_mut(i) {
            m.dead = true;
            m.asleep = false;
            m.companion = false;
        }
    }

    /// Replace the lake-fish lure points (the JS fish view owns the fish; the sim only needs their positions
    /// so an idle cat can pad to the water's edge after one). `xz` is a flat [x0,z0,x1,z1,…] buffer.
    pub fn set_fish(&mut self, xz: &[f64]) {
        self.fish.clear();
        self.fish.extend(xz.chunks_exact(2).map(|c| (c[0], c[1])));
    }

    /// Replace the solid obstacles agents route around. `flat` is a packed [x,z,r,hx,hz,cos,sin] per obstacle
    /// (7 f64s each); a CIRCLE is signalled by `hx = NaN` (then only x/z/r matter), else it's an oriented box.
    /// Rebuilds the obstacle grid (called only when the world's objects change, not per tick).
    pub fn set_obstacles(&mut self, flat: &[f64]) {
        self.obstacles.clear();
        self.ob_grid.clear();
        for c in flat.chunks_exact(7) {
            let o = Obstacle { x: c[0], z: c[1], r: c[2], hx: c[3], hz: c[4], cos: c[5], sin: c[6], is_box: !c[3].is_nan() };
            self.ob_grid.insert(o.x, o.z, self.obstacles.len() as u32);
            self.obstacles.push(o);
        }
        self.has_obstacles = !self.obstacles.is_empty();
    }

    /// Hard push-out: keep agent `i` out of every nearby solid (circle or oriented box), cancelling only the
    /// velocity component driving INTO it so it SLIDES along the surface. Port of the JS `#resolveObstacles`.
    fn resolve_obstacles(&mut self, i: usize) {
        if !self.has_obstacles {
            return;
        }
        let (ax, az) = (self.agents[i].agent.x, self.agents[i].agent.z);
        let mut scratch = std::mem::take(&mut self.ob_scratch);
        scratch.clear();
        self.ob_grid.for_each_neighbor(ax, az, |oi| scratch.push(oi));
        let radius = self.agents[i].radius;
        for &oi in &scratch {
            let o = self.obstacles[oi as usize];
            let a = &mut self.agents[i].agent;
            let dx = a.x - o.x;
            let dz = a.z - o.z;
            let nx;
            let nz;
            if o.is_box {
                // ORIENTED BOX — rotate into local frame, clamp, eject along the least-penetration axis
                let (cs, sn) = (o.cos, o.sin);
                let lx = dx * cs - dz * sn;
                let lz = dx * sn + dz * cs;
                let hx = o.hx + radius;
                let hz = o.hz + radius;
                if lx.abs() >= hx || lz.abs() >= hz {
                    continue; // outside the inflated box
                }
                let mut nlx = lx;
                let mut nlz = lz;
                let lnx;
                let lnz;
                if hx - lx.abs() < hz - lz.abs() {
                    nlx = if lx >= 0.0 { hx } else { -hx };
                    lnx = if lx >= 0.0 { 1.0 } else { -1.0 };
                    lnz = 0.0;
                } else {
                    nlz = if lz >= 0.0 { hz } else { -hz };
                    lnx = 0.0;
                    lnz = if lz >= 0.0 { 1.0 } else { -1.0 };
                }
                a.x = o.x + (nlx * cs + nlz * sn); // local → world
                a.z = o.z + (-nlx * sn + nlz * cs);
                nx = lnx * cs + lnz * sn; // push normal → world
                nz = -lnx * sn + lnz * cs;
            } else {
                let min = o.r + radius;
                let d2 = dx * dx + dz * dz;
                if d2 >= min * min || d2 == 0.0 {
                    continue;
                }
                let d = d2.sqrt();
                nx = dx / d;
                nz = dz / d;
                a.x = o.x + nx * min; // shove out to the footprint edge
                a.z = o.z + nz * min;
            }
            let vn = a.vx * nx + a.vz * nz; // cancel only the inward component → SLIDE, don't stick
            if vn < 0.0 {
                a.vx -= vn * nx;
                a.vz -= vn * nz;
                a.speed = a.vx.hypot(a.vz);
            }
        }
        self.ob_scratch = scratch;
    }

    /// Nearest fish to (x,z) within `r` — linear scan (fish are few); returns its position.
    fn nearest_fish(&self, x: f64, z: f64, r: f64) -> Option<(f64, f64)> {
        let r2 = r * r;
        let mut best: Option<(f64, f64)> = None;
        let mut best_d2 = r2;
        for &(fx, fz) in &self.fish {
            let d2 = (fx - x).powi(2) + (fz - z).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some((fx, fz));
            }
        }
        best
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
        let pspeed = if self.last_player.0.is_nan() {
            0.0
        } else {
            (px - self.last_player.0).hypot(pz - self.last_player.1) / DT
        };
        self.last_player = (px, pz);
        let n = self.agents.len();
        let danger2 = DANGER2 * (1.0 + 0.5 * self.night); // after dark prey flee from farther

        // a pet's wander leash tracks the player → it wanders near you, i.e. trails along wherever you go
        for m in self.agents.iter_mut() {
            if m.companion {
                m.agent.set_home(px, pz);
            }
        }

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

        // 2b. MOBBING TALLY — how many prey flee each hunter + the sum of their positions (centroid), so a
        // hunter knows when it's outnumbered and the herd can converge on it.
        for i in 0..n {
            if self.agents[i].dead {
                continue;
            }
            if let Some(t) = self.transient[i].threat {
                let mx = self.agents[i].agent.x;
                let mz = self.agents[i].agent.z;
                self.transient[t].mob_count += 1;
                self.transient[t].mob_x += mx;
                self.transient[t].mob_z += mz;
                // a mobber pressed into contact is actively attacking → it wounds the hunter + can be slashed
                let dx = mx - self.agents[t].agent.x;
                let dz = mz - self.agents[t].agent.z;
                let reach = self.agents[i].radius + self.agents[t].radius + CONTACT_PAD + 0.4;
                if dx * dx + dz * dz < reach * reach {
                    self.transient[t].attackers += 1;
                }
            }
        }

        // 2c. latch the mobbed state with HYSTERESIS — set at MOB_MIN, released only below MOB_RELEASE — so a
        // count hovering at the boundary can't flip the hunter flee↔chase every tick (which froze it).
        for i in 0..n {
            let c = self.transient[i].mob_count;
            if c >= MOB_MIN {
                self.agents[i].mobbed = true;
            } else if c < MOB_RELEASE {
                self.agents[i].mobbed = false;
            }
        }

        // 3. SLEEP — agents asleep AT TICK START (snapshot) recover faster, drift to a stop, and WAKE if
        // disturbed (a hunter near, a fresh scare, or the player too close). They stay grid neighbours but do
        // NOT act this tick (skipped in the flock/behaviour/metabolism/step passes via `slept`).
        self.slept.clear();
        self.slept.resize(n, false);
        for i in 0..n {
            self.slept[i] = self.agents[i].asleep && !self.agents[i].dead;
        }
        for i in 0..n {
            if !self.slept[i] {
                continue;
            }
            // heal + cooldown timers (the awake path does these in the metabolism pass)
            self.agents[i].health = (self.agents[i].health + HEAL * DT).min(1.0);
            if self.agents[i].spooked > 0.0 {
                self.agents[i].spooked -= DT;
            }
            if self.agents[i].give_up_cd > 0.0 {
                self.agents[i].give_up_cd -= DT;
            }
            if self.agents[i].wake_cd > 0.0 {
                self.agents[i].wake_cd -= DT;
            }
            // recover faster while asleep (a carnivore's only non-eating recovery); run the sleep timer down
            self.agents[i].stamina = (self.agents[i].stamina + RECOVER * self.agents[i].endurance * SLEEP_MULT * DT).min(1.0);
            self.agents[i].sleep_timer -= DT;
            // drift to a stop
            let decay = (1.0 - 3.0 * DT).max(0.0);
            self.agents[i].agent.vx *= decay;
            self.agents[i].agent.vz *= decay;
            let (vx, vz) = (self.agents[i].agent.vx, self.agents[i].agent.vz);
            self.agents[i].agent.x += vx * DT;
            self.agents[i].agent.z += vz * DT;
            self.agents[i].agent.speed = vx.hypot(vz);
            // wake check — you can tiptoe within WAKE_BASE of a sleeper, but SPRINT at it and it startles from
            // farther (sneaking past finally matters)
            let wake_r = (WAKE_BASE + (pspeed - 3.0).max(0.0) * 0.45).min(WAKE_MAX);
            let player_woke = (self.agents[i].agent.x - px).hypot(self.agents[i].agent.z - pz) < wake_r;
            let disturbed = self.transient[i].near_predator || self.transient[i].threat.is_some() || self.agents[i].spooked > 0.0 || player_woke;
            if self.agents[i].sleep_timer <= 0.0 || disturbed {
                self.agents[i].asleep = false;
                self.agents[i].meals = 0;
                self.agents[i].wake_cd = WAKE_REST; // don't immediately re-doze (anti sleep/wake flip)
                if player_woke && self.agents[i].rank < 4 {
                    self.agents[i].spooked = self.agents[i].spooked.max(1.0); // prey startles awake → bolts
                }
            }
        }

        // 4. compute every AWAKE agent's flock force from the previous positions (double-buffered)
        self.forces.clear();
        self.forces.resize(n, (0.0, 0.0, 0));
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            let f = self.flock(i, px, pz);
            self.forces[i] = f;
        }

        // 5. behaviour — act on the targeting: FLEE a threat, else STALK + CATCH prey (+ EAT / food-coma).
        // Adds to the flock force and sets the sprint boost / forced-move, from the previous positions.
        // (player-scatter / huntPlayer / rival / mobbing are the remaining behaviour bits.)
        self.behave.clear();
        self.behave.resize(n, (1.0, false));
        self.kills.clear();
        let hunt2 = HUNT2 * (1.0 + 0.4 * self.night); // keener at night
        let mut danger_now = 0.0_f64; // peak imminence of any player-hunting predator this tick
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            let ax = self.agents[i].agent.x;
            let az = self.agents[i].agent.z;
            let a_max = self.agents[i].agent.max_speed;
            let radius = self.agents[i].radius;
            let rank = self.agents[i].rank;
            let a_hunts = matches!(eco(self.agents[i].kind).hunts, Hunts::Lower);
            let can_sprint = self.agents[i].stamina > CAN_SPRINT;
            let threat_pos = self.transient[i].threat.map(|t| (self.agents[t].agent.x, self.agents[t].agent.z));
            let prey_info = self.transient[i].prey.map(|p| (p, self.agents[p].agent.x, self.agents[p].agent.z, self.agents[p].radius));
            let mobbed = self.agents[i].mobbed; // a hunter swarmed by ≥MOB_MIN prey (latched, §2c)
            if !mobbed {
                self.agents[i].slash_budget = self.agents[i].slash_max; // fresh ferocity for the next fight
                self.agents[i].slash_cd = 0.0;
            }

            // HUNT-PLAYER — a LONE (crowd<3) apex (rank≥4) predator, hungry + not otherwise busy, stalks the
            // player when you're within reach AND closer than its animal prey. Non-lethal (you're uncatchable);
            // it pressures you + raises the danger level. Keener + farther-reaching at night.
            let mut hunt_player = false;
            if rank >= 4
                && self.forces[i].2 < 3 // crowd (flock neighbours) — a lone hunter stalks; a pack just wanders
                && a_hunts
                && self.agents[i].hungry
                && can_sprint
                && !mobbed
                && self.agents[i].spooked <= 0.0
                && threat_pos.is_none()
            {
                let dp2 = (px - ax).powi(2) + (pz - az).powi(2);
                let reach = 15.0 * (1.0 + 0.6 * self.night);
                let prey_d2 = prey_info.map_or(f64::INFINITY, |(_, prx, prz, _)| (prx - ax).powi(2) + (prz - az).powi(2));
                if dp2 < reach * reach && dp2 < prey_d2 * 0.81 {
                    hunt_player = true;
                    danger_now = danger_now.max(1.0 - dp2.sqrt() / reach); // closer hunter → louder alarm
                }
            }
            self.agents[i].hunting = hunt_player; // transient: surfaced to the view so the stalker's eyes glare

            // TERRITORIAL TIMER — apex predators don't pack: accumulate time crowded by a same-rank rival (the
            // targeting sets `rival`), cooling off quickly once apart; ≥RIVAL_PATIENCE → they pick a fight.
            let rival = self.transient[i].rival;
            let rival_alive = rival.map_or(false, |r| !self.agents[r].dead);
            if rival_alive {
                self.agents[i].rival_time = (self.agents[i].rival_time + DT).min(RIVAL_PATIENCE + 0.5);
            } else {
                self.agents[i].rival_time = (self.agents[i].rival_time - DT * 1.5).max(0.0);
            }
            let fighting_rival = rival_alive && self.agents[i].rival_time >= RIVAL_PATIENCE && !mobbed && threat_pos.is_none();
            let rival_pos = if fighting_rival {
                rival.map(|r| (r, self.agents[r].agent.x, self.agents[r].agent.z, self.agents[r].radius))
            } else {
                None
            };
            let bully_pos = if self.agents[i].spooked > 0.0 {
                self.agents[i].bully.filter(|&b| !self.agents[b].dead).map(|b| (self.agents[b].agent.x, self.agents[b].agent.z))
            } else {
                None
            };
            // FISH-LURE — an idle cat (nothing better on) is drawn to a lake fish; it pads to the water's edge
            // and stalks the shallows. It never catches one: the pond is an obstacle the JS resolve-step halts
            // it at, so this is just the pull. Lowest priority (last in the chain, so any real business wins).
            let fish_pos = if self.agents[i].kind == Kind::Cat
                && !mobbed
                && threat_pos.is_none()
                && !fighting_rival
                && self.agents[i].spooked <= 0.0
            {
                self.nearest_fish(ax, az, LURE_R)
            } else {
                None
            };

            if mobbed {
                // outnumbered → BREAK AWAY from the swarm's centre (a fast hunter shakes them off)
                let mc = self.transient[i].mob_count.max(1) as f64;
                let cx = self.transient[i].mob_x / mc;
                let cz = self.transient[i].mob_z / mc;
                let dx = ax - cx;
                let dz = az - cz;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * FLEE_W;
                self.forces[i].1 += (dz / d) * a_max * FLEE_W;
                self.behave[i] = (if can_sprint { FLEE_BOOST } else { 1.0 }, true);
                // ...but if it CAN'T shake the attackers pressed against it, they WOUND it (faster the more of
                // them, and the weaker it already is) while it SLASHES back — thinning the mob in real time
                // until its ferocity is spent and the survivors drag it down.
                let attackers = self.transient[i].attackers;
                if attackers >= MOB_MIN {
                    self.agents[i].health = (self.agents[i].health - MOB_KILL_DPS * attackers as f64 * DT).max(0.0);
                    self.agents[i].slash_cd -= DT;
                    if self.agents[i].slash_cd <= 0.0 && self.agents[i].slash_budget > 0 {
                        if let Some(victim) = self.nearest_attacker(i) {
                            self.kills.push(victim); // slashed dead (corpse applied below)
                            self.agents[i].slash_budget -= 1;
                            self.agents[i].slash_cd = SLASH_CD;
                        }
                    }
                }
            } else if let Some((bx, bz)) = bully_pos {
                // freshly bullied (lost a rival fight) → keep fleeing that bully while spooked
                let dx = ax - bx;
                let dz = az - bz;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * FLEE_W;
                self.forces[i].1 += (dz / d) * a_max * FLEE_W;
                self.behave[i] = (if can_sprint { FLEE_BOOST } else { 1.0 }, true);
            } else if let Some((tx, tz)) = threat_pos {
                // if the hunter is MOBBED the herd has the numbers → CHARGE it (drive it off); else FLEE it
                let threat_mobbed = self.transient[i].threat.map_or(false, |t| self.agents[t].mobbed);
                let (dx, dz, w) = if threat_mobbed {
                    (tx - ax, tz - az, MOB_W) // converge on the hunter
                } else {
                    (ax - tx, az - tz, FLEE_W) // flee the hunter
                };
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * w;
                self.forces[i].1 += (dz / d) * a_max * w;
                self.behave[i] = (if can_sprint { FLEE_BOOST } else { 1.0 }, true);
            } else if let Some((r, rx, rz, rr)) = rival_pos {
                // TERRITORIAL FIGHT — charge the rival; on contact both bleed, so one breaks off wounded (then
                // flees its bully via the spooked branch) or is dragged down. Apex hunters spread out, not pack.
                let dx = rx - ax;
                let dz = rz - az;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * CHASE_W;
                self.forces[i].1 += (dz / d) * a_max * CHASE_W;
                self.behave[i] = (if can_sprint { CHASE_BOOST } else { 1.0 }, true);
                if d < radius + rr + CONTACT_PAD {
                    self.agents[i].health = (self.agents[i].health - RIVAL_DPS * DT).max(0.0);
                    if self.agents[i].health < HURT_AT {
                        self.agents[i].spooked = self.agents[i].spooked.max(2.5);
                        self.agents[i].bully = Some(r); // wounded → break off + flee
                    }
                }
            } else if hunt_player {
                // charge the player; sprint when close. Never catches — just bumps + pressures you.
                let dx = px - ax;
                let dz = pz - az;
                let d = dx.hypot(dz).max(0.1);
                let close = d * d < hunt2;
                self.forces[i].0 += (dx / d) * a_max * CHASE_W;
                self.forces[i].1 += (dz / d) * a_max * CHASE_W;
                self.behave[i] = (if close && can_sprint { CHASE_BOOST } else { 1.0 }, true);
            } else if let Some((p, prx, prz, pr)) = prey_info {
                // stalk toward prey; sprint once close; CATCH on contact
                let dx = prx - ax;
                let dz = prz - az;
                let d = dx.hypot(dz).max(0.1);
                let close = d * d < hunt2;
                self.forces[i].0 += (dx / d) * a_max * CHASE_W;
                self.forces[i].1 += (dz / d) * a_max * CHASE_W;
                self.behave[i] = (if close && can_sprint { CHASE_BOOST } else { 1.0 }, true);
                if close && d < radius + pr + CONTACT_PAD {
                    self.kills.push(p); // caught — turned to a corpse below
                    self.agents[i].meals += 1;
                    self.agents[i].chase_ox = f64::NAN; // the chase ended in a kill
                    // EAT — a kill refuels energy; once gorged (full_after kills) it drops into a food-coma
                    if eco(self.agents[i].kind).full_after.map_or(false, |fa| self.agents[i].meals >= fa) {
                        self.agents[i].stamina = self.agents[i].stamina.min(0.15);
                        self.agents[i].asleep = true; // (takes effect next tick — this tick it's still awake)
                        self.agents[i].sleep_timer = sleep_secs(self.agents[i].kind);
                    } else {
                        self.agents[i].stamina = (self.agents[i].stamina + EAT_GAIN).min(1.0);
                    }
                }
            } else if let Some((fx, fz)) = fish_pos {
                // pad toward the fish at a curious WALK (no sprint) — the pond obstacle halts the cat at the bank
                let dx = fx - ax;
                let dz = fz - az;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * CHASE_W * 0.6;
                self.forces[i].1 += (dz / d) * a_max * CHASE_W * 0.6;
                self.behave[i] = (1.0, true);
            }

            // PLAYER REACTION (skipped for a predator deliberately coming for you, and for the player's own pet,
            // which trusts you) — animals scatter from the player (skittishness falls with rank: rabbits bolt, an
            // apex dino ignores you), scaring from FARTHER when you RUN; and every animal gives the player a berth.
            if !hunt_player && !self.agents[i].companion {
                let skittish = ((5.0 - rank as f64) / 4.0).max(0.0); // rabbit 1 → 1.0 … dinosaur 5 → 0
                if skittish > 0.0 {
                    let dx = ax - px;
                    let dz = az - pz;
                    let d = dx.hypot(dz);
                    let scare_r = (2.5 + (pspeed - 3.0).max(0.0) * 0.5) * (0.6 + 0.7 * skittish) * (1.0 + 0.4 * self.night);
                    if d < scare_r && d > 0.01 {
                        let w = skittish * (1.0 - d / scare_r); // stronger the closer / more skittish
                        self.forces[i].0 += (dx / d) * a_max * FLEE_W * w;
                        self.forces[i].1 += (dz / d) * a_max * FLEE_W * w;
                        self.behave[i].1 = true; // pursuing (keep moving)
                        if can_sprint && w > 0.25 {
                            self.behave[i].0 = self.behave[i].0.max(FLEE_BOOST); // a real bolt when truly spooked
                        }
                    }
                }
                let adx = ax - px;
                let adz = az - pz;
                let ad = adx.hypot(adz);
                let avoid_r = radius + 1.5; // player body (~0.5) + a margin to round the corner early
                if ad < avoid_r && ad > 0.01 {
                    let w = 1.0 - ad / avoid_r;
                    self.forces[i].0 += (adx / ad) * a_max * AVOID_W * w;
                    self.forces[i].1 += (adz / ad) * a_max * AVOID_W * w;
                }
            }

            // a wound makes it LIMP — caps every gait (flee / charge / walk) so a healthy hunter runs it down
            if self.agents[i].health < HURT_AT {
                self.behave[i].0 *= HURT_SPEED;
            }
        }

        // ease the danger level toward this tick's peak → the UI vignette swells/fades smoothly
        self.danger += (danger_now - self.danger) * (6.0 * DT).min(1.0);

        // 6. apply kills → corpses (deferred so the behaviour pass reads only previous, live positions)
        for k in 0..self.kills.len() {
            let p = self.kills[k];
            self.agents[p].dead = true;
            self.agents[p].asleep = false;
            self.agents[p].agent.vx = 0.0;
            self.agents[p].agent.vz = 0.0;
        }

        // 7. metabolism (AWAKE agents) — sprinting + a carnivore's basal drain ebb stamina; prey/people
        // rest-recover. The LATCHED hunger (hysteresis LO/HI) is the flip-flop fix. Plus slow healing, the
        // cooldown timers, and the exhaustion-sleep trigger. (Asleep agents recovered in the sleep pass.)
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            // a slash / scrap that emptied the health bar this tick is fatal (checked BEFORE the heal regen)
            if self.agents[i].health <= 0.0 {
                self.agents[i].dead = true;
                self.agents[i].asleep = false;
                self.agents[i].agent.vx = 0.0;
                self.agents[i].agent.vz = 0.0;
                continue;
            }
            let boost = self.behave[i].0;
            let endurance = self.agents[i].endurance;
            let is_carnivore = matches!(eco(self.agents[i].kind).hunts, Hunts::Lower);
            let mut s = self.agents[i].stamina;
            if boost > 1.0 {
                s = (s - (EXERT_DRAIN / endurance) * DT).max(0.0);
            }
            if is_carnivore {
                s = (s - BASAL_DRAIN * DT).max(0.0); // always ebbs; refuels only by eating
            } else if boost <= 1.0 {
                s = (s + RECOVER * endurance * DT).min(1.0); // prey/people rest-recover
            }
            self.agents[i].stamina = s;
            if is_carnivore {
                if s < HUNGRY_LO {
                    self.agents[i].hungry = true;
                } else if s > HUNGRY_HI {
                    self.agents[i].hungry = false;
                }
            }
            self.agents[i].health = (self.agents[i].health + HEAL * DT).min(1.0);
            if self.agents[i].spooked > 0.0 {
                self.agents[i].spooked -= DT;
            }
            if self.agents[i].give_up_cd > 0.0 {
                self.agents[i].give_up_cd -= DT;
            }
            if self.agents[i].wake_cd > 0.0 {
                self.agents[i].wake_cd -= DT;
            }
            // an exhausted carnivore lies down to sleep it off — but never with a threat / nearby peer / fresh
            // scare keeping it on edge, and not right after waking (wake_cd → anti sleep/wake flip).
            if is_carnivore
                && self.agents[i].stamina <= 0.0
                && self.agents[i].wake_cd <= 0.0
                && self.agents[i].spooked <= 0.0
                && !self.agents[i].mobbed
                && self.transient[i].threat.is_none()
                && !self.transient[i].near_predator
            {
                self.agents[i].asleep = true;
                self.agents[i].sleep_timer = sleep_secs(self.agents[i].kind);
            }
        }

        // 8. step each AWAKE agent (write the next positions)
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            let (fx, fz, crowd) = self.forces[i];
            let (boost, pursuing) = self.behave[i];
            self.agents[i].crowd = crowd;
            let menu = menu_for(self.agents[i].kind);
            self.agents[i].agent.update(tick, DT, menu, Some((fx, fz)), boost, pursuing);
        }

        // 9. resolve obstacles — keep every live agent out of solid props/buildings/ponds (slides, no tunnelling)
        if self.has_obstacles {
            for i in 0..n {
                if self.agents[i].dead {
                    continue;
                }
                self.resolve_obstacles(i);
            }
        }
    }

    /// Reynolds flocking force for agent `i` (anti-overlap + density-gated comfort-spread + cohesion +
    /// alignment), reading only the previous positions. Returns (fx, fz, crowd).
    fn flock(&self, i: usize, px: f64, pz: f64) -> (f64, f64, u32) {
        let m = &self.agents[i];
        let (ax, az, avx, avz, a_max) = (m.agent.x, m.agent.z, m.agent.vx, m.agent.vz, m.agent.max_speed);
        let is_person = matches!(m.kind, Kind::Person);
        let sep_r = m.radius + if is_person { 2.1 } else { 1.7 }; // wider personal space → less crowding (was 1.5/1.2)
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
        // a hunter must NOT flock-separate from its own prey — the comfort-spread would shove it off before the
        // chase can close + catch (this regressed predation when the density gate dropped). The chase force
        // (behaviour pass) drives it to the prey instead; the catch fires on contact.
        let prey = self.transient[i].prey;
        self.grid.for_each_neighbor(ax, az, |j| {
            let j = j as usize;
            if j == i || agents[j].dead || prey == Some(j) {
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

        // COHESION + ALIGNMENT — now VERY weak so agents wander/disperse instead of clumping into clusters
        // (the user's repeated complaint). Cohesion was the force gluing them together; halved+ here.
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
            let coh_w = if is_person { 0.02 } else { 0.04 }; // was 0.06/0.1 — weak so they don't re-clump
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

    /// The closest living mobber pressed against hunter `i` (a prey currently fleeing it) — the one it slashes.
    fn nearest_attacker(&self, i: usize) -> Option<usize> {
        let ax = self.agents[i].agent.x;
        let az = self.agents[i].agent.z;
        let ri = self.agents[i].radius;
        let agents = &self.agents;
        let transient = &self.transient;
        let mut best: Option<usize> = None;
        let mut best_d2 = f64::INFINITY;
        self.grid.for_each_neighbor(ax, az, |ju| {
            let j = ju as usize;
            if j == i || agents[j].dead || transient[j].threat != Some(i) {
                return; // must be one of THIS hunter's own mobbers
            }
            let dx = agents[j].agent.x - ax;
            let dz = agents[j].agent.z - az;
            let d2 = dx * dx + dz * dz;
            let reach = ri + agents[j].radius + CONTACT_PAD + 0.4;
            if d2 < reach * reach && d2 < best_d2 {
                best_d2 = d2;
                best = Some(j);
            }
        });
        best
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
        w.set_player(1e4, 1e4); // park the player far away so the rabbits don't also flee IT
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
        // with the stronger dispersion tune they fan apart fast — often BEYOND the 4m neighbour radius within
        // 40 ticks (crowd → 0), which is the whole point. So just assert they separated well, not a fixed crowd.
        assert!(d1 > 1.0, "expected a strong fan-out, not a tight pair: d1={d1}");
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

    #[test]
    fn predator_catches_prey() {
        let mut w = World::new();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(0.5, 0.0, 11), Kind::Rabbit, 0.35, 11); // within contact (0.35+0.35+0.4)
        assert!(!w.agents[rabbit].dead);
        w.tick_once(1);
        assert!(w.agents[rabbit].dead, "cat in contact should catch + kill the rabbit");
        let _ = cat;
        // a corpse stops + no longer flocks/targets
        w.tick_once(2);
        assert_eq!(w.agents[rabbit].agent.vx, 0.0);
    }

    #[test]
    fn prey_flees_threat() {
        // cat at -20, rabbit at origin → the rabbit should flee +x (faster than the cat chases) and pull away
        let mut w = World::new();
        let cat = w.spawn(animal(-20.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(0.0, 0.0, 11), Kind::Rabbit, 0.35, 11);
        let gap0 = w.agents[cat].agent.x.hypot(0.0) - w.agents[rabbit].agent.x; // ~20
        for t in 1..=60 {
            w.tick_once(t);
        }
        assert!(!w.agents[rabbit].dead, "the rabbit is faster (FLEE_BOOST) → never caught");
        assert!(w.agents[rabbit].agent.x > 1.0, "rabbit fled +x, got {}", w.agents[rabbit].agent.x);
        let gap1 = (w.agents[rabbit].agent.x - w.agents[cat].agent.x).abs();
        assert!(gap1 > gap0 - 1.0, "the rabbit kept its distance");
    }

    #[test]
    fn eating_refuels() {
        let mut w = World::new();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(0.5, 0.0, 11), Kind::Rabbit, 0.35, 11); // in contact
        let s0 = w.agents[cat].stamina; // 0.45 (carnivores start hungry)
        w.tick_once(1);
        assert!(w.agents[rabbit].dead);
        assert_eq!(w.agents[cat].meals, 1);
        assert!(w.agents[cat].stamina > s0 + 0.4, "a kill refuels energy (got {})", w.agents[cat].stamina);
    }

    #[test]
    fn hunger_latch_has_hysteresis() {
        // a carnivore's `hungry` only flips at the LO/HI thresholds, holding in the gap (no per-tick flip-flop)
        let mut w = World::new();
        let cat = w.spawn(animal(0.0, 0.0, 1), Kind::Cat, 0.35, 1); // alone → no prey, no sprint
        // above HI → drops the hunger latch
        w.agents[cat].stamina = 0.8;
        w.agents[cat].hungry = true;
        w.tick_once(1);
        assert!(!w.agents[cat].hungry, "energy above HI clears hunger");
        // in the gap (LO..HI) → the latch HOLDS (both directions)
        w.agents[cat].stamina = 0.6;
        w.tick_once(2);
        assert!(!w.agents[cat].hungry, "in the gap, stays not-hungry");
        w.agents[cat].hungry = true;
        w.agents[cat].stamina = 0.6;
        w.tick_once(3);
        assert!(w.agents[cat].hungry, "in the gap, stays hungry");
        // below LO → latches hungry
        w.agents[cat].hungry = false;
        w.agents[cat].stamina = 0.4;
        w.tick_once(4);
        assert!(w.agents[cat].hungry, "energy below LO sets hunger");
    }

    #[test]
    fn prey_rest_recovers_carnivore_does_not() {
        let mut w = World::new();
        w.set_player(1e4, 1e4); // park the player far so the rabbit rests instead of fleeing it
        let rabbit = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let cat = w.spawn(animal(60.0, 0.0, 2), Kind::Cat, 0.35, 2); // far apart → idle, no chase
        w.agents[rabbit].stamina = 0.5;
        w.agents[cat].stamina = 0.5;
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert!(w.agents[rabbit].stamina > 0.5, "a resting rabbit recovers");
        assert!(w.agents[cat].stamina < 0.5, "an idle carnivore's energy still ebbs");
    }

    #[test]
    fn food_coma_after_full_after_kills() {
        let mut w = World::new();
        let lion = w.spawn(animal(50.0, 50.0, 5), Kind::Lion, 0.5, 5); // far from the player (no wake)
        let rabbit = w.spawn(animal(50.6, 50.0, 6), Kind::Rabbit, 0.35, 6); // in contact
        w.agents[lion].meals = 4; // one more kill → gorged (lion full_after = 5)
        w.tick_once(1);
        assert!(w.agents[rabbit].dead);
        assert_eq!(w.agents[lion].meals, 5);
        assert!(w.agents[lion].asleep, "the 5th kill drops the lion into a food-coma");
        assert!(w.agents[lion].sleep_timer > 0.0);
    }

    #[test]
    fn exhausted_carnivore_sleeps() {
        let mut w = World::new();
        let cat = w.spawn(animal(50.0, 50.0, 1), Kind::Cat, 0.35, 1); // alone, far from the player
        w.agents[cat].stamina = 0.0;
        w.tick_once(1);
        assert!(w.agents[cat].asleep, "an exhausted carnivore with nothing threatening it sleeps");
    }

    #[test]
    fn asleep_recovers_and_stays_asleep_undisturbed() {
        let mut w = World::new();
        let cat = w.spawn(animal(50.0, 50.0, 1), Kind::Cat, 0.35, 1);
        w.agents[cat].asleep = true;
        w.agents[cat].sleep_timer = 10.0;
        w.agents[cat].stamina = 0.2;
        w.tick_once(1);
        assert!(w.agents[cat].asleep, "still asleep (timer not up, nothing disturbing)");
        assert!(w.agents[cat].stamina > 0.2, "recovers faster while asleep");
    }

    #[test]
    fn sleeper_wakes_when_a_hunter_nears() {
        let mut w = World::new();
        let rabbit = w.spawn(animal(50.0, 50.0, 1), Kind::Rabbit, 0.35, 1);
        w.agents[rabbit].asleep = true;
        w.agents[rabbit].sleep_timer = 10.0;
        let _cat = w.spawn(animal(55.0, 50.0, 2), Kind::Cat, 0.35, 2); // 5 m away, hunting → marks the threat
        w.tick_once(1);
        assert!(!w.agents[rabbit].asleep, "a hunter within danger range startles the rabbit awake");
    }

    #[test]
    fn skittish_rabbit_flees_the_player() {
        let mut w = World::new(); // player at (0,0)
        let rabbit = w.spawn(animal(1.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        for t in 1..=40 {
            w.tick_once(t);
        }
        let d = w.agents[rabbit].agent.x.hypot(w.agents[rabbit].agent.z);
        assert!(d > 2.6, "a skittish rabbit bolts from the player (got {d})");
    }

    #[test]
    fn player_wakes_a_nearby_sleeper() {
        let mut w = World::new();
        let near = w.spawn(animal(1.0, 0.0, 1), Kind::Cat, 0.35, 1); // 1 m < WAKE_BASE 1.5
        let far = w.spawn(animal(12.0, 0.0, 2), Kind::Cat, 0.35, 2); // far from player AND the first cat
        for &c in &[near, far] {
            w.agents[c].asleep = true;
            w.agents[c].sleep_timer = 10.0;
        }
        w.tick_once(1);
        assert!(!w.agents[near].asleep, "the player tiptoeing within WAKE_BASE still wakes a close sleeper");
        assert!(w.agents[far].asleep, "a sleeper beyond WAKE_BASE (player not moving) stays down");
    }

    #[test]
    fn lone_apex_hunts_the_player_and_raises_danger() {
        let mut w = World::new(); // player at (0,0)
        let dino = w.spawn(animal(10.0, 0.0, 1), Kind::Dinosaur, 0.5, 1); // lone, hungry, within reach (15)
        let x0 = w.agents[dino].agent.x;
        for t in 1..=8 {
            w.tick_once(t); // (drains stamina fast; check while it's still charging)
        }
        assert!(w.agents[dino].agent.x < x0 - 0.5, "the apex charges toward the player (got {})", w.agents[dino].agent.x);
        assert!(w.danger > 0.05, "the danger level rises while you're hunted (got {})", w.danger);
        assert!(w.agents[dino].hunting, "the charging apex is flagged as hunting the player");
        let mut snap = Snapshot::default();
        snap.fill(&w);
        assert!(snap.flags[dino] & 8 != 0, "the hunting flag (bit3) surfaces in the read-back for the view");
    }

    #[test]
    fn sated_apex_does_not_hunt_the_player() {
        let mut w = World::new();
        let dino = w.spawn(animal(10.0, 0.0, 1), Kind::Dinosaur, 0.5, 1);
        w.agents[dino].stamina = 0.9; // above HUNGRY_HI → the latch keeps it sated, so it won't hunt you
        w.agents[dino].hungry = false;
        for t in 1..=10 {
            w.tick_once(t);
        }
        assert!(w.danger < 0.05, "a sated apex ignores you → no danger (got {})", w.danger);
    }

    #[test]
    fn outnumbered_hunter_is_mobbed_and_breaks_away() {
        let mut w = World::new();
        w.set_player(1e4, 1e4); // keep the player out of it
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        for k in 0..5 {
            // 5 rabbits clustered to +x of the lion (well inside its danger radius)
            w.spawn(animal(3.0 + k as f64 * 0.5, (k % 3) as f64 - 1.0, 100 + k), Kind::Rabbit, 0.35, 100 + k);
        }
        w.tick_once(1);
        assert!(w.agents[lion].mobbed, "5 prey fleeing one lion (≥MOB_MIN) → it is mobbed");
        for t in 2..=25 {
            w.tick_once(t);
        }
        assert!(w.agents[lion].agent.x < -0.5, "the mobbed lion breaks away from the swarm (got {})", w.agents[lion].agent.x);
    }

    #[test]
    fn too_few_prey_do_not_mob() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        w.spawn(animal(3.0, 0.0, 100), Kind::Rabbit, 0.35, 100);
        w.spawn(animal(3.5, 0.0, 101), Kind::Rabbit, 0.35, 101);
        w.tick_once(1);
        assert!(!w.agents[lion].mobbed, "only 2 prey is below MOB_MIN=4 → no mob");
    }

    fn ring(w: &mut World, n: usize, r: f64) -> Vec<usize> {
        (0..n)
            .map(|k| {
                let a = (k as f64 / n as f64) * std::f64::consts::TAU;
                w.spawn(animal(a.cos() * r, a.sin() * r, 100 + k as i32), Kind::Rabbit, 0.35, 100 + k as i32)
            })
            .collect()
    }

    #[test]
    fn mob_wounds_the_hunter_and_it_slashes_back() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        let rabbits = ring(&mut w, 6, 1.0); // 6 rabbits pressed into contact (reach ≈ 1.65)
        let h0 = w.agents[lion].health;
        for t in 1..=20 {
            w.tick_once(t);
        }
        assert!(w.agents[lion].health < h0, "the swarm wounds the cornered lion (got {})", w.agents[lion].health);
        let slashed = rabbits.iter().filter(|&&r| w.agents[r].dead).count();
        assert!(slashed >= 1, "the lion slashes back, killing attackers (got {slashed} dead)");
    }

    #[test]
    fn a_near_dead_hunter_is_dragged_down() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        ring(&mut w, 5, 1.0); // 5 attackers → ≥MOB_MIN
        w.agents[lion].health = 0.004; // one mob-tick (0.03·5·DT ≈ 0.005) from empty
        w.tick_once(1);
        assert!(w.agents[lion].dead, "the mob drags down a hunter whose health it empties");
    }

    #[test]
    fn crowded_rivals_fight_and_bleed() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let l1 = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        let l2 = w.spawn(animal(1.0, 0.0, 2), Kind::Lion, 0.5, 2); // same rank → rivals, within FIGHT_R2
        w.agents[l1].rival_time = 5.5; // fast-forward the patience so they fight now
        w.agents[l2].rival_time = 5.5;
        let h0 = w.agents[l1].health;
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert!(w.agents[l1].health < h0, "two crowded apex rivals bleed in a scrap (got {})", w.agents[l1].health);
        assert!(w.agents[l2].health < h0, "...and both of them take blows");
    }

    #[test]
    fn snapshot_mirrors_the_agents_by_index() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let r = w.spawn(animal(2.0, -3.0, 1), Kind::Rabbit, 0.35, 1);
        let l = w.spawn(animal(20.0, 0.0, 2), Kind::Lion, 0.5, 2);
        for t in 1..=20 {
            w.tick_once(t);
        }
        let mut snap = Snapshot::default();
        snap.fill(&w);
        assert_eq!(snap.xs.len(), 2);
        assert_eq!(snap.flags.len(), 2);
        // buffers mirror the live agents by index (= spawn order = JS index)
        assert_eq!(snap.xs[r], w.agents[r].agent.x as f32);
        assert_eq!(snap.zs[l], w.agents[l].agent.z as f32);
        assert_eq!(snap.headings[l], w.agents[l].agent.heading as f32);
        assert!(snap.xs.iter().all(|v| v.is_finite()));
        // a healthy, live, awake agent has no dead/asleep bits set
        assert_eq!(snap.flags[r] & 3, 0, "a live awake agent sets neither dead nor asleep");
        assert!((snap.healths[r] - w.agents[r].health as f32).abs() < 1e-6);
    }

    #[test]
    fn idle_agents_cycle_behaviours_into_the_snapshot() {
        let mut w = World::new();
        w.set_player(1e4, 1e4); // nothing to flee → it just idles + wanders
        w.spawn(animal(0.0, 0.0, 7), Kind::Rabbit, 0.35, 7);
        let mut snap = Snapshot::default();
        let mut seen_idle = false;
        for t in 1..=900 {
            w.tick_once(t);
            snap.fill(&w);
            if snap.behaviors[0] != 0 {
                seen_idle = true; // picked pause / lookAround / sit / groom / pounce, not just wander
            }
            assert!(snap.progress[0] >= 0.0 && snap.progress[0] <= 1.0, "progress is a 0..1 fraction");
        }
        assert!(seen_idle, "an idle animal cycles through the idle behaviours, not only wander → renderer can pose it");
    }

    #[test]
    fn opts_for_matches_the_view_configs() {
        // people EXPLORE most (widest leash + highest wanderlust → disperse); animals a bit tighter. Tuned UP
        // for dispersion (the clustering complaint): person leash 55 / wl 0.72, animal leash 42 / wl 0.52.
        assert_eq!(opts_for(Kind::Person, 1).home_radius, 55.0);
        assert_eq!(opts_for(Kind::Person, 1).wanderlust, 0.72);
        assert_eq!(opts_for(Kind::Rabbit, 1).home_radius, 42.0);
        assert_eq!(opts_for(Kind::Rabbit, 1).wanderlust, 0.52);
        assert!(opts_for(Kind::Person, 1).wanderlust > opts_for(Kind::Rabbit, 1).wanderlust);
        // max_speed is the per-individual eco roll
        assert_eq!(opts_for(Kind::Cat, 100).max_speed, eco::speed_for(Kind::Cat, 100));
    }

    #[test]
    fn a_circle_obstacle_pushes_agents_out() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let r = w.spawn(animal(2.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // spawned INSIDE the pond
        w.set_obstacles(&[0.0, 0.0, 5.0, f64::NAN, 0.0, 0.0, 0.0]); // circle: pond radius 5 at origin
        for t in 1..=30 {
            w.tick_once(t);
        }
        let d = w.agents[r].agent.x.hypot(w.agents[r].agent.z);
        assert!(d >= 5.0 + 0.35 - 1e-6, "the rabbit is shoved to the pond's edge, never inside (dist {d})");
    }

    #[test]
    fn an_oriented_box_ejects_agents() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(1.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // inside a 3×3 (half-extent) building
        // box: bounding r=hypot(3,3), half-extents 3×3, no rotation (cos=1,sin=0)
        w.set_obstacles(&[0.0, 0.0, 4.2426, 3.0, 3.0, 1.0, 0.0]);
        for t in 1..=30 {
            w.tick_once(t);
        }
        let (ax, az) = (w.agents[a].agent.x.abs(), w.agents[a].agent.z.abs());
        assert!(ax >= 3.0 - 1e-6 || az >= 3.0 - 1e-6, "the rabbit is ejected from the building interior (at {ax},{az})");
    }

    #[test]
    fn an_idle_cat_pads_toward_a_lake_fish() {
        let mut w = World::new();
        w.set_player(1e4, 1e4); // park the player far → no scatter
        let cat = w.spawn(animal(0.0, 0.0, 1), Kind::Cat, 0.4, 1);
        w.set_fish(&[6.0, 0.0]); // a fish 6 m away, within LURE_R
        let d0 = (6.0 - w.agents[cat].agent.x).hypot(-w.agents[cat].agent.z);
        for t in 1..=60 {
            w.tick_once(t);
        }
        let d1 = (6.0 - w.agents[cat].agent.x).hypot(-w.agents[cat].agent.z);
        assert!(d1 < d0 - 1.0, "an idle cat is drawn to the lake fish (dist {d0} → {d1})");
    }

    #[test]
    fn a_despawned_agent_goes_inert() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let cat = w.spawn(animal(0.0, 0.0, 1), Kind::Cat, 0.4, 1);
        let rabbit = w.spawn(animal(3.0, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.despawn(cat); // its object was removed → drop it from the sim
        let (cx, cz) = (w.agents[cat].agent.x, w.agents[cat].agent.z);
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert!(w.agents[cat].dead, "a despawned agent is marked dead → inert");
        assert_eq!(w.agents[cat].agent.x, cx, "a despawned agent is never stepped (frozen)");
        assert_eq!(w.agents[cat].agent.z, cz);
        assert!(
            w.transient[rabbit].threat != Some(cat),
            "the rabbit no longer fears the despawned cat (it stopped being a threat)"
        );
    }

    #[test]
    fn a_companion_pads_toward_the_player() {
        let mut w = World::new();
        w.set_player(0.0, 0.0);
        let pet = w.spawn(animal(50.0, 0.0, 1), Kind::Cat, 0.4, 1); // well outside the leash
        w.set_companion(pet);
        let d0 = w.agents[pet].agent.x.hypot(w.agents[pet].agent.z);
        for t in 1..=200 {
            w.tick_once(t);
        }
        let d1 = w.agents[pet].agent.x.hypot(w.agents[pet].agent.z);
        assert!(d1 < d0 - 5.0, "the pet's leash tracks the player so it trails toward you ({d0} → {d1})");
    }

    #[test]
    fn a_companion_does_not_flee_the_player() {
        let mut w = World::new();
        w.set_player(0.0, 0.0);
        let pet = w.spawn(animal(2.0, 0.0, 1), Kind::Cat, 0.4, 1); // within the scare radius
        w.set_companion(pet);
        let wild = w.spawn(animal(0.0, 2.0, 2), Kind::Cat, 0.4, 2); // same distance, but a wild cat → flees
        for t in 1..=60 {
            w.tick_once(t);
        }
        let pet_d = w.agents[pet].agent.x.hypot(w.agents[pet].agent.z);
        let wild_d = w.agents[wild].agent.x.hypot(w.agents[wild].agent.z);
        assert!(pet_d < wild_d, "the pet trusts you and stays close while a wild cat bolts (pet {pet_d}, wild {wild_d})");
    }

    #[test]
    fn a_distant_fish_does_not_lure() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let cat = w.spawn(animal(0.0, 0.0, 1), Kind::Cat, 0.4, 1);
        w.set_fish(&[40.0, 0.0]); // well beyond LURE_R → no pull
        for t in 1..=60 {
            w.tick_once(t);
        }
        assert!(w.agents[cat].agent.x < 12.0, "a fish beyond LURE_R exerts no pull (x {})", w.agents[cat].agent.x);
    }

    #[test]
    fn a_wounded_rival_breaks_off() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let l1 = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        let l2 = w.spawn(animal(0.8, 0.0, 2), Kind::Lion, 0.5, 2); // in contact
        w.agents[l1].rival_time = 5.5;
        w.agents[l2].rival_time = 5.5;
        w.agents[l1].health = 0.4; // already hurt → one scrap-tick tips it below HURT_AT
        w.tick_once(1);
        assert!(w.agents[l1].spooked > 0.0, "a wounded rival breaks off (spooked)");
        assert_eq!(w.agents[l1].bully, Some(l2), "...fleeing the rival that bullied it");
    }
}
