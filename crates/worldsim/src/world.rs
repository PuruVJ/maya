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
const DENSITY_THRESHOLD: f64 = 0.85; // gentle spread (0.4 was too aggressive → predators jitter-sprint to exhaustion)
const SEP_WEIGHT: f64 = 1.5; // gentle (1.8 jittered co-spawned predators into exhaustion)
const ALI_WEIGHT: f64 = 0.05; // was 0.4 → agents matched velocities + moved as ONE direction; near-off so they FAN OUT (KEEP)

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
const CHASE_BOOST: f64 = 1.95; // a committed chase BEATS a flee (was 1.45 < FLEE → prey always escaped, no kills)
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
// NIGHT-ONLY world (user: "we don't need extensive sleeping algo") → predators stay ACTIVE hunters, not
// sleepers: while not chasing they recover toward a "sprint-ready but still HUNGRY" level so they sustain the
// hunt without an exhaustion-sleep loop. Eating still refuels fully (+ food-coma). CARN_IDLE < HUNGRY_LO so an
// idling predator stays hungry → keeps hunting.
const CARN_RECOVER: f64 = 0.16; // /s a not-chasing carnivore recovers toward CARN_IDLE
const CARN_IDLE: f64 = 0.48; // the active-hunger stamina an idle predator settles at (just below HUNGRY_LO=0.5)
// REPRODUCTION (the world replenishes itself): two same-kind ADULTS, calm + well-fed, adjacent + off cooldown,
// under the per-kind cap + not over-crowded → a baby is born between them; both parents pay energy + a cooldown.
const BREED_ENERGY: f64 = 0.6; // fullness a parent needs to spare (lowered: 0.72 starved out reproduction → decline)
const BREED_COOLDOWN: f64 = 10.0; // seconds before a parent can breed again (16 still gave ~0 births → faster)
const BREED_R2: f64 = 5.0 * 5.0; // a mate within this range (3.2 was inside the flock comfort-spread → pairs never met)
const BREED_COST: f64 = 0.42; // fullness (energy) each parent spends on the birth (no free lunch)
// ISOLATION RULE (user principle): a pair breeds only when not in a CROWD — a clump can't chain-reproduce into a
// swarm. `crowd` is the neighbour count within the ~4 m flock radius (the mate counts as 1). Originally 2 ("only
// the two of them"), but that throttled births below the death rate → population decline; relaxed to 5 so a pair
// in light company can still breed while a true horde (5+ packed) still can't. Gestation + cooldown also brake it.
const BREED_CROWD: u32 = 5;
// FEAR vs the whole notice radius: prey FLEE any predator within 40 m (`danger²`), but they shouldn't be
// STERILIZED by one that's merely on the horizon — that froze ALL breeding in a predator-present world (the
// telemetry showed 0 births over 8000 ticks: a perpetual stalemate where prey were always "within 40 m of a
// hunter" so never bred, yet predators never closed the kill). Breeding is interrupted only by a hunter that's
// genuinely RIGHT THERE (≤14 m); a calm pair grazing with a lion on the skyline can still mate.
const BREED_FEAR_R2: f64 = 14.0 * 14.0;
/// Per-kind living cap — a TROPHIC PYRAMID, not one flat number. A flat 40-for-all let apex lions balloon (12 of
/// them, swamping the prey base). Real food webs are wide at the bottom (many rabbits) and narrow at the top (a
/// few lions): each predator needs a large prey base, so the higher the trophic rank, the lower the ceiling.
const fn pop_cap(kind: Kind) -> usize {
    match kind {
        Kind::Rabbit => 45,   // r-strategist prey — the broad base of the pyramid
        Kind::Kangaroo => 28, // larger prey, fewer
        Kind::Person => 22,   // omnivore settlers (want enough to grow a town)
        Kind::Cat => 14,      // meso-predator
        Kind::Lion => 6,      // apex — rare by design
        Kind::Dinosaur => 3,  // super-apex — a handful at most
    }
}
const JUVENILE_CD: f64 = 28.0; // a newborn carries this breed-cooldown → it must mature before it can breed
const BASAL_DRAIN: f64 = 0.02; // /s a carnivore's energy always ebbs → it must eat to sustain (no idle recover)
const EAT_GAIN: f64 = 0.6; // a kill refuels this much energy

// ── NUTRITION (the bottom-up population regulator) ──────────────────────────────────────────────────────────
// Every animal burns `energy` (fullness) and must EAT to refill it. Herbivores GRAZE — but grazing is DENSITY-
// DEPENDENT: a herbivore in a crowd is on overgrazed ground and refuels poorly, so a herd outgrows its food
// and starves back down. That's emergent CARRYING CAPACITY (a spatial regrowing food field is the next step).
// Carnivores already eat-or-weaken via stamina; this layer adds true STARVATION for everyone.
const ENERGY_DRAIN: f64 = 0.012; // /s fullness ebbs (~80 s empty if it never eats)
const GRAZE_RATE: f64 = 0.16; // /s a calm herbivore on UNcrowded ground refuels (~5 s to refill)
const GRAZE_CROWD: f64 = 10.0; // herd size at which ground is fully overgrazed → grazing yields ~nothing (raised: less starvation)
const STARVE_DAMAGE: f64 = 0.05; // /s health bleeds while fullness is empty (~20 s of famine is fatal; slow enough to recover if food's found)
const EAT_ENERGY: f64 = 0.7; // fullness a kill restores to the hunter

// ── GENETICS (evolution) ────────────────────────────────────────────────────────────────────────────────────
// A baby's VIGOR gene = the average of its parents' genes, ± a small mutation. Vigor scales max speed, so
// selection has something to act on: faster prey survive predators, faster predators catch prey → the
// population ADAPTS over generations. Clamped so a runaway lineage can't become absurd.
const GENE_MIN: f64 = 0.6;
const GENE_MAX: f64 = 1.6;
const GENE_MUT: f64 = 0.05; // ± mutation magnitude per birth
const CH_GENE: i32 = 20; // RNG channel for the mutation roll (distinct from eco/steering channels)

// ── AGING (generational turnover) ───────────────────────────────────────────────────────────────────────────
// Every animal accrues `age`; past SENESCENCE_FRAC of its lifespan it's an infertile elder, and at its lifespan
// it dies of old age (most are eaten / starve first — this is the backstop that keeps lineages cycling). Lifespan
// is per-kind × a seeded ±35% so a cohort dies spread out, not all at once. Game-scaled seconds, not real years.
const SENESCENCE_FRAC: f64 = 0.75; // past this fraction of lifespan → too old to breed
const CH_AGE: i32 = 21; // RNG channel for the lifespan-variation roll

// ── EMERGENT CITIES — people build houses ───────────────────────────────────────────────────────────────────
// A well-fed adult PERSON in a COMMUNITY (others gathered nearby) occasionally spends surplus energy to raise a
// house at their spot. Clusters of families therefore grow a town, then a multi-block city, bit by bit. The sim
// only emits build REQUESTS (where); JS places the house (grid-snapped, non-overlapping, globally capped).
const BUILD_ENERGY: f64 = 0.82; // a settler must be WELL-fed to afford building
const BUILD_COST: f64 = 0.55; // energy a build spends (so they must re-feed before the next)
const BUILD_COOLDOWN: f64 = 90.0; // seconds between one settler's builds → a town rises gradually
const BUILD_COMMUNITY: u32 = 3; // nearby neighbours (≈ people) that make a settlement worth building in
const CH_BUILD: i32 = 23; // RNG channel for the staggered initial build cooldown

// TELEMETRY event codes — the sim records [code, kind, x, z] so the agent can later READ what actually happened
// (causes of death, predation, births, building) rather than infer it. See `events`, /api/telemetry.
const EV_KILL: f32 = 1.0; // a predator caught prey
const EV_STARVE: f32 = 2.0; // died of starvation (empty belly)
const EV_OLDAGE: f32 = 3.0; // died of old age
const EV_BIRTH: f32 = 4.0; // a baby was delivered
const EV_BUILD: f32 = 5.0; // a settler raised a house
const EV_CONCEIVE: f32 = 6.0; // a pair mated (diagnostic: conceive≫birth ⇒ gestation/delivery is the bottleneck)

// ── GESTATION + LITTERS ─────────────────────────────────────────────────────────────────────────────────────
// Mating doesn't clone instantly: the FEMALE conceives and GESTATES for a period, then delivers a species-sized
// LITTER (small prey drop several young; big animals one). Game-scaled seconds (small fraction of a lifespan).
const CH_LITTER: i32 = 22; // RNG channel for the litter-size roll

/// Gestation period (seconds) by kind — bigger animals carry longer.
fn gestation(kind: Kind) -> f64 {
    match kind {
        Kind::Rabbit => 8.0,
        Kind::Cat => 12.0,
        Kind::Kangaroo => 12.0,
        Kind::Lion => 16.0,
        Kind::Dinosaur => 20.0,
        Kind::Person => 24.0,
    }
}

/// Litter size for a delivery — r-strategists (small prey) drop many; K-strategists (big animals/people) few.
/// Seeded per delivery so it varies. Inclusive [lo, hi].
fn litter_size(kind: Kind, seed_id: i32, tick: i32) -> u32 {
    let (lo, hi) = match kind {
        Kind::Rabbit => (3u32, 5u32),
        Kind::Cat => (2, 4),
        Kind::Kangaroo => (1, 2),
        Kind::Lion => (1, 3),
        Kind::Dinosaur => (1, 2),
        Kind::Person => (1, 1),
    };
    lo + (crate::simrng::rand(&[seed_id, tick, CH_LITTER]) * (hi - lo + 1) as f64).floor() as u32
}

/// Natural lifespan (seconds) by kind — small/fast prey are short-lived; big animals + people live longest.
fn base_lifespan(kind: Kind) -> f64 {
    match kind {
        Kind::Rabbit => 240.0,
        Kind::Kangaroo => 320.0,
        Kind::Cat => 360.0,
        Kind::Lion => 420.0,
        Kind::Dinosaur => 540.0,
        Kind::Person => 600.0,
    }
}
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
    pub stamina: f64, // 0..1 sprint resource (drained by running, recovered by resting)
    pub energy: f64,  // 0..1 NUTRITION / fullness — drains over time; refuel by EATING (herbivores graze,
    // carnivores kill). Hits 0 → starvation (health bleeds). Separate from stamina so a fed animal can still be
    // sprint-tired, and a rested animal can still be starving. This is the bottom-up population regulator.
    pub health: f64, // 0..1; ≤0 = death
    pub gene: f64,   // VIGOR — a heritable multiplier on max speed (≈1.0). Offspring inherit the average of both
    // parents' genes ± mutation, so traits compound across generations → natural selection (faster prey escape
    // predators, faster predators catch prey). The whole point of breeding being more than cloning.
    pub age: f64,      // seconds lived → drives senescence (infertile elder) + old-age death
    pub lifespan: f64, // this individual's natural lifespan (per-kind base ± seeded variation); age ≥ this = dies of old age
    pub pregnant: f64, // gestation remaining (s); >0 = a female carrying a litter (delivers at 0). Males stay 0.
    pub unborn_gene: f64, // the vigor the carried litter will inherit (averaged from both parents at conception)
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
    pub breed_cd: f64,        // seconds until it can breed again (>0 = on cooldown / a maturing juvenile)
    pub build_cd: f64,        // people only — seconds until this settler can raise another house (emergent cities)
    pub crowd: u32,           // flock neighbours this tick
}

/// Build a fully-seeded managed agent from its kind (so callers don't repeat the eco wiring).
/// Default steering opts for a kind — mirrors the JS `Critter`/`Npc` Agent configs so the bridge can spawn an
/// agent from just `(kind, seedId)`. People roam a wide leash and EXPLORE (high wanderlust → they disperse,
/// don't clump); animals keep a tighter leash + loose flocks. `max_speed` is the per-individual eco roll.
pub fn opts_for(kind: Kind, seed_id: i32) -> AgentOpts {
    let max_speed = eco::speed_for(kind, seed_id);
    // moderate wanderlust → some far-roaming explorers (helps spread), but not so high that predators + prey
    // never encounter each other (0.72/0.52 over-spread the world → hunters found nothing). Dispersion comes
    // mainly from the killed alignment + low cohesion now, not from flinging everyone to the map's edges.
    if kind == Kind::Person {
        AgentOpts { max_speed, home_radius: 46.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.6 }
    } else {
        AgentOpts { max_speed, home_radius: 36.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.42 }
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
        energy: 0.8, // start well-fed but not full → must eat to thrive + breed
        health: 1.0,
        gene: 1.0, // founders are baseline vigor; evolution emerges as mutation accumulates across births
        age: 0.0,
        lifespan: base_lifespan(kind) * (0.65 + 0.7 * crate::simrng::rand(&[seed_id, CH_AGE])), // ±35% per-individual → deaths spread out, not synchronized
        pregnant: 0.0,
        unborn_gene: 1.0,
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
        breed_cd: 0.0,
        // start partway through a build cooldown (seeded) so a fresh town doesn't raise every house on one tick
        build_cd: BUILD_COOLDOWN * crate::simrng::rand(&[seed_id, CH_BUILD]),
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

/// Deterministic SEX from the stable per-agent seed (≈50/50, no extra state). Breeding needs a male + a female,
/// so half of any same-kind pairing can't reproduce — a natural ~2× brake on population growth (with the
/// isolation rule the main one). LSB of the seed → even = female; seeds come from a hash so it's well-mixed.
fn is_female(seed_id: i32) -> bool {
    seed_id & 1 == 0
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
    births: Vec<f32>,              // this tick's births, flat [kindCode, x, z, gene, …] → JS spawns the babies
    builds: Vec<f32>,              // this step's house-build requests, flat [x, z, …] → JS places the houses
    events: Vec<f32>,              // TELEMETRY: this step's events, flat [code, kind, x, z, …] → JS posts to /api/telemetry
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
            births: Vec::new(),
            builds: Vec::new(),
            events: Vec::new(),
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

    /// Spawn into a renderer-owned stable slot. A slot is only recycled after `despawn()` made it inert;
    /// otherwise append rather than overwrite a still-visible corpse/live agent.
    pub fn spawn_at(&mut self, i: usize, agent: Agent, kind: Kind, radius: f64, seed_id: i32) -> usize {
        if i == self.agents.len() {
            self.agents.push(make_managed(agent, kind, radius, seed_id));
            return i;
        }
        if i < self.agents.len() && self.agents[i].dead {
            self.agents[i] = make_managed(agent, kind, radius, seed_id);
            return i;
        }
        self.spawn(agent, kind, radius, seed_id)
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
    /// stops affecting the food chain, instead of lingering as an invisible ghost. The renderer may later
    /// recycle this same index via `spawn_at`; until then the tombstone keeps snapshot indices stable.
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
        self.births.clear(); // births accumulate across this step's ticks; JS drains them after step()
        self.builds.clear(); // …same for house-build requests
        self.events.clear(); // …same for telemetry events
        let n = self.clock.advance(real_dt);
        for k in 0..n {
            let tick = self.clock.tick - (n - 1 - k) as i64;
            self.tick_once(tick as i32);
        }
    }

    /// This step's newborns, flat [kindCode, x, z, …] — JS reads this after step() and spawns the babies.
    pub fn births(&self) -> &[f32] {
        &self.births
    }

    /// This step's house-build requests, flat [x, z, …] — JS places the houses.
    pub fn builds(&self) -> &[f32] {
        &self.builds
    }

    /// This step's telemetry events, flat [code, kind, x, z, …] — JS posts them to /api/telemetry.
    pub fn events(&self) -> &[f32] {
        &self.events
    }

    /// The maturation cooldown JS should stamp on a newborn (single source of truth for both sides).
    pub fn juvenile_cd(&self) -> f64 {
        JUVENILE_CD
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
                // COWARDS never swarm: rabbits & kangaroos only flee. Only fighters (cat/person/lion) mob a
                // predator — so the tally (and the wound it deals) counts only them.
                if !matches!(self.agents[i].kind, Kind::Cat | Kind::Person | Kind::Lion) {
                    continue;
                }
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
                self.behave[i] = (if close || can_sprint { CHASE_BOOST } else { 1.0 }, true); // close → final lunge even if spent
            } else if let Some((p, prx, prz, pr)) = prey_info {
                // stalk toward prey; sprint once close; CATCH on contact
                let dx = prx - ax;
                let dz = prz - az;
                let d = dx.hypot(dz).max(0.1);
                let close = d * d < hunt2;
                self.forces[i].0 += (dx / d) * a_max * CHASE_W;
                self.forces[i].1 += (dz / d) * a_max * CHASE_W;
                self.behave[i] = (if close || can_sprint { CHASE_BOOST } else { 1.0 }, true); // close → final lunge even if spent
                if close && d < radius + pr + CONTACT_PAD {
                    self.kills.push(p); // caught — turned to a corpse below
                    self.events.extend_from_slice(&[EV_KILL, self.agents[p].kind as usize as f32, self.agents[p].agent.x as f32, self.agents[p].agent.z as f32]);
                    self.agents[i].meals += 1;
                    self.agents[i].chase_ox = f64::NAN; // the chase ended in a kill
                    self.agents[i].energy = (self.agents[i].energy + EAT_ENERGY).min(1.0); // the meal fills its belly
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
            // AGING: live a little; die of old age once past the natural lifespan (predation/starvation usually
            // get them first — this is the backstop that turns generations over). The player's pet is exempt.
            self.agents[i].age += DT;
            if !self.agents[i].companion && self.agents[i].age >= self.agents[i].lifespan {
                self.agents[i].dead = true;
                self.agents[i].asleep = false;
                self.agents[i].agent.vx = 0.0;
                self.agents[i].agent.vz = 0.0;
                let (k, ax, az) = (self.agents[i].kind as usize as f32, self.agents[i].agent.x as f32, self.agents[i].agent.z as f32);
                self.events.extend_from_slice(&[EV_OLDAGE, k, ax, az]);
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
                if boost > 1.0 {
                    s = (s - BASAL_DRAIN * DT).max(0.0); // chasing: exert (above) + basal drain
                } else if s < CARN_IDLE {
                    s = (s + CARN_RECOVER * DT).min(CARN_IDLE); // not chasing → recover toward the hunt-ready level
                } else {
                    s = (s - BASAL_DRAIN * DT).max(0.0); // sated (just ate) → ebb back down toward hunting
                }
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
            // ── NUTRITION: every animal burns fullness; NON-predators FORAGE it back (herbivores + people), but
            // overgrazed (crowded) ground yields little → a herd outgrows its food and starves back (carrying
            // capacity). Carnivores refill only by killing (above). The player's pet is exempt (magically fed).
            if self.agents[i].companion {
                self.agents[i].energy = 1.0;
            } else {
                let mut en = self.agents[i].energy - ENERGY_DRAIN * DT;
                if !is_carnivore && boost <= 1.0 && self.agents[i].spooked <= 0.0 {
                    let lushness = (1.0 - self.agents[i].crowd as f64 / GRAZE_CROWD).max(0.0);
                    en += GRAZE_RATE * lushness * DT;
                }
                self.agents[i].energy = en.clamp(0.0, 1.0);
            }
            // health heals when fed, but BLEEDS when the belly's empty (starvation) → overpopulation dies back
            if self.agents[i].energy > 0.0 {
                self.agents[i].health = (self.agents[i].health + HEAL * DT).min(1.0);
            } else {
                self.agents[i].health = (self.agents[i].health - STARVE_DAMAGE * DT).max(0.0);
                if self.agents[i].health <= 0.0 {
                    let (k, ax, az) = (self.agents[i].kind as usize as f32, self.agents[i].agent.x as f32, self.agents[i].agent.z as f32);
                    self.events.extend_from_slice(&[EV_STARVE, k, ax, az]); // famine claimed it (dies next tick)
                }
            }
            if self.agents[i].spooked > 0.0 {
                self.agents[i].spooked -= DT;
            }
            if self.agents[i].give_up_cd > 0.0 {
                self.agents[i].give_up_cd -= DT;
            }
            if self.agents[i].wake_cd > 0.0 {
                self.agents[i].wake_cd -= DT;
            }
            if self.agents[i].breed_cd > 0.0 {
                self.agents[i].breed_cd -= DT; // post-birth / juvenile maturation cooldown ebbs
            }
            if self.agents[i].build_cd > 0.0 {
                self.agents[i].build_cd -= DT; // settler's between-builds cooldown ebbs
            }
            // an exhausted carnivore lies down to sleep it off — but never with a threat / nearby peer / fresh
            // scare keeping it on edge, and not right after waking (wake_cd → anti sleep/wake flip).
            if is_carnivore
                && self.agents[i].stamina <= 0.0
                && self.transient[i].prey.is_none() // NEVER doze while there's prey to chase → stays an active hunter
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

        // 7.5 REPRODUCTION — GESTATION + LITTERS. Mating makes the FEMALE of a calm, well-fed, isolated, fertile
        // opposite-sex pair PREGNANT; she carries for a gestation period, then delivers a species-sized LITTER
        // (queued to `births`, spawned JS-side; buffer clears per step()). Per-kind cap holds at delivery.
        let mut pop = [0usize; 6];
        for m in self.agents.iter() {
            if !m.dead {
                pop[m.kind as usize] += 1;
            }
        }
        // A. GESTATION — advance every pregnancy; deliver the litter at the mother's spot when it completes.
        for i in 0..n {
            if self.agents[i].dead || self.agents[i].pregnant <= 0.0 {
                continue;
            }
            self.agents[i].pregnant -= DT;
            if self.agents[i].pregnant > 0.0 {
                continue; // still carrying
            }
            self.agents[i].pregnant = 0.0;
            let kind = self.agents[i].kind;
            let kc = kind as usize;
            let (mx, mz) = (self.agents[i].agent.x, self.agents[i].agent.z);
            let want = litter_size(kind, self.agents[i].seed_id, self.clock.tick as i32);
            let room = pop_cap(kind).saturating_sub(pop[kc]) as u32; // cap still holds — only deliver what fits
            let born = want.min(room);
            pop[kc] += born as usize; // count newborns against the cap NOW so a tickful of deliveries can't all see the same room and overshoot it
            for b in 0..born {
                // each littermate inherits the carried vigor ± a touch more mutation, so siblings vary a little
                let mu = (crate::simrng::rand(&[self.agents[i].seed_id, self.clock.tick as i32, b as i32, CH_GENE]) - 0.5) * 2.0 * GENE_MUT;
                let baby_gene = (self.agents[i].unborn_gene + mu).clamp(GENE_MIN, GENE_MAX);
                self.births.push(kc as f32);
                self.births.push(mx as f32);
                self.births.push(mz as f32);
                self.births.push(baby_gene as f32);
                self.events.extend_from_slice(&[EV_BIRTH, kc as f32, mx as f32, mz as f32]);
                pop[kc] += 1;
            }
        }
        // B. MATING — a fertile opposite-sex pair conceives: the female starts gestating; both pay the breed cost.
        for i in 0..n {
            if !self.breed_ready(i) {
                continue;
            }
            let kc = self.agents[i].kind as usize;
            if pop[kc] >= pop_cap(self.agents[i].kind) {
                continue; // at the kind's ceiling (incl. this tick's deliveries) → don't even conceive
            }
            if let Some(j) = self.find_mate(i) {
                let mom = if is_female(self.agents[i].seed_id) { i } else { j }; // the female of the pair carries
                // INHERIT: the litter's vigor = average of the parents' genes ± mutation (deterministic RNG), clamped
                let mu = (crate::simrng::rand(&[self.agents[i].seed_id, self.agents[j].seed_id, self.clock.tick as i32, CH_GENE]) - 0.5) * 2.0 * GENE_MUT;
                self.agents[mom].unborn_gene = (((self.agents[i].gene + self.agents[j].gene) * 0.5) + mu).clamp(GENE_MIN, GENE_MAX);
                self.agents[mom].pregnant = gestation(self.agents[mom].kind);
                self.events.extend_from_slice(&[EV_CONCEIVE, kc as f32, self.agents[mom].agent.x as f32, self.agents[mom].agent.z as f32]);
                self.agents[i].breed_cd = BREED_COOLDOWN;
                self.agents[j].breed_cd = BREED_COOLDOWN;
                self.agents[i].energy = (self.agents[i].energy - BREED_COST).max(0.0);
                self.agents[j].energy = (self.agents[j].energy - BREED_COST).max(0.0);
            }
        }

        // 7.6 EMERGENT CITIES — a well-fed adult PERSON in a community raises a house (JS places it). Clusters of
        // settled families therefore grow a town, then a city, bit by bit. The sim just emits where to build.
        for i in 0..n {
            let m = &self.agents[i];
            if m.dead
                || m.asleep
                || !matches!(m.kind, Kind::Person)
                || m.build_cd > 0.0
                || m.energy < BUILD_ENERGY
                || m.crowd < BUILD_COMMUNITY // needs neighbours → a settlement, not a lone wanderer
                || m.age < m.lifespan * 0.15 // an adult, not a child
                || self.transient[i].threat.is_some()
            {
                continue;
            }
            let (bx, bz) = (m.agent.x as f32, m.agent.z as f32);
            self.builds.push(bx);
            self.builds.push(bz);
            self.events.extend_from_slice(&[EV_BUILD, Kind::Person as usize as f32, bx, bz]);
            self.agents[i].energy -= BUILD_COST;
            self.agents[i].build_cd = BUILD_COOLDOWN;
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

    /// Is agent `i` ready to breed this tick? An adult (off cooldown), calm (no threat / not mobbed / not
    /// spooked / awake), well-fed (spare stamina), and not over-crowded.
    fn breed_ready(&self, i: usize) -> bool {
        let m = &self.agents[i];
        !m.dead
            && !m.asleep
            && m.breed_cd <= 0.0
            && m.energy > BREED_ENERGY // WELL-FED (nutrition), not merely rested → food limits breeding
            && m.age < m.lifespan * SENESCENCE_FRAC // fertile window: matured, not yet an elder
            && m.pregnant <= 0.0 // not already carrying a litter
            && !m.mobbed
            && m.spooked <= 0.0
            && m.crowd < BREED_CROWD
            && self.transient[i].threat_d > BREED_FEAR_R2 // a hunter must be RIGHT HERE to interrupt mating, not just within the 40 m flee radius
            && !self.transient[i].near_predator
    }

    /// Nearest same-kind, OPPOSITE-SEX, breed-ready mate within BREED_R2 (grid query). Skips `i` itself.
    fn find_mate(&self, i: usize) -> Option<usize> {
        let (ax, az) = (self.agents[i].agent.x, self.agents[i].agent.z);
        let kind = self.agents[i].kind;
        let my_sex = is_female(self.agents[i].seed_id);
        let mut found: Option<usize> = None;
        self.grid.for_each_neighbor(ax, az, |j| {
            let j = j as usize;
            // same kind, the OTHER sex (a male + a female make a baby), and not itself
            if found.is_some() || j == i || self.agents[j].kind != kind || is_female(self.agents[j].seed_id) == my_sex {
                return;
            }
            let d2 = (self.agents[j].agent.x - ax).powi(2) + (self.agents[j].agent.z - az).powi(2);
            if d2 <= BREED_R2 && self.breed_ready(j) {
                found = Some(j);
            }
        });
        found
    }

    /// Mark a spawned agent (a newborn) with a maturation cooldown so it can't breed until it grows up.
    pub fn set_breed_cooldown(&mut self, i: usize, cd: f64) {
        if let Some(m) = self.agents.get_mut(i) {
            m.breed_cd = cd;
        }
    }

    /// Apply an inherited VIGOR gene to a freshly-spawned bred baby: store it (so its own offspring inherit) and
    /// scale its max speed by it. Called once, right after the JS bridge spawns the baby. Founders skip this.
    pub fn set_gene(&mut self, i: usize, gene: f64) {
        if let Some(m) = self.agents.get_mut(i) {
            m.gene = gene;
            m.agent.max_speed *= gene; // vigor → faster (or slower) than the kind's base speed roll
        }
    }

    /// Reynolds flocking force for agent `i` (anti-overlap + density-gated comfort-spread + cohesion +
    /// alignment), reading only the previous positions. Returns (fx, fz, crowd).
    fn flock(&self, i: usize, px: f64, pz: f64) -> (f64, f64, u32) {
        let m = &self.agents[i];
        let (ax, az, avx, avz, a_max) = (m.agent.x, m.agent.z, m.agent.vx, m.agent.vz, m.agent.max_speed);
        let is_person = matches!(m.kind, Kind::Person);
        let sep_r = m.radius + if is_person { 1.6 } else { 1.3 }; // moderate spacing (2.1/1.7 was too pushy)
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
            if let Some(p) = self.transient[i].prey {
                if self.agents[i].chase_ox.is_nan() {
                    self.agents[i].chase_ox = ax;
                    self.agents[i].chase_oz = az;
                }
                // COMMITMENT: prey within ~10 m → the hunter is closing for the kill and NEVER abandons it (even
                // exhausted / far from the chase origin). Stops the "charged, then couldn't be bothered" give-up.
                let pdx = self.agents[p].agent.x - ax;
                let pdz = self.agents[p].agent.z - az;
                let prey_close = pdx * pdx + pdz * pdz < 100.0;
                let far = (ax - self.agents[i].chase_ox).powi(2) + (az - self.agents[i].chase_oz).powi(2) > MAX_CHASE2;
                if !prey_close && (far || self.agents[i].stamina < GIVEUP_ENERGY) {
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
    fn despawned_slot_is_recycled_without_growing_world() {
        let mut w = World::new();
        let old = w.spawn(animal(1.0, 2.0, 1), Kind::Rabbit, 0.35, 1);
        assert_eq!(old, 0);
        w.despawn(old);

        let reused = w.spawn_at(old, animal(9.0, 7.0, 2), Kind::Lion, 0.8, 2);
        assert_eq!(reused, old);
        assert_eq!(w.agents.len(), 1, "recycling must not extend the stable-slot vectors");
        assert!(!w.agents[old].dead);
        assert_eq!(w.agents[old].kind, Kind::Lion);
        assert_eq!(w.agents[old].agent.x, 9.0);
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
    fn tired_carnivore_recovers_and_stays_active() {
        // NIGHT-ONLY redesign (user: "we don't need extensive sleeping algo"): a tired, idle carnivore now
        // RECOVERS toward the hunt-ready level instead of dozing — so predators stay active hunters.
        let mut w = World::new();
        w.set_player(1e4, 1e4); // park the player far
        let cat = w.spawn(animal(50.0, 50.0, 1), Kind::Cat, 0.35, 1); // alone, no prey
        w.agents[cat].stamina = 0.0;
        for t in 1..=120 {
            w.tick_once(t);
        }
        assert!(!w.agents[cat].asleep, "no exhaustion-sleep loop — a tired predator recovers + keeps hunting");
        assert!(w.agents[cat].stamina > 0.2 && w.agents[cat].stamina <= CARN_IDLE + 1e-6, "recovers toward the active-hunger level ({}), not to full", w.agents[cat].stamina);
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
            // 5 CATS clustered to +x of the lion (cats are fighters → they mob; cowardly prey wouldn't)
            w.spawn(animal(3.0 + k as f64 * 0.5, (k % 3) as f64 - 1.0, 100 + k), Kind::Cat, 0.35, 100 + k);
        }
        w.tick_once(1);
        assert!(w.agents[lion].mobbed, "5 fighter-prey fleeing one lion (≥MOB_MIN) → it is mobbed");
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
        w.spawn(animal(3.0, 0.0, 100), Kind::Cat, 0.35, 100);
        w.spawn(animal(3.5, 0.0, 101), Kind::Cat, 0.35, 101);
        w.tick_once(1);
        assert!(!w.agents[lion].mobbed, "only 2 fighter-prey is below MOB_MIN=4 → no mob");
    }

    fn ring(w: &mut World, n: usize, r: f64) -> Vec<usize> {
        (0..n)
            .map(|k| {
                let a = (k as f64 / n as f64) * std::f64::consts::TAU;
                w.spawn(animal(a.cos() * r, a.sin() * r, 100 + k as i32), Kind::Cat, 0.35, 100 + k as i32)
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
        assert_eq!(opts_for(Kind::Person, 1).home_radius, 46.0);
        assert_eq!(opts_for(Kind::Person, 1).wanderlust, 0.6);
        assert_eq!(opts_for(Kind::Rabbit, 1).home_radius, 36.0);
        assert_eq!(opts_for(Kind::Rabbit, 1).wanderlust, 0.42);
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
    fn a_pair_gestates_then_delivers_a_litter() {
        let mut w = World::new();
        w.set_player(1e4, 1e4); // park the player far
        w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // seeds 1,2 → an opposite-sex pair
        w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2); // adjacent (< BREED_R2), well-fed
        // they conceive within a few ticks; rabbit gestation is 8 s → run well past it (tick_once accumulates births)
        for t in 1..=320 {
            w.tick_once(t);
        }
        let babies = w.births().len() / 4; // 4 floats/birth: kc,x,z,gene
        assert!(babies >= 3, "a rabbit pregnancy delivers a LITTER (3–5); got {babies}");
        assert_eq!(w.births()[0], 0.0, "the litter is rabbits (kind code 0)");
        // both parents are on a long breed cooldown → just the one litter this window, not a runaway
        assert!(babies <= 5, "one litter, not a runaway; got {babies}");
    }

    #[test]
    fn a_pair_breeds_though_a_predator_is_in_the_distance() {
        // REGRESSION: a hunter anywhere within the 40 m flee-notice radius used to set `threat`, which the breed
        // gate treated as "no mating" — so in any predator-present world the WHOLE herd was sterile (telemetry:
        // 0 births / 0 kills over 8000 ticks, a frozen stalemate). Breeding now only balks at a hunter ≤14 m.
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // seeds 1,2 → an opposite-sex, well-fed pair
        w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.spawn(animal(25.0, 0.0, 3), Kind::Cat, 0.35, 3); // a hunter in view (sets threat) but 25 m off, not on top of them
        // they should conceive almost immediately — before the cat can close the gap
        let mut conceived = false;
        for t in 1..=12 {
            w.tick_once(t);
            if w.agents.iter().any(|m| m.kind == Kind::Rabbit && m.pregnant > 0.0) {
                conceived = true;
                break;
            }
        }
        assert!(conceived, "a calm, well-fed pair breeds with a predator 25 m away (it must be ≤14 m to stop them)");
    }

    #[test]
    fn a_hungry_or_juvenile_agent_does_not_breed() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[a].energy = 0.3; // hungry → not breed-ready
        w.set_breed_cooldown(b, JUVENILE_CD); // a juvenile, still maturing
        for t in 1..=6 {
            w.tick_once(t);
        }
        assert_eq!(w.births().len(), 0, "a hungry parent + a juvenile mate → no birth");
    }

    #[test]
    fn a_lone_herbivore_grazes_its_fullness_back_up() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let r = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        w.agents[r].energy = 0.2; // got hungry
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert!(w.agents[r].energy > 0.3, "a calm, uncrowded herbivore grazes back up; got {}", w.agents[r].energy);
    }

    #[test]
    fn empty_fullness_starves_health() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        // a carnivore can't graze, and with no prey it can't refill → empty fullness must bleed health (starvation)
        let c = w.spawn(animal(0.0, 0.0, 1), Kind::Cat, 0.35, 1);
        w.agents[c].energy = 0.0;
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert!(w.agents[c].health < 1.0, "empty fullness starves (health bleeds); got {}", w.agents[c].health);
    }

    #[test]
    fn offspring_inherit_parent_vigor() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[a].gene = 1.4; // a fast lineage
        w.agents[b].gene = 1.4;
        for t in 1..=320 {
            w.tick_once(t); // conceive, then gestate (rabbit 8 s) → the litter delivers
        }
        let births = w.births();
        assert!(births.len() >= 4, "the pair bred a litter");
        let baby_gene = births[3] as f64; // layout: [kindCode, x, z, gene]
        // two mutation steps apply (at conception + per-littermate), each ±GENE_MUT
        assert!((baby_gene - 1.4).abs() <= 2.0 * GENE_MUT + 1e-6, "baby inherits ~the parents' vigor 1.4 (±mutation); got {baby_gene}");
        assert!((GENE_MIN..=GENE_MAX).contains(&baby_gene), "gene clamped in range");
    }

    #[test]
    fn an_animal_dies_of_old_age() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let r = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        w.agents[r].age = w.agents[r].lifespan - 0.02; // on the brink of its natural lifespan
        for t in 1..=4 {
            w.tick_once(t);
        }
        assert!(w.agents[r].dead, "an animal past its natural lifespan dies of old age");
    }

    #[test]
    fn an_elder_is_infertile() {
        let mut w = World::new();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2); // a fertile would-be mate
        w.agents[a].age = w.agents[a].lifespan * 0.9; // past senescence (0.75) → an infertile elder
        for t in 1..=6 {
            w.tick_once(t);
        }
        assert_eq!(w.births().len(), 0, "no fertile pair (the only mate is a senescent elder) → no birth");
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
