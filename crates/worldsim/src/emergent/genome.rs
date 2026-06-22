//! Tier 2 substrate — the **behaviour genome**: the per-agent utility WEIGHTS the emergent scorer
//! multiplies each need by (how much this individual values food vs safety vs company vs rest vs purpose).
//! It rides alongside the existing VIGOR gene and is inherited the same way (parent average ± seeded
//! mutation, see `inherit`), so lineages can DISCOVER strategies — cautious, industrious, nomadic — under
//! survival/breeding selection, with ZERO authored behaviours (design doc §3 Tier 2).
//!
//! Determinism: founders derive a genome deterministically from their stable `seed_id` (so a fresh
//! population already has strategy VARIATION to select on); births blend the parents' genomes with a seeded
//! mutation roll. `Genome::NEUTRAL` (all 1.0) reproduces the un-weighted baseline, so it never perturbs
//! Manual mode — Manual stores a neutral genome it simply ignores.

/// RNG channels for the genome rolls (kept clear of the eco/steering/vigor channels used elsewhere).
const CH_FOOD: i32 = 70;
const CH_SAFETY: i32 = 71;
const CH_SOCIAL: i32 = 72;
const CH_REST: i32 = 73;
const CH_INDUSTRY: i32 = 74;
const CH_GENOME_MUT: i32 = 75;

const W_MIN: f64 = 0.25;
const W_MAX: f64 = 2.2;
const W_MUT: f64 = 0.1; // ± mutation magnitude per birth (wider than the vigor gene's 0.05 → strategies spread fast)
const W_JACKPOT_P: f64 = 0.07; // chance a birth rolls a LARGE mutation (anti-fixation → keeps lost morphs returning)
const W_JACKPOT: f64 = 0.55; // ± magnitude of that rare large jump (spans ~half the weight band in one step)

#[derive(Clone, Copy, Debug)]
pub struct Genome {
    pub food: f64,     // drive to reduce HUNGER (forage / hunt / scavenge) — a glutton vs an ascetic
    pub safety: f64,   // drive to FLEE danger — a coward (high) vs a daredevil (low)
    pub social: f64,   // pull toward its own kind (gregarious vs solitary)
    pub rest: f64,     // tendency to REST/recover rather than push on while spent
    pub industry: f64, // people: PURPOSE — the drive to settle + build (a founder vs a drifter)
}

impl Genome {
    /// The un-weighted baseline. Manual mode carries this and ignores it; the emergent scorer treats it as
    /// "value every need at face worth", so a NEUTRAL world behaves as the plain needs model with no biases.
    pub const NEUTRAL: Genome = Genome { food: 1.0, safety: 1.0, social: 1.0, rest: 1.0, industry: 1.0 };

    /// A founder's genome, derived from its stable per-individual seed → a spawned population starts with a
    /// spread of strategies (0.6‥1.4 per weight) for selection to act on, deterministically.
    pub fn from_seed(seed_id: i32) -> Genome {
        let g = |ch: i32| 0.3 + 1.4 * crate::simrng::rand(&[seed_id, ch]); // 0.3‥1.7 — a WIDE founder spread so strategies visibly diverge
        Genome {
            food: g(CH_FOOD),
            safety: g(CH_SAFETY),
            social: g(CH_SOCIAL),
            rest: g(CH_REST),
            industry: g(CH_INDUSTRY),
        }
    }

    /// FORAGE PHENOTYPE — the trade-off that turns `safety` from a pure-downside knob into a real NICHE axis.
    /// A BOLD individual (low `safety`) ventures onto richer open ground → refuels FASTER (this bonus), but its
    /// low flee-drive (utility scorer) gets it CAUGHT more. A CAUTIOUS one (high `safety`) forages timidly near
    /// cover → refuels slower, but survives predators. Neither dominates: bold out-breeds when rare, gets culled
    /// when common (predators face abundant easy targets) → NEGATIVE FREQUENCY DEPENDENCE keeps both lineages
    /// alive. Returns 1.0 at the NEUTRAL safety=1.0, so Manual mode (neutral genome) forages exactly as before.
    pub fn forage(&self) -> f64 {
        (1.5 - 0.5 * self.safety).clamp(0.6, 1.55)
    }

    /// BREED-HASTE — the primary niche lever (r/K selection). A BOLD individual (low `safety`) lives fast: it
    /// recovers between litters SOONER (>1 → shorter breed cooldown) so it out-reproduces — paying for it by
    /// fleeing late and getting eaten (the utility scorer). A CAUTIOUS one (high `safety`) breeds slowly but
    /// survives. Bold wins when rare, is culled when common → the two strategies coexist instead of one sweeping.
    /// Prey aren't energy-limited (they graze to full in seconds), so REPRODUCTIVE rate — not forage — is the
    /// fitness currency the trade-off must spend. 1.0 at the NEUTRAL safety=1.0 → Manual mode is unchanged.
    pub fn breed_haste(&self) -> f64 {
        (1.6 - 0.6 * self.safety).clamp(0.7, 1.6)
    }

    /// CULTURE — lerp each weight a fraction `t` toward another genome (a role model). Used for memetic
    /// transmission: a young agent LEARNS from a successful elder, on top of parental inheritance. The
    /// niche dims (safety/social) stay pinned by selection; the unselected dims drift → local customs.
    pub fn blend_toward(&self, o: &Genome, t: f64) -> Genome {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Genome {
            food: mix(self.food, o.food),
            safety: mix(self.safety, o.safety),
            social: mix(self.social, o.social),
            rest: mix(self.rest, o.rest),
            industry: mix(self.industry, o.industry),
        }
    }

    // SOCIAL niche — RESOLVED via WATER. It first failed to co-exist alongside boldness when it spent the SAME
    // currencies (predation/breeding) → one joint optimum. The fix was an INDEPENDENT selective pressure: thirst.
    // Herders navigate to water reliably (herd knowledge → survive thirst) while loners risk it but breed freely
    // (low crowd). Because boldness spends predation and social spends thirst, the two polymorphisms now hold at
    // once (scenario_emergent_social_niche_via_water). The lever lives in world.rs (thirst-seek × herd_nav).

    /// A litter's inherited genome: the average of both parents' weights, ± a seeded mutation per weight,
    /// clamped to a sane band. Same shape as the vigor gene's inheritance, so the two evolve in lockstep.
    pub fn inherit(a: &Genome, b: &Genome, seed_a: i32, seed_b: i32, tick: i32) -> Genome {
        let blend = |x: f64, y: f64, k: i32| {
            let mut mu = (crate::simrng::rand(&[seed_a, seed_b, tick, CH_GENOME_MUT, k]) - 0.5) * 2.0 * W_MUT;
            // MUTATION JACKPOT — rarely (~7%) a much larger jump. This is what keeps a strategy that selection has
            // driven to local extinction from being GONE for good (the absorbing-boundary problem): a bold lineage
            // occasionally throws a cautious pup and vice-versa, so mutation-selection balance holds BOTH morphs
            // alive indefinitely and the polymorphism is seed-robust + long-run stable, not a lucky transient.
            if crate::simrng::rand(&[seed_a, seed_b, tick, CH_GENOME_MUT, k + 100]) < W_JACKPOT_P {
                mu += (crate::simrng::rand(&[seed_a, seed_b, tick, CH_GENOME_MUT, k + 200]) - 0.5) * 2.0 * W_JACKPOT;
            }
            ((x + y) * 0.5 + mu).clamp(W_MIN, W_MAX)
        };
        Genome {
            food: blend(a.food, b.food, 0),
            safety: blend(a.safety, b.safety, 1),
            social: blend(a.social, b.social, 2),
            rest: blend(a.rest, b.rest, 3),
            industry: blend(a.industry, b.industry, 4),
        }
    }
}
