//! L'analyse d'un spot isolé, après la partie : une capture d'écran (ou une image d'une
//! vidéo) donne les cartes, le joueur indique la position, le tapis, le pot et le montant à
//! suivre. Le spot est reconstruit dans le moteur de règles, puis comparé à la référence de
//! l'IA du format (équilibre push/fold ou référence EV) et à son modèle de décision.

use serde::Serialize;
use stratyx_poker::game::{Action, Ante, Blinds, HandSetup, PlayerStatus};
use stratyx_poker::position::Position;
use stratyx_poker::{Card, HandState, HoleCards, Range, Specialist, Street};
use stratyx_shared::Chips;
use stratyx_strategy::{
    Basis, Decision, HandFeatures, Inputs, Spot, decision_features, mc, reference,
};

use crate::{bb, bundle_for, pct};

/// Ce qui s'est passé avant la décision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Situation {
    /// Préflop, personne n'est entré dans le coup.
    Unopened,
    /// Préflop, un joueur a relancé (montant total en BB).
    FacingRaise { raise_to_bb: f32 },
    /// Préflop, un joueur a fait tapis.
    FacingAllIn,
    /// Après le flop : `opponents` adversaires en jeu (le montant à suivre dit s'il y a une mise).
    Postflop { opponents: u8 },
}

/// Les données d'un spot.
#[derive(Debug, Clone, PartialEq)]
pub struct SpotInput {
    pub specialist: Option<Specialist>,
    /// Joueurs servis à la table (2 à 9).
    pub players: u8,
    pub position: Position,
    pub hole: HoleCards,
    pub board: Vec<Card>,
    /// Tapis effectif, en BB, avant la décision.
    pub effective_bb: f32,
    /// Pot total affiché (mises du tour comprises), en BB. Ignoré au préflop.
    pub pot_bb: f32,
    /// Montant à suivre, en BB (0 : parole possible). Ignoré au préflop.
    pub to_call_bb: f32,
    pub situation: Situation,
}

/// Le conseil pour un spot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpotAdvice {
    pub best: Decision,
    pub best_text: String,
    pub basis: Basis,
    pub ev: [Option<f32>; 4],
    pub equity: f32,
    pub pot_odds: f32,
    pub facing_bet: bool,
    pub equilibrium: Option<f32>,
    pub model: Option<(Decision, f32)>,
    pub engine: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpotError {
    #[error("{0}")]
    Invalid(String),
}

/// La grosse blinde du spot reconstruit, en jetons.
const BB: u64 = 100;

/// Plus grand montant accepté, en BB : bien au-delà de toute partie réelle.
pub const MAX_SPOT_BB: f32 = 100_000.0;

/// Vérifie les montants saisis ou lus sur une image, avant de reconstruire le spot :
/// « inf », « NaN » ou « 1e30 » passent la lecture d'un nombre, et finissaient en message
/// incompréhensible (« montants démesurés (au-delà de 2^62 jetons) »).
fn check_amounts(input: &SpotInput) -> Result<(), SpotError> {
    let mut amounts = vec![("tapis effectif", input.effective_bb)];
    if input.board.len() >= 3 {
        amounts.push(("pot", input.pot_bb));
        amounts.push(("montant à suivre", input.to_call_bb));
    } else if let Situation::FacingRaise { raise_to_bb } = input.situation {
        amounts.push(("relance", raise_to_bb));
    }
    for (what, value) in amounts {
        if !value.is_finite() {
            return Err(SpotError::Invalid(format!("{what} illisible")));
        }
        if value < 0.0 {
            return Err(SpotError::Invalid(format!("{what} négatif ({value} BB)")));
        }
        if value > MAX_SPOT_BB {
            return Err(SpotError::Invalid(format!(
                "{what} démesuré ({value} BB, au plus {MAX_SPOT_BB} BB)"
            )));
        }
    }
    Ok(())
}

fn chips(bb: f32) -> u64 {
    (bb.max(0.0) * BB as f32).round() as u64
}

/// Reconstruit le spot dans le moteur : renvoie l'état (le joueur parle) et son index.
fn build(input: &SpotInput) -> Result<(HandState, usize), SpotError> {
    let n = usize::from(input.players.clamp(2, 9));
    let layout =
        Position::layout(n).ok_or_else(|| SpotError::Invalid("table de 2 à 9 joueurs".into()))?;
    let offset = layout
        .iter()
        .position(|p| *p == input.position)
        .ok_or_else(|| {
            SpotError::Invalid(format!(
                "pas de {} à {n} joueurs",
                input.position.abbreviation()
            ))
        })?;
    let ante = if input.specialist == Some(Specialist::Tournament6Max) {
        Ante::Classic(Chips(BB / 8))
    } else {
        Ante::None
    };
    let eff = input.effective_bb.max(1.0);
    let preflop = input.board.len() < 3;
    // Au préflop, chacun a le tapis effectif ; après le flop, on remet ce qui a été investi.
    let invested_each = if preflop {
        0.0
    } else {
        ((input.pot_bb - input.to_call_bb).max(1.5) / 2.0).max(1.0)
    };
    let stack = chips(eff + invested_each);
    // Le joueur au siège 0 ; le bouton est placé pour lui donner sa position.
    let button = (n - offset) % n;
    let blinds = Blinds::new(BB / 2, BB).with_ante(ante);
    let setup = HandSetup::new(vec![Chips(stack); n], button, blinds);
    let mut st = HandState::new(&setup).map_err(|e| SpotError::Invalid(e.to_string()))?;
    let hero = 0usize;
    let fold_to = |st: &mut HandState, target: usize| -> Result<(), SpotError> {
        while let Some(p) = st.to_act() {
            if p == target {
                break;
            }
            st.apply(p, Action::Fold)
                .map_err(|e| SpotError::Invalid(e.to_string()))?;
        }
        Ok(())
    };
    let err = |e: stratyx_poker::game::GameError| SpotError::Invalid(e.to_string());
    if preflop {
        match input.situation {
            Situation::Unopened | Situation::Postflop { .. } => fold_to(&mut st, hero)?,
            Situation::FacingRaise { raise_to_bb } => {
                let raiser = st.to_act().unwrap_or(hero);
                if raiser == hero {
                    return Err(SpotError::Invalid(
                        "personne ne parle avant cette position".into(),
                    ));
                }
                st.apply(raiser, Action::Raise(Chips(chips(raise_to_bb.max(2.0)))))
                    .map_err(err)?;
                fold_to(&mut st, hero)?;
            }
            Situation::FacingAllIn => {
                let shover = st.to_act().unwrap_or(hero);
                if shover == hero {
                    return Err(SpotError::Invalid(
                        "personne ne parle avant cette position".into(),
                    ));
                }
                st.apply(shover, Action::AllIn).map_err(err)?;
                fold_to(&mut st, hero)?;
            }
        }
        return Ok((st, hero));
    }
    // Après le flop : les adversaires et le joueur suivent une relance qui donne le pot.
    let opponents = match input.situation {
        Situation::Postflop { opponents } => usize::from(opponents.clamp(1, 8)).min(n - 1),
        _ => 1,
    };
    let keep: Vec<usize> = (1..n).rev().take(opponents).collect();
    let raise_to = chips(invested_each).max(BB);
    let mut raised = false;
    while let Some(p) = st.to_act() {
        let action = if p == hero || keep.contains(&p) {
            if raised {
                Action::Call
            } else {
                raised = true;
                if raise_to > BB {
                    Action::Raise(Chips(raise_to))
                } else {
                    Action::Call
                }
            }
        } else {
            Action::Fold
        };
        let action = if st.clone().apply(p, action).is_ok() {
            action
        } else if st.legal_actions().is_some_and(|l| l.check) {
            Action::Check
        } else {
            Action::Call
        };
        st.apply(p, action).map_err(err)?;
    }
    let mut dealt = 0usize;
    while dealt < input.board.len() {
        let k = match st.phase() {
            stratyx_poker::game::Phase::AwaitingBoard(s) => s.cards_dealt(),
            _ => break,
        };
        st.deal(&input.board[dealt..(dealt + k).min(input.board.len())])
            .map_err(err)?;
        dealt += k;
        if dealt >= input.board.len() {
            break;
        }
        // Tours intermédiaires : tout le monde parle.
        while let Some(p) = st.to_act() {
            st.apply(p, Action::Check).map_err(err)?;
        }
    }
    // Le dernier adversaire mise le montant à suivre, les autres parlent jusqu'au joueur.
    let to_call = chips(input.to_call_bb);
    let mut guard = 0;
    while let Some(p) = st.to_act() {
        guard += 1;
        if guard > 30 {
            break;
        }
        if p == hero {
            // Le joueur parle en premier : il parle, puis l'adversaire mise.
            if to_call > 0 && st.current_bet().is_zero() {
                st.apply(hero, Action::Check).map_err(err)?;
                continue;
            }
            break;
        }
        let action = if to_call > 0 && st.current_bet().is_zero() {
            Action::Bet(Chips(to_call))
        } else if to_call > 0 {
            Action::Call
        } else {
            Action::Check
        };
        let action = if st.clone().apply(p, action).is_ok() {
            action
        } else {
            Action::AllIn
        };
        st.apply(p, action).map_err(err)?;
    }
    if st.to_act() != Some(hero) {
        return Err(SpotError::Invalid(
            "le spot ne laisse pas la parole au joueur".into(),
        ));
    }
    Ok((st, hero))
}

/// Analyse un spot.
pub fn analyse_spot(input: &SpotInput, samples: usize) -> Result<SpotAdvice, SpotError> {
    check_amounts(input)?;
    let mut used = input.hole.set();
    for c in &input.board {
        if !used.insert(*c) {
            return Err(SpotError::Invalid(format!("carte {c} en double")));
        }
    }
    if !matches!(input.board.len(), 0 | 3 | 4 | 5) {
        return Err(SpotError::Invalid(
            "le board compte 0, 3, 4 ou 5 cartes".into(),
        ));
    }
    let (state, hero) = build(input)?;
    let models = bundle_for(input.specialist);
    let inputs = Inputs {
        chart: models.map(|m| &m.pushfold),
        population: models.map(|m| &m.population),
        samples,
        seed: 0x5354_5241_5459_5821,
    };
    let r = reference(&state, input.hole, &inputs, None)
        .ok_or_else(|| SpotError::Invalid("aucune décision possible ici".into()))?;
    let spot = Spot::from_state(&state, hero);
    let facing = spot.to_call_bb > 0.0;
    let model = models.map(|m| {
        let opponents = state
            .players()
            .iter()
            .enumerate()
            .filter(|(i, s)| *i != hero && s.status != PlayerStatus::Folded)
            .count()
            .max(1);
        let eq = if state.street() == Street::Preflop {
            0.0
        } else {
            mc::equity(
                input.hole,
                &vec![Range::empty(); opponents],
                state.board(),
                400,
                7,
            )
        };
        let x = decision_features(
            &spot,
            &HandFeatures::new(input.hole, state.board(), opponents, eq),
        );
        let (i, conf) = m.decision.best(&x);
        (Decision::from_index(i), conf)
    });
    let raise_bb = r.raise_to.map(|t| t as f32 / BB as f32);
    let best_text = match (r.best, raise_bb) {
        (Decision::Raise, Some(to)) => format!("{} à {}", r.best.label(facing), bb(to)),
        _ => r.best.label(facing).to_owned(),
    };
    let engine = match input.specialist {
        Some(Specialist::Expresso3Max) => "IA Expresso 3-Max",
        Some(Specialist::Tournament6Max) => "IA Tournois 6-Max",
        None => "moteur commun",
    }
    .to_owned();
    let mut summary = format!(
        "{} {} en {} à {} effectifs : ",
        input.hole.class(),
        if input.board.is_empty() {
            "préflop".to_owned()
        } else {
            format!(
                "sur {}",
                input
                    .board
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        },
        input.position.abbreviation(),
        bb(spot.effective_bb)
    );
    match r.basis {
        Basis::PushFold => summary.push_str(&format!(
            "l'équilibre push/fold de l'{engine} dit « {best_text} » ({} du temps).",
            pct(r.equilibrium.unwrap_or(0.0))
        )),
        Basis::Ev => {
            summary.push_str(&format!("{best_text}. Équité estimée {}", pct(r.equity)));
            if facing {
                summary.push_str(&format!(" pour {} nécessaire", pct(r.pot_odds)));
            }
            summary.push('.');
        }
    }
    if let Some((d, c)) = model {
        summary.push_str(&format!(
            " Modèle de décision : {} ({}).",
            d.label(facing),
            pct(c)
        ));
    }
    Ok(SpotAdvice {
        best: r.best,
        best_text,
        basis: r.basis,
        ev: r.ev,
        equity: r.equity,
        pot_odds: r.pot_odds,
        facing_bet: spot.raises > 0 || (spot.street != Street::Preflop && facing),
        equilibrium: r.equilibrium,
        model,
        engine,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(position: Position, hole: &str, eff: f32, situation: Situation) -> SpotInput {
        SpotInput {
            specialist: Some(Specialist::Expresso3Max),
            players: 3,
            position,
            hole: hole.parse().unwrap(),
            board: Vec::new(),
            effective_bb: eff,
            pot_bb: 0.0,
            to_call_bb: 0.0,
            situation,
        }
    }

    #[test]
    fn unreadable_amounts_get_a_clear_message() {
        let message = |i: &SpotInput| analyse_spot(i, 200).unwrap_err().to_string();
        let mut i = input(Position::Button, "AhKd", f32::INFINITY, Situation::Unopened);
        assert_eq!(message(&i), "tapis effectif illisible");
        i.effective_bb = f32::NAN;
        assert_eq!(message(&i), "tapis effectif illisible");
        i.effective_bb = -5.0;
        assert_eq!(message(&i), "tapis effectif négatif (-5 BB)");
        i.effective_bb = 1e30;
        assert!(message(&i).starts_with("tapis effectif démesuré"));
        i.effective_bb = 20.0;
        i.situation = Situation::FacingRaise {
            raise_to_bb: f32::NAN,
        };
        assert_eq!(message(&i), "relance illisible");
        // Après le flop, le pot et le montant à suivre comptent ; avant, ils sont ignorés.
        i.situation = Situation::Postflop { opponents: 1 };
        i.pot_bb = f32::INFINITY;
        assert!(analyse_spot(&i, 200).is_ok());
        i.board = stratyx_poker::card::parse_cards("Qh7c2s").unwrap();
        assert_eq!(message(&i), "pot illisible");
        i.pot_bb = 6.0;
        i.to_call_bb = -1.0;
        assert_eq!(message(&i), "montant à suivre négatif (-1 BB)");
        i.to_call_bb = 2.0;
        assert!(analyse_spot(&i, 200).is_ok());
    }

    #[test]
    fn preflop_spots() {
        let a = analyse_spot(
            &input(Position::SmallBlind, "KhTd", 8.0, Situation::Unopened),
            1500,
        )
        .unwrap();
        assert_eq!(a.basis, Basis::PushFold);
        assert_eq!(a.best, Decision::AllIn);
        let a = analyse_spot(
            &input(Position::BigBlind, "7c2d", 15.0, Situation::FacingAllIn),
            1500,
        )
        .unwrap();
        assert_eq!(a.best, Decision::Fold);
        let a = analyse_spot(
            &input(
                Position::BigBlind,
                "AsAh",
                30.0,
                Situation::FacingRaise { raise_to_bb: 2.5 },
            ),
            1500,
        )
        .unwrap();
        assert!(matches!(a.best, Decision::Raise | Decision::AllIn), "{a:?}");
        assert!(
            analyse_spot(
                &input(Position::Button, "AsAh", 30.0, Situation::FacingAllIn),
                500
            )
            .is_err()
        );
    }

    #[test]
    fn postflop_spot() {
        let mut i = input(
            Position::Button,
            "QsQd",
            20.0,
            Situation::Postflop { opponents: 1 },
        );
        i.board = stratyx_poker::card::parse_cards("Qh7c2s").unwrap();
        i.pot_bb = 9.0;
        i.to_call_bb = 3.0;
        let a = analyse_spot(&i, 1500).unwrap();
        assert!(a.equity > 0.8, "{a:?}");
        assert!(a.facing_bet);
        assert!(matches!(
            a.best,
            Decision::Raise | Decision::AllIn | Decision::Passive
        ));
        i.hole = "4c3d".parse().unwrap();
        i.board = stratyx_poker::card::parse_cards("AhKc9s").unwrap();
        let a = analyse_spot(&i, 1500).unwrap();
        assert_eq!(a.best, Decision::Fold, "{a:?}");
    }
}
