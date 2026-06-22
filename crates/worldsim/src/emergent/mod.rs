//! Emergent behaviour — the switchable AI MODE (design doc `docs/emergent-behavior.md`). This is the
//! `BehaviorMode::Emergent` decision pass: a needs + primitives + utility-scorer brain that REPLACES only the
//! manual behaviour pass (world.rs `tick_once` section 5). Everything else — perception/targeting, mobbing
//! tally, sleep, flocking (sections 1–4) and metabolism, breeding, city-building, stepping, collision
//! (sections 6–9) — is SHARED, untouched, and still tested by the 100+ manual tests. So both brains feed the
//! identical downstream physics + the same SoA read-back; the render layer never knows which one ran.
//!
//! The novelty vs Manual is purely in HOW an agent picks its action: Manual runs a fixed priority chain
//! (mob → bully → threat → rival → hunt-player → prey → carrion → fish → wander); Emergent SCORES each
//! feasible primitive by how much it relieves the agent's needs, weighted by its evolved behaviour genome,
//! and takes the max. Same primitives, same effect-resolution (a catch on contact still kills + feeds, a mob
//! still bleeds the hunter) — but the CHOICE is bottom-up, so strategies (cautious, bold, industrious)
//! emerge + are selected on instead of being hand-authored. Manual stays the default + the safety net.

pub mod genome;
pub mod needs;
pub mod primitives;
pub mod utility;

use crate::clock::DT;
use crate::eco::{eco, sleep_secs, Hunts, Kind};
use needs::Needs;
use primitives::{Options, Primitive};

use super::World;

/// The Emergent decision pass — fills `forces[i]` / `behave[i]` (+ resolves catches, scavenging, mob damage,
/// rival scraps, player pressure) for every awake agent, exactly the output contract `decide_manual` produces.
/// Returns this tick's peak player-danger (the caller eases the UI vignette toward it, as in Manual).
pub fn decide(world: &mut World, px: f64, pz: f64, pspeed: f64, danger2: f64, hunt2: f64) -> f64 {
    let n = world.agents.len();
    let mut danger_now = 0.0_f64;

    for i in 0..n {
        if world.agents[i].dead || world.slept[i] {
            continue;
        }
        if world.agents[i].feeding > 0.0 {
            world.behave[i] = (1.0, true); // hunkered over a fresh kill → settle + eat, don't fidget/re-target
            continue;
        }
        let ax = world.agents[i].agent.x;
        let az = world.agents[i].agent.z;
        let a_max = world.agents[i].agent.max_speed;
        let radius = world.agents[i].radius;
        let rank = world.agents[i].rank;
        let kind = world.agents[i].kind;
        let a_hunts = matches!(eco(kind).hunts, Hunts::Lower);
        let can_sprint = world.agents[i].stamina > super::CAN_SPRINT;
        let mobbed = world.agents[i].mobbed;
        if !mobbed {
            world.agents[i].slash_budget = world.agents[i].slash_max; // fresh ferocity for the next fight
            world.agents[i].slash_cd = 0.0;
        }

        // ── perception (read from the shared targeting + flock passes) ───────────────────────────────────
        let threat = world.transient[i].threat;
        let threat_pos = threat.map(|t| (world.agents[t].agent.x, world.agents[t].agent.z));
        let threat_d2 = threat_pos.map(|(tx, tz)| (tx - ax).powi(2) + (tz - az).powi(2));
        let prey_info = world.transient[i].prey.map(|p| (p, world.agents[p].agent.x, world.agents[p].agent.z, world.agents[p].radius));
        let crowd = world.forces[i].2;

        // ── MOBBED hunter: it bleeds from attackers + slashes back whatever it then chooses (same as Manual) ─
        if mobbed {
            let attackers = world.transient[i].attackers;
            if attackers >= super::MOB_MIN {
                world.agents[i].health = (world.agents[i].health - super::MOB_KILL_DPS * attackers as f64 * DT).max(0.0);
                world.agents[i].slash_cd -= DT;
                if world.agents[i].slash_cd <= 0.0 && world.agents[i].slash_budget > 0 {
                    if let Some(victim) = world.nearest_attacker(i) {
                        world.kills.push(victim);
                        world.agents[i].slash_budget -= 1;
                        world.agents[i].slash_cd = super::SLASH_CD;
                    }
                }
            }
        }

        // ── TERRITORIAL timer (apexes don't pack) — accumulate crowding by a same-rank rival (Manual §5) ───
        let rival = world.transient[i].rival;
        let rival_alive = rival.map_or(false, |r| !world.agents[r].dead);
        if rival_alive {
            world.agents[i].rival_time = (world.agents[i].rival_time + DT).min(super::RIVAL_PATIENCE + 0.5);
        } else {
            world.agents[i].rival_time = (world.agents[i].rival_time - DT * 1.5).max(0.0);
        }
        let fighting_rival = rival_alive && world.agents[i].rival_time >= super::RIVAL_PATIENCE && !mobbed && threat_pos.is_none();
        let rival_pos = if fighting_rival {
            rival.map(|r| (r, world.agents[r].agent.x, world.agents[r].agent.z, world.agents[r].radius))
        } else {
            None
        };
        let bully_pos = if world.agents[i].spooked > 0.0 {
            // guard the stored index against a compacted buffer (see decide_manual's note) — dangles otherwise.
            world.agents[i].bully.filter(|&b| b < world.agents.len() && !world.agents[b].dead).map(|b| (world.agents[b].agent.x, world.agents[b].agent.z))
        } else {
            None
        };

        // ── carcass / fish feasibility (gathered like Manual; the scorer decides whether to act on them) ───
        let carrion_pos = if a_hunts && world.agents[i].hungry && !mobbed && threat_pos.is_none() && !fighting_rival && world.agents[i].spooked <= 0.0 {
            let mut scratch = std::mem::take(&mut world.seek_neighbors);
            let found = world.nearest_carrion(ax, az, super::SCAVENGE_R, &mut scratch);
            world.seek_neighbors = scratch;
            found
        } else {
            None
        };
        let fish_pos = if kind == Kind::Cat && !mobbed && threat_pos.is_none() && !fighting_rival && world.agents[i].spooked <= 0.0 {
            world.nearest_fish(ax, az, super::LURE_R)
        } else {
            None
        };

        // ── lone-apex player pressure eligibility (Manual §5 hunt-player) ──────────────────────────────────
        let mut menace_ok = false;
        let mut menace_frac = 0.0;
        if !world.player_immune && rank >= 4 && crowd < 3 && a_hunts && world.agents[i].hungry && can_sprint && !mobbed && world.agents[i].spooked <= 0.0 && threat_pos.is_none() {
            let dp2 = (px - ax).powi(2) + (pz - az).powi(2);
            let reach = 15.0 * (1.0 + 0.6 * world.night);
            let prey_d2 = prey_info.map_or(f64::INFINITY, |(_, prx, prz, _)| (prx - ax).powi(2) + (prz - az).powi(2));
            if dp2 < reach * reach && dp2 < prey_d2 * 0.81 {
                menace_ok = true;
                menace_frac = 1.0 - dp2.sqrt() / reach;
            }
        }

        // when mobbed with no real threat, the swarm's centroid IS the thing to flee — feed it as the threat so
        // the scorer weighs BREAK-AWAY (flee) against COMMITTING to the kill (hunt), per the agent's drives.
        let (flee_x, flee_z, flee_frac, has_flee) = if let Some((tx, tz)) = threat_pos {
            let d = threat_d2.unwrap_or(danger2).sqrt().max(0.1);
            (tx, tz, (1.0 - d / danger2.sqrt()).clamp(0.0, 1.0), true)
        } else if let Some((bx, bz)) = bully_pos {
            (bx, bz, 1.0, true)
        } else if mobbed {
            let mc = world.transient[i].mob_count.max(1) as f64;
            (world.transient[i].mob_x / mc, world.transient[i].mob_z / mc, 1.0, true)
        } else {
            (0.0, 0.0, 0.0, false)
        };

        // a fellow of its kind to gather toward when its social drive bites (nearest same-kind flock neighbour)
        let fellow_pos = if crowd < 4 { nearest_fellow(world, i, ax, az, kind) } else { None };

        // ── score the primitives ─────────────────────────────────────────────────────────────────────────
        let is_carnivore = a_hunts;
        let needs = Needs::assess(world.agents[i].energy, world.agents[i].stamina, flee_frac, crowd, is_carnivore, world.agents[i].hungry);
        // how close the chosen prey is (0 far … 1 adjacent) → the Hunt commit bonus, mirroring the manual lunge
        let prey_close = prey_info.map_or(0.0, |(_, prx, prz, _)| (1.0 - (prx - ax).hypot(prz - az) / super::SEEK).clamp(0.0, 1.0));
        let opts = Options {
            threat: has_flee && (threat_pos.is_some() || mobbed),
            threat_frac: flee_frac,
            bully: bully_pos.is_some(),
            prey: prey_info.is_some(),
            prey_close,
            carrion: carrion_pos.is_some(),
            menace_player: menace_ok,
            rival: rival_pos.is_some(),
            fish: fish_pos.is_some(),
            fellow: fellow_pos.is_some(),
            exhausted: is_carnivore && world.agents[i].stamina <= super::CAN_SPRINT && prey_info.is_none() && !mobbed && threat_pos.is_none(),
        };
        let mut action = utility::choose(&needs, &world.agents[i].weights, &opts);
        // a badly wounded hunter never commits through a mob — it always breaks away (Manual's health>HURT_AT gate)
        if mobbed && action == Primitive::Hunt && world.agents[i].health <= super::HURT_AT {
            action = Primitive::Flee;
        }

        // ── apply the chosen primitive ───────────────────────────────────────────────────────────────────
        let mut hunting = false;
        match action {
            Primitive::Flee if has_flee => {
                let (mut ux, mut uz) = unit(ax - flee_x, az - flee_z);
                // a fleeing PERSON curves toward the nearest house (home = safety), unless it lies past the hunter
                if matches!(kind, Kind::Person) {
                    if let Some((hx, hz)) = world.nearest_refuge(ax, az, super::REFUGE_R) {
                        let (rux, ruz) = unit(hx - ax, hz - az);
                        if rux * ux + ruz * uz > -0.2 {
                            let (bx, bz) = unit(ux + rux * super::REFUGE_PULL, uz + ruz * super::REFUGE_PULL);
                            ux = bx;
                            uz = bz;
                        }
                    }
                }
                world.forces[i].0 += ux * a_max * super::FLEE_W;
                world.forces[i].1 += uz * a_max * super::FLEE_W;
                world.behave[i] = (if can_sprint { super::FLEE_BOOST } else { 1.0 }, true);
            }
            Primitive::RivalFight => {
                if let Some((r, rx, rz, rr)) = rival_pos {
                    let (ux, uz) = unit(rx - ax, rz - az);
                    world.forces[i].0 += ux * a_max * super::CHASE_W;
                    world.forces[i].1 += uz * a_max * super::CHASE_W;
                    world.behave[i] = (if can_sprint { super::CHASE_BOOST } else { 1.0 }, true);
                    let d = (rx - ax).hypot(rz - az);
                    if d < radius + rr + super::CONTACT_PAD {
                        world.agents[i].health = (world.agents[i].health - super::RIVAL_DPS * DT).max(0.0);
                        if world.agents[i].health < super::HURT_AT {
                            world.agents[i].spooked = world.agents[i].spooked.max(2.5);
                            world.agents[i].bully = Some(r);
                        }
                    }
                }
            }
            Primitive::MenacePlayer => {
                hunting = true;
                danger_now = danger_now.max(menace_frac);
                let (ux, uz) = unit(px - ax, pz - az);
                let close = (px - ax).powi(2) + (pz - az).powi(2) < hunt2;
                world.forces[i].0 += ux * a_max * super::CHASE_W;
                world.forces[i].1 += uz * a_max * super::CHASE_W;
                world.behave[i] = (if close || can_sprint { super::CHASE_BOOST } else { 1.0 }, true);
            }
            Primitive::Hunt => {
                if let Some((p, prx, prz, pr)) = prey_info {
                    let dx = prx - ax;
                    let dz = prz - az;
                    let d = dx.hypot(dz).max(0.1);
                    let close = d * d < hunt2;
                    world.forces[i].0 += (dx / d) * a_max * super::CHASE_W;
                    world.forces[i].1 += (dz / d) * a_max * super::CHASE_W;
                    world.behave[i] = (if close || can_sprint { super::CHASE_BOOST } else { 1.0 }, true);
                    if close && d < radius + pr + super::CONTACT_PAD {
                        let finishing = world.agents[p].health <= super::STRIKE_DMG; // else just a deep wound (struggle)
                        world.agents[p].health = (world.agents[p].health - super::STRIKE_DMG).max(0.0);
                        world.agents[p].spooked = world.agents[p].spooked.max(2.0); // wounded → it bolts
                        if !finishing {
                            // a wounding bite — keep chasing to finish it; no meal yet
                        } else {
                        world.kills.push(p);
                        world.events.extend_from_slice(&[super::EV_KILL, world.agents[p].kind as usize as f32, world.agents[p].agent.x as f32, world.agents[p].agent.z as f32]);
                        world.agents[i].meals += 1;
                        world.agents[i].fed_meat = super::MEAT_SATED; // meat meal → people can breed a while (no-op for others)
                        world.agents[i].feeding = super::FEED_SECS; // hunker down + eat (no fidget) a few seconds
                        world.agents[i].chase_ox = f64::NAN;
                        world.agents[i].energy = (world.agents[i].energy + super::EAT_ENERGY).min(1.0);
                        if eco(kind).full_after.map_or(false, |fa| world.agents[i].meals >= fa) {
                            world.agents[i].stamina = world.agents[i].stamina.min(0.15);
                            world.agents[i].asleep = true;
                            world.agents[i].sleep_timer = sleep_secs(kind);
                        } else {
                            world.agents[i].stamina = (world.agents[i].stamina + super::EAT_GAIN).min(1.0);
                        }
                        }
                    }
                }
            }
            Primitive::Scavenge => {
                if let Some((cx, cz, ci)) = carrion_pos {
                    let dx = cx - ax;
                    let dz = cz - az;
                    let d = dx.hypot(dz).max(0.1);
                    let contact = radius + world.agents[ci].radius + super::CONTACT_PAD + 0.3;
                    if d < contact {
                        // AT the carcass → SETTLE and feed: zero approach force kills the overshoot-then-re-aim ORBIT
                        // that made cats fidget on rabbit corpses; pursuing flag held so the idle FSM can't frolic on it.
                        world.behave[i] = (1.0, true);
                        world.agents[i].energy = (world.agents[i].energy + super::SCAVENGE_GAIN * DT).min(1.0);
                        world.agents[ci].carrion = (world.agents[ci].carrion - super::SCAVENGE_DRAIN * DT).max(0.0);
                    } else {
                        world.forces[i].0 += (dx / d) * a_max * super::CHASE_W * 0.7;
                        world.forces[i].1 += (dz / d) * a_max * super::CHASE_W * 0.7;
                        world.behave[i] = (1.0, true);
                    }
                }
            }
            Primitive::Drink => {
                if let Some((fx, fz)) = fish_pos {
                    let (ux, uz) = unit(fx - ax, fz - az);
                    world.forces[i].0 += ux * a_max * super::CHASE_W * 0.6;
                    world.forces[i].1 += uz * a_max * super::CHASE_W * 0.6;
                    world.behave[i] = (1.0, true);
                }
            }
            Primitive::Follow => {
                if let Some((fx, fz)) = fellow_pos {
                    let (ux, uz) = unit(fx - ax, fz - az);
                    world.forces[i].0 += ux * a_max * super::BAND_SEEK_W * 0.6; // gentle gather (below a flee/chase)
                    world.forces[i].1 += uz * a_max * super::BAND_SEEK_W * 0.6;
                    world.behave[i].1 = true;
                }
            }
            // Rest / Wander / (Flee with no target) — let the steering wander + the metabolism pass recover/doze.
            _ => {}
        }
        world.agents[i].hunting = hunting;

        // ── PLAYER REACTION — scatter + give a berth (shared with Manual; skipped when menacing you / your pet)
        if !hunting && !world.agents[i].companion {
            let skittish = ((5.0 - rank as f64) / 4.0).max(0.0);
            if skittish > 0.0 {
                let dx = ax - px;
                let dz = az - pz;
                let d = dx.hypot(dz);
                let scare_r = (2.5 + (pspeed - 3.0).max(0.0) * 0.5) * (0.6 + 0.7 * skittish) * (1.0 + 0.4 * world.night);
                if d < scare_r && d > 0.01 {
                    let w = skittish * (1.0 - d / scare_r);
                    world.forces[i].0 += (dx / d) * a_max * super::FLEE_W * w;
                    world.forces[i].1 += (dz / d) * a_max * super::FLEE_W * w;
                    world.behave[i].1 = true;
                    if can_sprint && w > 0.25 {
                        world.behave[i].0 = world.behave[i].0.max(super::FLEE_BOOST);
                    }
                }
            }
            let adx = ax - px;
            let adz = az - pz;
            let ad = adx.hypot(adz);
            let avoid_r = radius + 1.5;
            if ad < avoid_r && ad > 0.01 {
                let w = 1.0 - ad / avoid_r;
                world.forces[i].0 += (adx / ad) * a_max * super::AVOID_W * w;
                world.forces[i].1 += (adz / ad) * a_max * super::AVOID_W * w;
            }
        }

        // ── injury limp + senescence frailty (shared with Manual) ─────────────────────────────────────────
        if world.agents[i].health < super::HURT_AT {
            world.behave[i].0 *= super::HURT_SPEED;
        }
        if !world.agents[i].companion {
            let life = world.agents[i].age / world.agents[i].lifespan.max(1.0);
            if life > super::FRAIL_ONSET {
                let t = ((life - super::FRAIL_ONSET) / (1.0 - super::FRAIL_ONSET)).min(1.0);
                world.behave[i].0 *= 1.0 - t * (1.0 - super::FRAIL_MIN);
            }
        }
    }

    danger_now
}

/// Unit vector of (dx, dz), guarding the degenerate zero case.
fn unit(dx: f64, dz: f64) -> (f64, f64) {
    let d = dx.hypot(dz).max(0.1);
    (dx / d, dz / d)
}

/// Nearest LIVE same-kind flock neighbour of agent `i` (for the Follow / gather primitive).
fn nearest_fellow(world: &World, i: usize, ax: f64, az: f64, kind: Kind) -> Option<(f64, f64)> {
    let agents = &world.agents;
    let mut best: Option<(f64, f64)> = None;
    let mut best_d2 = f64::INFINITY;
    world.grid.for_each_neighbor(ax, az, |ju| {
        let j = ju as usize;
        if j == i || agents[j].dead || agents[j].kind != kind {
            return;
        }
        let dx = agents[j].agent.x - ax;
        let dz = agents[j].agent.z - az;
        let d2 = dx * dx + dz * dz;
        if d2 < best_d2 {
            best_d2 = d2;
            best = Some((agents[j].agent.x, agents[j].agent.z));
        }
    });
    best
}
