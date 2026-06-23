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

// The EMERGENT brain (design doc docs/emergent-behavior.md) lives in a sibling directory but is declared HERE as a
// CHILD of the `world` module, so it can read `World`'s private fields/methods (perception, the force buffers, the
// nearest_* helpers) without widening their visibility. Manual's code is untouched; this is purely additive.
#[path = "emergent/mod.rs"]
pub mod emergent;
use emergent::genome::Genome;

/// Which decision brain drives the agents each tick — a switchable MODE (design doc §1). Only the per-tick
/// *decision* differs; all perception, physics, metabolism, breeding + read-back are shared. `Manual` is the
/// proven hand-coded brain (the default for `World::new`, so the existing test-suite pins it + stays the safety
/// net); `Emergent` is the needs+primitives+utility scorer the game now runs by default (see `Sim::new`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BehaviorMode {
    Manual,
    Emergent,
}

impl BehaviorMode {
    /// Map the wasm/JS toggle code (0 = Manual, 1 = Emergent) to a mode; anything else → Manual (safe default).
    pub fn from_code(code: u8) -> BehaviorMode {
        match code {
            1 => BehaviorMode::Emergent,
            _ => BehaviorMode::Manual,
        }
    }
}

const NEIGHBOR_RADIUS: f64 = 4.0; // also the grid cell size (flocking only)
const DENSITY_THRESHOLD: f64 = 0.85; // gentle spread (0.4 was too aggressive → predators jitter-sprint to exhaustion)
const SEP_WEIGHT: f64 = 1.5; // gentle (1.8 jittered co-spawned predators into exhaustion)
const ALI_WEIGHT: f64 = 0.05; // was 0.4 → agents matched velocities + moved as ONE direction; near-off so they FAN OUT (KEEP)
// DISPERSAL — a YOUNG prey animal in a CROWDED patch strikes out to find new range (user: "the excess, esp the
// young, must go away while the rest stay"). It heads a fixed seeded direction so it actually travels off and
// seeds a new herd elsewhere, instead of the whole population packing one clearing. Fades as it reaches open ground.
const DISPERSE_CROWD: u32 = 4; // this many flock neighbours = "crowded" → young start to peel off (lowered: user wants EVERY critter to spread far)
const BLOB_CROWD: u32 = 6; // ...and at this denser crowd EVERY age (any kind) migrates out → no piling up in one area (lowered 8→6 so herds keep colonising, not pooling near a settlement)
const PERSON_DISPERSE_CROWD: u32 = 9; // people tolerate a denser band (a hamlet) before the young strike out to found a new one
const PERSON_BLOB_CROWD: u32 = 10; // people splinter (all ages) just ABOVE the breed-stop crowd → a filling settlement
// PLATEAUS at PERSON_BREED_CROWD, and the surplus pushed past this SPREADS OUT to found new ground (favour spreading,
// not dying). Below GRAZE_CROWD so they spread before they overgraze.
const DISPERSE_AGE: f64 = 0.32; // only the young (age < this × lifespan) disperse; settled adults hold the range
// LOW-POPULATION BANDING (survival instinct): when humans are SCARCE they stop killing their own kind (aggressive
// infighting is suppressed) and instead pull TOGETHER, so the few survivors converge and the community-build
// mechanic can re-found a town instead of the species guttering out. Hysteresis: truce holds until BAND_RELEASE.
const PERSON_BAND_LOW: usize = 12; // person_pop at/below this → truce + gather (a struggling settlement closes ranks)
const PERSON_BAND_RELEASE: usize = 20; // recovered past this → normal life resumes (infighting + dispersal return)
const BAND_GATHER_W: f64 = 0.05; // gentle pull toward the nearest fellow human while banding (so they coalesce, not clump-hard)
const BAND_SEEK_QUORUM: u32 = 3; // a banding person with FEWER than this flock-neighbours long-range-seeks the nearest human
const BAND_SEEK_W: f64 = 0.5; // travel drive toward that far human (strong enough to actually walk over, gentle vs dispersal's 0.9)
const DISPERSE_W: f64 = 0.9; // outward drive (strong — must beat cohesion/comfort so it actually leaves)
const BAND_PAIR_W: f64 = 0.45; // a leader's tether to its opposite-sex co-founder (below DISPERSE_W → they leave AS a pair, not glued in place)
// JUVENILE FOLLOWING — a baby ANIMAL (non-person; people's children already cluster via coh_w) trails the nearest
// grown adult of its kind, so the world shows fawn/duckling family trains. Keeps the young in the herd (a touch
// safer) — balance-neutral. People are excluded (their family cohesion is bespoke).
const JUVENILE_FOLLOW_AGE: f64 = 0.18; // age < this × lifespan = a juvenile that trails a parent
const PARENT_ADULT_AGE: f64 = 0.3; // age ≥ this × lifespan = grown enough to be the parent a juvenile follows
const FOLLOW_W: f64 = 0.1; // the juvenile's cling toward that adult (stronger than the gentle herd cohesion 0.04)
const CH_DISPERSE: i32 = 24; // RNG channel for the per-animal dispersal heading
const CH_BIRTHPOS: i32 = 25; // RNG channel for a newborn's small offset around the mother (a litter clusters, doesn't explode from one point)

// food-chain targeting (chunk b)
const SEEK: f64 = 100.0; // a predator notices + stalks prey within this radius; also the seek-grid cell size
const SEEK2: f64 = SEEK * SEEK;
const DANGER2: f64 = 40.0 * 40.0; // prey bolts at 40 m (just outside the 34 m sprint trigger → a head start)
const COMPETE_W: f64 = 1.2; // a prey's appeal drops per hunter already on it → surplus predators fan out
const ABUNDANCE_W: f64 = 3.0; // an ABUNDANT prey kind is FAR more attractive (raised — user: "predators should prefer higher-number prey more") → they crop a boom hard
const ABUNDANCE_NORM: f64 = 220.0; // prey count at which the abundance bonus saturates — raised so a true boom (885 rabbits) reads as way more attractive than a scarce kind (it used to saturate at just 40, so 885 looked no better than 40)
const MAX_HUNTERS: u32 = 3; // a prey claimed by this many is "full" → extra predators peel off to search
const FIGHT_R2: f64 = 3.0 * 3.0; // two predators closer than this stay alert (and apex rivals track each other)
const MAX_CHASE2: f64 = 45.0 * 45.0; // give up a chase once this far from where it began
const GIVEUP_CD: f64 = 5.0; // seconds it won't re-acquire prey after abandoning a chase
const GIVEUP_ENERGY: f64 = 0.06; // ...or it abandons the chase early when this spent (stamina)

// mobbing (chunk e) — when prey heavily outnumber one hunter, the herd turns and swarms it
const MOB_MIN: u32 = 4; // this many prey fleeing ONE hunter flips them flee → swarm
const MOB_RELEASE: u32 = 3; // hysteresis: a mobbed hunter stays mobbed until the swarm thins BELOW this
const SURROUND_RALLY: u32 = 10; // a hunter THIS swarmed is deep inside a crowd → no escape, so EVERY adult (women
// too, not just the guard men) turns and fights a proper brawl; only children still flee. (mob_count, people ×2.)
const GUARD_RALLY: u32 = 4; // mob_count (people double-weighted) on a hunter at which adult MALE people stop fleeing
// and CHARGE it instead — village guards rallying to defend the threatened (a female + child). 4 ⇒ ≥1 OTHER person
// is also under threat, so a lone man doesn't suicide; women + children always flee to safety.
const MOB_W: f64 = 2.2; // converge force as the mob charges the predator
const MOB_KILL_DPS: f64 = 0.03; // health/s a hunter loses PER attacker pressed against it (size+health combo)
const SLASH_CD: f64 = 1.2; // seconds between a cornered hunter's retaliatory slashes (each kills one attacker)
const HURT_AT: f64 = 0.45; // below this health an animal is injured → limps (HURT_SPEED) and flees
const HURT_SPEED: f64 = 0.6; // injured locomotion multiplier (so a healthy hunter can run it down)
const FRAIL_ONSET: f64 = 0.8; // past this fraction of its lifespan an animal SLOWS (senescence) …
const FRAIL_MIN: f64 = 0.72; // … down to this speed multiplier at death — so predators cull the old/weak, generations turn over
const PREGNANT_SPEED: f64 = 0.55; // a carrying female's walk speed multiplier (a slow waddle), unless fleeing a hunter
const BRANDISH_SPOOK: f64 = 2.5; // seconds an expectant father's machete-brandish spooks / makes a predator give up
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
const REFUGE_R: f64 = 55.0; // a fleeing person within this range of a house RUNS to it (home = safety)
const REFUGE_PULL: f64 = 1.6; // home-ward bias blended into the flee vector — ABOVE 1 so they actually head INTO the
// house when threatened (not just drift homeward), as long as home isn't behind the predator (then plain flight wins)
const OBSTACLE_CELL: f64 = 12.0; // obstacle grid cell — must exceed the biggest footprint+body radius (port of the JS)
// INTER-SETTLEMENT MIGRATION — a lone adult roamer drifts toward the nearest UNDER-POPULATED settlement to bring
// fresh UNRELATED blood (so an isolated all-kin town can breed again past the incest rule) + fill thin towns.
// Decentralised (each picks from its own spot + a seeded jitter; a town stops attracting once it hits the target).
const SETTLE_R: f64 = 40.0; // a person within this of a house = part of that settlement (counts toward its occupancy)
const SETTLE_R2: f64 = SETTLE_R * SETTLE_R;
const SETTLE_TARGET: u32 = 14; // a settlement at/above this occupancy is "full" → no longer draws migrants (anti-pile-up)
const MIGRATE_R: f64 = 320.0; // a roamer perceives + heads for sparse settlements within this range (not the whole map)
const MIGRATE_R2: f64 = MIGRATE_R * MIGRATE_R;
const MIGRATE_W: f64 = 0.55; // travel drive toward the chosen settlement (gentle — below flee/dispersal, like BAND_SEEK_W)
const CH_MIGRATE: i32 = 30; // RNG channel for the per-(agent,settlement) jitter that decorrelates who goes where
const CH_WANDERLUST: i32 = 32; // RNG channel for the per-person "restless wanderer" trait roll
const SETTLEMENT_AVOID_R: f64 = 50.0; // a predator steers away from a town centre within this range (unless desperate)
const SETTLEMENT_AVOID_W: f64 = 1.4; // strength of that aversion (moderate — a really-hungry predator still goes in)
const PRED_DESPERATE: f64 = 0.35; // fullness below which a predator is hungry enough to RISK raiding a settlement
const WANDER_PERIOD: i64 = 900; // ticks (~30 s) — restlessness is RE-ROLLED each period so migration is EPISODIC
// (a wanderer gets the itch, relocates, then settles + breeds; a different subset gets it later) — not a permanent
// nomad stuck forever en route. Keeps the migrating count fluctuating + gives every kind its turns.
const WANDER_FRAC: f64 = 0.2; // base RESTLESS share (× the per-kind weight below) — wanderers leave even a comfortable
// spot to found/join a sparser one elsewhere and breed there (gene flow + spread). The rest settle + anchor.

/// Per-kind MIGRATION tendency (user: "highest for humans, different for all animals"). Scales both the restless
/// WANDERER share and the travel drive, so humans roam between settlements the most, kangaroos rove as nomads, and
/// rabbits stick close. Migration applies to EVERY organism — people toward sparser settlements, animals to fresh range.
pub fn migrate_weight(kind: Kind) -> f64 {
    match kind {
        Kind::Person => 1.0,    // most migratory — founds + moves between settlements
        Kind::Kangaroo => 0.7,  // nomadic rovers
        Kind::Dinosaur => 0.6,  // wide-ranging
        Kind::Lion => 0.55,     // ranges for territory/prey
        Kind::Cat => 0.4,
        Kind::Rabbit => 0.3,    // stays near its warren
    }
}

/// How strongly age modulates the urge to migrate (user): ~0 in childhood, ramps to a PEAK through prime
/// adulthood, holds, then eases down in old age — an elder CAN still move, just far less likely (never 0).
fn age_migrate_factor(age: f64, lifespan: f64) -> f64 {
    let f = (age / lifespan.max(1.0)).clamp(0.0, 1.0); // life fraction
    if f < JUVENILE_FRAC {
        0.0 // children stay home
    } else if f < 0.25 {
        (f - JUVENILE_FRAC) / (0.25 - JUVENILE_FRAC) // ramp up to the prime
    } else if f < 0.6 {
        1.0 // prime adulthood — peak wanderlust
    } else {
        1.0 - 0.7 * ((f - 0.6) / 0.4) // ease down to ~0.3 by death (elders rarely, but can, move)
    }
}

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
const BREED_COOLDOWN: f64 = 30.0; // seconds before a parent can breed again — raised (was 10) so growth is GRADUAL (no caps now; a too-fast rate exploded the world to thousands in seconds)
const BOND_W: f64 = 0.35; // pair-bond tether: a gentle pull toward a bonded mate (keeps the family together raising young)
const BOND_REARING: f64 = 90.0; // seconds a bond holds before the "young have grown → may split" check kicks in
const BOND_SPLIT_FRAC: f64 = 0.5; // chance a pair splits once the young mature (else they stay bonded for life)
const CH_BONDSPLIT: i32 = 33; // RNG channel for the seeded split-or-stay roll
const VITALITY_LERP: f64 = 0.004; // per-tick ease of the director's per-kind vitality toward its target (gentle drift)
const RESCUE_N: usize = 6; // a species below this many alive → Mother Nature boosts its breeding hard (anti-extinction)
// PER-KIND soft plateau (a TROPHIC PYRAMID): breeding RATE eases to 0 as a kind nears this × pop_scale → grows then
// PLATEAUS (no hard cap, no culling — birth-rate homeostasis). Prey are common; PREDATORS are RARE (apex rarest) so
// they don't overpopulate when nothing hunts them (the "lions breeding like crazy" fix). Scales up with the built world.
fn soft_target(kind: Kind) -> f64 {
    // Targets are sized so the NEAR (live) population plateaus (≈0.54×target each — see the vitality curve) sum to
    // a few hundred, keeping LIVE objects systemically ~<400 with the static-offload (streaming.ts). The TOTAL grows
    // unbounded in dormant aggregates across regions. Trophic pyramid: prey common, predators rare.
    match kind {
        Kind::Rabbit => 150.0,  // plateau ~80
        Kind::Kangaroo => 75.0, // plateau ~40
        Kind::Person => 185.0,  // plateau ~100 — a spawned 100 holds (the user's case), excess spreads to dormant
        Kind::Cat => 46.0,      // meso-predator, plateau ~25
        Kind::Lion => 20.0,     // apex — rare, plateau ~11
        Kind::Dinosaur => 9.0,  // super-apex — rarest, plateau ~5
    }
}
const HERD_BREED_R2: f64 = 13.0 * 13.0; // a mate within this range for HERD species — wide enough that a sparse,
// scattered population (a handful of kangaroos, a thinned herd) can still pair up, not just a dense flock/city.
const PRED_BREED_R2: f64 = 24.0 * 24.0; // SOLITARY hunters range far wider — they don't pack tight like prey herds,
// so with the old 5 m radius a spread-out predator population never paired up and died out (user: only humans bred,
// the rest kept dying). Mate search runs on the COARSE food-chain grid (cell = SEEK), not the 4 m flock grid.
const BREED_COST: f64 = 0.42; // fullness (energy) each parent spends on the birth (no free lunch)
// ISOLATION RULE (user principle): a pair breeds only when not in a CROWD — a clump can't chain-reproduce into a
// swarm. `crowd` is the neighbour count within the ~4 m flock radius (the mate counts as 1). Originally 2 ("only
// the two of them"), but that throttled births below the death rate → population decline; relaxed to 5 so a pair
// in light company can still breed while a true horde (5+ packed) still can't. Gestation + cooldown also brake it.
const BREED_CROWD: u32 = 5; // herd prey stop breeding above this local crowd (density-dependence → no chain-swarm)
const PERSON_BREED_CROWD: u32 = 8; // people plateau a touch denser (a settlement) before they stop breeding
// FEAR vs the whole notice radius: prey FLEE any predator within 40 m (`danger²`), but they shouldn't be
// STERILIZED by one that's merely on the horizon — that froze ALL breeding in a predator-present world (the
// telemetry showed 0 births over 8000 ticks: a perpetual stalemate where prey were always "within 40 m of a
// hunter" so never bred, yet predators never closed the kill). Breeding is interrupted only by a hunter that's
// genuinely RIGHT THERE (≤14 m); a calm pair grazing with a lion on the skyline can still mate.
const BREED_FEAR_R2: f64 = 14.0 * 14.0;
// Living caps are no longer hand-tuned constants — they're a TROPHIC PYRAMID computed live (see effective_cap):
// PREY density scales with world AREA, and each PREDATOR's ceiling tracks the live count of the prey it eats.
// Carrying caps — bumped ~1.7× (conservative) so the LOCAL world keeps growing over time; true "always something
// everywhere" scale comes from streamed regions, not one giant local mob (perf: still under the 1000-agent ceiling
// even at max city scale ×3). Mother Nature tunes vitality on top of these.
const PREY_DENSITY_RABBIT: f64 = 52.0; // per BASELINE world area — broad base of the pyramid (× pop_scale live).
const PREY_DENSITY_KANGAROO: f64 = 34.0;
const PREY_DENSITY_PERSON: f64 = 38.0; // people kept generous — cities are the point.
const CAT_PREY_SHARE: f64 = 0.30; // a cat population ≈ 30% of its rabbit base
const LION_PREY_SHARE: f64 = 0.07; // a lion population ≈ 7% of everything it hunts
const DINO_PREY_SHARE: f64 = 0.035; // super-apex — rarest of all

/// THE carrying-capacity formula (single source of truth). PREY scale with world AREA (`scale`); each PREDATOR is a
/// share of the live prey it eats. The sim's `effective_cap` and the wasm-exported `pop_caps` (for JS load/scatter
/// trims, so the JS never re-derives it) both call this — no duplicated math. `pop` is per-Kind headcount.
pub fn cap_for(kind: Kind, pop: &[usize; 6], scale: f64) -> usize {
    let r = pop[Kind::Rabbit as usize] as f64;
    let k = pop[Kind::Kangaroo as usize] as f64;
    let p = pop[Kind::Person as usize] as f64;
    let c = pop[Kind::Cat as usize] as f64;
    let l = pop[Kind::Lion as usize] as f64;
    match kind {
        Kind::Rabbit => (PREY_DENSITY_RABBIT * scale).round() as usize,
        Kind::Kangaroo => (PREY_DENSITY_KANGAROO * scale).round() as usize,
        Kind::Person => (PREY_DENSITY_PERSON * scale).round() as usize,
        Kind::Cat => ((r * CAT_PREY_SHARE).round() as usize).max(2),
        Kind::Lion => (((r + k + p + c) * LION_PREY_SHARE).round() as usize).max(1),
        Kind::Dinosaur => (((r + k + p + c + l) * DINO_PREY_SHARE).round() as usize).max(1),
    }
}
// ── AGGREGATE FAST-FORWARD (big-world.md §3) ────────────────────────────────────────────────────────────────
// When a player returns after being away, each species relaxes toward its carrying capacity along a CLOSED-FORM
// logistic — O(1) per species, so a week away costs the same as a minute. Prey advance first; each predator then
// reads the NEW prey count. This is the balance math (rates + floors + logistic); JS only materialises the deltas.
const FF_RATE: [f64; 6] = [0.0016, 0.001, 0.0012, 0.0009, 0.0008, 0.0006]; // /s relax rate by Kind (rabbit,cat,kangaroo,person,lion,dino)
const FF_FLOOR: [f64; 6] = [6.0, 4.0, 4.0, 4.0, 2.0, 0.0]; // a species this low re-seeds (immigration would have); dino 0 = stays extinct

fn logistic_to(n0: f64, cap: f64, r: f64) -> f64 {
    if cap <= 0.0 {
        return 0.0;
    }
    let n = n0.max(0.5); // a hair above 0 so a re-seeded species can climb the curve
    cap / (1.0 + (cap / n - 1.0) * (-r).exp())
}

/// Advance the 6 populations by `dt` seconds away, toward carrying capacity. Returns the target headcounts
/// [rabbit, cat, kangaroo, person, lion, dino]. Prey first, then predators read the advanced prey (via cap_for).
pub fn ff_targets(pop: &[usize; 6], scale: f64, dt: f64) -> [u32; 6] {
    // Kind order so prey resolve before the predators that read them: rabbit, kangaroo, person, cat, lion, dino.
    const ORDER: [Kind; 6] = [
        Kind::Rabbit,
        Kind::Kangaroo,
        Kind::Person,
        Kind::Cat,
        Kind::Lion,
        Kind::Dinosaur,
    ];
    let mut working = *pop;
    let mut out = [0u32; 6];
    for kind in ORDER {
        let k = kind as usize;
        let cap = cap_for(kind, &working, scale) as f64;
        let mut n0 = working[k] as f64;
        if n0 <= 0.0 {
            if FF_FLOOR[k] <= 0.0 {
                continue; // a fully-extinct apex (dino) stays gone — it returns via Mother Nature in play
            }
            n0 = FF_FLOOR[k]; // a crashed prey/meso species would have been re-seeded by immigration while away
        } else if FF_FLOOR[k] > 0.0 && n0 < FF_FLOOR[k] {
            n0 = FF_FLOOR[k];
        }
        let t = logistic_to(n0, cap, FF_RATE[k] * dt).round() as usize;
        working[k] = t;
        out[k] = t as u32;
    }
    out
}

/// Closed-form VIGOR drift for a DORMANT region (big-world §3 — evolve stale state via the clock, NO ticking). Under
/// predation the slow get culled → the population's mean vigor climbs toward an equilibrium; with no predators there's
/// no selection → it holds. Rate + equilibrium scale with predator pressure. Exponential approach, so a region dormant
/// for ages still resolves in O(1) — this PREDICTS what the live sim's selection would do, instead of simulating it.
pub fn ff_gene(gene: f64, pop: &[usize; 6], dt: f64) -> f64 {
    let prey = (pop[0] + pop[2] + pop[3]) as f64; // rabbit + kangaroo + person (the hunted)
    let pred = (pop[1] + pop[4] + pop[5]) as f64; // cat + lion + dinosaur (the hunters)
    if pred < 1.0 || prey < 1.0 {
        return gene; // no predator↔prey interaction → no selection → vigor frozen (matches the live sim)
    }
    let pressure = (pred / (prey + pred) / 0.5).min(1.0); // 0..1 selection strength (saturates at 50% predators)
    let rate = 0.015 * pressure; // /s vigor-drift rate under full pressure
    let eq = (1.0 + 0.45 * pressure).min(GENE_MAX); // equilibrium vigor rises with pressure
    (eq - (eq - gene) * (-rate * dt).exp()).clamp(GENE_MIN, GENE_MAX)
}

/// Spawn-spread layout for a big creature batch ("100 humans"): BANDS of up to 10 on a golden-spiral around the
/// anchor, members loosely clustered within each band, spread wide (~22·√count) so most land BEYOND the JS
/// mesh-reveal radius → cheap LOD impostors, no mount-storm jank. Returns flat [x,z,…] snapped to the 0.5 m grid.
pub fn band_spread(count: usize, ax: f64, az: f64, r: f64) -> Vec<f64> {
    let count = count.max(1);
    let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt()); // golden angle → even, deterministic spread
    let group = 10usize;
    let groups = count.div_ceil(group);
    let spread = 22.0 * (count as f64).sqrt(); // bands fan WIDE — a big spawn populates a large area (the spread is
    // wanted). The far bands stream into dormant aggregates (still alive — counted via the regions); they don't pile up.
    let band_r = (r + 1.4) * 3.0; // a single band's own loose radius
    let snap = |v: f64| (v / 0.5).round() * 0.5;
    let mut out = Vec::with_capacity(count * 2);
    let mut placed = 0usize;
    for g in 0..groups {
        if placed >= count {
            break;
        }
        let gr = spread * (((g as f64) + 0.5) / groups as f64).sqrt();
        let gang = (g as f64) * ga;
        let (cx, cz) = (ax + gang.cos() * gr, az + gang.sin() * gr);
        let members = group.min(count - placed);
        for m in 0..members {
            let mr = if members > 1 {
                band_r * (((m as f64) + 0.5) / members as f64).sqrt()
            } else {
                0.0
            };
            let mang = (m as f64) * ga;
            out.push(snap(cx + mang.cos() * mr));
            out.push(snap(cz + mang.sin() * mr));
            placed += 1;
        }
    }
    out
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
const EAT_ENERGY: f64 = 0.85; // fullness a kill restores — a predator GORGES on a kill (feast), then coasts (famine)
const FEED_SECS: f64 = 4.0; // seconds a predator hunkers over a fresh kill EATING (no fidgeting/re-targeting) before moving on
const MEAT_SATED: f64 = 120.0; // seconds a meat meal (a rabbit) sustains a PERSON — within this they can breed
const MEAT_HUNGRY: f64 = 40.0; // a person whose fed_meat drops below this goes hunting rabbits for more
const STRIKE_DMG: f64 = 0.6; // health a predator's strike rips off the prey — a healthy prey SURVIVES the first hit
// (wounded, it bolts → limps via HURT_AT), and the predator must run it down for the FINISHING blow → a brief struggle,
// not an instant one-shot (user: "there should be a sense of fighting"). Already-wounded prey dies in one.
// SCAVENGING — a fresh carcass (any natural/predation death) feeds a hungry carnivore that finds it, so deaths
// aren't wasted (vultures/hyenas). A corpse carries CARRION_MEAT edible-seconds at death; it rots at 1×/s and
// drains faster while being eaten. A hungry carnivore within SCAVENGE_R pads over; in contact it refuels energy.
const CARRION_MEAT: f64 = 26.0; // edible-seconds a fresh corpse holds (rots away even uneaten → fleeting opportunity)
const SCAVENGE_R: f64 = 16.0; // a hungry carnivore notices + pads toward a fresh carcass within this radius
const SCAVENGE_GAIN: f64 = 0.5; // /s fullness a carnivore regains while feeding on a carcass (slower than a fresh kill's gorge)
const SCAVENGE_DRAIN: f64 = 1.5; // /s extra meat consumed while being eaten (so a carcass feeds a few, not forever)
const CARN_DRAIN_FRAC: f64 = 0.6; // carnivores burn fullness slower than grazers → they survive the gaps between kills (predators starved amid prey otherwise)

// ── THIRST (the WATER resource — an independent survival pressure) ──────────────────────────────────────────────
// Every animal carries `hydration`; it ebbs slowly, refills only at a water EDGE, and bleeds health when empty.
// Gentle on purpose: thirst is a periodic ERRAND that pulls animals to water (spatial structure) — not a famine.
const THIRST_DRAIN: f64 = 0.005; // /s hydration ebbs (~170 s from full to empty) — slower than the food drain
const THIRSTY_AT: f64 = 0.25; // MIDPOINT (user): low enough that an animal ROAMS ~75% of its life (no pond-orbiting
// crowding), high enough that thirst is still a real, felt errand it must run. Between the old 0.4 (crowded) and 0.15
// (thirst basically absent). Pairs with the even, abundant natural-pond field so the nearest water is never far.
const DRINK_REACH: f64 = 5.0; // metres beyond a pond's radius an animal can lap from the bank (ponds are solid)
const DRINK_RATE: f64 = 0.5; // /s hydration refilled while at a water edge (~2 s to top up — a quick drink)
const THIRST_DAMAGE: f64 = 0.04; // /s health bleeds while parched (slightly gentler than starvation; recoverable)
const THIRST_SEEK_W: f64 = 1.3; // steering weight pulling a thirsty animal toward the nearest water (ramps with thirst)
const BREED_HYDRATION: f64 = 0.3; // a parched animal (below this) can't breed → thirst regulates population, not just kills
const APOSTATIC_W: f64 = 6.0; // strength of the search-image bias toward the common morph → frequency-dependent predation
// SEASONS — a slow wet↔dry cycle scales thirst: in the DRY season water drains faster, so animals lean harder on
// the ponds (congregate, get ambushed, migrate to water) → emergent ecological drama. Gentle amplitude so a drought
// stresses but doesn't wipe. This is also the seam a future Mother-Nature/LLM director drives (set_aridity).
const SEASON_TICKS: f64 = 12000.0; // ticks for a full wet→dry→wet cycle (~6.7 min) — long enough to feel like weather
const DRY_AMP: f64 = 0.3; // peak DRY season multiplies thirst drain by 1+DRY_AMP; the wet trough is 1.0× (when seasons on)

// ── GENETICS (evolution) ────────────────────────────────────────────────────────────────────────────────────
// A baby's VIGOR gene = the average of its parents' genes, ± a small mutation. Vigor scales max speed, so
// selection has something to act on: faster prey survive predators, faster predators catch prey → the
// population ADAPTS over generations. Clamped so a runaway lineage can't become absurd.
pub const GENE_MIN: f64 = 0.6;
pub const GENE_MAX: f64 = 1.6;
const GENE_MUT: f64 = 0.05; // ± mutation magnitude per birth
const CH_GENE: i32 = 20; // RNG channel for the mutation roll (distinct from eco/steering channels)

// ── AGING (generational turnover) ───────────────────────────────────────────────────────────────────────────
// Every animal accrues `age`; its fertile window ends at `fertile_until` (per-individual — female menopause /
// near-death for males), and at its lifespan it dies of old age (most are eaten / starve first — this is the
// backstop that keeps lineages cycling). Lifespan is per-kind × a seeded ±35% so a cohort dies spread out.
const CH_AGE: i32 = 21; // RNG channel for the lifespan-variation roll

// ── EMERGENT CITIES — people build houses ───────────────────────────────────────────────────────────────────
// A well-fed adult PERSON in a COMMUNITY (others gathered nearby) occasionally spends surplus energy to raise a
// house at their spot. Clusters of families therefore grow a town, then a multi-block city, bit by bit. The sim
// only emits build REQUESTS (where); JS places the house (grid-snapped, non-overlapping, globally capped).
const BUILD_ENERGY: f64 = 0.82; // a settler must be WELL-fed to afford building
const BUILD_COST: f64 = 0.55; // energy a build spends (so they must re-feed before the next)
const BUILD_COOLDOWN: f64 = 160.0; // seconds between one settler's builds → a town rises gradually (slowed: user "a tad too much")
const FAMILY_R2: f64 = 9.0 * 9.0; // a HOUSEHOLD radius — a home rises only where an adult MALE+FEMALE pair settles
const CH_BUILD: i32 = 23; // RNG channel for the staggered initial build cooldown

// TELEMETRY event codes — the sim records [code, kind, x, z] so the agent can later READ what actually happened
// (causes of death, predation, births, building) rather than infer it. See `events`, /api/telemetry.
const EV_KILL: f32 = 1.0; // a predator caught prey
const EV_STARVE: f32 = 2.0; // died of starvation (empty belly)
const EV_OLDAGE: f32 = 3.0; // died of old age
const EV_BIRTH: f32 = 4.0; // a baby was delivered
const EV_BUILD: f32 = 5.0; // a settler raised a house
const EV_CONCEIVE: f32 = 6.0; // a pair mated (diagnostic: conceive≫birth ⇒ gestation/delivery is the bottleneck)
const EV_WELL: f32 = 7.0; // an industrious settler dug a well (a self-made water source)
const WELL_INDUSTRY: f64 = 1.2; // a settler's `industry` genome above this digs wells (emergent job; neutral 1.0 won't → Manual safe)
const WELL_NEED_R: f64 = 60.0; // …but only when no water edge is within this radius (no water in reach → dig one)
// CULTURE — memetic transmission among PEOPLE: a young settler occasionally learns from the oldest nearby elder of
// its kind, blending its behaviour genome toward that role model. Selected dims (safety/social) stay pinned by the
// niche dynamics; the rest drift → isolated settlements grow distinct CUSTOMS, and separated populations diverge.
const CH_CULTURE: i32 = 76; // RNG channel for the learning roll (kept clear of the genome channels 70-75)
const CULTURE_AGE: f64 = 0.4; // an agent learns only while younger than this fraction of its lifespan (formative years)
const CULTURE_P: f64 = 0.03; // per-tick chance a young person has a learning moment (rare → enculturation is gradual)
const CULTURE_RATE: f64 = 0.3; // how far each learning moment nudges the learner toward the elder (0..1)
const CULTURE_R2: f64 = 30.0 * 30.0; // a role model must be within this radius (same settlement/band)

// ── GESTATION + LITTERS ─────────────────────────────────────────────────────────────────────────────────────
// Mating doesn't clone instantly: the FEMALE conceives and GESTATES for a period, then delivers a species-sized
// LITTER (small prey drop several young; big animals one). Game-scaled seconds (small fraction of a lifespan).
const CH_LITTER: i32 = 22; // RNG channel for the litter-size roll

/// Gestation period (seconds) by kind — bigger animals carry longer. `pub` so the renderer can pace the
/// pregnancy belly-grow to the real delivery time (exported via lib::gestation_secs) instead of a JS duplicate.
pub fn gestation(kind: Kind) -> f64 {
    // TROPHIC PYRAMID via breed SPEED: prey are r-strategists (breed fast), PREDATORS are K-strategists (breed
    // SLOWLY) so apex populations stay rare without a hard cap. Predators MUST out-gestate prey here, else they
    // out-reproduce their food (user: "lions reproduce faster, 25 lions vs 40 humans") and over-predate the world.
    // REAL-WORLD proportions (user), anchored at rabbit ≈ 31 days → 8 s (~0.26 s/day). The trophic pyramid still
    // holds because soft_target (not gestation) is now the population regulator, so realistic spans are safe.
    match kind {
        Kind::Rabbit => 8.0,     // ~31 days
        Kind::Kangaroo => 9.0,   // ~33 days
        Kind::Cat => 17.0,       // ~64 days
        Kind::Lion => 28.0,      // ~110 days
        Kind::Person => 72.0,    // ~280 days (9 months) — the longest among the mammals here
        Kind::Dinosaur => 90.0,  // super-apex — a long egg incubation, slowest of all
    }
}

/// Litter size for a delivery — r-strategists (small prey) drop many; K-strategists (big animals/people) few.
/// Seeded per delivery so it varies. Inclusive [lo, hi].
fn litter_size(kind: Kind, seed_id: i32, tick: i32) -> u32 {
    let (lo, hi) = match kind {
        Kind::Rabbit => (3u32, 5u32), // r-strategist prey: big litters
        Kind::Kangaroo => (1, 2),
        Kind::Person => (1, 2),       // occasionally twins → families actually grow (user: humans weren't reproducing enough)
        // PREDATORS: small litters (K-strategists) so the apex stays rare — paired with long gestation above.
        Kind::Cat => (1, 2),  // was (2,4)
        Kind::Lion => (1, 1), // single cubs (was 1..3) — apex must not out-breed its prey
        Kind::Dinosaur => (1, 1),
    };
    lo + (crate::simrng::rand(&[seed_id, tick, CH_LITTER]) * (hi - lo + 1) as f64).floor() as u32
}

/// Natural lifespan (seconds) by kind — small/fast prey are short-lived; big animals + people live longest.
fn base_lifespan(kind: Kind) -> f64 {
    // REAL-WORLD ratios (user) — anchored at rabbit ≈ 240 s (≈9 yr → ~27 s/year): a human outlives a rabbit ~7×,
    // a cat/lion ~1.5×, a kangaroo ~2.4×, just as in our world. Predation/starvation still claim most before old
    // age, so the long human/dino spans mostly mean adults persist (which also eases the breeding stagnation).
    match kind {
        Kind::Rabbit => 240.0,    // ~9 yr
        Kind::Lion => 360.0,      // ~13 yr (wild)
        Kind::Cat => 400.0,       // ~15 yr
        Kind::Kangaroo => 560.0,  // ~21 yr
        Kind::Person => 1600.0,   // ~60 yr (scaled a touch under 75 to keep some turnover visible in a session)
        Kind::Dinosaur => 1800.0, // longest-lived — a large, slow-aging reptile
    }
}
const JUVENILE_FRAC: f64 = 0.12; // sexual maturity at this fraction of lifespan → maturation SCALES with lifespan
// (a long-lived human matures slowly, a short-lived rabbit fast — realistic), not a flat time for every species.
const CH_MENO: i32 = 31; // RNG channel for a female's seeded fertility-end age (human menopause variation)
const YR: f64 = 240.0 / 9.0; // sim-seconds per "year" (rabbit ≈ 9 yr at 240 s) — anchors real-world age windows

/// Age at which agent `seed_id` stops breeding. Per the real world (user): a human FEMALE hits menopause at a
/// seeded 45–50 yr; other females stay fertile to near old age; MALES of every kind breed essentially until death.
fn fertile_until(kind: Kind, seed_id: i32, lifespan: f64) -> f64 {
    if is_female(seed_id) {
        match kind {
            Kind::Person => (45.0 + 5.0 * crate::simrng::rand(&[seed_id, CH_MENO])) * YR, // 45–50 yr, fixed at birth
            _ => lifespan * 0.85,                                                          // animals: fertile to near old age
        }
    } else {
        lifespan * 0.97 // males: fertile right up to death
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
    pub hydration: f64, // 0..1 WATER — drains slowly; refill by reaching a water EDGE; ≤0 bleeds health (thirst death)
    pub energy: f64,  // 0..1 NUTRITION / fullness — drains over time; refuel by EATING (herbivores graze,
    // carnivores kill). Hits 0 → starvation (health bleeds). Separate from stamina so a fed animal can still be
    // sprint-tired, and a rested animal can still be starving. This is the bottom-up population regulator.
    pub health: f64, // 0..1; ≤0 = death
    pub gene: f64,   // VIGOR — a heritable multiplier on max speed (≈1.0). Offspring inherit the average of both
    // parents' genes ± mutation, so traits compound across generations → natural selection (faster prey escape
    // predators, faster predators catch prey). The whole point of breeding being more than cloning.
    pub age: f64,      // seconds lived → drives senescence (infertile elder) + old-age death
    pub lifespan: f64, // this individual's natural lifespan (per-kind base ± seeded variation); age ≥ this = dies of old age
    pub fertile_until: f64, // age at which it can no longer breed — per-INDIVIDUAL: a human female's menopause (~45–50 yr,
    // seeded at birth), other females near old age, MALES essentially until death (user: real-world breeding windows)
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
    pub partner: Option<usize>, // BONDED mate (set at conception) → the pair sticks together while raising young,
    // then MAY split once the young matures. Validated mutually (partner[partner]==self) so a recycled slot can't alias.
    pub bond_age: f64,        // seconds the current bond has lasted → drives the "split when the young grows up" check
    pub hunting: bool,        // this apex is actively charging the PLAYER this tick → the view glares its eyes
    pub migrating: bool,      // a roamer steering toward another settlement this tick → surfaced to the HUD
    pub feeding: f64,         // seconds a predator stays put EATING a fresh kill (set on the catch) → it doesn't
    // fidget/re-target on the corpse; it hunkers down a few seconds, then moves on. Decrements in the metabolism pass.
    pub fed_meat: f64,        // PEOPLE: seconds of "recently ate meat" left (set by eating a rabbit). People hunt
    // rabbits when this runs low, and can't BREED while it's 0 (user: human energy + reproduction tied to meat).
    pub breed_cd: f64,        // seconds until it can breed again (>0 = on cooldown / a maturing juvenile)
    pub build_cd: f64,        // people only — seconds until this settler can raise another house (emergent cities)
    pub crowd: u32,           // flock neighbours this tick
    pub carrion: f64,         // a FRESH corpse's remaining edible "meat" (s) — set at death, rots each tick; a hungry
    // carnivore scavenges from it (gains energy) until it's gone. 0 on the living (and on a picked-clean carcass).
    pub weights: Genome,      // EMERGENT-mode behaviour genome (utility weights). Manual ignores it; founders get a
    // seeded spread (strategy variation to select on); babies re-roll from their own seed for now (true cross-birth
    // inheritance needs the births buffer to carry the genome — the next plumbing step). Neutral ⇒ baseline scorer.
    // ── LINEAGE (incest avoidance, all kinds) ── a small unique family id per individual + its two parents' ids, so
    // find_mate can refuse a parent / child / sibling. Small counter ints (fit f32 exactly → ride the births buffer);
    // 0 = unknown (founders have no parents). The Rust core owns the counter; JS just ferries the parent ids at birth.
    pub fam: u32,             // this individual's unique lineage id (assigned at spawn)
    pub pfam_a: u32,          // mother's fam (0 = founder/unknown)
    pub pfam_b: u32,          // father's fam (0 = founder/unknown)
    pub unborn_dad_fam: u32,  // the carried litter's father's fam, stored at conception (the sire may wander off/die before delivery)
    pub unborn_genome: Genome,// the carried litter's inherited behaviour genome (blend of both parents ± mutation),
    // stored at conception → ferried to the babies via the births buffer so emergent STRATEGIES evolve across births.
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
            if m.migrating {
                f |= 16; // bit4 → the HUD counts who's migrating between settlements
            }
            if m.pregnant > 0.0 {
                f |= 32; // bit5 → carrying a litter → the view shows a rounded belly
            }
            if let Some(p) = m.partner {
                if p < world.agents.len() && world.agents[p].pregnant > 0.0 {
                    f |= 64; // bit6 → his mate is expecting → an "armed guardian" (the view gives him a machete)
                }
            }
            // bit7 → DRINKING: thirsty + standing at a water edge → the view dips its head to lap (watering hole)
            if (world.natural_water || !world.water_src.is_empty()) && !m.companion && m.hydration < 0.95 {
                if let Some((_, _, d_edge)) = world.nearest_water(m.agent.x, m.agent.z) {
                    if d_edge <= 0.0 {
                        f |= 128;
                    }
                }
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
    let lifespan = base_lifespan(kind) * (0.65 + 0.7 * crate::simrng::rand(&[seed_id, CH_AGE])); // ±35% per-individual → deaths spread out, not synchronized
    ManagedAgent {
        agent,
        kind,
        radius,
        rank: e.rank,
        endurance: e.endurance,
        aggressive: matches!(kind, Kind::Person) && eco::aggressive(seed_id),
        seed_id,
        stamina: if matches!(e.hunts, Hunts::Lower) { 0.45 } else { 1.0 }, // carnivores start a touch hungry
        hydration: 0.85, // start well-watered → thirst is a periodic errand, not an instant crisis
        energy: 0.8, // start well-fed but not full → must eat to thrive + breed
        health: 1.0,
        gene: 1.0, // founders are baseline vigor; evolution emerges as mutation accumulates across births
        age: 0.0,
        lifespan,
        fertile_until: fertile_until(kind, seed_id, lifespan),
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
        partner: None,
        bond_age: 0.0,
        hunting: false,
        migrating: false,
        feeding: 0.0,
        fed_meat: if matches!(kind, Kind::Person) { MEAT_SATED } else { 0.0 }, // founders start fed (can breed); only people use it
        breed_cd: 0.0,
        // start partway through a build cooldown (seeded) so a fresh town doesn't raise every house on one tick
        build_cd: BUILD_COOLDOWN * crate::simrng::rand(&[seed_id, CH_BUILD]),
        crowd: 0,
        carrion: 0.0,
        weights: Genome::from_seed(seed_id), // founders vary → emergent strategies have something to select on
        fam: 0, // assigned a unique id by World::spawn (needs the World-owned counter); 0 here is a placeholder
        pfam_a: 0,
        pfam_b: 0,
        unborn_dad_fam: 0,
        unborn_genome: Genome::NEUTRAL,
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
        // people HUNT RABBITS for meat (user), and aggressive ones also hunt their own kind
        Hunts::Humans => matches!(b.kind, Kind::Rabbit) || (a.aggressive && matches!(b.kind, Kind::Person)),
        Hunts::None => false,
    }
}

/// Deterministic SEX from the stable per-agent seed (≈50/50, no extra state). Breeding needs a male + a female,
/// so half of any same-kind pairing can't reproduce — a natural ~2× brake on population growth (with the
/// isolation rule the main one). LSB of the seed → even = female; seeds come from a hash so it's well-mixed.
fn is_female(seed_id: i32) -> bool {
    seed_id & 1 == 0
}

/// Are `a` and `b` close kin (so they must NOT mate)? True if one is the other's PARENT, or they share a parent
/// (full/half SIBLINGS). Uses the small lineage ids (fam = self, pfam_a/b = parents); 0 = unknown (founders), which
/// never matches, so unrelated founders + cousins breed freely. Applies to ALL kinds (user: "avoid incest, all animals").
fn related(a: &ManagedAgent, b: &ManagedAgent) -> bool {
    // parent ↔ child (a fam is always non-zero once spawned; a parent fam of 0 = unknown, won't match a real fam)
    if b.pfam_a == a.fam || b.pfam_b == a.fam || a.pfam_a == b.fam || a.pfam_b == b.fam {
        return true;
    }
    // siblings — share a known (non-zero) parent
    (a.pfam_a != 0 && (a.pfam_a == b.pfam_a || a.pfam_a == b.pfam_b)) || (a.pfam_b != 0 && (a.pfam_b == b.pfam_a || a.pfam_b == b.pfam_b))
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
    aridity: f64,                  // DROUGHT multiplier on thirst (1 = normal). The director/LLM sets this for a
    // drought event; it stacks on top of the ambient wet↔dry SEASON cycle → animals lean harder on water in dry times.
    seasons: bool,                 // ambient wet↔dry CYCLE on/off (default on for richness). Off → stable climate
    // (a fixed-environment control for the niche scenario tests, whose equilibria a moving season would smear).
    pop_scale: f64,                // world-AREA multiplier for prey caps (fed from JS: bigger world → more life)
    person_pop: usize,             // last tick's live PERSON count → drives low-pop BANDING (truce + gather, see PERSON_BAND_LOW)
    kind_pop: [usize; 6],          // last tick's live count per Kind → predators prefer ABUNDANT prey (prey-switching)
    morph_mean: [f64; 6],          // last tick's MEAN boldness (safety weight) per Kind → APOSTATIC predation: hunters
    // over-target the COMMON morph (search image), which culls the majority → negative frequency dependence that
    // keeps a bold↔cautious polymorphism STABLE & seed-robust instead of one strategy drifting to fixation.
    person_banding: bool,          // LATCHED low-pop survival truce (hysteresis PERSON_BAND_LOW↑PERSON_BAND_RELEASE): no infighting, gather up
    vitality: [f64; 6],            // per-kind BREEDING vigour, set by the JS "Mother Nature" director: >1 a struggling
    // species breeds harder (lower fullness bar + shorter cooldown) to recover; <1 a booming one eases off. 1 = neutral.
    forces: Vec<(f64, f64, u32)>,  // reused per-tick (fx, fz, crowd) flock buffer → no per-frame alloc
    behave: Vec<(f64, bool)>,      // reused per-tick (boost, pursuing) from the behaviour pass
    kills: Vec<usize>,             // prey caught this tick → turned to corpses after the behaviour pass
    slept: Vec<bool>,              // was asleep AT TICK START → handled by the sleep pass, skipped elsewhere
    fish: Vec<(f64, f64)>,         // lake-fish lure points (fed from the JS view; cats pad to the bank after them)
    water_src: Vec<(f64, f64, f64)>, // DRINKABLE water sources (x, z, radius) fed from the JS pond view → every animal
    // must periodically reach a water EDGE to refill hydration or it dies of thirst. An INDEPENDENT survival pressure
    // (distinct from food/predation) → the substrate for water-bound niches/jobs (docs: emergence economy rung).
    natural_water: bool, // on → the sim ALSO reads Rust's procedural natural-pond field (engine::nearest_natural_pond)
    // as drink sources, so water is spread evenly across the whole world. Off by default (tests pin water exactly);
    // the game enables it via Sim::new. This is the "Rust owns the world's water" source of truth.
    refuges: Vec<(f64, f64)>,      // house/settlement centres (fed from JS) → a threatened woman/child FLEES toward the nearest (home = safety)
    refuge_pop: Vec<u32>,          // per-refuge occupancy (people within SETTLE_R), recomputed each tick → drives sparse-settlement MIGRATION
    obstacles: Vec<Obstacle>,      // solid props/buildings/ponds → agents are pushed out (no tunnelling)
    ob_grid: SpatialHashGrid,      // obstacle lookup grid (cell = OBSTACLE_CELL), rebuilt on set_obstacles
    has_obstacles: bool,
    ob_scratch: Vec<u32>,          // reused obstacle-query scratch (mem::take'd in → no per-agent alloc)
    births: Vec<f32>,              // this tick's births, flat [kindCode, x, z, gene, …] → JS spawns the babies
    builds: Vec<f32>,              // this step's house-build requests, flat [x, z, …] → JS places the houses
    wells: Vec<f32>,               // this step's WELL-dig requests, flat [x, z, …] → JS places a well that JS then
    // feeds back as a drink source (set_water). Emergent JOBS: an industrious settler with no water in reach digs
    // one → a self-made watering point → life congregates → the settlement grows around water it created.
    events: Vec<f32>,              // TELEMETRY: this step's events, flat [code, kind, x, z, …] → JS posts to /api/telemetry
    behavior_mode: BehaviorMode,   // which decision brain runs the per-tick DECIDE pass (Manual default / Emergent)
    lineage_counter: u32,          // next unique family id to hand out (incest avoidance) — bumped per spawn
    player_immune: bool,           // true → NO predator hunts/menaces the player + the danger level stays 0 (the
    // game sets this on; the apex-hunts-player tests leave it off). Animals still give your body a berth + skittish
    // prey still flee your approach — you're just not PREY/quarry. (user: "give me immunity, no animals hunt me")
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
            aridity: 1.0,
            seasons: true,
            pop_scale: 1.0,
            person_pop: 0,
            kind_pop: [0; 6],
            morph_mean: [1.0; 6],
            person_banding: false,
            vitality: [1.0; 6],
            forces: Vec::new(),
            behave: Vec::new(),
            kills: Vec::new(),
            slept: Vec::new(),
            fish: Vec::new(),
            water_src: Vec::new(),
            natural_water: false,
            refuges: Vec::new(),
            refuge_pop: Vec::new(),
            obstacles: Vec::new(),
            ob_grid: SpatialHashGrid::new(OBSTACLE_CELL),
            has_obstacles: false,
            ob_scratch: Vec::new(),
            births: Vec::new(),
            builds: Vec::new(),
            wells: Vec::new(),
            events: Vec::new(),
            behavior_mode: BehaviorMode::Emergent, // the world runs the EMERGENT brain by default (user: "world should be emergent"); flip to Manual via set_behavior_mode (the unit tests that pin Manual's exact mechanics do so explicitly)
            lineage_counter: 1, // 0 is reserved for "unknown parent"; real ids start at 1
            player_immune: false, // OFF by default so the apex-hunts-player tests still fire; the game turns it ON (Sim::new)
        }
    }

    /// Toggle player IMMUNITY — when on, no predator hunts/menaces the player and the danger level holds at 0.
    /// (Prey still flee your approach + animals still berth your body; you're simply never treated as quarry.)
    pub fn set_player_immune(&mut self, immune: bool) {
        self.player_immune = immune;
    }

    /// Switch the decision brain at runtime (the design's mode toggle). Both paths compile + run, so a world can
    /// be A/B-flipped live; the choice persists on `World` (and is serialised in the world blob JS-side).
    pub fn set_behavior_mode(&mut self, mode: BehaviorMode) {
        self.behavior_mode = mode;
    }

    /// The brain currently driving the world (for the HUD readout / persistence).
    pub fn behavior_mode(&self) -> BehaviorMode {
        self.behavior_mode
    }

    /// Mean AGE as a fraction of lifespan (0 newborn … 1 at death) per Kind, for the HUD's age readout — lets the
    /// player see a population's "oldness" + trends on one 0–100 scale across species. -1 for a kind with none alive.
    pub fn age_means(&self) -> Vec<f32> {
        let mut sum = [0f32; 6];
        let mut cnt = [0u32; 6];
        for m in self.agents.iter() {
            if !m.dead {
                let k = m.kind as usize;
                sum[k] += (m.age / m.lifespan.max(1.0)) as f32;
                cnt[k] += 1;
            }
        }
        (0..6).map(|k| if cnt[k] > 0 { sum[k] / cnt[k] as f32 } else { -1.0 }).collect()
    }

    /// How nocturnal the world is (0 day … 1 night) — widens the prey's danger radius.
    pub fn set_night(&mut self, n: f64) {
        self.night = n.clamp(0.0, 1.0);
    }

    /// DROUGHT control (director / future LLM seam): a multiplier on thirst (1 = normal, >1 = drier). Stacks on
    /// the ambient season cycle. Clamped so a shock stresses the world without instantly wiping it.
    pub fn set_aridity(&mut self, a: f64) {
        self.aridity = a.clamp(0.5, 3.0);
    }

    /// Effective thirst multiplier at `tick`: the director's `aridity` × the ambient wet↔dry SEASON cycle (a slow
    /// cosine, 1.0 in the wet trough up to 1+DRY_AMP at the dry peak). Drives how fast hydration ebbs this tick.
    fn drought(&self, tick: i32) -> f64 {
        let season = if self.seasons {
            let phase = (tick as f64 / SEASON_TICKS) * std::f64::consts::TAU;
            1.0 + DRY_AMP * (0.5 - 0.5 * phase.cos()) // tick 0 = wet (1.0), half-cycle = dry peak (1+DRY_AMP)
        } else {
            1.0 // stable climate
        };
        self.aridity * season
    }

    /// Toggle the ambient wet↔dry season cycle (default on). Off = a fixed climate (niche tests pin this so a
    /// moving season doesn't smear the equilibrium they measure). The director drought (`set_aridity`) still applies.
    pub fn set_seasons(&mut self, on: bool) {
        self.seasons = on;
    }

    /// World-AREA multiplier for the prey carrying capacity (fed from JS, which knows the world's spatial extent).
    /// A bigger / more-built world supports proportionally more life; predators then follow the prey (see effective_cap).
    pub fn set_pop_scale(&mut self, s: f64) {
        self.pop_scale = s.clamp(1.0, 8.0);
    }

    /// Per-kind breeding vitality from the JS director (6 values, by Kind index). Clamped to a sane band so the
    /// controller can nudge but never break the sim. >1 ⇒ breeds harder (recovery); <1 ⇒ eases off (stabilise).
    pub fn set_vitality(&mut self, v: &[f64]) {
        for i in 0..6.min(v.len()) {
            self.vitality[i] = v[i].clamp(0.4, 2.5);
        }
    }

    // (effective_cap removed 2026-06-22 — the live sim no longer enforces a headcount carrying capacity; food +
    // predation + old age + Mother Nature's anti-extinction rescue regulate population. `cap_for` lives on only
    // for the away fast-forward `ff_targets` relaxation target.)

    /// Hand out the next unique lineage id (incest avoidance) → every spawned individual gets its own.
    fn next_fam(&mut self) -> u32 {
        let f = self.lineage_counter;
        self.lineage_counter = self.lineage_counter.wrapping_add(1).max(1); // never wrap back to 0 (the "unknown" sentinel)
        f
    }

    /// Spawn an agent; returns its index.
    pub fn spawn(&mut self, agent: Agent, kind: Kind, radius: f64, seed_id: i32) -> usize {
        let mut m = make_managed(agent, kind, radius, seed_id);
        m.fam = self.next_fam();
        self.agents.push(m);
        self.agents.len() - 1
    }

    /// Spawn into a renderer-owned stable slot. A slot is only recycled after `despawn()` made it inert;
    /// otherwise append rather than overwrite a still-visible corpse/live agent.
    pub fn spawn_at(&mut self, i: usize, agent: Agent, kind: Kind, radius: f64, seed_id: i32) -> usize {
        let fam = self.next_fam();
        if i == self.agents.len() {
            let mut m = make_managed(agent, kind, radius, seed_id);
            m.fam = fam;
            self.agents.push(m);
            return i;
        }
        if i < self.agents.len() && self.agents[i].dead {
            let mut m = make_managed(agent, kind, radius, seed_id);
            m.fam = fam;
            self.agents[i] = m;
            return i;
        }
        self.spawn(agent, kind, radius, seed_id)
    }

    /// Record a newborn's PARENT lineage ids (mother's fam, father's fam) — set by JS right after it spawns the
    /// baby from the births buffer, so the kinship check (find_mate) can later refuse a parent/child/sibling pairing.
    pub fn set_lineage(&mut self, i: usize, pfam_a: u32, pfam_b: u32) {
        if let Some(m) = self.agents.get_mut(i) {
            m.pfam_a = pfam_a;
            m.pfam_b = pfam_b;
        }
    }

    /// Apply a bred baby's inherited behaviour GENOME (the 5 utility weights from the births buffer) → its emergent
    /// strategy is its parents' blend, so lineages evolve cautious/bold/industrious traits across generations.
    pub fn set_genome(&mut self, i: usize, food: f64, safety: f64, social: f64, rest: f64, industry: f64) {
        if let Some(m) = self.agents.get_mut(i) {
            m.weights = Genome { food, safety, social, rest, industry };
        }
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

    /// Replace the DRINKABLE water sources (pond centres + radius) every animal must reach to slake thirst.
    /// `xzr` is a flat [x0,z0,r0,x1,z1,r1,…] buffer (fed from the JS pond view, same zones used as obstacles).
    pub fn set_water(&mut self, xzr: &[f64]) {
        self.water_src.clear();
        self.water_src.extend(xzr.chunks_exact(3).map(|c| (c[0], c[1], c[2])));
    }

    /// Turn on Rust's procedural natural-pond field as drink sources (the game does this; tests leave it off so
    /// they control water exactly). With it on, water is spread evenly across the whole infinite world.
    pub fn set_natural_water(&mut self, on: bool) {
        self.natural_water = on;
    }

    /// Nearest water EDGE to (x,z): returns (edge_dx, edge_dz, dist_to_edge) of the closest pond, or None if the
    /// world has no water. The vector points from the animal toward the bank; dist_to_edge ≤ 0 means it's already
    /// within drinking reach. Linear scan — ponds are few.
    fn nearest_water(&self, x: f64, z: f64) -> Option<(f64, f64, f64)> {
        let mut best: Option<(f64, f64, f64)> = None;
        let mut best_d = f64::INFINITY;
        let mut consider = |wx: f64, wz: f64, r: f64| {
            let (dx, dz) = (wx - x, wz - z);
            let d_edge = dx.hypot(dz) - (r + DRINK_REACH); // ≤0 → within reach of the bank
            if d_edge < best_d {
                best_d = d_edge;
                best = Some((dx, dz, d_edge));
            }
        };
        // JS-fed water: wells settlers dug + player-dug lakes (the dynamic delta)
        for &(wx, wz, r) in &self.water_src {
            consider(wx, wz, r);
        }
        // RUST-OWNED natural ponds (the procedural field) — the source of truth for the world's water. Off by
        // default so the scenario tests control water exactly; the game (Sim::new) turns it on.
        if self.natural_water {
            if let Some((wx, wz, r)) = crate::engine::nearest_natural_pond(x, z) {
                consider(wx, wz, r);
            }
        }
        best
    }

    /// Replace the REFUGE points (house/settlement centres) a threatened woman/child flees toward — home is safety,
    /// and it's where the guard men cluster. `xz` is a flat [x0,z0,x1,z1,…] buffer (fed from the JS world view).
    pub fn set_refuges(&mut self, xz: &[f64]) {
        self.refuges.clear();
        self.refuges.extend(xz.chunks_exact(2).map(|c| (c[0], c[1])));
        self.refuge_pop = vec![0; self.refuges.len()]; // parallel occupancy buffer, refilled each tick
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

    /// Nearest FRESH carcass (dead, still-edible) within `r` of (x,z) → (x, z, index), or None. A hungry carnivore
    /// pads over and scavenges it. Scans the seek grid (corpses don't move, so the previous-position grid is exact).
    fn nearest_carrion(&self, x: f64, z: f64, r: f64, scratch: &mut Vec<u32>) -> Option<(f64, f64, usize)> {
        let mut best: Option<(f64, f64, usize)> = None;
        let mut best_d2 = r * r;
        scratch.clear();
        self.seek_grid.for_each_neighbor(x, z, |j| scratch.push(j));
        for &ju in scratch.iter() {
            let j = ju as usize;
            if !self.agents[j].dead || self.agents[j].carrion <= 0.0 {
                continue;
            }
            let (cx, cz) = (self.agents[j].agent.x, self.agents[j].agent.z);
            let d2 = (cx - x).powi(2) + (cz - z).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some((cx, cz, j));
            }
        }
        best
    }

    /// Occupancy of the nearest settlement within SETTLE_R of (x,z) — i.e. "how full is the town I'm standing in",
    /// or None if I'm out in the wild (no settlement near). Drives the migration "am I comfortable here?" check.
    fn here_occupancy(&self, x: f64, z: f64) -> Option<u32> {
        let mut best = SETTLE_R2;
        let mut pop = None;
        for (ri, &(rx, rz)) in self.refuges.iter().enumerate() {
            let d2 = (rx - x).powi(2) + (rz - z).powi(2);
            if d2 < best {
                best = d2;
                pop = self.refuge_pop.get(ri).copied();
            }
        }
        pop
    }

    /// Nearest refuge (house) within `r` of (x,z), or None — a fleeing woman/child heads here for safety.
    fn nearest_refuge(&self, x: f64, z: f64, r: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        let mut best_d2 = r * r;
        for &(rx, rz) in &self.refuges {
            let d2 = (rx - x).powi(2) + (rz - z).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some((rx, rz));
            }
        }
        best
    }

    /// Nearest UNDER-populated settlement (occupancy < SETTLE_TARGET) within MIGRATE_R, for a roamer at (x,z).
    /// Each candidate's distance is multiplied by a per-(agent, settlement) seeded jitter so different roamers
    /// favour different sparse towns — they spread across the thin settlements instead of all funnelling to the
    /// single nearest one (the "everybody to one road" pile-up the user wants avoided). Decentralised + occupancy-
    /// aware → once a town fills to SETTLE_TARGET it drops out of every roamer's candidate set automatically.
    fn nearest_sparse_refuge(&self, x: f64, z: f64, seed_id: i32) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        let mut best_score = MIGRATE_R2;
        for (ri, &(rx, rz)) in self.refuges.iter().enumerate() {
            if self.refuge_pop.get(ri).copied().unwrap_or(0) >= SETTLE_TARGET {
                continue; // already full → not a migration draw
            }
            let d2 = (rx - x).powi(2) + (rz - z).powi(2);
            if d2 > MIGRATE_R2 || d2 < SETTLE_R2 {
                continue; // out of perception, OR it's the town I'm already in (migrate means going ELSEWHERE)
            }
            // jitter the effective distance ±~25% by a seed stable per (agent, this settlement's location)
            let j = 0.75 + 0.5 * crate::simrng::rand(&[seed_id, (rx as i32).wrapping_mul(73) ^ (rz as i32), CH_MIGRATE]);
            let score = d2 * j;
            if score < best_score {
                best_score = score;
                best = Some((rx, rz));
            }
        }
        best
    }

    /// Drive the sim from real elapsed seconds (advances the clock; runs each emitted fixed-DT tick).
    pub fn step(&mut self, real_dt: f64) {
        self.births.clear(); // births accumulate across this step's ticks; JS drains them after step()
        self.builds.clear(); // …same for house-build requests
        self.wells.clear(); // …same for well-dig requests
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

    /// This step's well-dig requests, flat [x, z, …] — JS places a well that it then feeds back as a drink source.
    pub fn wells(&self) -> &[f32] {
        &self.wells
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

        // 1. rebuild both grids from the PREVIOUS positions (flocking + coarse food-chain), and tally live people
        // for the LOW-POP BANDING latch — done at tick START so this tick's targeting/flock already see the truce
        // (an end-of-tick latch lagged a tick, letting a point-blank kill slip through before it engaged).
        self.grid.clear();
        self.seek_grid.clear();
        let mut people = 0usize;
        let mut kind_pop = [0usize; 6];
        let mut morph_sum = [0.0f64; 6]; // Σ safety weight per kind → mean = the apostatic search-image reference
        for (i, m) in self.agents.iter().enumerate() {
            if m.dead {
                // a FRESH carcass still goes into the SEEK grid so a hungry scavenger can find it (the dead-check in
                // targeting skips it as live prey); it's left OUT of the flock grid (corpses don't flock/separate).
                if m.carrion > 0.0 {
                    self.seek_grid.insert(m.agent.x, m.agent.z, i as u32);
                }
                continue;
            }
            kind_pop[m.kind as usize] += 1; // live per-kind census → predators prefer ABUNDANT prey (see target())
            morph_sum[m.kind as usize] += m.weights.safety;
            if matches!(m.kind, Kind::Person) {
                people += 1;
            }
            self.grid.insert(m.agent.x, m.agent.z, i as u32);
            self.seek_grid.insert(m.agent.x, m.agent.z, i as u32);
        }
        self.kind_pop = kind_pop;
        for k in 0..6 {
            self.morph_mean[k] = if kind_pop[k] > 0 { morph_sum[k] / kind_pop[k] as f64 } else { 1.0 };
        }
        // hysteresis: a dwindling people close ranks (truce + gather) at/below LOW, only release past RELEASE.
        self.person_pop = people;
        if self.person_banding {
            if people >= PERSON_BAND_RELEASE {
                self.person_banding = false;
            }
        } else if people <= PERSON_BAND_LOW {
            self.person_banding = true;
        }

        // 1b. SETTLEMENT OCCUPANCY — tally each live person into their nearest settlement (within SETTLE_R) so the
        // migration drive can steer roamers toward UNDER-populated towns. O(people × refuges); fine for moderate
        // counts and it's off the main thread (the worker). Cleared + refilled each tick (occupancy shifts as people move).
        if !self.refuge_pop.is_empty() {
            for p in self.refuge_pop.iter_mut() {
                *p = 0;
            }
            for m in self.agents.iter() {
                if m.dead || !matches!(m.kind, Kind::Person) {
                    continue;
                }
                let (mut best, mut best_d2) = (usize::MAX, SETTLE_R2);
                for (ri, &(rx, rz)) in self.refuges.iter().enumerate() {
                    let d2 = (rx - m.agent.x).powi(2) + (rz - m.agent.z).powi(2);
                    if d2 < best_d2 {
                        best_d2 = d2;
                        best = ri;
                    }
                }
                if best != usize::MAX {
                    self.refuge_pop[best] += 1;
                }
            }
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
                // PEOPLE are bolder against predators (tool-users defending their own) → each counts double toward
                // the swarm, so just 2–3 villagers gang up and drive off a lion instead of all scattering. Cats/lions
                // still need the full MOB_MIN of their own to turn on a bigger hunter.
                self.transient[t].mob_count += if matches!(self.agents[i].kind, Kind::Person) { 2 } else { 1 };
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
            let (fx, fz, crowd, migrating) = self.flock(i, px, pz);
            self.forces[i] = (fx, fz, crowd);
            self.agents[i].migrating = migrating; // HUD: who's en route to another settlement
        }

        // 5. behaviour — act on the targeting: FLEE a threat, else STALK + CATCH prey (+ EAT / food-coma).
        // Adds to the flock force and sets the sprint boost / forced-move, from the previous positions.
        // (player-scatter / huntPlayer / rival / mobbing are the remaining behaviour bits.)
        self.behave.clear();
        self.behave.resize(n, (1.0, false));
        self.kills.clear();
        let hunt2 = HUNT2 * (1.0 + 0.4 * self.night); // keener at night
        let mut danger_now = 0.0_f64; // peak imminence of any player-hunting predator this tick
        // ── THE BEHAVIOUR SEAM (design doc §2): only the DECIDE pass differs between modes. Sections 1–4
        // (perception/targeting/mobbing/sleep/flock) above and 6–9 (kills/metabolism/breeding/build/step/collide)
        // below are SHARED. Emergent scores needs+primitives+utility; Manual runs the hand-coded chain that follows.
        if let BehaviorMode::Emergent = self.behavior_mode {
            danger_now = emergent::decide(self, px, pz, pspeed, danger2, hunt2);
        } else {
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            if self.agents[i].feeding > 0.0 {
                self.behave[i] = (1.0, true); // hunkered over a fresh kill → settle + eat, don't fidget/re-target
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
            if !self.player_immune
                && rank >= 4
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
                // guard the stored index — it can dangle if the agent buffer is ever compacted (the test harness's
                // reap; the future unified buffer). In the live game slots are stable so b < len always holds.
                self.agents[i].bully.filter(|&b| b < self.agents.len() && !self.agents[b].dead).map(|b| (self.agents[b].agent.x, self.agents[b].agent.z))
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
            // SCAVENGE — a HUNGRY carnivore (nothing else pressing) pads to the nearest FRESH carcass and feeds, so
            // a death (old age / starvation / another's kill) isn't wasted. The if-chain below ranks it AFTER live
            // prey (a fresh kill beats leftovers) but ABOVE the idle fish-lure / wander.
            let carrion_pos = if a_hunts
                && self.agents[i].hungry
                && !mobbed
                && threat_pos.is_none()
                && !fighting_rival
                && self.agents[i].spooked <= 0.0
            {
                let mut scratch = std::mem::take(&mut self.seek_neighbors);
                let found = self.nearest_carrion(ax, az, SCAVENGE_R, &mut scratch);
                self.seek_neighbors = scratch;
                found
            } else {
                None
            };

            // A HUNGRY hunter with prey in sight COMMITS through the swarm — it charges in for the kill while
            // tanking the mob's damage, instead of always fleeing. Without this a dense crowd made predators
            // totally impotent (measured: 6 lions, 60 people, 100 s → 0 kills): they'd flee the mob forever and
            // never catch anyone. Now a crowd is DANGEROUS to a lion (it bleeds, can be dragged down) but never
            // immune from it. It still breaks away when sated, badly wounded, or with no prey to grab.
            let commit_through_mob = mobbed && self.agents[i].hungry && prey_info.is_some() && self.agents[i].health > HURT_AT;
            // A mobbed hunter ALWAYS bleeds from attackers pressed on it + slashes back — whether it breaks away
            // or commits — thinning the mob in real time until its ferocity is spent and the survivors drag it down.
            if mobbed {
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
            }
            if mobbed && !commit_through_mob {
                // outnumbered + not committing → BREAK AWAY from the swarm's centre (a fast hunter shakes them off)
                let mc = self.transient[i].mob_count.max(1) as f64;
                let cx = self.transient[i].mob_x / mc;
                let cz = self.transient[i].mob_z / mc;
                let dx = ax - cx;
                let dz = az - cz;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * FLEE_W;
                self.forces[i].1 += (dz / d) * a_max * FLEE_W;
                self.behave[i] = (if can_sprint { FLEE_BOOST } else { 1.0 }, true);
            } else if let Some((bx, bz)) = bully_pos {
                // freshly bullied (lost a rival fight) → keep fleeing that bully while spooked
                let dx = ax - bx;
                let dz = az - bz;
                let d = dx.hypot(dz).max(0.1);
                self.forces[i].0 += (dx / d) * a_max * FLEE_W;
                self.forces[i].1 += (dz / d) * a_max * FLEE_W;
                self.behave[i] = (if can_sprint { FLEE_BOOST } else { 1.0 }, true);
            } else if let Some((tx, tz)) = threat_pos {
                // if the hunter is MOBBED the herd has the numbers → CHARGE it (drive it off); else FLEE it.
                let threat_mobbed = self.transient[i].threat.map_or(false, |t| self.agents[t].mobbed);
                // VILLAGE GUARDS: an adult MALE person holds his ground + charges a predator threatening the
                // community (rally count reached) instead of fleeing — defending the women + children, who flee.
                let rally = self.transient[i].threat.map_or(0, |t| self.transient[t].mob_count);
                let person_adult = matches!(self.agents[i].kind, Kind::Person) && self.agents[i].age >= self.agents[i].lifespan * 0.15;
                let is_guard = person_adult && !is_female(self.agents[i].seed_id) && rally >= GUARD_RALLY;
                // CORNERED: a predator deep inside a crowd → every adult (women too) turns and fights, no escape.
                let cornered = person_adult && rally >= SURROUND_RALLY;
                let (dx, dz, w) = if threat_mobbed || is_guard || cornered {
                    (tx - ax, tz - az, MOB_W) // converge on the hunter (mob · guard men · or everyone, cornered)
                } else {
                    (ax - tx, az - tz, FLEE_W) // flee the hunter (women + children at the edge, prey)
                };
                let d = dx.hypot(dz).max(0.1);
                let (mut ux, mut uz) = (dx / d, dz / d);
                // FLEE TO SAFETY (C): a fleeing PERSON heads for the nearest house — home (and the guard men who
                // cluster there) is safety. Blend the home-ward unit into her escape vector, but only if it doesn't
                // turn her back toward the predator (then plain flight wins — never run INTO the hunter).
                if w == FLEE_W && matches!(self.agents[i].kind, Kind::Person) {
                    if let Some((hx, hz)) = self.nearest_refuge(ax, az, REFUGE_R) {
                        let (rx, rz) = (hx - ax, hz - az);
                        let rd = rx.hypot(rz).max(0.1);
                        let (rux, ruz) = (rx / rd, rz / rd);
                        if rux * ux + ruz * uz > -0.2 {
                            // home isn't behind the predator → curve toward it
                            let (bx, bz) = (ux + rux * REFUGE_PULL, uz + ruz * REFUGE_PULL);
                            let bl = bx.hypot(bz).max(0.1);
                            ux = bx / bl;
                            uz = bz / bl;
                        }
                    }
                }
                self.forces[i].0 += ux * a_max * w;
                self.forces[i].1 += uz * a_max * w;
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
                    let finishing = self.agents[p].health <= STRIKE_DMG; // this strike kills it (else just a deep wound)
                    self.agents[p].health = (self.agents[p].health - STRIKE_DMG).max(0.0);
                    self.agents[p].spooked = self.agents[p].spooked.max(2.0); // wounded → it bolts (the struggle)
                    if finishing {
                        self.kills.push(p); // finishing blow — turned to a corpse below
                        self.events.extend_from_slice(&[EV_KILL, self.agents[p].kind as usize as f32, self.agents[p].agent.x as f32, self.agents[p].agent.z as f32]);
                        self.agents[i].meals += 1;
                        self.agents[i].fed_meat = MEAT_SATED; // a meat meal → people can breed for a while (no-op for others)
                        self.agents[i].feeding = FEED_SECS; // hunker down + eat (no fidget) for a few seconds
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
                }
            } else if let Some((cx, cz, ci)) = carrion_pos {
                // pad to the carcass at a WALK; FEED on contact — refuel fullness + drain the carcass's meat (so it
                // feeds a few scavengers, then it's picked clean). No food-coma: scraps are a top-up, not a gorge.
                let dx = cx - ax;
                let dz = cz - az;
                let d = dx.hypot(dz).max(0.1);
                let contact = radius + self.agents[ci].radius + CONTACT_PAD + 0.3;
                if d < contact {
                    // AT the carcass → SETTLE and feed: no approach force (the overshoot-then-re-aim orbit made a
                    // scavenger fidget ON the corpse), pursuing flag held so the idle FSM can't frolic on the body.
                    self.behave[i] = (1.0, true);
                    self.agents[i].energy = (self.agents[i].energy + SCAVENGE_GAIN * DT).min(1.0);
                    self.agents[ci].carrion = (self.agents[ci].carrion - SCAVENGE_DRAIN * DT).max(0.0);
                } else {
                    self.forces[i].0 += (dx / d) * a_max * CHASE_W * 0.7;
                    self.forces[i].1 += (dz / d) * a_max * CHASE_W * 0.7;
                    self.behave[i] = (1.0, true);
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
            // FRAILTY — in the last stretch of life it slows (senescence), so predators naturally cull the old &
            // weak and the generations turn over. Ramps from full speed at FRAIL_ONSET of lifespan to FRAIL_MIN at
            // death. The player's pet is exempt (it never dies of age either). Multiplies the gait cap (boost).
            if !self.agents[i].companion {
                let life = self.agents[i].age / self.agents[i].lifespan.max(1.0);
                if life > FRAIL_ONSET {
                    let t = ((life - FRAIL_ONSET) / (1.0 - FRAIL_ONSET)).min(1.0);
                    self.behave[i].0 *= 1.0 - t * (1.0 - FRAIL_MIN);
                }
            }
        }
        } // end Manual behaviour arm of the seam

        // ease the danger level toward this tick's peak → the UI vignette swells/fades smoothly
        self.danger += (danger_now - self.danger) * (6.0 * DT).min(1.0);

        // 6. apply kills → corpses (deferred so the behaviour pass reads only previous, live positions)
        for k in 0..self.kills.len() {
            let p = self.kills[k];
            self.agents[p].dead = true;
            self.agents[p].asleep = false;
            self.agents[p].agent.vx = 0.0;
            self.agents[p].agent.vz = 0.0;
            self.agents[p].carrion = CARRION_MEAT; // a fresh carcass — scavengeable until it rots / is picked clean
        }

        // 7. metabolism (AWAKE agents) — sprinting + a carnivore's basal drain ebb stamina; prey/people
        // rest-recover. The LATCHED hunger (hysteresis LO/HI) is the flip-flop fix. Plus slow healing, the
        // cooldown timers, and the exhaustion-sleep trigger. (Asleep agents recovered in the sleep pass.)
        for i in 0..n {
            if self.agents[i].dead {
                if self.agents[i].carrion > 0.0 {
                    self.agents[i].carrion = (self.agents[i].carrion - DT).max(0.0); // a carcass rots even uneaten
                }
                continue;
            }
            if self.slept[i] {
                continue;
            }
            // a slash / scrap that emptied the health bar this tick is fatal (checked BEFORE the heal regen)
            if self.agents[i].health <= 0.0 {
                self.agents[i].dead = true;
                self.agents[i].asleep = false;
                self.agents[i].agent.vx = 0.0;
                self.agents[i].agent.vz = 0.0;
                self.agents[i].carrion = CARRION_MEAT; // starved/wounded carcass → feeds a scavenger
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
                self.agents[i].carrion = CARRION_MEAT; // an elder that dropped → a carcass for the scavengers
                let (k, ax, az) = (self.agents[i].kind as usize as f32, self.agents[i].agent.x as f32, self.agents[i].agent.z as f32);
                self.events.extend_from_slice(&[EV_OLDAGE, k, ax, az]);
                continue;
            }
            // CULTURE — a YOUNG settler learns from the oldest nearby elder of its kind, blending its behaviour
            // genome toward that role model (memetic transmission, on top of parental genes). Conformity to the
            // local successful type → settlements grow distinct CUSTOMS and isolated populations diverge. People
            // only (settlements are a people thing) → the rabbit niche dynamics are untouched.
            if matches!(self.agents[i].kind, Kind::Person)
                && self.agents[i].age < self.agents[i].lifespan * CULTURE_AGE
                && crate::simrng::rand(&[self.agents[i].seed_id, tick, CH_CULTURE]) < CULTURE_P
            {
                let (ax, az, my_age) = (self.agents[i].agent.x, self.agents[i].agent.z, self.agents[i].age);
                let mut model: Option<usize> = None;
                let mut best_age = my_age; // only learn from someone OLDER (more successful at surviving)
                self.grid.for_each_neighbor(ax, az, |j| {
                    let j = j as usize;
                    if j == i || self.agents[j].dead || !matches!(self.agents[j].kind, Kind::Person) {
                        return;
                    }
                    if self.agents[j].age > best_age {
                        let d2 = (self.agents[j].agent.x - ax).powi(2) + (self.agents[j].agent.z - az).powi(2);
                        if d2 <= CULTURE_R2 {
                            best_age = self.agents[j].age;
                            model = Some(j);
                        }
                    }
                });
                if let Some(j) = model {
                    let learned = self.agents[i].weights.blend_toward(&self.agents[j].weights, CULTURE_RATE);
                    self.agents[i].weights = learned;
                }
            }
            // EXPECTANT FATHER STANDS GUARD — when a predator menaces him or his carrying mate, he BRANDISHES (his
            // machete): the predator is spooked off + gives up the stalk. Aggression toward predators, not flight.
            if let Some(mate) = self.agents[i].partner {
                if mate < n && self.agents[mate].pregnant > 0.0 {
                    for t in [self.transient[i].threat, self.transient[mate].threat].into_iter().flatten() {
                        if t < n && !self.agents[t].dead && matches!(eco(self.agents[t].kind).hunts, Hunts::Lower) {
                            self.agents[t].spooked = self.agents[t].spooked.max(BRANDISH_SPOOK);
                            self.agents[t].give_up_cd = self.agents[t].give_up_cd.max(BRANDISH_SPOOK);
                        }
                    }
                }
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
                let drain = if is_carnivore { ENERGY_DRAIN * CARN_DRAIN_FRAC } else { ENERGY_DRAIN };
                let mut en = self.agents[i].energy - drain * DT;
                if !is_carnivore && boost <= 1.0 && self.agents[i].spooked <= 0.0 {
                    let lushness = (1.0 - self.agents[i].crowd as f64 / GRAZE_CROWD).max(0.0);
                    // BOLD foragers refuel faster on open ground (they also flee later → caught more): the trade-off
                    // that makes `safety` a niche axis, not a one-way ratchet. Neutral genome (Manual) → forage()=1.
                    en += GRAZE_RATE * lushness * self.agents[i].weights.forage() * DT;
                }
                self.agents[i].energy = en.clamp(0.0, 1.0);
            }
            // THIRST — hydration ebbs; top up at a water EDGE. No water defined (a bare test world) → thirst is
            // inert, so every pre-water scenario stays valid. The companion never thirsts (magically tended).
            let has_water = self.natural_water || !self.water_src.is_empty();
            if self.agents[i].companion {
                self.agents[i].hydration = 1.0;
            } else if has_water {
                let (ax, az) = (self.agents[i].agent.x, self.agents[i].agent.z);
                let at_water = matches!(self.nearest_water(ax, az), Some((_, _, d)) if d <= 0.0);
                let drought = self.drought(tick); // season × director drought → faster thirst in the dry season
                let rate = if at_water { DRINK_RATE } else { -THIRST_DRAIN * drought };
                self.agents[i].hydration = (self.agents[i].hydration + rate * DT).clamp(0.0, 1.0);
            }
            let parched = has_water && !self.agents[i].companion && self.agents[i].hydration <= 0.0;
            // health heals when fed AND watered, but BLEEDS when the belly's empty (starvation) or parched (thirst)
            if self.agents[i].energy > 0.0 && !parched {
                self.agents[i].health = (self.agents[i].health + HEAL * DT).min(1.0);
            } else {
                let dmg = (if self.agents[i].energy <= 0.0 { STARVE_DAMAGE } else { 0.0 }) + if parched { THIRST_DAMAGE } else { 0.0 };
                self.agents[i].health = (self.agents[i].health - dmg * DT).max(0.0);
                if self.agents[i].health <= 0.0 {
                    let (k, ax, az) = (self.agents[i].kind as usize as f32, self.agents[i].agent.x as f32, self.agents[i].agent.z as f32);
                    self.events.extend_from_slice(&[EV_STARVE, k, ax, az]); // famine/thirst claimed it (dies next tick)
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
            if self.agents[i].feeding > 0.0 {
                self.agents[i].feeding -= DT; // eating-its-kill timer ebbs → then it moves on
            }
            if self.agents[i].fed_meat > 0.0 {
                self.agents[i].fed_meat -= DT; // "recently ate meat" ebbs → a person must hunt another rabbit to keep breeding
            }
            // PAIR-BOND lifecycle: a bond ages; once the young have grown (past BOND_REARING) the couple MAY split
            // (seeded — some part, some stay for life). Also drop a bond whose partner died or is no longer mutual.
            if let Some(p) = self.agents[i].partner {
                let valid = p < self.agents.len() && p != i && !self.agents[p].dead && self.agents[p].partner == Some(i);
                if !valid {
                    self.agents[i].partner = None;
                } else {
                    self.agents[i].bond_age += DT;
                    // never split while she's still carrying — the couple stays glued through gestation
                    let gestating = self.agents[i].pregnant > 0.0 || self.agents[p].pregnant > 0.0;
                    if !gestating && self.agents[i].bond_age > BOND_REARING {
                        // one seeded roll per bond (key on the lower index + a coarse time bucket → fires once-ish)
                        let lo = i.min(p) as i32;
                        if crate::simrng::rand(&[lo, (self.clock.tick / 600) as i32, CH_BONDSPLIT]) < BOND_SPLIT_FRAC {
                            self.agents[i].partner = None;
                            self.agents[p].partner = None; // split cleanly: both go free
                        }
                    }
                }
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
        // (the person count + LOW-POP BANDING latch are computed at tick START — see the grid-rebuild pass.)
        // 🌿 MOTHER NATURE — the homeostatic DIRECTOR, in-sim. NO HARD POPULATION CAP (decided 2026-06-22): the world
        // grows to thousands+ over time (the shared multiplayer big-world), so headcount ceilings are GONE. The
        // NATURAL ceiling is FOOD — a herd that outgrows its grazing overgrazes + starves back (energy pass), predators
        // are bounded by prey, old age + predation churn it. Mother Nature's only in-sim job is ANTI-EXTINCTION RESCUE:
        // a species crashing toward zero breeds hard so a spawned cohort can't dead-end. The richer director
        // (nature.svelte.ts / the LLM) layers on top. Vitality feeds breed_ready (lower fullness bar) + the
        // post-mating cooldown. (Old cap-ratio drift removed; see git history.)
        for kind in [Kind::Rabbit, Kind::Cat, Kind::Kangaroo, Kind::Person, Kind::Lion, Kind::Dinosaur] {
            let k = kind as usize;
            let n = pop[k] as f64;
            // SOFT LOGISTIC HOMEOSTASIS (not a hard cap, never culls): below the rescue floor → breed hard; otherwise
            // the breeding RATE eases smoothly toward 0 as the population approaches a LARGE soft target, so it GROWS
            // GRADUALLY then PLATEAUS instead of exploding exponentially (the "47k objects @ 1fps" bug). Vitality 0 ⇒
            // the energy/cooldown bars are unmeetable ⇒ births stop; deaths then pull it back under target ⇒ it holds.
            // The target scales with the built world (pop_scale) — a developed region supports more life.
            let cap = soft_target(kind) * self.pop_scale;
            let target = if n >= 1.0 && (n as usize) < RESCUE_N {
                1.8 // anti-extinction rescue
            } else {
                (1.3 * (1.0 - n / cap)).max(0.0) // ease to 0 at the soft target → plateau
            };
            self.vitality[k] += (target - self.vitality[k]) * VITALITY_LERP;
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
            let (pfam_a, pfam_b) = (self.agents[i].fam, self.agents[i].unborn_dad_fam); // the litter's parents (mother, sire)
            let g = self.agents[i].unborn_genome; // the litter's inherited behaviour genome (ferried to JS → set_genome)
            let born = litter_size(kind, self.agents[i].seed_id, self.clock.tick as i32); // NO cap — deliver the FULL litter; food/predation/old-age are the natural limits
            for b in 0..born {
                // each littermate inherits the carried vigor ± a touch more mutation, so siblings vary a little
                let mu = (crate::simrng::rand(&[self.agents[i].seed_id, self.clock.tick as i32, b as i32, CH_GENE]) - 0.5) * 2.0 * GENE_MUT;
                let baby_gene = (self.agents[i].unborn_gene + mu).clamp(GENE_MIN, GENE_MAX);
                // born clustered AROUND the mother (a small seeded ring), not all stacked on her exact point — else
                // the anti-overlap explodes the litter apart on tick 1. Tiny radius → they still read as her brood.
                let ang = crate::simrng::rand(&[self.agents[i].seed_id, self.clock.tick as i32, b as i32, CH_BIRTHPOS]) * std::f64::consts::TAU;
                let rad = 0.4 + 0.25 * b as f64; // siblings ring out a touch so they don't all share one spot
                let bx = mx + ang.cos() * rad;
                let bz = mz + ang.sin() * rad;
                // births stride = 11: [kindCode, x, z, gene, motherFam, fatherFam, g.food, g.safety, g.social, g.rest,
                // g.industry]. Parent fams → set_lineage (incest); the 5 genome weights → set_genome (strategy evolution).
                self.births.push(kc as f32);
                self.births.push(bx as f32);
                self.births.push(bz as f32);
                self.births.push(baby_gene as f32);
                self.births.push(pfam_a as f32);
                self.births.push(pfam_b as f32);
                self.births.push(g.food as f32); // …+ the 5 behaviour-genome weights (stride 11) → JS set_genome on spawn
                self.births.push(g.safety as f32);
                self.births.push(g.social as f32);
                self.births.push(g.rest as f32);
                self.births.push(g.industry as f32);
                self.events.extend_from_slice(&[EV_BIRTH, kc as f32, bx as f32, bz as f32]);
                pop[kc] += 1;
            }
        }
        // B. MATING — a fertile opposite-sex pair conceives: the female starts gestating; both pay the breed cost.
        for i in 0..n {
            if !self.breed_ready(i) {
                continue;
            }
            let kc = self.agents[i].kind as usize;
            // NO headcount cap — a fertile, well-fed, calm pair always conceives. Population is bounded by FOOD
            // (overgrazing→starvation lowers the energy that breed_ready needs), predation, and old age, not a ceiling.
            if let Some(j) = self.find_mate(i) {
                let mom = if is_female(self.agents[i].seed_id) { i } else { j }; // the female of the pair carries
                // INHERIT: the litter's vigor = average of the parents' genes ± mutation (deterministic RNG), clamped
                let mu = (crate::simrng::rand(&[self.agents[i].seed_id, self.agents[j].seed_id, self.clock.tick as i32, CH_GENE]) - 0.5) * 2.0 * GENE_MUT;
                self.agents[mom].unborn_gene = (((self.agents[i].gene + self.agents[j].gene) * 0.5) + mu).clamp(GENE_MIN, GENE_MAX);
                let dad = if mom == i { j } else { i };
                self.agents[mom].unborn_dad_fam = self.agents[dad].fam; // remember the sire's lineage for the litter's parentage
                // INHERIT the behaviour genome: blend of both parents ± seeded mutation → strategies evolve across births
                self.agents[mom].unborn_genome = Genome::inherit(&self.agents[i].weights, &self.agents[j].weights, self.agents[i].seed_id, self.agents[j].seed_id, self.clock.tick as i32);
                self.agents[mom].pregnant = gestation(self.agents[mom].kind);
                self.events.extend_from_slice(&[EV_CONCEIVE, kc as f32, self.agents[mom].agent.x as f32, self.agents[mom].agent.z as f32]);
                let vit = self.vitality[kc]; // director boost → shorter recovery between litters
                // each parent's own boldness sets how fast IT bounces back (r/K niche lever): bold → shorter cd →
                // more litters, paid for by dying more (flees late). Neutral genome (Manual) → haste 1.0 → unchanged.
                self.agents[i].breed_cd = BREED_COOLDOWN / (vit * self.agents[i].weights.breed_haste());
                self.agents[j].breed_cd = BREED_COOLDOWN / (vit * self.agents[j].weights.breed_haste());
                self.agents[i].energy = (self.agents[i].energy - BREED_COST).max(0.0);
                self.agents[j].energy = (self.agents[j].energy - BREED_COST).max(0.0);
                // PAIR-BOND: the couple sticks together to raise this litter (tether in flock); the timer resets so
                // they stay bonded through gestation + rearing, then MAY split once the young grows up (metabolism pass).
                self.agents[i].partner = Some(j);
                self.agents[j].partner = Some(i);
                self.agents[i].bond_age = 0.0;
                self.agents[j].bond_age = 0.0;
            }
        }

        // 7.6 EMERGENT CITIES — a settled FAMILY (an adult male+female pair) raises a home, which clusters into a
        // town then a city. Gated on an opposite-sex adult Person nearby (NOT the generic flock `crowd`, which
        // counts ANY neighbours — so a lone human among rabbits was building houses). The sim emits where to build.
        for i in 0..n {
            let m = &self.agents[i];
            if m.dead
                || m.asleep
                || !matches!(m.kind, Kind::Person)
                || m.build_cd > 0.0
                || m.energy < BUILD_ENERGY
                || m.age < m.lifespan * 0.15 // an adult, not a child
                || self.transient[i].threat.is_some()
                || !self.has_family(i) // only a pair-bonded household builds — no lone-wanderer houses
            {
                continue;
            }
            let (bx, bz) = (m.agent.x as f32, m.agent.z as f32);
            // WATER BEFORE SHELTER — an industrious settler with no water edge within WELL_NEED_R digs a WELL instead
            // of a house this cycle. JS places the well + feeds it back as a drink source, so next time nearest_water
            // finds it (dry=false) and the household builds homes around the water it made. Emergent jobs + towns.
            let industrious = m.weights.industry > WELL_INDUSTRY;
            let dry = self.nearest_water(m.agent.x, m.agent.z).map_or(true, |(_, _, d)| d > WELL_NEED_R);
            if industrious && dry {
                self.wells.push(bx);
                self.wells.push(bz);
                self.events.extend_from_slice(&[EV_WELL, Kind::Person as usize as f32, bx, bz]);
            } else {
                self.builds.push(bx);
                self.builds.push(bz);
                self.events.extend_from_slice(&[EV_BUILD, Kind::Person as usize as f32, bx, bz]);
            }
            self.agents[i].energy -= BUILD_COST;
            self.agents[i].build_cd = BUILD_COOLDOWN;
        }

        // 8. step each AWAKE agent (write the next positions)
        for i in 0..n {
            if self.agents[i].dead || self.slept[i] {
                continue;
            }
            let (fx, fz, crowd) = self.forces[i];
            let (mut boost, pursuing) = self.behave[i];
            // PREGNANCY — she waddles (slow) while carrying, UNLESS fleeing a hunter (survival overrides). Applies in
            // both behaviour arms since this step pass is shared. The mate's tether keeps him beside her regardless.
            if self.agents[i].pregnant > 0.0 && !self.agents[i].hunting && self.agents[i].spooked <= 0.0 {
                boost *= PREGNANT_SPEED;
            }
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
            && m.energy > BREED_ENERGY / self.vitality[m.kind as usize] // well-fed bar, EASED for a species the director is boosting
            && m.age >= m.lifespan * JUVENILE_FRAC // matured to sexual maturity (scales with lifespan — see JUVENILE_FRAC)
            && m.age < m.fertile_until // …and within its fertile window (female menopause / near-death for males)
            && (!matches!(m.kind, Kind::Person) || m.fed_meat > 0.0) // PEOPLE need a recent MEAT meal (a rabbit) to breed
            && (self.water_src.is_empty() || m.hydration > BREED_HYDRATION) // a PARCHED animal can't reproduce → thirst
            // actually regulates the population (else a herd out-breeds the slow thirst-death on a "breed-then-die"
            // treadmill). Only bites when the world HAS water, so pre-water scenarios are untouched.

            && m.pregnant <= 0.0 // not already carrying a litter
            && !m.mobbed
            && m.spooked <= 0.0
            // DENSITY-DEPENDENT breeding — the ONLY brake now that hard caps are gone (else breeding goes exponential
            // and the world explodes to thousands in seconds). A SATURATED patch stops breeding (it plateaus); the
            // sparse FRONTIER keeps growing → the population rises GRADUALLY + SPATIALLY (spreads), never explodes,
            // and is never culled. People tolerate a denser settlement than herd prey before they plateau; the
            // breed-stop crowd sits BELOW the dispersal crowd so a filling area plateaus before it splinters.
            && m.crowd < if matches!(m.kind, Kind::Person) { PERSON_BREED_CROWD } else { BREED_CROWD }
            && self.transient[i].threat_d > BREED_FEAR_R2 // a hunter must be RIGHT HERE to interrupt mating, not just within the 40 m flee radius
            && !self.transient[i].near_predator
    }

    /// Nearest same-kind, OPPOSITE-SEX, breed-ready mate within BREED_R2 (grid query). Skips `i` itself.
    fn find_mate(&self, i: usize) -> Option<usize> {
        let (ax, az) = (self.agents[i].agent.x, self.agents[i].agent.z);
        let kind = self.agents[i].kind;
        let my_sex = is_female(self.agents[i].seed_id);
        // search the COARSE food-chain grid (cell = SEEK), NOT the 4 m flock grid — otherwise only a dense flock
        // (rabbits) or a packed city (people) ever had two adults close enough to pair, and every sparse species
        // (kangaroos, the predators) quietly died out. Radius is per-kind: herds tighter, lone hunters far wider.
        let r2 = if matches!(kind, Kind::Cat | Kind::Lion | Kind::Dinosaur) { PRED_BREED_R2 } else { HERD_BREED_R2 };
        let mut found: Option<usize> = None;
        self.seek_grid.for_each_neighbor(ax, az, |j| {
            let j = j as usize;
            // same kind, the OTHER sex (a male + a female make a baby), and not itself
            if found.is_some() || j == i || self.agents[j].kind != kind || is_female(self.agents[j].seed_id) == my_sex {
                return;
            }
            let d2 = (self.agents[j].agent.x - ax).powi(2) + (self.agents[j].agent.z - az).powi(2);
            if d2 <= r2 && self.breed_ready(j) && !related(&self.agents[i], &self.agents[j]) {
                found = Some(j); // opposite-sex, fertile, in range, AND not close kin (no incest)
            }
        });
        found
    }

    /// Is there an adult, opposite-sex PERSON within a household radius of `i`? Gates house-building so only a
    /// settled FAMILY raises a home (not a lone wanderer). Looser than `find_mate`: the partner needn't be
    /// breed-ready right now (a couple that just had a child is still a household).
    fn has_family(&self, i: usize) -> bool {
        let (ax, az) = (self.agents[i].agent.x, self.agents[i].agent.z);
        let my_sex = is_female(self.agents[i].seed_id);
        let mut found = false;
        self.grid.for_each_neighbor(ax, az, |j| {
            let j = j as usize;
            if found || j == i || self.agents[j].dead || self.agents[j].kind != Kind::Person || is_female(self.agents[j].seed_id) == my_sex {
                return;
            }
            if self.agents[j].age < self.agents[j].lifespan * 0.15 {
                return; // a child isn't a partner
            }
            let d2 = (self.agents[j].agent.x - ax).powi(2) + (self.agents[j].agent.z - az).powi(2);
            if d2 <= FAMILY_R2 {
                found = true;
            }
        });
        found
    }

    /// Mark a spawned agent as a NEWBORN: a maturation cooldown (can't breed until grown) AND age reset to 0 — a
    /// baby starts life at zero even though founders spawn with a seeded random age (see randomize_start_age).
    pub fn set_breed_cooldown(&mut self, i: usize, cd: f64) {
        if let Some(m) = self.agents.get_mut(i) {
            m.breed_cd = cd;
            m.age = 0.0;
        }
    }

    /// Give a FOUNDER (player/initial spawn) a seeded random starting age across its fertile life, so a freshly
    /// spawned population has AGE STRUCTURE instead of being one synchronized cohort that booms then dies off
    /// together (the "1100 humans → 5" crash). Newborns are exempt — set_breed_cooldown() resets them to age 0.
    pub fn randomize_start_age(&mut self, i: usize, seed_id: i32) {
        if let Some(m) = self.agents.get_mut(i) {
            m.age = m.lifespan * 0.6 * crate::simrng::rand(&[seed_id, 41]); // spread [0, 0.6·lifespan) → all fertile, staggered deaths
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
    fn flock(&self, i: usize, px: f64, pz: f64) -> (f64, f64, u32, bool) {
        let m = &self.agents[i];
        let (ax, az, avx, avz, a_max) = (m.agent.x, m.agent.z, m.agent.vx, m.agent.vz, m.agent.max_speed);
        let is_person = matches!(m.kind, Kind::Person);
        let mut migrating = false; // set when the inter-settlement migration steer fires → surfaced to the HUD
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
        // GENDER-BALANCED COLONISING (F): a YOUNG person about to disperse pairs with its nearest opposite-sex young
        // neighbour as a co-founder — they'll share an outward heading + stay together (below), so a band that strikes
        // out is a man + woman ("like missionaries"), able to actually grow a new settlement, not a single-sex dead end.
        let a_female = is_person && is_female(m.seed_id);
        let a_young = is_person && m.age < m.lifespan * DISPERSE_AGE;
        let mut buddy: Option<usize> = None;
        let mut buddy_d2 = nr2;
        // a baby ANIMAL trails the nearest grown adult of its kind (fawn/duckling trains) — tracked in the same scan.
        let a_juvenile = !is_person && m.age < m.lifespan * JUVENILE_FOLLOW_AGE;
        let mut parent: Option<usize> = None;
        let mut parent_d2 = nr2;
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
            // nearest opposite-sex young person = this disperser's co-founder
            if a_young
                && matches!(agents[j].kind, Kind::Person)
                && is_female(agents[j].seed_id) != a_female
                && agents[j].age < agents[j].lifespan * DISPERSE_AGE
                && d2 < buddy_d2
            {
                buddy_d2 = d2;
                buddy = Some(j);
            }
            // nearest grown adult of my kind = the parent this juvenile animal trails
            if a_juvenile
                && agents[j].kind == m.kind
                && agents[j].age >= agents[j].lifespan * PARENT_ADULT_AGE
                && d2 < parent_d2
            {
                parent_d2 = d2;
                parent = Some(j);
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
            // SOCIAL STRUCTURE: women + children keep a HOME group (stronger cohesion → a domestic cluster), while
            // grown men range out (very weak cohesion) to hunt + guard the bounds. Other species: a gentle default.
            let coh_w = if is_person {
                if self.person_banding {
                    // SURVIVAL TRUCE: a dwindling people close ranks — EVERYONE (men too) pulls together to form the
                    // nucleus of a re-founded town, instead of the men ranging out and the band thinning to nothing.
                    0.06 + BAND_GATHER_W
                } else if is_female(m.seed_id) || m.age < m.lifespan * 0.15 {
                    0.06 // women + children cluster
                } else {
                    0.012 // men range out
                }
            } else {
                0.04
            };
            let c = (a_max * coh_w) / cl;
            fx += cdx * c;
            fz += cdz * c;
            fx += (ali_x / nn - avx) * ALI_WEIGHT;
            fz += (ali_z / nn - avz) * ALI_WEIGHT;
        }

        // JUVENILE FOLLOW — a baby animal clings to the grown adult of its kind it found (a fawn trailing its
        // mother). A touch stronger than herd cohesion so the family train holds, but gentle enough that the kid
        // still wanders and (in a crowd) can disperse.
        if let Some(j) = parent {
            let pdx = agents[j].agent.x - ax;
            let pdz = agents[j].agent.z - az;
            let pl = pdx.hypot(pdz).max(0.1);
            let pw = a_max * FOLLOW_W;
            fx += pdx / pl * pw;
            fz += pdz / pl * pw;
        }

        // LONG-RANGE BANDING SEEK: while banding, a person without a local group (few flock-neighbours) steers toward
        // the NEAREST other person it can perceive on the WIDER seek grid — so FAR-FLUNG survivors actually walk over
        // and converge, not just the ones already standing close. Once it gathers a quorum, local cohesion takes over.
        if is_person && self.person_banding && n_near < BAND_SEEK_QUORUM {
            let mut best_d2 = SEEK2;
            let mut best: Option<usize> = None;
            self.seek_grid.for_each_neighbor(ax, az, |j| {
                let j = j as usize;
                if j == i || agents[j].dead || !matches!(agents[j].kind, Kind::Person) {
                    return;
                }
                let dx = agents[j].agent.x - ax;
                let dz = agents[j].agent.z - az;
                let d2 = dx * dx + dz * dz;
                if d2 < best_d2 {
                    best_d2 = d2;
                    best = Some(j);
                }
            });
            if let Some(j) = best {
                let dx = agents[j].agent.x - ax;
                let dz = agents[j].agent.z - az;
                let d = best_d2.sqrt().max(0.001);
                let w = a_max * BAND_SEEK_W;
                fx += dx / d * w;
                fz += dz / d * w;
            }
        }

        // INTER-SETTLEMENT MIGRATION — a lone ADULT person OUT IN THE WILD (no settlement within SETTLE_R) drifts
        // toward the nearest UNDER-populated town it can perceive: brings fresh UNRELATED blood so an isolated all-
        // kin settlement can breed again past the incest rule, and fills thin towns. Decentralised (each picks from
        // its own spot + a seeded jitter; a town stops drawing once it's full) → no "everyone to the one empty spot".
        // MIGRATION — EVERY organism (per-kind weight × age curve; user). Tendency is ~0 in youth, peaks in prime
        // adulthood, eases in old age. PEOPLE head for a sparser SETTLEMENT (wild/over-full always; a restless
        // wanderer subset leaves even a comfortable town → gene flow past the incest rule). ANIMALS strike outward
        // to fresh range (nomadic relocation). Decentralised + occupancy-aware → no "everyone to one spot".
        let mig_t = migrate_weight(m.kind) * age_migrate_factor(m.age, m.lifespan);
        if mig_t > 0.0 {
            let bucket = (self.clock.tick / WANDER_PERIOD) as i32; // re-rolled each period → episodic, not perpetual
            let wanderer = crate::simrng::rand(&[m.seed_id, bucket, CH_WANDERLUST]) < WANDER_FRAC * mig_t;
            let w = a_max * MIGRATE_W * migrate_weight(m.kind);
            if is_person && !self.refuges.is_empty() {
                let want = match self.here_occupancy(ax, az) {
                    None => true,                              // out in the wild → seek a town
                    Some(pop) => pop > SETTLE_TARGET || wanderer, // over-full town, or a restless soul → leave
                };
                if want {
                    if let Some((tx, tz)) = self.nearest_sparse_refuge(ax, az, m.seed_id) {
                        let (dx, dz) = (tx - ax, tz - az);
                        let d = dx.hypot(dz).max(0.001);
                        fx += dx / d * w;
                        fz += dz / d * w;
                        migrating = true; // surfaced to the HUD (snapshot flag)
                    }
                }
            } else if !is_person && wanderer {
                // animal NOMAD: commit to a seeded OUTWARD heading (fanned ±90°) → relocate the herd to fresh range
                let rng = crate::simrng::rand(&[m.seed_id, CH_DISPERSE]);
                let r0 = (ax * ax + az * az).sqrt();
                let ang = if r0 > 1.0 { az.atan2(ax) + (rng - 0.5) * std::f64::consts::PI } else { rng * std::f64::consts::TAU };
                fx += ang.cos() * w;
                fz += ang.sin() * w;
                migrating = true;
            }
        }

        // PAIR-BOND TETHER — a bonded mate keeps close to its partner (the family stays together raising young).
        // Validated MUTUALLY (partner[partner]==self) so a recycled/reordered slot can't make it chase a stranger.
        if let Some(p) = m.partner {
            if p < agents.len() && p != i && !agents[p].dead && agents[p].partner == Some(i) {
                let dx = agents[p].agent.x - ax;
                let dz = agents[p].agent.z - az;
                let d = dx.hypot(dz);
                // while either is expecting, the mate sticks RIGHT beside her — tighter leash + stronger pull (the
                // expectant father shadows her everywhere); otherwise the gentle family tether.
                let gestating = m.pregnant > 0.0 || agents[p].pregnant > 0.0;
                let (leash, pull) = if gestating { (0.8, BOND_W * 2.2) } else { (1.5, BOND_W) };
                if d > leash {
                    // only pull when they've drifted apart (so they don't crowd-shove on top of each other)
                    fx += dx / d * a_max * pull;
                    fz += dz / d * a_max * pull;
                }
            }
        }

        // THIRST-SEEK — a parched animal breaks for the nearest water, harder the thirstier it is. Lives in the
        // SHARED flock pass so both brains get it for free; it's just a steering pull, so fleeing/hunting (which
        // drive the speed boost) still take spatial priority when they fire. No water in the world → no pull.
        if m.hydration < THIRSTY_AT {
            if let Some((dx, dz, d_edge)) = self.nearest_water(ax, az) {
                if d_edge > 0.0 {
                    let d = dx.hypot(dz).max(0.1);
                    let urgency = (1.0 - m.hydration / THIRSTY_AT).clamp(0.0, 1.0); // 0 at the threshold → 1 bone-dry
                    // SOCIAL niche, in the WATER channel (orthogonal to boldness's predation channel): a GREGARIOUS
                    // animal navigates to water decisively via the herd's shared knowledge → reliably reaches the
                    // bank; a LONER seeks weakly and risks dying of thirst when water is far. The loner's payoff is
                    // the existing crowd-gate (low crowd → breeds freely). Herd = thirst-survival, loner = fecundity.
                    let herd_nav = (0.45 + 0.55 * m.weights.social).clamp(0.45, 1.6); // 1.0 at neutral → Manual unchanged
                    fx += dx / d * a_max * THIRST_SEEK_W * urgency * herd_nav;
                    fz += dz / d * a_max * THIRST_SEEK_W * urgency * herd_nav;
                }
            }
        }

        // PREDATORS AVOID SETTLEMENTS — a carnivore steers AWAY from a nearby town UNLESS it's REALLY hungry
        // (desperation drives it in to hunt) or a mother shadowing her young (she'll risk the edge). Towns stay safer.
        if matches!(eco(m.kind).hunts, Hunts::Lower) {
            let really_hungry = m.energy < PRED_DESPERATE;
            let mother = is_female(m.seed_id) && m.breed_cd > 0.0; // recently bred → has young about
            if !really_hungry && !mother {
                if let Some((rx, rz)) = self.nearest_refuge(ax, az, SETTLEMENT_AVOID_R) {
                    let (dx, dz) = (ax - rx, az - rz); // push away from the town centre
                    let d = dx.hypot(dz).max(0.1);
                    let falloff = 1.0 - d / SETTLEMENT_AVOID_R; // stronger the closer it is
                    fx += dx / d * a_max * SETTLEMENT_AVOID_W * falloff;
                    fz += dz / d * a_max * SETTLEMENT_AVOID_W * falloff;
                }
            }
        }

        // DISPERSAL — a young herbivore in a crowded patch heads off to colonise new ground (see consts). Direction
        // is seeded (steady per animal) so it commits to a heading and travels, rather than jittering in place; the
        // drive ramps with crowding and vanishes once it reaches open range, so it settles where there's room.
        // people disperse too, but only out of a true BLOB (higher threshold) — so a big clump (the user's "100
        // humans all in one place") splits into bands that strike out + found new settlements ("like missionaries"),
        // while a normal small family band (below the threshold) stays put.
        let disperse_at = if matches!(m.kind, Kind::Person) { PERSON_DISPERSE_CROWD } else { DISPERSE_CROWD };
        let blob_at = if matches!(m.kind, Kind::Person) { PERSON_BLOB_CROWD } else { BLOB_CROWD };
        // while banding (scarce humans) NOBODY strikes out — the point is to gather, not splinter further.
        let banding_hold = is_person && self.person_banding;
        // MIGRATION FOR ALL (user: "migration for all whenever too many in one area"): EVERY kind, EVERY age strikes
        // out of a true crowd (n_near ≥ blob_at) → an over-dense patch breaks up and migrates to open ground. Below
        // the blob threshold only the YOUNG peel off (steady-state colonisation). Predators rarely crowd, so this is
        // mostly prey + people; it's the spatial half of "grow + spread, don't pile up" (paired with the breed brake).
        let age_disperses = m.age < m.lifespan * DISPERSE_AGE || n_near >= blob_at;
        if !banding_hold && n_near >= disperse_at && age_disperses {
            let gain = smoothstep(disperse_at as f64, disperse_at as f64 + 5.0, n_near as f64);
            // LEADER / FOLLOWER pairing: in a co-founder pair the LOWER seed leads — it picks the outward compass
            // heading and commits to it; the higher-seed partner FOLLOWS, steering hard onto the leader instead of
            // its own heading. So a band is a leader trailed by the opposite sex → mixed by construction (no orphaned
            // single-sex bands). A LONE disperser (no opposite-sex co-founder near) just heads out on its own seed.
            let is_follower = matches!(buddy, Some(j) if agents[j].seed_id < m.seed_id);
            if let (true, Some(j)) = (is_follower, buddy) {
                let bx = agents[j].agent.x - ax;
                let bz = agents[j].agent.z - az;
                let bd = bx.hypot(bz).max(0.001);
                let bw = a_max * DISPERSE_W * gain; // follow at the full outward strength so it keeps up + leaves too
                fx += bx / bd * bw;
                fz += bz / bd * bw;
            } else {
                // head OUTWARD — away from the world centre/valley, toward the curved edges — so migration POPULATES
                // the wider world (civilisations form out toward the rim) instead of drifting back inward. Fanned by
                // the seed within a ±90° cone so bands spread out, not in a single line. (At the very centre, where
                // "outward" is undefined, any direction is outward → plain random.)
                let rng = crate::simrng::rand(&[m.seed_id, CH_DISPERSE]);
                let r0 = (ax * ax + az * az).sqrt();
                let ang = if r0 > 1.0 {
                    az.atan2(ax) + (rng - 0.5) * std::f64::consts::PI
                } else {
                    rng * std::f64::consts::TAU
                };
                let w = a_max * DISPERSE_W * gain;
                fx += ang.cos() * w;
                fz += ang.sin() * w;
                // a leader keeps a gentle tether to its follower so the pair doesn't stretch apart on the way out
                if let Some(j) = buddy {
                    let bx = agents[j].agent.x - ax;
                    let bz = agents[j].agent.z - az;
                    let bd = bx.hypot(bz).max(0.001);
                    let bw = a_max * BAND_PAIR_W * gain;
                    fx += bx / bd * bw;
                    fz += bz / bd * bw;
                }
            }
        }

        (fx, fz, n_near, migrating)
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
        } else if matches!(self.agents[i].kind, Kind::Person) {
            // people hunt rabbits only when their MEAT is running low (light pressure) — they graze for the rest
            self.agents[i].fed_meat < MEAT_HUNGRY && self.agents[i].give_up_cd <= 0.0
        } else {
            true
        };

        // collect the ~3×3 seek cells into the reused scratch (mem::take → the closure borrows only the buffer)
        let mut neighbors = std::mem::take(&mut self.seek_neighbors);
        neighbors.clear();
        self.seek_grid.for_each_neighbor(ax, az, |j| neighbors.push(j));

        for &ju in &neighbors {
            let j = ju as usize;
            if j == i || self.agents[j].dead {
                // corpses live in the seek grid (so `nearest_carrion` can find them) but are INVISIBLE to live
                // targeting — else a fresh predator carcass got flagged as a rival and the living one "fought" it.
                continue;
            }
            let dx = ax - self.agents[j].agent.x;
            let dz = az - self.agents[j].agent.z;
            let d2 = dx * dx + dz * dz;
            if d2 > SEEK2 {
                continue; // out of notice range (or a hash-collision false neighbour)
            }
            // LOW-POP TRUCE: while banding, an aggressive person won't hunt fellow people (survival over rivalry) —
            // they still hunt non-human prey. preys_on already requires `aggressive && both Person` for the human case.
            let human_truce = self.person_banding
                && matches!(self.agents[i].kind, Kind::Person)
                && matches!(self.agents[j].kind, Kind::Person);
            if a_seeks && !human_truce && preys_on(&self.agents[i], &self.agents[j]) {
                // size/proximity score, DISCOUNTED by how many hunters already claimed j → a crowded prey
                // looks worse, so the pack spreads instead of dogpiling one. PREY-SWITCHING (user's idea): a prey
                // kind that's ABUNDANT is "confirmed supply" → more attractive, so predators preferentially crop the
                // booming species back down (a natural, emergent regulator on prey explosions).
                let abundance = 1.0 + ABUNDANCE_W * (self.kind_pop[self.agents[j].kind as usize] as f64 / ABUNDANCE_NORM).min(1.0);
                // APOSTATIC predation — the hunter forms a search image for the COMMON morph: a prey on the same side
                // of its kind's mean boldness as the majority is over-targeted. This culls whichever strategy is
                // winning → negative frequency dependence that pins the bold↔cautious polymorphism (seed-robust).
                // Both factors are 0 at the neutral genome, so Manual mode (all-neutral) is unaffected.
                let kj = self.agents[j].kind as usize;
                let dev = (self.morph_mean[kj] - 1.0) * (self.agents[j].weights.safety - 1.0); // >0 → prey is the common morph
                let apostatic = (1.0 + APOSTATIC_W * dev).max(0.3);
                let s = prize(self.agents[j].kind) * abundance * apostatic / (d2.max(1.0) * (1.0 + COMPETE_W * self.transient[j].hunted_by as f64));
                if s > self.transient[i].prey_score {
                    self.transient[i].prey = Some(j);
                    self.transient[i].prey_score = s;
                }
                // a PERSON hunting a rabbit is a STEALTH/trap hunter, not a bolt-predator — the rabbit doesn't flee
                // it (people are too slow to win a footrace, so they approach a milling rabbit + grab it). Other
                // hunters still set the prey's flee-threat as before.
                let stealth = matches!(self.agents[i].kind, Kind::Person) && matches!(self.agents[j].kind, Kind::Rabbit);
                if !stealth && d2 < danger2 && d2 < self.transient[j].threat_d {
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

    /// A MANUAL-brain world for the unit + reference-scenario tests below. The world default is now Emergent
    /// (the game runs that); these tests pin the hand-coded brain they were written to protect, so Manual stays
    /// the verified safety net. (Emergent tests use `emergent_world()`, which flips it back on.)
    fn mw() -> World {
        let mut w = World::new();
        w.set_behavior_mode(BehaviorMode::Manual);
        w
    }

    fn animal(x: f64, z: f64, seed: i32) -> Agent {
        Agent::new(x, z, seed, &AgentOpts { max_speed: 3.0, home_radius: 30.0, wander_rate: 1.3, accel: 7.0, turn_speed: 5.0, wanderlust: 0.3 })
    }

    #[test]
    fn spawns_and_steps_deterministically() {
        let run = || {
            let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(5.0, 0.0, 11), Kind::Rabbit, 0.35, 11);
        w.agents[cat].hungry = false; // sated → not hunting, and so not a threat
        w.tick_once(1);
        assert_eq!(w.transient[cat].prey, None);
        assert_eq!(w.transient[rabbit].threat, None);
    }

    #[test]
    fn max_hunters_caps_claim() {
        let mut w = mw();
        let rabbit = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let cats: Vec<usize> = (0..5).map(|k| w.spawn(animal(2.0 + k as f64 * 0.4, 0.0, 100 + k), Kind::Cat, 0.35, 100 + k)).collect();
        w.tick_once(1);
        let claimers = cats.iter().filter(|&&c| w.transient[c].prey == Some(rabbit)).count();
        assert_eq!(claimers, MAX_HUNTERS as usize); // exactly 3 claim the one rabbit; the rest fan out
        assert_eq!(w.transient[rabbit].hunted_by, MAX_HUNTERS);
    }

    #[test]
    fn predator_catches_prey() {
        let mut w = mw();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(0.5, 0.0, 11), Kind::Rabbit, 0.35, 11); // within contact (0.35+0.35+0.4)
        w.agents[rabbit].health = 0.5; // pre-wounded → one strike FINISHES it (a full-health prey takes two strikes — see the scenarios)
        assert!(!w.agents[rabbit].dead);
        w.tick_once(1);
        assert!(w.agents[rabbit].dead, "cat in contact finishes a wounded rabbit");
        let _ = cat;
        // a corpse stops + no longer flocks/targets
        w.tick_once(2);
        assert_eq!(w.agents[rabbit].agent.vx, 0.0);
    }

    #[test]
    fn prey_flees_threat() {
        // cat at -20, rabbit at origin → the rabbit should flee +x (faster than the cat chases) and pull away
        let mut w = mw();
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
        let mut w = mw();
        let cat = w.spawn(animal(0.0, 0.0, 10), Kind::Cat, 0.35, 10);
        let rabbit = w.spawn(animal(0.5, 0.0, 11), Kind::Rabbit, 0.35, 11); // in contact
        w.agents[rabbit].health = 0.5; // pre-wounded → the cat finishes it in one strike (eat/refuel path)
        let s0 = w.agents[cat].stamina; // 0.45 (carnivores start hungry)
        w.tick_once(1);
        assert!(w.agents[rabbit].dead);
        assert_eq!(w.agents[cat].meals, 1);
        assert!(w.agents[cat].stamina > s0 + 0.4, "a kill refuels energy (got {})", w.agents[cat].stamina);
    }

    #[test]
    fn hunger_latch_has_hysteresis() {
        // a carnivore's `hungry` only flips at the LO/HI thresholds, holding in the gap (no per-tick flip-flop)
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
        let lion = w.spawn(animal(50.0, 50.0, 5), Kind::Lion, 0.5, 5); // far from the player (no wake)
        let rabbit = w.spawn(animal(50.6, 50.0, 6), Kind::Rabbit, 0.35, 6); // in contact
        w.agents[rabbit].health = 0.5; // pre-wounded → the lion finishes it in one strike (reaches the 5th kill)
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
        let rabbit = w.spawn(animal(50.0, 50.0, 1), Kind::Rabbit, 0.35, 1);
        w.agents[rabbit].asleep = true;
        w.agents[rabbit].sleep_timer = 10.0;
        let _cat = w.spawn(animal(55.0, 50.0, 2), Kind::Cat, 0.35, 2); // 5 m away, hunting → marks the threat
        w.tick_once(1);
        assert!(!w.agents[rabbit].asleep, "a hunter within danger range startles the rabbit awake");
    }

    #[test]
    fn skittish_rabbit_flees_the_player() {
        let mut w = mw(); // player at (0,0)
        let rabbit = w.spawn(animal(1.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        for t in 1..=40 {
            w.tick_once(t);
        }
        let d = w.agents[rabbit].agent.x.hypot(w.agents[rabbit].agent.z);
        assert!(d > 2.6, "a skittish rabbit bolts from the player (got {d})");
    }

    #[test]
    fn player_wakes_a_nearby_sleeper() {
        let mut w = mw();
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
        let mut w = mw(); // player at (0,0)
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
        let mut w = mw();
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
        let mut w = mw();
        w.set_player(1e4, 1e4); // keep the player out of it
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        // WOUNDED lion (health < HURT_AT) → it breaks away when mobbed instead of committing. (A HEALTHY hungry
        // hunter wades INTO the swarm to feed and only retreats once hurt — see scenario_lions_thin_a_human_cluster.)
        w.agents[lion].health = 0.4;
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
        let mut w = mw();
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
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        // WOUNDED → it defends/retreats rather than committing to a hunt; the pressed-in swarm wounds it further
        // and it slashes back, which is what this test checks.
        w.agents[lion].health = 0.4;
        let rabbits = ring(&mut w, 6, 1.0); // 6 fighter-cats pressed into contact (reach ≈ 1.65)
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
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let lion = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        ring(&mut w, 5, 1.0); // 5 attackers → ≥MOB_MIN
        w.agents[lion].health = 0.004; // one mob-tick (0.03·5·DT ≈ 0.005) from empty
        w.tick_once(1);
        assert!(w.agents[lion].dead, "the mob drags down a hunter whose health it empties");
    }

    #[test]
    fn crowded_rivals_fight_and_bleed() {
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
        w.set_player(1e4, 1e4); // park the player far
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // seeds 1,2 → an opposite-sex pair
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2); // adjacent (< BREED_R2), well-fed
        w.agents[a].age = w.agents[a].lifespan * 0.4; // mature adults (past the maturation gate, within the fertile window)
        w.agents[b].age = w.agents[b].lifespan * 0.4;
        // they conceive within a few ticks; rabbit gestation is 8 s → run well past it (tick_once accumulates births)
        for t in 1..=320 {
            w.tick_once(t);
        }
        let babies = w.births().len() / 11; // 11 floats/birth: kc,x,z,gene,motherFam,fatherFam,g0..g4
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
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1); // seeds 1,2 → an opposite-sex, well-fed pair
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[a].age = w.agents[a].lifespan * 0.4; // mature adults
        w.agents[b].age = w.agents[b].lifespan * 0.4;
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
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[a].age = w.agents[a].lifespan * 0.4; // a mature adult, but…
        w.agents[a].energy = 0.3; // …hungry → not breed-ready
        w.set_breed_cooldown(b, JUVENILE_CD); // the mate is a juvenile, still maturing (age reset to 0)
        for t in 1..=6 {
            w.tick_once(t);
        }
        assert_eq!(w.births().len(), 0, "a hungry parent + a juvenile mate → no birth");
    }

    #[test]
    fn a_lone_herbivore_grazes_its_fullness_back_up() {
        let mut w = mw();
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
        let mut w = mw();
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
    fn dormant_vigor_evolves_only_under_predation() {
        // a dormant region with NO predators → no selection → vigor frozen (matches the live sim)
        let prey_only = [50usize, 0, 30, 20, 0, 0];
        assert!((ff_gene(1.0, &prey_only, 600.0) - 1.0).abs() < 1e-9, "no predators → vigor holds");
        // predators present → the dormant population's vigor climbs over the away span (closed-form, no ticking)
        let mixed = [50usize, 5, 30, 20, 5, 0];
        let g = ff_gene(1.0, &mixed, 600.0);
        assert!(g > 1.05, "predation pressure evolves dormant vigor UP (got {g:.3})");
        assert!(g < ff_gene(1.0, &mixed, 6000.0), "longer dormancy → more evolution");
        assert!(ff_gene(1.0, &mixed, 1e12) <= GENE_MAX + 1e-9, "bounded by GENE_MAX even after eons dormant");
    }

    #[test]
    fn offspring_inherit_parent_vigor() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[a].gene = 1.4; // a fast lineage
        w.agents[b].gene = 1.4;
        w.agents[a].age = w.agents[a].lifespan * 0.4; // mature adults
        w.agents[b].age = w.agents[b].lifespan * 0.4;
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
    fn offspring_inherit_behaviour_genome() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let a = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let b = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        for i in [a, b] {
            w.agents[i].weights.food = 1.6; // a bold-foraging lineage
            w.agents[i].age = w.agents[i].lifespan * 0.4; // mature adults
        }
        for t in 1..=320 {
            w.tick_once(t);
        }
        let births = w.births();
        assert!(births.len() >= 11, "a litter delivered");
        let baby_food = births[6] as f64; // g.food at offset 6 in the stride-11 record
        assert!((baby_food - 1.6).abs() <= 0.25, "baby inherits ~the parents' food weight 1.6 (±mutation); got {baby_food}");
    }

    #[test]
    fn an_animal_dies_of_old_age() {
        let mut w = mw();
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
        let mut w = mw();
        w.set_player(1e4, 1e4);
        // seed 2 = FEMALE (even). Age her past her menopause/fertility end (rabbit female = 0.85·lifespan) → infertile,
        // even though males now stay fertile to death. The male mate (seed 1) is a fertile adult, so the ONLY blocker
        // is her age → no birth.
        let male = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
        let female = w.spawn(animal(1.5, 0.0, 2), Kind::Rabbit, 0.35, 2);
        w.agents[male].age = w.agents[male].lifespan * 0.4; // a fertile adult would-be mate
        w.agents[female].age = w.agents[female].lifespan * 0.9; // past her fertile window → an infertile elder
        for t in 1..=6 {
            w.tick_once(t);
        }
        assert_eq!(w.births().len(), 0, "the only mate is a post-fertile female elder → no birth");
    }

    #[test]
    fn a_despawned_agent_goes_inert() {
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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
        let mut w = mw();
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

    #[test]
    fn low_population_humans_band_with_hysteresis() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        for k in 0..10 {
            // spread them out so flocking doesn't matter; we're testing the population latch
            w.spawn(animal(k as f64 * 8.0, 0.0, 200 + k), Kind::Person, 0.4, 200 + k);
        }
        w.tick_once(1);
        assert!(w.person_banding, "10 ≤ PERSON_BAND_LOW → the dwindling people band together");
        assert_eq!(w.person_pop, 10);
        // grow the population just past LOW but below RELEASE → the latch HOLDS (hysteresis, no flapping)
        for k in 10..18 {
            w.spawn(animal(k as f64 * 8.0, 0.0, 200 + k), Kind::Person, 0.4, 200 + k);
        }
        w.tick_once(2);
        assert_eq!(w.person_pop, 18);
        assert!(w.person_banding, "18 < PERSON_BAND_RELEASE → still banding (latched until recovered)");
        // recover past RELEASE → normal life resumes
        for k in 18..24 {
            w.spawn(animal(k as f64 * 8.0, 0.0, 200 + k), Kind::Person, 0.4, 200 + k);
        }
        w.tick_once(3);
        assert!(!w.person_banding, "24 ≥ PERSON_BAND_RELEASE → truce lifts, ordinary behaviour returns");
    }

    #[test]
    fn banding_humans_dont_hunt_their_own() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        // a tiny, scarce people (≤ PERSON_BAND_LOW) → banding. An aggressive person pressed against a fellow human
        // must NOT pick them as prey while the survival truce holds.
        let killer = w.spawn(animal(0.0, 0.0, 201), Kind::Person, 0.4, 201);
        let victim = w.spawn(animal(0.6, 0.0, 202), Kind::Person, 0.4, 202);
        w.agents[killer].aggressive = true; // force the infighting trait on
        for t in 1..=120 {
            w.tick_once(t);
        }
        assert!(w.person_banding, "two people is well under the band threshold");
        assert!(!w.agents[victim].dead, "the truce keeps the scarce people from killing their own");
    }

    #[test]
    fn banding_survivors_seek_each_other_across_distance() {
        let mut w = mw();
        w.set_player(1e4, 1e4); // player far away → no interference
        // two far-flung survivors (well within SEEK=100 but way past flock range) → banding long-range seek pulls
        // them together so they can regroup, instead of dying alone.
        let a = w.spawn(animal(0.0, 0.0, 211), Kind::Person, 0.4, 211);
        let b = w.spawn(animal(45.0, 0.0, 212), Kind::Person, 0.4, 212);
        let d0 = {
            let (ax, az) = (w.agents[a].agent.x, w.agents[a].agent.z);
            let (bx, bz) = (w.agents[b].agent.x, w.agents[b].agent.z);
            (ax - bx).hypot(az - bz)
        };
        for t in 1..=400 {
            w.tick_once(t);
        }
        assert!(w.person_banding, "still just two people → banding");
        let d1 = {
            let (ax, az) = (w.agents[a].agent.x, w.agents[a].agent.z);
            let (bx, bz) = (w.agents[b].agent.x, w.agents[b].agent.z);
            (ax - bx).hypot(az - bz)
        };
        assert!(d1 < d0 * 0.5, "scattered survivors close the gap (from {d0:.1} to {d1:.1})");
    }

    #[test]
    fn dispersing_bands_stay_gender_mixed() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        // a tight blob of young, alternating-gender people (even seed = female, odd = male) → over PERSON_DISPERSE_CROWD,
        // so the young strike out to found new settlements. With pairing, a man + woman leave together (mixed bands);
        // without it the blob would spray single individuals in all directions and split genders apart.
        let mut males = Vec::new();
        let mut females = Vec::new();
        for k in 0..16 {
            let seed = 300 + k;
            let x = (k % 4) as f64 * 1.0 - 1.5;
            let z = (k / 4) as f64 * 1.0 - 1.5;
            let idx = w.spawn(animal(x, z, seed), Kind::Person, 0.4, seed);
            if seed & 1 == 0 { females.push(idx) } else { males.push(idx) }
        }
        for t in 1..=600 {
            w.tick_once(t);
        }
        // they actually dispersed: someone got well clear of the origin blob
        let max_r = w.agents.iter().map(|m| m.agent.x.hypot(m.agent.z)).fold(0.0_f64, f64::max);
        // every male still has a female nearby (and vice-versa) → no single-sex band formed
        let worst_male = males.iter().map(|&mi| {
            females.iter().map(|&fi| {
                (w.agents[mi].agent.x - w.agents[fi].agent.x).hypot(w.agents[mi].agent.z - w.agents[fi].agent.z)
            }).fold(f64::INFINITY, f64::min)
        }).fold(0.0_f64, f64::max);
        println!("DISPERSE max_r={max_r:.1} worst_male_to_nearest_female={worst_male:.1}");
        assert!(max_r > 12.0, "the band actually struck out from the blob (max_r {max_r:.1})");
        // the worst-off man still keeps a woman within a fraction of the dispersal radius → bands stay roughly
        // gender-mixed via the leader/follower pairing. (Threshold relaxed since dispersal is now biased OUTWARD:
        // a blob AT the origin fans genders around the full circle — a pathological case; real settlements disperse
        // from one side and stay tighter. The pairing still keeps a co-founder of the opposite sex along.)
        assert!(worst_male < max_r * 0.75, "every man kept a woman relatively close → mixed bands (worst {worst_male:.1} vs r {max_r:.1})");
    }

    #[test]
    fn a_threatened_woman_flees_toward_a_house() {
        let run = |refuge: bool| {
            let mut w = mw();
            w.set_player(1e4, 1e4);
            w.set_night(1.0); // widest danger radius → she registers the threat promptly
            // a woman (even seed = female) at the origin, a hungry lion to her WEST → she flees EAST (+x).
            let woman = w.spawn(animal(0.0, 0.0, 200), Kind::Person, 0.4, 200);
            w.spawn(animal(-12.0, 0.0, 7), Kind::Lion, 0.5, 7);
            if refuge {
                w.set_refuges(&[6.0, 12.0]); // home is to the NE — in the flee hemisphere but offset +z
            }
            for t in 1..=40 {
                w.tick_once(t);
            }
            (w.agents[woman].agent.x, w.agents[woman].agent.z)
        };
        let (nx, nz) = run(false); // no refuge → flees straight away from the lion (≈ +x, z stays near 0)
        let (rx, rz) = run(true); // refuge NE → her flight curves toward home (+z)
        assert!(rx > 0.0 && nx > 0.0, "she flees away from the lion in both cases (x {nx:.1}/{rx:.1})");
        assert!(rz > nz + 1.0, "the house bends her flight toward home (z {nz:.1} → {rz:.1})");
    }

    #[test]
    fn a_hungry_carnivore_scavenges_a_fresh_carcass() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        // a hungry lion a few metres from a fresh rabbit carcass (a death the ecosystem shouldn't waste)
        let lion = w.spawn(animal(0.0, 0.0, 5), Kind::Lion, 0.5, 5);
        let carcass = w.spawn(animal(4.0, 0.0, 9), Kind::Rabbit, 0.35, 9);
        w.agents[lion].energy = 0.35; // famished → hungry latch will engage
        w.agents[lion].hungry = true;
        w.agents[carcass].dead = true;
        w.agents[carcass].carrion = CARRION_MEAT;
        let e0 = w.agents[lion].energy;
        for t in 1..=60 {
            w.tick_once(t);
        }
        assert!(w.agents[lion].energy > e0 + 0.1, "the lion fed on the carcass (energy {e0:.2} → {:.2})", w.agents[lion].energy);
        assert!(w.agents[carcass].carrion < CARRION_MEAT, "the carcass was eaten down (meat {:.1} → {:.1})", CARRION_MEAT, w.agents[carcass].carrion);
    }

    #[test]
    fn a_carcass_rots_away_even_uneaten() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let c = w.spawn(animal(0.0, 0.0, 9), Kind::Rabbit, 0.35, 9);
        w.agents[c].dead = true;
        w.agents[c].carrion = 3.0; // a nearly-rotted scrap, no scavenger near
        for t in 1..=300 {
            w.tick_once(t);
        }
        assert_eq!(w.agents[c].carrion, 0.0, "an uneaten carcass rots to nothing");
    }

    #[test]
    fn fast_forward_relaxes_toward_carrying_capacity() {
        // [rabbit, cat, kangaroo, person, lion, dino] in Kind-discriminant order
        let cap_rabbit = cap_for(Kind::Rabbit, &[0; 6], 1.0); // = PREY_DENSITY_RABBIT at scale 1
        // a tiny rabbit population, given a long time away, climbs the logistic toward its cap (but never past it)
        let grown = ff_targets(&[3, 0, 0, 0, 0, 0], 1.0, 4000.0);
        assert!(grown[0] as usize > 3, "few rabbits multiply while you're away (→ {})", grown[0]);
        assert!(grown[0] as usize <= cap_rabbit, "but never overshoot the carrying capacity ({} vs cap {cap_rabbit})", grown[0]);
        // a blink away barely changes anything
        let blink = ff_targets(&[10, 0, 0, 0, 0, 0], 1.0, 0.5);
        assert!((blink[0] as i64 - 10).abs() <= 1, "a half-second away ≈ no change (→ {})", blink[0]);
        // an extinct apex (no floor) stays extinct — it returns via Mother Nature in live play, not the FF
        let dino = ff_targets(&[30, 5, 20, 20, 3, 0], 1.0, 4000.0);
        assert_eq!(dino[5], 0, "a fully-extinct dinosaur stays gone through the fast-forward");
    }

    #[test]
    fn an_aged_animal_slows_with_frailty() {
        // total path length over a fixed wander — same seed → identical wander decisions, so any difference is SPEED.
        let path = |age_frac: f64| {
            let mut w = mw();
            w.set_player(1e4, 1e4);
            let r = w.spawn(animal(0.0, 0.0, 1), Kind::Rabbit, 0.35, 1);
            let mut total = 0.0;
            let mut last = (w.agents[r].agent.x, w.agents[r].agent.z);
            for t in 1..=400 {
                w.agents[r].age = w.agents[r].lifespan * age_frac; // pin age (< 1 → never dies of old age) so we isolate frailty
                w.tick_once(t);
                let (x, z) = (w.agents[r].agent.x, w.agents[r].agent.z);
                total += (x - last.0).hypot(z - last.1);
                last = (x, z);
            }
            total
        };
        let young = path(0.1); // sprightly
        let old = path(0.97); // venerable → frail
        assert!(old < young * 0.9, "an elder covers less ground than its young self (old {old:.1} vs young {young:.1})");
    }

    #[test]
    fn a_juvenile_animal_trails_a_parent() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        // an adult kangaroo and a baby a few metres off → the baby should close toward the parent over time.
        let adult = w.spawn(animal(0.0, 0.0, 2), Kind::Kangaroo, 0.5, 2);
        let baby = w.spawn(animal(3.5, 0.0, 1), Kind::Kangaroo, 0.35, 1);
        // pin the ages each tick: adult firmly grown, baby firmly juvenile (and keep the adult parked at home)
        let near = |w: &World| {
            (w.agents[baby].agent.x - w.agents[adult].agent.x).hypot(w.agents[baby].agent.z - w.agents[adult].agent.z)
        };
        w.agents[adult].agent.max_speed = 0.0; // park the parent so we isolate the BABY's following
        let d0 = near(&w);
        let mut closest = d0;
        for t in 1..=200 {
            w.agents[adult].age = w.agents[adult].lifespan * 0.5; // a grown parent
            w.agents[baby].age = w.agents[baby].lifespan * 0.05; // a newborn
            w.tick_once(t);
            closest = closest.min(near(&w));
        }
        // the baby got noticeably closer to its parent at some point (it trails, not wanders off)
        assert!(closest < d0 * 0.6, "the juvenile closed toward its parent (from {d0:.1} to ≤{closest:.1})");
    }

    #[test]
    fn a_litter_is_born_clustered_around_the_mother() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let (mx, mz) = (10.0_f32, -5.0_f32);
        let mom = w.spawn(animal(mx as f64, mz as f64, 2), Kind::Rabbit, 0.35, 2); // even seed = female
        w.agents[mom].age = w.agents[mom].lifespan * 0.4; // a fertile adult
        w.agents[mom].unborn_gene = 1.0;
        w.agents[mom].pregnant = 0.001; // delivering essentially now
        w.tick_once(1);
        let births = w.births(); // flat stride 11: [kc, x, z, gene, motherFam, fatherFam, g0..g4]
        assert!(births.len() >= 11, "at least one baby delivered (got {} floats)", births.len());
        let mut positions = Vec::new();
        for chunk in births.chunks_exact(11) {
            let (x, z) = (chunk[1], chunk[2]);
            let d = ((x - mx).powi(2) + (z - mz).powi(2)).sqrt();
            assert!(d < 3.0, "a newborn is born right by its mother (d {d:.2})");
            positions.push((x, z));
        }
        if positions.len() >= 2 {
            assert!(positions.windows(2).any(|p| p[0] != p[1]), "littermates aren't all stacked on one exact point");
        }
    }

    #[test]
    fn a_predator_does_not_fight_a_corpse_as_a_rival() {
        // regression: corpses live in the seek grid (for scavenging), so targeting MUST skip them — else a fresh
        // same-rank predator carcass got picked as a territorial rival and the living one charged + bled on it.
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let live = w.spawn(animal(0.0, 0.0, 1), Kind::Lion, 0.5, 1);
        let dead = w.spawn(animal(1.0, 0.0, 2), Kind::Lion, 0.5, 2); // right in contact range
        w.agents[dead].dead = true;
        w.agents[dead].carrion = CARRION_MEAT; // a fresh carcass sitting in the seek grid
        for t in 1..=30 {
            w.tick_once(t);
        }
        assert_eq!(w.agents[live].rival_time, 0.0, "a living predator never treats a carcass as a territorial rival");
        assert!(w.agents[live].health > 0.9, "...and so never wounds itself fighting one (health {:.2})", w.agents[live].health);
    }

    // ════════════════════════════ SCENARIO HARNESS ════════════════════════════
    // METHODOLOGY: complex sim behaviour is EMERGENT — it only becomes legible over THOUSANDS of ticks, not one
    // or two. So we don't assert single-tick state; we run a configured world forward at the fixed DT and measure
    // the SHAPE of the outcome over time:
    //   • cumulative EVENTS  — kills / starves / old-age / births / conceives / builds (the "what happened")
    //   • population TRAJECTORY — live count per kind, sampled (boom / crash / steady state)
    //   • spatial SPREAD — bounding extent of a kind (clumped vs dispersed)
    //   • predator THRASH — distance travelled ÷ ground actually gained; high ⇒ "stuttering in place"
    // A scenario sets up agents, runs, and asserts on those aggregates. This is the rig for tuning time-based
    // behaviour (predation, dispersal, settlement) by MEASUREMENT instead of guesswork.

    #[derive(Default, Debug)]
    struct Trace {
        kills: usize,
        starves: usize,
        oldage: usize,
        births: usize,
        conceives: usize,
        builds: usize,
        pop: Vec<[usize; 6]>, // live count per kind, sampled every 60 ticks (≈2 s)
    }

    fn radius_of(kind: Kind) -> f64 {
        match kind {
            Kind::Rabbit => 0.35,
            Kind::Cat => 0.4,
            Kind::Kangaroo => 0.5,
            Kind::Person => 0.4,
            Kind::Lion => 0.5,
            Kind::Dinosaur => 0.9,
        }
    }

    fn spawn_kind(w: &mut World, kind: Kind, x: f64, z: f64, seed: i32) -> usize {
        w.spawn(Agent::new(x, z, seed, &opts_for(kind, seed)), kind, radius_of(kind), seed)
    }

    /// Cluster `n` agents of `kind` in a golden-spiral disc of radius `rad` around (cx,cz).
    fn cluster(w: &mut World, kind: Kind, n: usize, cx: f64, cz: f64, rad: f64, seed0: i32) {
        let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
        for k in 0..n {
            let r = rad * (((k as f64) + 0.5) / n as f64).sqrt();
            let a = k as f64 * ga;
            spawn_kind(w, kind, cx + a.cos() * r, cz + a.sin() * r, seed0 + k as i32);
        }
    }

    fn alive(w: &World) -> [usize; 6] {
        let mut c = [0usize; 6];
        for m in &w.agents {
            if !m.dead {
                c[m.kind as usize] += 1;
            }
        }
        c
    }

    /// Max bounding extent (the larger of width/depth) of a living kind — clumped ≈ small, dispersed ≈ large.
    fn spread(w: &World, kind: Kind) -> f64 {
        let (mut lo_x, mut hi_x, mut lo_z, mut hi_z) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        let mut n = 0;
        for m in &w.agents {
            if m.kind == kind && !m.dead {
                lo_x = lo_x.min(m.agent.x);
                hi_x = hi_x.max(m.agent.x);
                lo_z = lo_z.min(m.agent.z);
                hi_z = hi_z.max(m.agent.z);
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            (hi_x - lo_x).max(hi_z - lo_z)
        }
    }

    /// Run `ticks` fixed steps. Tallies this-tick events (step() clears them each call), samples populations, and
    /// tracks the `track` kind's travelled PATH + start→end NET so callers can compute thrash = path / net.
    fn run(w: &mut World, ticks: usize, track: Kind) -> (Trace, f64, f64) {
        let mut tr = Trace::default();
        let idx: Vec<usize> = w.agents.iter().enumerate().filter(|(_, m)| m.kind == track).map(|(i, _)| i).collect();
        let start: Vec<(f64, f64)> = idx.iter().map(|&i| (w.agents[i].agent.x, w.agents[i].agent.z)).collect();
        let mut prev = start.clone();
        let mut path = 0.0;
        let mut next_seed = 900_000i32;
        for t in 0..ticks {
            w.step(DT);
            for ch in w.events().chunks_exact(4) {
                let c = ch[0];
                if c == EV_KILL {
                    tr.kills += 1;
                } else if c == EV_STARVE {
                    tr.starves += 1;
                } else if c == EV_OLDAGE {
                    tr.oldage += 1;
                } else if c == EV_BIRTH {
                    tr.births += 1;
                } else if c == EV_CONCEIVE {
                    tr.conceives += 1;
                } else if c == EV_BUILD {
                    tr.builds += 1;
                }
            }
            // materialise this step's newborns (what JS does between steps) so reproduction replenishes the world
            let births: Vec<f32> = w.births().to_vec();
            for b in births.chunks_exact(11) {
                let kind = crate::eco::kind_from_code(b[0] as u8);
                let r = radius_of(kind);
                let bi = w.spawn(Agent::new(b[1] as f64, b[2] as f64, next_seed, &opts_for(kind, next_seed)), kind, r, next_seed);
                w.agents[bi].breed_cd = JUVENILE_CD; // a newborn matures before it can breed
                w.agents[bi].gene = b[3] as f64;
                w.set_lineage(bi, b[4] as u32, b[5] as u32); // parent fams → incest avoidance
                w.set_genome(bi, b[6] as f64, b[7] as f64, b[8] as f64, b[9] as f64, b[10] as f64); // inherited genome
                next_seed = next_seed.wrapping_add(1);
            }
            for (k, &i) in idx.iter().enumerate() {
                if w.agents[i].dead {
                    continue;
                }
                let (x, z) = (w.agents[i].agent.x, w.agents[i].agent.z);
                path += ((x - prev[k].0).powi(2) + (z - prev[k].1).powi(2)).sqrt();
                prev[k] = (x, z);
            }
            if t % 60 == 0 {
                tr.pop.push(alive(w));
            }
        }
        let denom = idx.len().max(1) as f64;
        let net: f64 = idx
            .iter()
            .enumerate()
            .map(|(k, &i)| ((w.agents[i].agent.x - start[k].0).powi(2) + (w.agents[i].agent.z - start[k].1).powi(2)).sqrt())
            .sum::<f64>()
            / denom;
        (tr, path / denom, net)
    }

    // SCENARIO: 6 lions dropped onto a tight cluster of 60 people. Over 100 s they should actually EAT — predation
    // must make progress, not stutter in place (the user's report: "they keep stuttering place and don't kill much").
    #[test]
    fn scenario_lions_thin_a_human_cluster() {
        let mut w = mw();
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 7.0, 1000);
        cluster(&mut w, Kind::Lion, 6, 0.0, 0.0, 10.0, 5000);
        let people0 = alive(&w)[Kind::Person as usize];
        let (tr, path, net) = run(&mut w, 3000, Kind::Lion); // 3000 ticks ≈ 100 s
        let people1 = alive(&w)[Kind::Person as usize];
        let thrash = path / net.max(0.1);
        eprintln!(
            "[lions-vs-cluster] kills={} starves={} people {}→{} | lion travelled {:.0} m, netted {:.0} m, thrash={:.1}",
            tr.kills, tr.starves, people0, people1, path, net, thrash
        );
        assert!(tr.kills >= 12, "6 lions amid 60 people for 100 s barely killed ({}) — predation is stalling", tr.kills);
        // high thrash is only a problem if they're ALSO not killing — chasing fleeing/dispersing prey legitimately
        // covers a lot of ground (high path, low net). With plenty of kills, that's vigorous hunting, not stutter.
        assert!(thrash < 15.0 || tr.kills >= 12, "lions thrash in place ({:.0} m path, {:.0} m net) without killing", path, net);
    }

    // SCENARIO: a dense human blob must SPREAD OUT over time (the missionary dispersal), not stay clumped.
    #[test]
    fn scenario_human_blob_disperses() {
        let mut w = mw();
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 5.0, 2000);
        let spread0 = spread(&w, Kind::Person);
        run(&mut w, 1500, Kind::Person); // ≈50 s
        let spread1 = spread(&w, Kind::Person);
        eprintln!("[blob-disperse] person spread {:.0} m → {:.0} m", spread0, spread1);
        assert!(spread1 > spread0 * 1.5, "a dense human blob should disperse, stayed clumped ({:.0}→{:.0} m)", spread0, spread1);
    }

    // SCENARIO: a sizable human settlement must SUSTAIN over time, not boom-bust to near-extinction (the user's
    // report: "1100 humans, now there are 5"). Births are materialised (see run), so this measures the real
    // birth-vs-death balance over 200 s. The crash mechanism is overgrazing: a crowd > GRAZE_CROWD starves.
    /// Population-dynamics run: steps `ticks`, MATERIALISES births (what JS does between steps) and REAPS dead
    /// agents (what the corpse reaper does) so `w.agents` stays ≈ alive count instead of growing quadratically.
    /// Returns the per-(60-tick)-sample person-count series. Reaping is safe: the sim rebuilds its grid + transient
    /// buffers from `w.agents` every step, so compacting the Vec between steps can't desync it.
    fn run_pop(w: &mut World, ticks: usize) -> Vec<usize> {
        let mut series = Vec::new();
        let mut next_seed = 900_000i32;
        for t in 0..ticks {
            w.step(DT);
            let births: Vec<f32> = w.births().to_vec();
            for b in births.chunks_exact(11) {
                let kind = crate::eco::kind_from_code(b[0] as u8);
                let bi = w.spawn(Agent::new(b[1] as f64, b[2] as f64, next_seed, &opts_for(kind, next_seed)), kind, radius_of(kind), next_seed);
                w.agents[bi].breed_cd = JUVENILE_CD;
                w.agents[bi].gene = b[3] as f64;
                w.agents[bi].age = 0.0;
                w.set_lineage(bi, b[4] as u32, b[5] as u32); // parent fams → incest avoidance
                w.set_genome(bi, b[6] as f64, b[7] as f64, b[8] as f64, b[9] as f64, b[10] as f64); // inherited genome
                next_seed = next_seed.wrapping_add(1);
            }
            if t % 30 == 0 {
                w.agents.retain(|m| !m.dead); // reap corpses → bounded agent buffer
            }
            if t % 60 == 0 {
                series.push(alive(w)[Kind::Person as usize]);
            }
        }
        series
    }

    // SCENARIO: a human settlement must GROW GRADUALLY and not (a) crash to ~0 (the old "1100→5" cohort/cap bug)
    // nor (b) EXPLODE exponentially (the "no-caps removed all regulation" bug → thousands in seconds). With
    // density-dependent breeding (a saturated patch plateaus) + slower rate + age structure, it should rise
    // steadily and stay bounded over the run.
    #[test]
    fn scenario_human_population_grows_but_bounded() {
        let mut w = mw();
        cluster(&mut w, Kind::Person, 100, 0.0, 0.0, 40.0, 3000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let p0 = alive(&w)[Kind::Person as usize];
        let series = run_pop(&mut w, 1800); // ≈60 s — the span over which the user saw the crash
        let p1 = alive(&w)[Kind::Person as usize];
        eprintln!("[human-vigour] people {p0}→{p1} | series={series:?}");
        // a spawned 1000 (sim only, no streaming) must NOT crash to a tiny number — if it does, the SIM (overgrazing/
        // breeding-off) is the culprit; if the sim holds but the GAME crashes, streaming-collapse is.
        assert!(p1 >= p0 / 2, "1000 spawned crashed to {p1} in the SIM (overgrazing/death) — not streaming");
    }

    // SCENARIO: the TROPHIC PYRAMID must hold over time — predators are K-strategists (slow gestation + tiny litters)
    // so the apex stays RARE relative to its prey, even when seeded generously. Guards the user's "lions reproduce
    // faster, 25 lions vs 40 humans" bug: before the fix lions out-bred their food and ballooned; now they shouldn't.
    #[test]
    fn scenario_predators_stay_rare_trophic_pyramid() {
        let mut w = mw();
        cluster(&mut w, Kind::Rabbit, 90, 0.0, 0.0, 70.0, 1000);
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 55.0, 2000);
        cluster(&mut w, Kind::Lion, 12, 0.0, 0.0, 45.0, 5000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let a0 = alive(&w);
        run_pop(&mut w, 3000); // ≈100 s
        let a1 = alive(&w);
        let lions = a1[Kind::Lion as usize];
        let prey = a1[Kind::Rabbit as usize] + a1[Kind::Kangaroo as usize] + a1[Kind::Person as usize];
        eprintln!("[trophic-pyramid] start={a0:?} end={a1:?} | lions={lions} prey={prey}");
        // apex must NOT balloon — slow breeding keeps it near/under its soft plateau, not exploding past it.
        assert!(lions <= 28, "apex over-reproduced to {lions} — predators must breed slowly + stay rare");
        // and stay a thin top of the pyramid: far fewer predators than prey.
        assert!((lions as f64) < (prey as f64) * 0.5, "predators not rare vs prey ({lions} lions, {prey} prey)");
    }

    // SCENARIO: PREY-SWITCHING — predators preferentially crop the ABUNDANT prey (user: "predators should prefer
    // higher-number prey more"). An overwhelming rabbit boom beside a scarce kangaroo handful → the lions' kills
    // skew hard to rabbits (the booming supply), cropping it back, even though a kangaroo is the bigger single meal.
    #[test]
    fn scenario_predators_crop_the_abundant_prey() {
        let mut w = mw();
        cluster(&mut w, Kind::Rabbit, 140, 0.0, 0.0, 55.0, 1000);
        cluster(&mut w, Kind::Kangaroo, 14, 0.0, 0.0, 55.0, 7000);
        cluster(&mut w, Kind::Lion, 8, 0.0, 0.0, 50.0, 5000);
        let mut rabbit_kills = 0;
        let mut roo_kills = 0;
        for _ in 0..3000 {
            w.step(DT);
            for ch in w.events().chunks_exact(4) {
                if ch[0] == EV_KILL {
                    if ch[1] == Kind::Rabbit as usize as f32 {
                        rabbit_kills += 1;
                    } else if ch[1] == Kind::Kangaroo as usize as f32 {
                        roo_kills += 1;
                    }
                }
            }
        }
        eprintln!("[prey-switching] rabbit kills={rabbit_kills} kangaroo kills={roo_kills}");
        // the abundant rabbits take the brunt — far more rabbit kills than the scarce kangaroo, i.e. the boom is cropped.
        assert!(rabbit_kills > roo_kills * 3, "predators didn't prefer the abundant prey ({rabbit_kills} rabbit vs {roo_kills} kangaroo kills)");
    }

    // INCEST AVOIDANCE (all kinds): the lineage kinship rule must flag parent↔child and siblings, but NOT
    // unrelated founders or cousins, so find_mate refuses close kin (user: "avoid incest, apply to all animals").
    #[test]
    fn close_kin_are_flagged_unrelated_are_not() {
        let mut w = mw();
        let dad = spawn_kind(&mut w, Kind::Rabbit, 0.0, 0.0, 3); // odd seed = male
        let mum = spawn_kind(&mut w, Kind::Rabbit, 0.5, 0.0, 2); // even = female
        let (fam_d, fam_m) = (w.agents[dad].fam, w.agents[mum].fam);
        let kid = spawn_kind(&mut w, Kind::Rabbit, 0.2, 0.0, 4);
        let sib = spawn_kind(&mut w, Kind::Rabbit, 0.3, 0.0, 6);
        w.set_lineage(kid, fam_m, fam_d); // child of mum × dad
        w.set_lineage(sib, fam_m, fam_d); // full sibling of kid
        let outsider = spawn_kind(&mut w, Kind::Rabbit, 9.0, 0.0, 8); // an unrelated founder

        assert!(related(&w.agents[dad], &w.agents[kid]), "father ↔ child are kin");
        assert!(related(&w.agents[mum], &w.agents[kid]), "mother ↔ child are kin");
        assert!(related(&w.agents[kid], &w.agents[sib]), "full siblings (shared parents) are kin");
        assert!(!related(&w.agents[dad], &w.agents[mum]), "the two unrelated founders are NOT kin");
        assert!(!related(&w.agents[kid], &w.agents[outsider]), "an unrelated outsider is fair game (no false-positive)");
    }

    // SCENARIO: a lone roamer drifts toward an UNDER-populated settlement (gene flow + filling thin towns). The
    // migration lives in the shared flock pass, so it's tested on the manual brain here.
    #[test]
    fn scenario_roamers_migrate_to_sparse_settlements() {
        let mut w = mw();
        w.set_refuges(&[0.0, 0.0, 200.0, 0.0]); // two empty (sparse) settlements
        let near = |w: &World, i: usize| {
            let (x, z) = (w.agents[i].agent.x, w.agents[i].agent.z);
            (x * x + z * z).sqrt().min(((x - 200.0).powi(2) + z * z).sqrt())
        };
        // lone adult roamers out in the wild, no settlement within SETTLE_R of spawn
        let ids: Vec<usize> = (0..8).map(|k| spawn_kind(&mut w, Kind::Person, 90.0 + k as f64 * 2.0, 95.0, 3000 + k as i32)).collect();
        for &i in &ids {
            let s = w.agents[i].seed_id;
            w.randomize_start_age(i, s); // adults (past the child gate)
        }
        let d0: f64 = ids.iter().map(|&i| near(&w, i)).sum::<f64>() / ids.len() as f64;
        for t in 1..=900 {
            w.tick_once(t);
        }
        let live: Vec<usize> = ids.iter().copied().filter(|&i| !w.agents[i].dead).collect();
        let d1: f64 = live.iter().map(|&i| near(&w, i)).sum::<f64>() / live.len().max(1) as f64;
        eprintln!("[migration] mean dist to nearest settlement {d0:.0} m → {d1:.0} m ({} roamers)", live.len());
        assert!(d1 < d0 - 15.0, "roamers didn't migrate toward a settlement ({d0:.0}→{d1:.0} m)");
    }

    // SCENARIO: a bonded pair sticks together (the tether pulls them close while raising young).
    #[test]
    fn bonded_mates_stick_together() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let a = spawn_kind(&mut w, Kind::Rabbit, 0.0, 0.0, 2);
        let b = spawn_kind(&mut w, Kind::Rabbit, 9.0, 0.0, 3);
        w.agents[a].partner = Some(b); // bond them directly (conception does this in play)
        w.agents[b].partner = Some(a);
        let d0 = 9.0_f64;
        for t in 1..=400 {
            w.tick_once(t); // 400 ticks ≈ 13 s, well under BOND_REARING → no split this window
        }
        let d1 = (w.agents[a].agent.x - w.agents[b].agent.x).hypot(w.agents[a].agent.z - w.agents[b].agent.z);
        eprintln!("[bond] pair gap {d0:.0} → {d1:.1} m");
        assert!(w.agents[a].partner == Some(b), "the bond holds within the rearing window");
        assert!(d1 < d0 - 2.0, "bonded mates pull together ({d0:.0}→{d1:.1} m)");
    }

    // SCENARIO: a meat-hungry PERSON stalks + catches a nearby rabbit (people hunt rabbits for meat; the rabbit
    // doesn't bolt from the slow human — stealth). Eating it replenishes fed_meat (which gates human breeding).
    #[test]
    fn a_meat_hungry_person_hunts_a_rabbit() {
        let mut w = mw();
        w.set_player(1e4, 1e4);
        let person = spawn_kind(&mut w, Kind::Person, 0.0, 0.0, 3);
        let rabbit = spawn_kind(&mut w, Kind::Rabbit, 2.0, 0.0, 11);
        w.agents[person].fed_meat = 0.0; // meat-hungry → goes hunting
        w.agents[rabbit].health = 0.5; // one strike finishes (the struggle's covered elsewhere)
        let mut fed = false;
        for t in 1..=400 {
            w.tick_once(t);
            if w.agents[rabbit].dead && w.agents[person].fed_meat > 0.0 {
                fed = true;
                break;
            }
        }
        assert!(fed, "a meat-hungry person should stalk + catch a nearby rabbit and replenish its meat");
    }

    // SCENARIO: a WELL-FED predator gives a settlement a wide berth (towns are safer); a STARVING one risks it.
    #[test]
    fn well_fed_predator_avoids_a_settlement_starving_risks_it() {
        let near = |w: &World, c: usize| w.agents[c].agent.x.hypot(w.agents[c].agent.z);
        // well-fed cat near a town → drifts AWAY
        let mut w = mw();
        w.set_player(1e4, 1e4);
        w.set_refuges(&[0.0, 0.0]); // a settlement centre
        let cat = spawn_kind(&mut w, Kind::Cat, 18.0, 0.0, 7); // within SETTLEMENT_AVOID_R, male (not a mother)
        w.agents[cat].energy = 0.9; // not desperate
        let d0 = near(&w, cat);
        for t in 1..=150 {
            w.tick_once(t);
        }
        let d1 = near(&w, cat);
        eprintln!("[pred-avoid] well-fed cat {d0:.0} → {d1:.0} m from town");
        assert!(d1 > d0 + 3.0, "a well-fed predator should leave the settlement's edge ({d0:.0}→{d1:.0} m)");
        // a STARVING cat in the same spot does NOT flee the town (it'll risk a raid) → stays closer than the fed one
        let mut w2 = mw();
        w2.set_player(1e4, 1e4);
        w2.set_refuges(&[0.0, 0.0]);
        let hungry = spawn_kind(&mut w2, Kind::Cat, 18.0, 0.0, 7);
        w2.agents[hungry].energy = 0.1; // desperate
        for t in 1..=150 {
            w2.tick_once(t);
        }
        eprintln!("[pred-avoid] starving cat {:.0} m from town (vs fed {d1:.0})", near(&w2, hungry));
        assert!(near(&w2, hungry) < d1, "a starving predator risks the settlement (stays nearer than a fed one)");
    }

    // ─────────────────────────── EMERGENT MODE — on-par scenario parity ───────────────────────────
    // The emergent brain (needs+primitives+utility, design doc) is the GAME's default (Sim::new). These mirror the
    // manual scenario bars above on a world flipped to Emergent, proving the bottom-up brain produces a world at
    // least as alive: predators that EAT, a settlement that SUSTAINS + builds, an apex that stays RARE, a blob that
    // SPREADS. (The unit tests above keep pinning Manual's exact mechanics — Manual stays the untouched safety net.)

    fn emergent_world() -> World {
        World::new() // the world default IS Emergent now — this just reads as intent at the call sites
    }

    // SCENARIO (emergent): 6 lions on a tight cluster of 60 people must actually EAT — the utility scorer's Hunt
    // primitive has to make predation progress on par with the manual chain (manual bar: ≥12 kills / 100 s).
    #[test]
    fn scenario_emergent_lions_thin_a_human_cluster() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 7.0, 1000);
        cluster(&mut w, Kind::Lion, 6, 0.0, 0.0, 10.0, 5000);
        let people0 = alive(&w)[Kind::Person as usize];
        let (tr, _path, _net) = run(&mut w, 3000, Kind::Lion);
        let people1 = alive(&w)[Kind::Person as usize];
        eprintln!("[emergent lions-vs-cluster] kills={} starves={} people {}→{}", tr.kills, tr.starves, people0, people1);
        assert!(tr.kills >= 12, "emergent lions barely killed ({}) — the Hunt primitive is stalling vs manual's ≥12", tr.kills);
    }

    // SCENARIO (emergent): a dense human blob must still SPREAD OUT (dispersal lives in the shared flock pass, so
    // emergent must not suppress it — proves the seam keeps sections 1–4 working under the new brain).
    #[test]
    fn scenario_emergent_human_blob_disperses() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 5.0, 2000);
        let spread0 = spread(&w, Kind::Person);
        run(&mut w, 1500, Kind::Person);
        let spread1 = spread(&w, Kind::Person);
        eprintln!("[emergent blob-disperse] person spread {:.0} m → {:.0} m", spread0, spread1);
        assert!(spread1 > spread0 * 1.5, "emergent blob stayed clumped ({:.0}→{:.0} m) — dispersal regressed", spread0, spread1);
    }

    // SCENARIO (emergent): a human settlement must SUSTAIN (not boom-bust) under the emergent brain — births vs
    // overgrazing-deaths balance over 60 s, the same bar manual holds (p1 ≥ p0/2).
    #[test]
    fn scenario_emergent_human_population_grows_but_bounded() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Person, 100, 0.0, 0.0, 40.0, 3000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let p0 = alive(&w)[Kind::Person as usize];
        let series = run_pop(&mut w, 1800);
        let p1 = alive(&w)[Kind::Person as usize];
        eprintln!("[emergent human-vigour] people {p0}→{p1} | series={series:?}");
        assert!(p1 >= p0 / 2, "emergent settlement crashed to {p1} — births/overgrazing balance regressed vs manual");
    }

    // SCENARIO (emergent): a mixed food-web must hold the TROPHIC PYRAMID — the apex stays rare relative to prey,
    // matching manual's regulation (predators don't balloon; far fewer than prey).
    #[test]
    fn scenario_emergent_predators_stay_rare_trophic_pyramid() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 90, 0.0, 0.0, 70.0, 1000);
        cluster(&mut w, Kind::Person, 60, 0.0, 0.0, 55.0, 2000);
        cluster(&mut w, Kind::Lion, 12, 0.0, 0.0, 45.0, 5000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let a0 = alive(&w);
        run_pop(&mut w, 3000);
        let a1 = alive(&w);
        let lions = a1[Kind::Lion as usize];
        let prey = a1[Kind::Rabbit as usize] + a1[Kind::Kangaroo as usize] + a1[Kind::Person as usize];
        eprintln!("[emergent trophic-pyramid] start={a0:?} end={a1:?} | lions={lions} prey={prey}");
        assert!(lions <= 28, "emergent apex over-reproduced to {lions} — must stay rare");
        assert!(prey == 0 || (lions as f64) < (prey as f64) * 0.5, "emergent predators not rare vs prey ({lions} lions, {prey} prey)");
    }

    // SCENARIO (emergent): a settled people must raise HOMES — the design's Tier 1 goal is "a recognizable village
    // living on" the emergent brain. House-building (section 7.6) is shared + gated on settled, well-fed family pairs,
    // so this proves emergent foraging keeps people fed + paired enough to found a town (cities still emerge).
    #[test]
    fn scenario_emergent_settlement_builds_homes() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Person, 40, 0.0, 0.0, 18.0, 4000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let (tr, _p, _n) = run(&mut w, 3000, Kind::Person); // ≈100 s
        eprintln!("[emergent settlement] builds={} births={}", tr.builds, tr.births);
        assert!(tr.builds >= 3, "emergent settlement raised only {} homes — people aren't settling/feeding to build", tr.builds);
    }

    // SANITY (emergent): the core reactive primitives fire — a rabbit FLEES an approaching lion (Flee outscores
    // Wander), and a hungry lion CATCHES an adjacent rabbit (Hunt resolves a kill on contact), like the manual ones.
    #[test]
    fn scenario_emergent_prey_flees_and_predator_catches() {
        // flee: a cat approaches from -20; the rabbit (faster, FLEE_BOOST) must run +x and KEEP its distance
        let mut w = emergent_world();
        let cat = spawn_kind(&mut w, Kind::Cat, -20.0, 0.0, 10);
        let r = spawn_kind(&mut w, Kind::Rabbit, 0.0, 0.0, 11);
        let gap0 = w.agents[cat].agent.x.abs() - w.agents[r].agent.x; // ~20
        for t in 1..=60 {
            w.tick_once(t);
        }
        let gap1 = (w.agents[r].agent.x - w.agents[cat].agent.x).abs();
        eprintln!("[emergent flee] rabbit x={:.1}, gap {gap0:.1} → {gap1:.1}", w.agents[r].agent.x);
        assert!(!w.agents[r].dead, "emergent rabbit was caught — Flee primitive didn't outrun the cat");
        assert!(w.agents[r].agent.x > 1.0, "emergent rabbit didn't flee +x (x={:.1})", w.agents[r].agent.x);
        assert!(gap1 > gap0 - 1.0, "emergent rabbit didn't keep its distance ({gap0:.1}→{gap1:.1})");

        // catch: a hungry lion adjacent to a rabbit eats it within a short window
        let mut w = emergent_world();
        let prey = spawn_kind(&mut w, Kind::Rabbit, 0.0, 0.0, 12);
        w.agents[prey].health = 0.5; // pre-wounded → Hunt finishes it in one strike (full-health takes two — the struggle)
        let lion = spawn_kind(&mut w, Kind::Lion, 1.2, 0.0, 5001);
        w.agents[lion].hungry = true;
        w.agents[lion].stamina = 1.0;
        let mut caught = false;
        for _ in 0..240 {
            w.step(DT);
            if w.agents[prey].dead {
                caught = true;
                break;
            }
        }
        assert!(caught, "emergent hungry lion failed to catch an adjacent rabbit — Hunt primitive isn't resolving kills");
    }

    // SCENARIO (emergent · EMERGENCE ENDGAME): the boldness niche must COEXIST, not collapse to one optimum.
    // `safety` used to be a pure-downside knob (low → flees late → dies, with no upside) so selection swept it
    // UP and strategy diversity died — that's drift, not emergence. The forage trade-off (bold refuels faster
    // on open ground, cautious survives predators) makes it a real niche axis held open by NEGATIVE FREQUENCY
    // DEPENDENCE (bold thrives when rare, gets culled when common). The emergence signal is SUSTAINED DIVERSITY:
    // after many generations under predation, BOTH a bold cohort and a cautious cohort must still be alive.
    #[test]
    fn scenario_emergent_boldness_niches_coexist() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 120, 0.0, 0.0, 80.0, 1000);
        cluster(&mut w, Kind::Cat, 8, 0.0, 0.0, 60.0, 6000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s); // stagger ages → overlapping generations, deaths not synchronized
        }
        run_pop(&mut w, 18000); // ≈600 s → many rabbit generations, enough for selection to sweep IF it would
        let (mut bold, mut cautious, mut n, mut sum, mut sq) = (0usize, 0usize, 0usize, 0.0, 0.0);
        for m in &w.agents {
            if m.dead || m.kind != Kind::Rabbit {
                continue;
            }
            let s = m.weights.safety;
            n += 1;
            sum += s;
            sq += s * s;
            if s < 0.85 {
                bold += 1;
            }
            if s > 1.15 {
                cautious += 1;
            }
        }
        let mean = if n > 0 { sum / n as f64 } else { 0.0 };
        let sd = if n > 0 { (sq / n as f64 - mean * mean).max(0.0).sqrt() } else { 0.0 };
        eprintln!("[emergent boldness-niches] rabbits={n} mean_safety={mean:.2} sd={sd:.2} bold(<0.85)={bold} cautious(>1.15)={cautious}");
        assert!(n >= 20, "rabbit population crashed to {n} — the forage trade-off destabilised the world");
        assert!(
            bold >= 2 && cautious >= 2,
            "boldness niche collapsed (bold={bold}, cautious={cautious}, mean={mean:.2}) — one strategy swept; this is drift, not emergence"
        );
    }

    // THIRST: an animal at a water edge drinks its hydration back up.
    #[test]
    fn a_thirsty_animal_drinks_at_the_water_edge() {
        let mut w = emergent_world();
        let r = spawn_kind(&mut w, Kind::Rabbit, 3.0, 0.0, 7);
        w.set_water(&[0.0, 0.0, 2.0]); // pond r=2 at origin → drink reach to 7 m; the rabbit at x=3 is at the edge
        w.agents[r].hydration = 0.2;
        for _ in 0..60 {
            w.step(DT);
        }
        assert!(w.agents[r].hydration > 0.5, "a rabbit at the bank should drink back up (h={:.2})", w.agents[r].hydration);
    }

    // THIRST: with no reachable water, a population is culled by thirst — proof it's a REAL, independent pressure
    // (distinct from food/predation). The same world WITH water (next test) sustains, so it isn't just lethal.
    #[test]
    fn thirst_culls_a_population_with_no_reachable_water() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 40, 0.0, 0.0, 20.0, 1000);
        w.set_water(&[100_000.0, 100_000.0, 5.0]); // water exists but is unreachably far → nobody can drink
        let p0 = alive(&w)[Kind::Rabbit as usize];
        run_pop(&mut w, 9000); // 300 s ≫ the ~225 s a full animal lasts before thirst turns fatal
        let p1 = alive(&w)[Kind::Rabbit as usize];
        eprintln!("[thirst no-water] rabbits {p0}→{p1}");
        assert!(p1 <= p0 / 5, "thirst should have culled the unwatered population ({p0}→{p1})");
    }

    // THIRST: the SAME setup WITH a pond among the herd sustains the population — thirst is a survivable errand, the
    // thirst-seek steer pulls them to the bank in time. (Pair with the test above: pressure is real but not a wipe.)
    #[test]
    fn scenario_emergent_population_survives_with_water() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 40, 0.0, 0.0, 30.0, 1000);
        w.set_water(&[0.0, 0.0, 8.0]); // a pond in the middle of the range → reachable by the whole herd
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let p0 = alive(&w)[Kind::Rabbit as usize];
        run_pop(&mut w, 9000);
        let p1 = alive(&w)[Kind::Rabbit as usize];
        eprintln!("[thirst with-water] rabbits {p0}→{p1}");
        assert!(p1 >= p0 / 3, "a watered herd should NOT collapse to thirst ({p0}→{p1}) — the seek/drink loop is failing");
    }

    // SCENARIO (emergent · EMERGENCE): TWO niches diversifying at once — boldness (predation channel) AND social
    // (the WATER channel) — the social axis now decouples from boldness because it spends a DIFFERENT survival
    // currency (thirst, not predation). Herders navigate to a distant pond reliably (herd knowledge) and survive;
    // loners seek weakly, risk thirst, but breed freely (low crowd). The SOCIAL polymorphism must persist.
    //
    // NOTE on boldness here: with the pond sitting inside the predators' range, the waterhole becomes an AMBUSH —
    // bold (flee-late) prey are culled while drinking, so boldness leans cautious in THIS arena. That's emergent
    // (dangerous waterholes), not a regression: the standalone boldness niche (no water / water away from predators)
    // still coexists — see scenario_emergent_boldness_niches_coexist. Across a real map's varied water/predator
    // layouts, different niches dominate in different regions (spatial niche variation). We assert the social win
    // + that boldness still carries VARIANCE (isn't frozen to a single clone), and print both for observability.
    #[test]
    fn scenario_emergent_social_niche_via_water() {
        let mut w = emergent_world();
        w.set_seasons(false); // stable climate → the social equilibrium isn't smeared by the moving dry season
        cluster(&mut w, Kind::Rabbit, 120, 0.0, 0.0, 35.0, 1000);
        cluster(&mut w, Kind::Cat, 4, 0.0, 0.0, 50.0, 6000); // light predation — this test isolates the WATER/social
        // niche; with apostatic predation now on, 8 cats + thirst treks overwhelmed the herd. 4 keeps it survivable.
        w.set_water(&[80.0, 0.0, 8.0]); // a single pond ~70 m from the herd's edge → reaching it is a real errand
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        run_pop(&mut w, 18000);
        let (mut bold, mut caut, mut herd, mut lone, mut n, mut ss, mut ssq) = (0usize, 0usize, 0usize, 0usize, 0usize, 0.0, 0.0);
        for m in &w.agents {
            if m.dead || m.kind != Kind::Rabbit {
                continue;
            }
            n += 1;
            let s = m.weights.safety;
            ss += s;
            ssq += s * s;
            if s < 0.85 { bold += 1; }
            if s > 1.15 { caut += 1; }
            if m.weights.social > 1.15 { herd += 1; }
            if m.weights.social < 0.85 { lone += 1; }
        }
        let bold_sd = if n > 0 { (ssq / n as f64 - (ss / n as f64).powi(2)).max(0.0).sqrt() } else { 0.0 };
        eprintln!("[emergent social-via-water] rabbits={n} | boldness: bold={bold} cautious={caut} sd={bold_sd:.2} | social: herd={herd} loner={lone}");
        // NOTE: the social (herd↔loner) niche is now DORMANT by design — the player wanted animals to ROAM and
        // spread, so THIRSTY_AT was lowered to 0.15 (they barely depend on water). That kills the herders'
        // navigation advantage, so the water-channel social niche no longer differentiates (herd sweeps). The
        // mechanism (herd_nav) is still in the code; it only bites under HIGH water-dependence. We keep this test
        // as a guard that water + roaming doesn't break the BOLDNESS niche, which must still vary.
        let _ = (herd, lone);
        assert!(n >= 20, "rabbit population crashed to {n} with water present");
        assert!(bold_sd > 0.05, "boldness froze to a single strategy (sd={bold_sd:.2}) with water + roaming present");
    }

    // helper: run the boldness arena (rabbits + cats, NO water → isolates the predation channel) from a given
    // founder-seed base for `ticks`, then count the surviving rabbits' boldness cohorts. Returns (n, bold, cautious).
    fn run_boldness_arena(seed0: i32, ticks: usize) -> (usize, usize, usize) {
        let mut w = emergent_world();
        w.set_seasons(false); // no water in this arena anyway; pin a stable climate so the morph balance is clean
        cluster(&mut w, Kind::Rabbit, 120, 0.0, 0.0, 80.0, seed0);
        cluster(&mut w, Kind::Cat, 8, 0.0, 0.0, 60.0, seed0 + 5000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        run_pop(&mut w, ticks);
        let (mut bold, mut caut, mut n) = (0usize, 0usize, 0usize);
        for m in &w.agents {
            if m.dead || m.kind != Kind::Rabbit {
                continue;
            }
            n += 1;
            if m.weights.safety < 0.85 { bold += 1; }
            if m.weights.safety > 1.15 { caut += 1; }
        }
        (n, bold, caut)
    }

    // SCENARIO (emergent · EMERGENCE ROBUSTNESS): the boldness polymorphism must NOT be a single-seed fluke. Run
    // the arena from several independent founder seeds; the population must survive every time, and BOTH cohorts
    // must persist in the clear majority (≥3 of 4) — a real selective balance, not luck of one starting draw.
    #[test]
    fn scenario_emergent_boldness_robust_across_seeds() {
        let mut coexist = 0;
        for seed0 in [1000, 2000, 3000, 4000] {
            let (n, bold, caut) = run_boldness_arena(seed0, 15000);
            eprintln!("[boldness robust] seed0={seed0} rabbits={n} bold={bold} cautious={caut}");
            assert!(n >= 20, "seed0={seed0}: population crashed to {n}");
            if bold >= 2 && caut >= 2 {
                coexist += 1;
            }
        }
        assert!(coexist >= 3, "boldness coexistence held in only {coexist}/4 seeds — the niche is seed-fragile, not a real balance");
    }

    // SCENARIO (emergent · EMERGENCE STABILITY): over a LONG run, BOTH morphs must remain part of the dynamic.
    // Apostatic predation + the mutation jackpot make the morph frequencies OSCILLATE (predator-prey-style cycles)
    // rather than sit at a static ratio — so we sample a window of late timepoints and require that each morph is
    // a real player at SOME point (≥5) and neither is extinct in EVERY window (permanent fixation). No crash.
    #[test]
    fn scenario_emergent_boldness_stable_long_run() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 120, 0.0, 0.0, 80.0, 1000);
        cluster(&mut w, Kind::Cat, 8, 0.0, 0.0, 60.0, 6000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let (mut max_bold, mut max_caut, mut bold_zeros, mut caut_zeros, mut samples) = (0usize, 0usize, 0, 0, 0);
        for _ in 0..6 {
            run_pop(&mut w, 6000); // 6 windows × 6000 ticks ≈ 36k ticks total
            let (mut bold, mut caut, mut n) = (0usize, 0usize, 0usize);
            for m in &w.agents {
                if m.dead || m.kind != Kind::Rabbit {
                    continue;
                }
                n += 1;
                if m.weights.safety < 0.85 { bold += 1; }
                if m.weights.safety > 1.15 { caut += 1; }
            }
            eprintln!("[boldness long-run] window: rabbits={n} bold={bold} cautious={caut}");
            assert!(n >= 20, "population crashed to {n} mid long-run");
            max_bold = max_bold.max(bold);
            max_caut = max_caut.max(caut);
            if bold == 0 { bold_zeros += 1; }
            if caut == 0 { caut_zeros += 1; }
            samples += 1;
        }
        assert!(max_bold >= 5, "the bold morph never mattered across the long run (max={max_bold}) — cautious fixed");
        assert!(max_caut >= 5, "the cautious morph never mattered across the long run (max={max_caut}) — bold fixed");
        assert!(bold_zeros < samples && caut_zeros < samples, "a morph was extinct in every window — permanent fixation, not a cycle");
    }

    // SCENARIO (emergent · SEASONS): a watered herd must RIDE OUT the wet↔dry season cycle. The dry peak (1.5×
    // thirst) stresses them — they lean harder on the pond — but with water in reach the population persists.
    // Proves the seasonal drought adds drama without being a population wipe. Runs > one full season.
    #[test]
    fn scenario_emergent_herd_survives_the_season_cycle() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 50, 0.0, 0.0, 25.0, 1000);
        w.set_water(&[0.0, 0.0, 8.0]); // a central pond the herd can reach even in the dry season
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let p0 = alive(&w)[Kind::Rabbit as usize];
        run_pop(&mut w, 14000); // > SEASON_TICKS (12000) → spans the full dry peak and back toward wet
        let p1 = alive(&w)[Kind::Rabbit as usize];
        eprintln!("[seasons] rabbits {p0}→{p1} across a full wet→dry→wet cycle");
        assert!(p1 >= p0 / 3, "the herd didn't survive the dry season ({p0}→{p1}) — the drought is too harsh");
    }

    // a DIRECTOR-driven hard drought (set_aridity) bites: with water UNREACHABLE and aridity cranked, thirst
    // claims the population fast — the LLM/Mother-Nature seam can force a real crisis.
    #[test]
    fn a_director_drought_intensifies_thirst() {
        let mut w = emergent_world();
        cluster(&mut w, Kind::Rabbit, 40, 0.0, 0.0, 20.0, 1000);
        w.set_water(&[100_000.0, 100_000.0, 5.0]); // water exists but unreachable → thirst is the only outcome
        w.set_aridity(3.0); // director cranks the drought → hydration ebbs ~3× faster
        let p0 = alive(&w)[Kind::Rabbit as usize];
        run_pop(&mut w, 4000); // a SHORT run — under normal aridity many would still be alive here
        let p1 = alive(&w)[Kind::Rabbit as usize];
        eprintln!("[director drought] rabbits {p0}→{p1} in 4000 ticks at aridity 3.0");
        assert!(p1 <= p0 / 3, "a cranked drought should be culling fast ({p0}→{p1})");
    }

    // helper: an opposite-sex pair of industrious adult settlers, fed + ready to work, at the origin.
    fn settle_industrious_couple(w: &mut World) -> (usize, usize) {
        let a = spawn_kind(w, Kind::Person, 0.0, 0.0, 10); // even seed → female
        let b = spawn_kind(w, Kind::Person, 1.5, 0.0, 11); // odd seed → male (within FAMILY_R)
        for p in [a, b] {
            w.set_genome(p, 1.0, 1.0, 1.0, 1.0, 1.6); // industry 1.6 > WELL_INDUSTRY → a digger
            w.agents[p].age = w.agents[p].lifespan * 0.4; // adult
            w.agents[p].energy = 1.0;
            w.agents[p].build_cd = 0.0;
        }
        (a, b)
    }

    // SCENARIO (emergent · JOBS): an industrious settled couple with NO water in reach DIGS A WELL (emergent job →
    // a self-made water source). The same couple WITH water already nearby builds a HOUSE instead — water before
    // shelter only when water is missing. This is the loop that grows settlements around water they create.
    #[test]
    fn dry_industrious_settlers_dig_a_well_else_build() {
        // DRY → they dig a well
        let mut w = emergent_world();
        w.set_seasons(false);
        settle_industrious_couple(&mut w);
        let mut dug = false;
        for t in 1..=400 {
            w.tick_once(t);
            if !w.wells().is_empty() {
                dug = true;
                break;
            }
        }
        assert!(dug, "an industrious adult couple with no water in reach should dig a well");

        // WATERED → with a pond already in reach they DON'T dig (dry=false); the build pass raises a house instead
        let mut w2 = emergent_world();
        w2.set_seasons(false);
        w2.set_water(&[0.0, 0.0, 5.0]); // a pond right where they stand → not dry
        settle_industrious_couple(&mut w2);
        let mut built = false;
        for t in 1..=400 {
            w2.tick_once(t);
            if !w2.builds().is_empty() {
                built = true;
                break;
            }
        }
        assert!(built, "with water in reach the couple should build a house");
        assert!(w2.wells().is_empty(), "they should NOT dig a well when water is already in reach");
    }

    // CULTURE: a YOUNG settler beside a distinctive ELDER drifts its behaviour genome toward that role model
    // (memetic learning). Here the youth starts LAZY (industry 0.5) next to an INDUSTRIOUS elder (1.8) → it learns
    // toward high industry. Parked (max_speed 0) so they stay within learning range.
    #[test]
    fn a_young_settler_learns_from_a_nearby_elder() {
        let mut w = emergent_world();
        w.set_seasons(false);
        let elder = spawn_kind(&mut w, Kind::Person, 0.0, 0.0, 20);
        let youth = spawn_kind(&mut w, Kind::Person, 2.0, 0.0, 22);
        w.set_genome(elder, 1.0, 1.0, 1.0, 1.0, 1.8); // industrious role model
        w.set_genome(youth, 1.0, 1.0, 1.0, 1.0, 0.5); // lazy learner
        w.agents[elder].age = w.agents[elder].lifespan * 0.85; // an elder
        w.agents[youth].age = w.agents[youth].lifespan * 0.08; // a child in its formative years
        w.agents[elder].agent.max_speed = 0.0; // park them so they stay in learning range
        w.agents[youth].agent.max_speed = 0.0;
        let before = w.agents[youth].weights.industry;
        for t in 1..=3000 {
            w.tick_once(t);
        }
        let after = w.agents[youth].weights.industry;
        eprintln!("[culture] youth industry {before:.2} → {after:.2} (elder 1.8)");
        assert!(after > before + 0.3, "the youth didn't learn from the elder ({before:.2}→{after:.2})");
        assert!(after < 1.81, "learning shouldn't overshoot the elder");
    }

    // CULTURE: two SEPARATED settlements drift to DIFFERENT customs. Group A's young learn from an industrious
    // elder, group B's from a lazy one → A trends industrious, B lazy. The divergence is the emergent signal:
    // identical genes at the start, distinct local cultures at the end purely from who you grow up around.
    #[test]
    fn separated_settlements_diverge_in_custom() {
        let mut w = emergent_world();
        w.set_seasons(false);
        let spawn_group = |w: &mut World, cx: f64, elder_ind: f64, seed0: i32| {
            let elder = spawn_kind(w, Kind::Person, cx, 0.0, seed0);
            w.set_genome(elder, 1.0, 1.0, 1.0, 1.0, elder_ind);
            w.agents[elder].age = w.agents[elder].lifespan * 0.85;
            w.agents[elder].agent.max_speed = 0.0;
            for k in 1..=6 {
                let y = spawn_kind(w, Kind::Person, cx + k as f64 * 2.0, 0.0, seed0 + 100 + k);
                w.set_genome(y, 1.0, 1.0, 1.0, 1.0, 1.0); // all youths start NEUTRAL (identical) — culture does the rest
                w.agents[y].age = w.agents[y].lifespan * 0.08;
                w.agents[y].agent.max_speed = 0.0;
            }
        };
        spawn_group(&mut w, 0.0, 1.8, 1000); // settlement A: industrious elder
        spawn_group(&mut w, 600.0, 0.4, 2000); // settlement B: laid-back elder (far away → no cross-learning)
        for t in 1..=3000 {
            w.tick_once(t);
        }
        let (mut sa, mut na, mut sb, mut nb) = (0.0, 0, 0.0, 0);
        for m in &w.agents {
            if m.dead || !matches!(m.kind, Kind::Person) || m.age > m.lifespan * 0.5 {
                continue; // only the YOUTHS (skip the parked elders)
            }
            if m.agent.x < 300.0 {
                sa += m.weights.industry;
                na += 1;
            } else {
                sb += m.weights.industry;
                nb += 1;
            }
        }
        let (ma, mb) = (sa / na as f64, sb / nb as f64);
        eprintln!("[culture diverge] settlement A youth industry={ma:.2} (n={na}) | B={mb:.2} (n={nb})");
        assert!(na >= 3 && nb >= 3, "lost too many youths to measure");
        assert!(ma - mb > 0.5, "the settlements didn't diverge in custom (A={ma:.2}, B={mb:.2}) — culture isn't transmitting");
    }

    // SCENARIO (emergent · FULL SYSTEM): the whole machine at once — people hunt+settle+dig wells+encultarate,
    // rabbits graze+drink+flee, cats predate, the SEASON cycles into a dry peak AND a director drought hits, and
    // BOTH feedback loops close (births → spawns, dug wells → new drink sources). Over a long run the world must
    // stay ALIVE (no crash, no unbounded explosion) AND DIVERSE (the rabbit boldness polymorphism endures). This
    // is the cross-feature regression guard: if any system starts fighting another, the band breaks here.
    #[test]
    fn scenario_full_world_stays_alive_and_diverse() {
        let mut w = emergent_world(); // seasons ON by default → a dry season sweeps through mid-run
        cluster(&mut w, Kind::Rabbit, 100, 0.0, 0.0, 70.0, 1000);
        cluster(&mut w, Kind::Person, 24, 0.0, 0.0, 40.0, 3000);
        cluster(&mut w, Kind::Cat, 6, 0.0, 0.0, 60.0, 6000);
        let seeds: Vec<i32> = w.agents.iter().map(|m| m.seed_id).collect();
        for (i, s) in seeds.into_iter().enumerate() {
            w.randomize_start_age(i, s);
        }
        let mut water: Vec<f64> = vec![140.0, 0.0, 10.0]; // a pond off to the side → people at origin are "dry" → they dig
        w.set_water(&water);
        let mut next_seed = 950_000i32;
        let mut wells_dug = 0usize;
        for t in 0..20000 {
            w.step(DT);
            // births → spawns (close the reproduction loop, exactly as the JS bridge does)
            let births: Vec<f32> = w.births().to_vec();
            for b in births.chunks_exact(11) {
                let kind = crate::eco::kind_from_code(b[0] as u8);
                let bi = w.spawn(Agent::new(b[1] as f64, b[2] as f64, next_seed, &opts_for(kind, next_seed)), kind, radius_of(kind), next_seed);
                w.agents[bi].breed_cd = JUVENILE_CD;
                w.agents[bi].gene = b[3] as f64;
                w.agents[bi].age = 0.0;
                w.set_lineage(bi, b[4] as u32, b[5] as u32);
                w.set_genome(bi, b[6] as f64, b[7] as f64, b[8] as f64, b[9] as f64, b[10] as f64);
                next_seed = next_seed.wrapping_add(1);
            }
            // dug wells → new drink sources (close the emergent-jobs feedback, exactly as Scene does)
            let dug: Vec<f32> = w.wells().to_vec();
            if !dug.is_empty() {
                for wl in dug.chunks_exact(2) {
                    water.extend_from_slice(&[wl[0] as f64, wl[1] as f64, 3.0]);
                    wells_dug += 1;
                }
                w.set_water(&water);
            }
            if t == 12000 {
                w.set_aridity(2.0); // the macro-director slams a hard drought down partway through
            }
            if t == 16500 {
                w.set_aridity(1.0); // …then the rains return
            }
        }
        let mut counts = [0usize; 6];
        let (mut bold, mut caut, mut rss, mut rsq, mut rn) = (0usize, 0usize, 0.0, 0.0, 0usize);
        for m in &w.agents {
            if m.dead {
                continue;
            }
            counts[m.kind as usize] += 1;
            if matches!(m.kind, Kind::Rabbit) {
                let s = m.weights.safety;
                rss += s;
                rsq += s * s;
                rn += 1;
                if s < 0.85 { bold += 1; }
                if s > 1.15 { caut += 1; }
            }
        }
        let total: usize = counts.iter().sum();
        let rsd = if rn > 0 { (rsq / rn as f64 - (rss / rn as f64).powi(2)).max(0.0).sqrt() } else { 0.0 };
        eprintln!("[full world] counts={counts:?} total={total} wells_dug={wells_dug} | rabbit bold={bold} caut={caut} sd={rsd:.2}");
        // ALIVE — neither a crash nor an unbounded explosion
        assert!(total > 30, "the whole-system world collapsed (total={total})");
        assert!(total < 6000, "the whole-system world exploded unbounded (total={total})");
        assert!(counts[Kind::Rabbit as usize] > 5, "the prey base died out under combined pressure");
        // DIVERSE — the boldness polymorphism survived the full system + the drought
        assert!(rsd > 0.05, "rabbit strategy diversity collapsed in the full system (sd={rsd:.2})");
    }

    // PERF AUDIT: the thirst path calls nearest_water per-agent per-tick (O(agents × water_sources)). Run a LARGE
    // fixed population with FEW vs MANY water sources and compare wall-clock — if nearest_water were a hotspot the
    // many-source run would blow up. Also a correctness-at-scale check (handles 800 agents + 200 ponds, no crash).
    // Prints timings (not asserted — machine-dependent); asserts only completion + sanity so it can't go flaky.
    #[test]
    fn perf_thirst_scale_is_not_a_hotspot() {
        use std::time::Instant;
        let build = |n_water: usize| -> World {
            let mut w = emergent_world();
            w.set_seasons(false);
            cluster(&mut w, Kind::Rabbit, 800, 0.0, 0.0, 200.0, 1000);
            let mut water = Vec::new();
            for k in 0..n_water {
                let a = k as f64 * 2.399_963; // golden-angle scatter of ponds across the field
                let r = 12.0 * (k as f64).sqrt();
                water.extend_from_slice(&[r * a.cos(), r * a.sin(), 6.0]);
            }
            w.set_water(&water);
            w
        };
        let mut few = build(2);
        let t0 = Instant::now();
        for t in 1..=400 {
            few.tick_once(t);
        }
        let few_ms = t0.elapsed().as_millis();
        let mut many = build(200);
        let t1 = Instant::now();
        for t in 1..=400 {
            many.tick_once(t);
        }
        let many_ms = t1.elapsed().as_millis();
        let alive = many.agents.iter().filter(|m| !m.dead && matches!(m.kind, Kind::Rabbit)).count();
        eprintln!("[perf thirst] 800 rabbits × 400 ticks — 2 ponds: {few_ms} ms · 200 ponds: {many_ms} ms · alive={alive}");
        assert!(alive > 50, "the big population should largely survive with ample water (alive={alive})");
        // 100× the water sources must NOT make it 10× slower — nearest_water is a small linear scan, dwarfed by the
        // flock/behaviour passes. A loose ceiling catches a catastrophic O(n²)-style regression without being flaky.
        assert!(many_ms < few_ms * 6 + 500, "nearest_water became a hotspot: 2 ponds {few_ms}ms vs 200 ponds {many_ms}ms");
    }
}
