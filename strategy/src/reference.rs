//! La décision de référence d'un spot, **indépendante du résultat de la main** (cahier des
//! charges, section 6).
//!
//! - Tapis courts au préflop, personne n'a misé avant (ou face à un tapis) : l'équilibre
//!   push/fold du format ([`crate::pushfold`]).
//! - Sinon, une **référence EV** : équité contre les ranges probables des adversaires
//!   ([`crate::villain`]), cotes du pot, fold equity (modèle de population quand il est
//!   fourni) et réalisation de l'équité selon la position. Toutes les EV sont en grosses
//!   blindes, relativement à « se coucher » (EV 0).

use serde::{Deserialize, Serialize};
use stratyx_poker::game::{Action, PlayerStatus};
use stratyx_poker::{HandState, HoleCards, Range, Street};
use stratyx_shared::Chips;

use crate::mc;
use crate::model::Classifier;
use crate::preflop;
use crate::pushfold::{Chart, Node};
use crate::spot::{Decision, Spot};
use crate::villain;

/// D'où vient la référence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Équilibre push/fold.
    PushFold,
    /// Référence EV (équité, cotes, fold equity).
    Ev,
}

/// La référence d'un spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub basis: Basis,
    pub best: Decision,
    /// EV de chaque décision possible (`None` : impossible), en grosses blindes.
    pub ev: [Option<f32>; 4],
    /// Équité contre les ranges des adversaires en jeu.
    pub equity: f32,
    /// Équité nécessaire pour suivre.
    pub pot_odds: f32,
    /// Probabilité que tous les adversaires se couchent face à la relance de référence.
    pub fold_equity: f32,
    /// Montant (total « à ») de la relance évaluée.
    pub raise_to: Option<u64>,
    /// Probabilité d'équilibre de l'action agressive (push/fold seulement).
    pub equilibrium: Option<f32>,
    /// Part des mains dans la range estimée de chaque adversaire en jeu.
    pub villain_ranges: Vec<(usize, f32)>,
}

impl Reference {
    /// L'EV perdue (≥ 0) en jouant `d` plutôt que la meilleure décision.
    #[must_use]
    pub fn loss(&self, d: Decision) -> f32 {
        let best = self.ev[self.best.index()].unwrap_or(0.0);
        match self.ev[d.index()] {
            Some(v) => (best - v).max(0.0),
            None => 0.0,
        }
    }
}

/// Ce dont la référence a besoin.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    pub chart: Option<&'a Chart>,
    pub population: Option<&'a Classifier>,
    /// Tirages Monte-Carlo par calcul d'équité.
    pub samples: usize,
    pub seed: u64,
}

/// Probabilité qu'un adversaire se couche face à une agression, sans modèle.
fn default_fold(spot: &Spot) -> f32 {
    if spot.facing_all_in {
        return 0.6;
    }
    match (spot.street, spot.raises) {
        (Street::Preflop, 0 | 1) if spot.unopened => 0.7,
        (Street::Preflop, 1) => {
            if spot.behind == 0 {
                0.5
            } else {
                0.75
            }
        }
        (Street::Preflop, 2) => 0.55,
        (Street::Preflop, _) => 0.5,
        (_, 0) => 0.45,
        (_, 1) => 0.5,
        _ => 0.45,
    }
}

/// Probabilité qu'un adversaire se couche, d'après le modèle de population s'il existe.
fn fold_probability(state: &HandState, v: usize, population: Option<&Classifier>) -> f32 {
    let spot = Spot::from_state(state, v);
    match population {
        Some(m) => m.predict(&spot.context_features())[Decision::Fold.index()].clamp(0.02, 0.98),
        None => default_fold(&spot),
    }
}

/// La relance de référence (total « à ») : 2,5 BB pour ouvrir (+1 par limper), 3× (en
/// position) ou 3,5× une relance, deux tiers du pot après le flop, 3× une mise.
fn default_raise_to(state: &HandState, spot: &Spot) -> u64 {
    let bb = state.blinds().big_blind.get();
    let current = state.current_bet().get();
    let pot = state.pot_total().get();
    if spot.street == Street::Preflop {
        if spot.unopened || spot.raises == 0 {
            let limpers = u64::from(spot.callers);
            return bb * 5 / 2 + limpers * bb;
        }
        let mult = if spot.in_position { 3.0 } else { 3.5 };
        return (current as f32 * mult) as u64 + u64::from(spot.callers) * current;
    }
    if current == 0 {
        (pot * 2).div_ceil(3).max(bb)
    } else {
        current * 3
    }
}

/// Calcule la référence du joueur qui parle dans `state`, qui a `hole`.
/// `actual_raise_to` : si le joueur a relancé, le montant joué (évalué à la place du
/// montant de référence).
#[must_use]
pub fn reference(
    state: &HandState,
    hole: HoleCards,
    inputs: &Inputs<'_>,
    actual_raise_to: Option<u64>,
) -> Option<Reference> {
    let player = state.to_act()?;
    let legal = state.legal_actions()?;
    let spot = Spot::from_state(state, player);
    let bb = state.blinds().big_blind.get().max(1) as f32;
    let me = &state.players()[player];
    let to_call = legal.call.map_or(0, Chips::get);
    let pot = state.pot_total().get();
    let class = preflop::hole_class(hole);
    let opponents: Vec<usize> = (0..state.players().len())
        .filter(|i| *i != player && state.players()[*i].status != PlayerStatus::Folded)
        .collect();
    let ranges: Vec<Range> = opponents
        .iter()
        .map(|v| villain::range_of(state, *v, inputs.chart))
        .collect();
    let villain_ranges: Vec<(usize, f32)> = opponents
        .iter()
        .zip(&ranges)
        .map(|(v, r)| (*v, r.fraction() as f32))
        .collect();

    // Push/fold : préflop, tapis courts, personne n'a ouvert (ou un tapis à suivre). Une
    // relance qui n'est pas un tapis sort du cadre push/fold : elle passe par la référence EV.
    let all_in_total = (me.stack + me.street_bet).get();
    let small_raise = actual_raise_to.is_some_and(|t| t < all_in_total);
    if let Some(chart) = inputs.chart
        && !small_raise
        && spot.street == Street::Preflop
        && spot.effective_bb <= chart.max_depth() * 1.15
    {
        let node = if spot.unopened {
            Some(Node::FirstIn)
        } else if spot.facing_all_in && spot.raises == 1 {
            let n = state.players().len();
            let shover = state
                .actions()
                .iter()
                .find(|a| a.street == Street::Preflop && a.kind.is_aggressive())
                .map_or(0, |a| {
                    let off = (a.player + n - state.button()) % n;
                    if n == 2 { off } else { (off + n - 3) % n }
                });
            Some(if spot.callers == 0 {
                Node::VsShove { shover }
            } else {
                Node::VsShoveAndCall { shover }
            })
        } else {
            None
        };
        if let Some(node) = node {
            let p = chart.probability(
                node,
                spot.effective_bb,
                usize::from(spot.preflop_order),
                usize::from(spot.players),
                class,
            );
            let aggressive = if node == Node::FirstIn {
                Decision::AllIn
            } else {
                Decision::Passive
            };
            // Grosse blinde non relancée : parler ne coûte rien.
            let passive_free = node == Node::FirstIn && to_call == 0;
            let equity = mc::equity(hole, &ranges, state.board(), inputs.samples, inputs.seed);
            // Les rangs préflop de ceux qui ont déjà fait tapis ou suivi.
            let n = state.players().len();
            let pre_rank = |i: usize| {
                let off = (i + n - state.button()) % n;
                if n == 2 { off } else { (off + n - 3) % n }
            };
            let shoved: Vec<usize> = state
                .actions()
                .iter()
                .filter(|a| {
                    a.street == Street::Preflop
                        && matches!(
                            a.kind,
                            stratyx_poker::game::ActionKind::Raise
                                | stratyx_poker::game::ActionKind::Bet
                                | stratyx_poker::game::ActionKind::Call
                        )
                        && a.kind.is_voluntary()
                })
                .map(|a| pre_rank(a.player))
                .collect();
            let (ev_in, ev_fold) = crate::pushfold::spot_ev(
                chart,
                spot.effective_bb,
                n,
                usize::from(spot.preflop_order),
                &shoved,
                hole,
                inputs.samples.max(1500),
                inputs.seed,
            );
            let mut ev = [Some(0.0), None, None, None];
            let rel = (ev_in - ev_fold) as f32;
            ev[aggressive.index()] = Some(rel);
            if passive_free {
                // Parler en grosse blinde : la main continue, EV au moins nulle.
                ev[Decision::Fold.index()] = None;
                ev[Decision::Passive.index()] = Some(0.0);
            }
            let best = if rel > 0.0 {
                aggressive
            } else if passive_free {
                Decision::Passive
            } else {
                Decision::Fold
            };
            return Some(Reference {
                basis: Basis::PushFold,
                best,
                ev,
                equity,
                pot_odds: spot.pot_odds(),
                fold_equity: 0.0,
                raise_to: (aggressive == Decision::AllIn)
                    .then_some((me.stack + me.street_bet).get()),
                equilibrium: Some(p),
                villain_ranges,
            });
        }
    }

    // Référence EV.
    let board = state.board();
    let equity = mc::equity(hole, &ranges, board, inputs.samples, inputs.seed);
    let realization = match (spot.street, spot.in_position) {
        (Street::River, _) => 1.0,
        (Street::Preflop, true) => 0.92,
        (Street::Preflop, false) => 0.78,
        (_, true) => 0.95,
        (_, false) => 0.85,
    };
    let to_bb = |c: f32| c / bb;
    // Valeur des tours suivants quand le coup continue sans tapis : une main favorite gagne
    // encore une partie du tapis restant (cotes implicites).
    let future = |eq: f32, pot_after: f32, behind: f32| -> f32 {
        let gamma = match spot.street {
            Street::Preflop => 0.2,
            Street::Flop => 0.45,
            Street::Turn => 0.3,
            Street::River => 0.0,
        };
        (2.0 * eq - 1.0).max(0.0) * pot_after.min(behind.max(0.0)) * gamma
    };
    let mut ev: [Option<f32>; 4] = [None; 4];
    ev[Decision::Fold.index()] = Some(if to_call == 0 { f32::NEG_INFINITY } else { 0.0 });
    let behind_after_call = me.stack.get().saturating_sub(to_call) as f32;
    let passive = if to_call == 0 {
        realization * equity * pot as f32
    } else {
        realization * equity * (pot + to_call) as f32 - to_call as f32
    } + future(equity, (pot + 2 * to_call) as f32, behind_after_call);
    ev[Decision::Passive.index()] = Some(to_bb(passive));
    // Sans mise à suivre, se coucher revient à parler : même EV que « parler » au pire.
    if to_call == 0 {
        ev[Decision::Fold.index()] = Some(to_bb(passive).min(0.0));
    }

    let all_in_to = (me.stack + me.street_bet).get();
    let mut fold_equity = 0.0;
    let mut raise_to = None;
    let evaluate_aggression = |to: u64, all_in: bool| -> Option<(f32, f32)> {
        let mut next = state.clone();
        let action = if all_in {
            Action::AllIn
        } else if state.current_bet().is_zero() {
            Action::Bet(Chips(to))
        } else {
            Action::Raise(Chips(to))
        };
        next.apply(player, action).ok()?;
        let added = next.players()[player].invested().get() - me.invested().get();
        // Chaque adversaire répond tour à tour ; on suppose qu'il se couche pour estimer
        // la réponse du suivant.
        let mut fe = 1.0f32;
        let mut probe = next.clone();
        let mut caller: Option<(usize, f32)> = None;
        let mut guard = 0;
        while let Some(v) = probe.to_act() {
            guard += 1;
            if v == player || guard > 12 {
                break;
            }
            let pf = fold_probability(&probe, v, inputs.population);
            if caller.is_none_or(|(_, c)| 1.0 - pf > c) {
                caller = Some((v, 1.0 - pf));
            }
            fe *= pf;
            if probe.apply(v, Action::Fold).is_err() {
                break;
            }
        }
        let (v, cont) = caller?;
        let idx = opponents.iter().position(|o| *o == v)?;
        let cont_range = villain::continuing(&ranges[idx], board, cont.max(0.05));
        let eq_called = mc::equity(
            hole,
            &[cont_range],
            board,
            inputs.samples,
            inputs.seed ^ 0x9e37,
        );
        let villain = &state.players()[v];
        let villain_call = (to.min((villain.stack + villain.street_bet).get()))
            .saturating_sub(villain.street_bet.get());
        let r = if all_in { 1.0 } else { realization.max(0.85) };
        let pot_called = (pot + added + villain_call) as f32;
        let won_if_called = r * eq_called * pot_called;
        // Si le tapis adverse est plus petit, l'excédent revient au joueur.
        let refund = added.saturating_sub(villain_call + to_call) as f32 * (1.0 - r * eq_called);
        let implied = if all_in {
            0.0
        } else {
            let behind = next.players()[player].stack.get().min(villain.stack.get()) as f32;
            future(eq_called, pot_called, behind)
        };
        let ev = fe * pot as f32 + (1.0 - fe) * (won_if_called - added as f32 + refund + implied);
        Some((to_bb(ev), fe))
    };
    if let Some((min, max)) = legal.bet.or(legal.raise) {
        let target = actual_raise_to
            .filter(|t| *t < all_in_to)
            .unwrap_or_else(|| default_raise_to(state, &spot))
            .clamp(min.get(), max.get());
        if target < all_in_to && (target as f32) < 0.7 * all_in_to as f32 {
            if let Some((v, fe)) = evaluate_aggression(target, false) {
                ev[Decision::Raise.index()] = Some(v);
                fold_equity = fe;
                raise_to = Some(target);
            }
        }
    }
    if all_in_to > state.current_bet().get()
        && let Some((v, fe)) = evaluate_aggression(all_in_to, true)
    {
        ev[Decision::AllIn.index()] = Some(v);
        if raise_to.is_none() {
            fold_equity = fe;
        }
    }
    let best = Decision::ALL
        .into_iter()
        .filter(|d| ev[d.index()].is_some_and(f32::is_finite))
        .max_by(|a, b| {
            ev[a.index()]
                .unwrap_or(f32::MIN)
                .total_cmp(&ev[b.index()].unwrap_or(f32::MIN))
                // À EV égale, l'action la plus simple (parole avant mise).
                .then(b.index().cmp(&a.index()))
        })
        .unwrap_or(Decision::Fold);
    Some(Reference {
        basis: Basis::Ev,
        best,
        ev,
        equity,
        pot_odds: spot.pot_odds(),
        fold_equity,
        raise_to,
        equilibrium: None,
        villain_ranges,
    })
}

/// La famille d'une action jouée dans `state` par le joueur qui parle.
#[must_use]
pub fn classify(state: &HandState, action: Action) -> Decision {
    let Some(p) = state.to_act() else {
        return Decision::Fold;
    };
    let me = &state.players()[p];
    let all_in_to = me.stack + me.street_bet;
    match action {
        Action::Fold => {
            if state.current_bet() <= me.street_bet {
                Decision::Passive
            } else {
                Decision::Fold
            }
        }
        Action::Check | Action::Call => Decision::Passive,
        Action::AllIn => {
            if all_in_to <= state.current_bet() {
                Decision::Passive
            } else {
                Decision::AllIn
            }
        }
        Action::Bet(to) | Action::Raise(to) => {
            if to >= all_in_to {
                Decision::AllIn
            } else {
                Decision::Raise
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::game::{Blinds, HandSetup};

    fn inputs() -> Inputs<'static> {
        Inputs {
            chart: None,
            population: None,
            samples: 3000,
            seed: 1,
        }
    }

    #[test]
    fn calling_a_shove_needs_equity() {
        // Tête-à-tête 30 BB : le bouton pousse, la grosse blinde décide.
        let setup = HandSetup::new(vec![Chips(3000), Chips(3000)], 0, Blinds::new(50, 100));
        let mut st = HandState::new(&setup).unwrap();
        st.apply(0, Action::AllIn).unwrap();
        let aa = reference(&st, "AsAh".parse().unwrap(), &inputs(), None).unwrap();
        assert_eq!(aa.best, Decision::Passive);
        let trash = reference(&st, "7c2d".parse().unwrap(), &inputs(), None).unwrap();
        assert_eq!(trash.best, Decision::Fold);
        assert!(trash.loss(Decision::Passive) > 3.0, "{:?}", trash.ev);
        assert!(trash.equity < trash.pot_odds);
    }

    #[test]
    fn strong_hands_bet_for_value() {
        let setup = HandSetup::new(vec![Chips(5000), Chips(5000)], 0, Blinds::new(50, 100));
        let mut st = HandState::new(&setup).unwrap();
        st.apply(0, Action::Raise(Chips(250))).unwrap();
        st.apply(1, Action::Call).unwrap();
        st.deal(&stratyx_poker::card::parse_cards("Kd7c2h").unwrap())
            .unwrap();
        st.apply(1, Action::Check).unwrap();
        let r = reference(&st, "KsKh".parse().unwrap(), &inputs(), None).unwrap();
        assert!(matches!(r.best, Decision::Raise | Decision::AllIn), "{r:?}");
        assert_eq!(classify(&st, Action::Check), Decision::Passive);
        assert_eq!(classify(&st, Action::Bet(Chips(200))), Decision::Raise);
    }
}
