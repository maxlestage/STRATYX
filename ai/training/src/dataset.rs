//! Des mains aux exemples d'apprentissage : chaque décision volontaire d'une main rejouée
//! dans le moteur devient un exemple.
//!
//! - Modèle de **population** : toutes les décisions, caractéristiques du contexte, et
//!   l'action réellement jouée comme étiquette (les tendances des joueurs).
//! - Modèle de **décision** : les décisions dont les cartes sont connues, caractéristiques
//!   du contexte et de la main, et la décision de référence (équilibre push/fold ou
//!   référence EV, indépendante du résultat) comme étiquette.

use rayon::prelude::*;
use stratyx_poker::game::{HandSetup, PlayerStatus};
use stratyx_poker::history::{HandEvent, HandRecord};
use stratyx_poker::{HandState, HoleCards, Range, Street};
use stratyx_strategy::model::Classifier;
use stratyx_strategy::pushfold::Chart;
use stratyx_strategy::{
    Decision, HandFeatures, Inputs, Spot, classify, decision_features, mc, reference,
};

/// Un exemple : caractéristiques, étiquette, et la main d'origine (pour séparer
/// entraînement et validation par main).
#[derive(Debug, Clone)]
pub struct Example {
    pub x: Vec<f32>,
    pub y: usize,
    pub hand: u64,
}

/// Rejoue une main et appelle `visit` avant chaque action (état, joueur, cartes connues,
/// action jouée).
pub fn walk(
    record: &HandRecord,
    mut visit: impl FnMut(&HandState, usize, Option<HoleCards>, stratyx_poker::Action),
) -> bool {
    let mut order: Vec<_> = record.seats.iter().collect();
    order.sort_by_key(|s| s.seat);
    let Some(button) = order.iter().position(|s| s.seat == record.button_seat) else {
        return false;
    };
    let setup = HandSetup::new(
        order.iter().map(|s| s.stack.value).collect(),
        button,
        record.blinds.value,
    )
    .with_structure(record.format.structure);
    let Ok(mut state) = HandState::new(&setup) else {
        return false;
    };
    let mut holes: Vec<Option<HoleCards>> = order
        .iter()
        .map(|s| s.hole_cards.as_ref().map(|h| h.value))
        .collect();
    // Les cartes montrées à l'abattage sont connues dès le début (archives réelles).
    for ev in &record.events {
        if let HandEvent::Show { seat, cards } = &ev.value
            && let Some(i) = order.iter().position(|s| s.seat == *seat)
        {
            holes[i].get_or_insert(*cards);
        }
    }
    for ev in &record.events {
        match &ev.value {
            HandEvent::Action { seat, action } => {
                let Some(p) = order.iter().position(|s| s.seat == *seat) else {
                    return false;
                };
                if state.to_act() == Some(p) {
                    visit(&state, p, holes[p], *action);
                }
                if state.apply(p, *action).is_err() {
                    return false;
                }
            }
            HandEvent::Board { cards } if state.deal(cards).is_err() => return false,
            _ => {}
        }
    }
    true
}

fn hand_hash(id: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in id.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Les caractéristiques de la main du joueur qui parle (équité contre des mains au
/// hasard estimée par Monte-Carlo après le flop).
#[must_use]
pub fn hand_features(
    state: &HandState,
    player: usize,
    hole: HoleCards,
    samples: usize,
    seed: u64,
) -> HandFeatures {
    let opponents = state
        .players()
        .iter()
        .enumerate()
        .filter(|(i, p)| *i != player && p.status != PlayerStatus::Folded)
        .count()
        .max(1);
    let eq = if state.street() == Street::Preflop {
        0.0
    } else {
        mc::equity(
            hole,
            &vec![Range::empty(); opponents],
            state.board(),
            samples,
            seed,
        )
    };
    HandFeatures::new(hole, state.board(), opponents, eq)
}

/// Les exemples du modèle de population.
#[must_use]
pub fn population(records: &[HandRecord]) -> Vec<Example> {
    records
        .par_iter()
        .flat_map_iter(|r| {
            let hand = hand_hash(&r.id);
            let mut out = Vec::new();
            walk(r, |state, p, _, action| {
                out.push(Example {
                    x: Spot::from_state(state, p).context_features(),
                    y: classify(state, action).index(),
                    hand,
                });
            });
            out
        })
        .collect()
}

/// Les exemples du modèle de décision, étiquetés par la référence.
#[must_use]
pub fn decisions(
    records: &[HandRecord],
    chart: &Chart,
    population: Option<&Classifier>,
    samples: usize,
) -> Vec<Example> {
    records
        .par_iter()
        .flat_map_iter(|r| {
            let hand = hand_hash(&r.id);
            let mut out = Vec::new();
            let mut k = 0u64;
            walk(r, |state, p, hole, action| {
                let Some(hole) = hole else { return };
                k += 1;
                let seed = hand ^ (k << 48);
                let inputs = Inputs {
                    chart: Some(chart),
                    population,
                    samples,
                    seed,
                };
                let actual_to = match action {
                    stratyx_poker::Action::Bet(c) | stratyx_poker::Action::Raise(c) => {
                        Some(c.get())
                    }
                    _ => None,
                };
                let Some(refr) = reference(state, hole, &inputs, actual_to) else {
                    return;
                };
                let spot = Spot::from_state(state, p);
                let hf = hand_features(state, p, hole, samples / 2, seed ^ 1);
                out.push(Example {
                    x: decision_features(&spot, &hf),
                    y: refr.best.index(),
                    hand,
                });
            });
            out
        })
        .collect()
}

/// Sépare les exemples par main : environ 20 % des mains vont à la validation.
#[must_use]
pub fn split(examples: Vec<Example>) -> (Vec<Example>, Vec<Example>) {
    examples.into_iter().partition(|e| e.hand % 5 != 0)
}

/// Répartition des étiquettes.
#[must_use]
pub fn label_counts(examples: &[Example]) -> [usize; 4] {
    let mut c = [0usize; 4];
    for e in examples {
        c[e.y.min(3)] += 1;
    }
    c
}

/// Libellés des étiquettes, pour les rapports.
#[must_use]
pub fn label_names() -> [&'static str; 4] {
    [
        Decision::Fold.label(true),
        "parler / suivre",
        "miser / relancer",
        Decision::AllIn.label(true),
    ]
}
