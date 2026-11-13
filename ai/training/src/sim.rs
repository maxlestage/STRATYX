//! Le simulateur d'auto-jeu : des joueurs aux profils variés (serré, large, passif,
//! agressif, joueur d'équilibre) jouent des mains complètes dans le moteur de règles.
//! Chaque main devient un [`HandRecord`] validé par rejeu, de provenance
//! [`Source::Simulation`].
//!
//! - **Expresso 3-Max** : des sit & go complets à 3 joueurs (500 jetons, niveaux de
//!   blindes rapides), jusqu'au vainqueur ; le tête-à-tête final est donc bien représenté.
//! - **Tournois 6-Max** : des mains de table de tournoi, tapis tirés entre 4 et 80 grosses
//!   blindes, ante de 12,5 %, parfois à 5 ou 4 joueurs.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_poker::game::{Action, Ante, Blinds, HandSetup, Phase};
use stratyx_poker::history::{HandEvent, HandMeta, HandRecord, SeatRecord};
use stratyx_poker::{Card, CardSet, GameFormat, HandState, HoleCards, Street};
use stratyx_shared::{Chips, Observed, Source};
use stratyx_strategy::charts::{Chart, Node};
use stratyx_strategy::classes::{self, draw};
use stratyx_strategy::villain::{call_shove_fraction, open_fraction, shove_fraction};
use stratyx_strategy::{Spot, mc};

/// Le style d'un joueur simulé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub name: &'static str,
    /// Part des mains jouées depuis le bouton à 6 (adaptée à la position).
    pub vpip: f32,
    /// Part des mains jouées qui sont relancées plutôt que limpées.
    pub pfr: f32,
    /// Part des mains qui 3-bettent.
    pub three_bet: f32,
    /// Probabilité de miser une main forte après le flop.
    pub aggression: f32,
    /// Probabilité de bluffer une main faible.
    pub bluff: f32,
    /// Marge d'équité exigée au-delà de la cote pour suivre (négative : suit trop).
    pub call_margin: f32,
    /// En dessous de ce tapis (BB), il joue push/fold.
    pub shove_depth: f32,
    /// Joue l'équilibre push/fold quand il est court.
    pub nash: bool,
}

pub const PROFILES: [Profile; 6] = [
    Profile {
        name: "serré",
        vpip: 0.16,
        pfr: 0.8,
        three_bet: 0.04,
        aggression: 0.55,
        bluff: 0.05,
        call_margin: 0.08,
        shove_depth: 10.0,
        nash: false,
    },
    Profile {
        name: "régulier",
        vpip: 0.26,
        pfr: 0.85,
        three_bet: 0.08,
        aggression: 0.7,
        bluff: 0.18,
        call_margin: 0.02,
        shove_depth: 14.0,
        nash: true,
    },
    Profile {
        name: "large agressif",
        vpip: 0.36,
        pfr: 0.8,
        three_bet: 0.12,
        aggression: 0.8,
        bluff: 0.32,
        call_margin: 0.0,
        shove_depth: 14.0,
        nash: false,
    },
    Profile {
        name: "calling station",
        vpip: 0.48,
        pfr: 0.25,
        three_bet: 0.025,
        aggression: 0.35,
        bluff: 0.04,
        call_margin: -0.12,
        shove_depth: 8.0,
        nash: false,
    },
    Profile {
        name: "maniaque",
        vpip: 0.62,
        pfr: 0.7,
        three_bet: 0.2,
        aggression: 0.85,
        bluff: 0.45,
        call_margin: -0.06,
        shove_depth: 18.0,
        nash: false,
    },
    Profile {
        name: "équilibre",
        vpip: 0.3,
        pfr: 0.9,
        three_bet: 0.09,
        aggression: 0.65,
        bluff: 0.22,
        call_margin: 0.03,
        shove_depth: 16.0,
        nash: true,
    },
];

/// Tirage pondéré d'un profil : la population compte plus de réguliers que de maniaques.
fn pick_profile(rng: &mut ChaCha8Rng) -> Profile {
    const WEIGHTS: [f32; 6] = [0.18, 0.26, 0.14, 0.16, 0.08, 0.18];
    let mut x = rng.random::<f32>();
    for (p, w) in PROFILES.iter().zip(WEIGHTS) {
        if x < w {
            return *p;
        }
        x -= w;
    }
    PROFILES[1]
}

/// La décision d'un joueur simulé.
fn decide(
    state: &HandState,
    player: usize,
    hole: HoleCards,
    profile: &Profile,
    chart: Option<&Chart>,
    rng: &mut ChaCha8Rng,
) -> Action {
    let Some(legal) = state.legal_actions() else {
        return Action::Fold;
    };
    let spot = Spot::from_state(state, player);
    let bb = state.blinds().big_blind.get().max(1);
    let me = &state.players()[player];
    let all_in_to = (me.stack + me.street_bet).get();
    let to_call = legal.call.map_or(0, Chips::get);
    let check_or_fold = if legal.check {
        Action::Check
    } else {
        Action::Fold
    };
    let sized = |to: u64| -> Action {
        match legal.bet.or(legal.raise) {
            Some((min, max)) => {
                let to = to.clamp(min.get(), max.get());
                if to as f32 >= 0.8 * max.get() as f32 {
                    Action::AllIn
                } else if legal.bet.is_some() {
                    Action::Bet(Chips(to))
                } else {
                    Action::Raise(Chips(to))
                }
            }
            None => {
                if to_call > 0 {
                    Action::Call
                } else {
                    Action::Check
                }
            }
        }
    };
    let class = classes::hole_class(hole);
    let pct = classes::percentile(class);
    // Un peu de bruit : un même joueur ne joue pas deux fois la même main à l'identique.
    let noise = 1.0 + (rng.random::<f32>() - 0.5) * 0.3;

    if spot.street == Street::Preflop {
        let order = usize::from(spot.preflop_order);
        let n = usize::from(spot.players);
        let behind = usize::from(spot.behind);
        let short = spot.effective_bb <= profile.shove_depth;
        let facing_shove = spot.facing_all_in && spot.raises == 1;
        if short && (spot.unopened || facing_shove) {
            let p = match chart {
                Some(c) if profile.nash => {
                    let node = if spot.unopened {
                        Node::FirstIn
                    } else {
                        let shover = state
                            .actions()
                            .iter()
                            .find(|a| a.street == Street::Preflop && a.kind.is_aggressive())
                            .map_or(0, |a| {
                                let off = (a.player + n - state.button()) % n;
                                if n == 2 { off } else { (off + n - 3) % n }
                            });
                        if spot.callers == 0 {
                            Node::VsShove { shover }
                        } else {
                            Node::VsShoveAndCall { shover }
                        }
                    };
                    c.probability(node, spot.effective_bb, order, n, class)
                }
                _ => {
                    let frac = if spot.unopened {
                        shove_fraction(spot.effective_bb, behind) * (profile.vpip / 0.26).sqrt()
                    } else {
                        call_shove_fraction(spot.effective_bb) * (1.0 - profile.call_margin * 3.0)
                    };
                    f32::from(u8::from(pct <= frac * noise))
                }
            };
            return if rng.random::<f32>() < p {
                if spot.unopened {
                    Action::AllIn
                } else {
                    Action::Call
                }
            } else {
                check_or_fold
            };
        }
        let position_scale = open_fraction(behind) / open_fraction(2);
        if spot.raises == 0 {
            let play = (profile.vpip * position_scale * noise).min(0.95);
            let raise = play * profile.pfr;
            if pct <= raise {
                let limpers = u64::from(spot.callers);
                return sized(bb * 5 / 2 + limpers * bb);
            }
            if pct <= play && to_call <= bb {
                return if to_call == 0 {
                    Action::Check
                } else {
                    Action::Call
                };
            }
            return check_or_fold;
        }
        let current = state.current_bet().get();
        if spot.raises == 1 {
            if pct <= profile.three_bet * noise {
                if spot.effective_bb < 25.0 {
                    return Action::AllIn;
                }
                return sized(current * 3 + u64::from(spot.callers) * current);
            }
            let defend = if spot.behind == 0 { 1.6 } else { 1.0 };
            let call_frac = profile.vpip * 0.55 * defend * noise
                + (0.3 - spot.pot_odds()).max(0.0) * 0.5
                - profile.call_margin;
            if pct <= call_frac.max(0.02) && to_call as f32 <= all_in_to as f32 * 0.4 {
                return Action::Call;
            }
            if pct <= call_frac.max(0.02) * 0.5 {
                return Action::Call;
            }
            return check_or_fold;
        }
        // 3-bet et plus.
        if pct <= profile.three_bet * 0.35 * noise {
            return Action::AllIn;
        }
        if pct <= profile.three_bet * 0.9 * noise {
            return Action::Call;
        }
        return check_or_fold;
    }

    // Après le flop : équité contre des mains au hasard.
    let opponents = usize::from(spot.active.saturating_sub(1)).max(1);
    let ranges = vec![stratyx_poker::Range::empty(); opponents];
    let eq = mc::equity(hole, &ranges, state.board(), 80, rng.random());
    let pot = state.pot_total().get();
    let strong = 0.55 + 0.05 * opponents as f32;
    if to_call == 0 {
        if eq > strong && rng.random::<f32>() < profile.aggression {
            return sized((pot * 2).div_ceil(3).max(bb));
        }
        if eq < 0.35 && rng.random::<f32>() < profile.bluff * 0.6 {
            return sized((pot / 2).max(bb));
        }
        return Action::Check;
    }
    if eq > strong + 0.2 && rng.random::<f32>() < profile.aggression * 0.6 {
        return sized(state.current_bet().get() * 3);
    }
    if eq > spot.pot_odds() + profile.call_margin + 0.02 * (noise - 1.0) * 10.0 {
        return Action::Call;
    }
    if rng.random::<f32>() < profile.bluff * 0.12 {
        return sized(state.current_bet().get() * 3);
    }
    Action::Fold
}

/// Une table simulée : sièges, profils, tapis.
#[derive(Debug)]
pub struct Table {
    pub format: GameFormat,
    pub blinds: Blinds,
    pub button_seat: u8,
    /// (siège, nom, tapis, profil).
    pub players: Vec<(u8, String, u64, Profile)>,
}

/// Joue une main complète ; renvoie l'historique et les gains nets de chaque joueur.
pub fn play_hand(
    table: &Table,
    id: String,
    chart: Option<&Chart>,
    rng: &mut ChaCha8Rng,
) -> Option<(HandRecord, Vec<i64>)> {
    let n = table.players.len();
    let button = table
        .players
        .iter()
        .position(|p| p.0 == table.button_seat)?;
    let setup = HandSetup::new(
        table.players.iter().map(|p| Chips(p.2)).collect(),
        button,
        table.blinds,
    );
    let mut state = HandState::new(&setup).ok()?;
    let mut used = CardSet::EMPTY;
    let holes: Vec<HoleCards> = (0..n)
        .map(|_| {
            let a = draw(&mut used, rng);
            let b = draw(&mut used, rng);
            HoleCards::new(a, b)
        })
        .collect::<Option<_>>()?;
    let mut events = Vec::new();
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 200 {
            return None;
        }
        match state.phase() {
            Phase::Betting => {
                let p = state.to_act()?;
                let action = decide(&state, p, holes[p], &table.players[p].3, chart, rng);
                let action = if state.clone().apply(p, action).is_ok() {
                    action
                } else if state.legal_actions().is_some_and(|l| l.check) {
                    Action::Check
                } else {
                    Action::Fold
                };
                state.apply(p, action).ok()?;
                events.push(sim(HandEvent::Action {
                    seat: table.players[p].0,
                    action,
                }));
            }
            Phase::AwaitingBoard(street) => {
                let k = street.cards_dealt();
                let cards: Vec<Card> = (0..k).map(|_| draw(&mut used, rng)).collect();
                state.deal(&cards).ok()?;
                events.push(sim(HandEvent::Board { cards }));
            }
            Phase::Showdown | Phase::Finished => break,
        }
    }
    let record = HandRecord {
        id,
        format: table.format,
        blinds: sim(table.blinds),
        button_seat: table.button_seat,
        seats: table
            .players
            .iter()
            .zip(&holes)
            .map(|((seat, name, stack, _), h)| SeatRecord {
                seat: *seat,
                player: name.clone(),
                stack: sim(Chips(*stack)),
                hole_cards: Some(sim(*h)),
            })
            .collect(),
        events,
        meta: HandMeta {
            provenance: Some("simulation (auto-jeu)".to_owned()),
            ..HandMeta::default()
        },
    };
    let replay = record.replay().ok()?;
    let net = match replay.outcome {
        stratyx_poker::history::Outcome::Settled(s) => s.net,
        _ => return None,
    };
    Some((record, net))
}

fn sim<T>(v: T) -> Observed<T> {
    Observed::certain(v, Source::Simulation)
}

/// Les niveaux de blindes d'une Expresso (petite / grosse blinde).
const EXPRESSO_LEVELS: [(u64, u64); 12] = [
    (10, 20),
    (15, 30),
    (20, 40),
    (25, 50),
    (30, 60),
    (40, 80),
    (50, 100),
    (60, 120),
    (80, 160),
    (100, 200),
    (125, 250),
    (150, 300),
];

/// Simule `count` mains d'Expresso (des sit & go complets à 3 joueurs).
#[must_use]
pub fn expresso(count: usize, seed: u64, chart: Option<&Chart>) -> Vec<HandRecord> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(count);
    let mut tournament = 0u32;
    while out.len() < count {
        tournament += 1;
        let mut players: Vec<(u8, String, u64, Profile)> = (1..=3)
            .map(|s| {
                (
                    s,
                    format!("E{tournament}-J{s}"),
                    500,
                    pick_profile(&mut rng),
                )
            })
            .collect();
        let mut button = rng.random_range(1..=3u8);
        let hands_per_level = rng.random_range(4..=7usize);
        let mut hand = 0usize;
        while players.len() > 1 && out.len() < count && hand < 400 {
            let (sb, bb) = EXPRESSO_LEVELS[(hand / hands_per_level).min(EXPRESSO_LEVELS.len() - 1)];
            // Le bouton avance au prochain joueur encore en jeu.
            if !players.iter().any(|p| p.0 == button) {
                button = players[0].0;
            }
            let table = Table {
                format: GameFormat::EXPRESSO,
                blinds: Blinds::new(sb, bb),
                button_seat: button,
                players: players.clone(),
            };
            let id = format!("sim-expresso-{seed:x}-{tournament}-{hand}");
            if let Some((record, net)) = play_hand(&table, id, chart, &mut rng) {
                for (p, d) in players.iter_mut().zip(&net) {
                    p.2 = (p.2 as i64 + d).max(0) as u64;
                }
                out.push(record);
            }
            players.retain(|p| p.2 > 0);
            let pos = players.iter().position(|p| p.0 > button).unwrap_or(0);
            if let Some(p) = players.get(pos) {
                button = p.0;
            }
            hand += 1;
        }
    }
    out
}

/// Simule `count` mains de table de tournoi 6-max.
#[must_use]
pub fn tournament_6max(count: usize, seed: u64, chart: Option<&Chart>) -> Vec<HandRecord> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(count);
    let mut i = 0u64;
    while out.len() < count {
        i += 1;
        let seated = match rng.random::<f32>() {
            x if x < 0.8 => 6,
            x if x < 0.92 => 5,
            _ => 4,
        };
        let bb: u64 = [100, 200, 300, 400, 600, 800, 1000, 1500, 2000][rng.random_range(0..9)];
        let blinds = Blinds::new(bb / 2, bb).with_ante(Ante::Classic(Chips(bb / 8)));
        let mut seats: Vec<u8> = (1..=6).collect();
        while seats.len() > seated {
            let k = rng.random_range(0..seats.len());
            seats.remove(k);
        }
        let players: Vec<(u8, String, u64, Profile)> = seats
            .iter()
            .map(|s| {
                let depth = (rng.random_range(4.0f32.ln()..80.0f32.ln())).exp();
                let stack = ((depth * bb as f32) as u64).max(bb);
                (*s, format!("T{i}-J{s}"), stack, pick_profile(&mut rng))
            })
            .collect();
        let table = Table {
            format: GameFormat::TOURNAMENT_6MAX,
            blinds,
            button_seat: seats[rng.random_range(0..seats.len())],
            players,
        };
        if let Some((record, _)) =
            play_hand(&table, format!("sim-6max-{seed:x}-{i}"), chart, &mut rng)
        {
            out.push(record);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_hands_replay() {
        let hands = expresso(60, 3, None);
        assert_eq!(hands.len(), 60);
        for h in &hands {
            assert!(h.replay().is_ok(), "{}", h.id);
        }
        let hands = tournament_6max(40, 4, None);
        assert_eq!(hands.len(), 40);
        assert!(hands.iter().all(|h| h.replay().is_ok()));
    }
}
