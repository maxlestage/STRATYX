//! Historique de main : format pivot entre la vision artificielle, la revue utilisateur,
//! les bases de données et les analyses.
//!
//! Chaque information porte sa provenance et sa confiance ([`Observed`]). Un historique
//! est **validé** en le rejouant intégralement dans le moteur de règles ([`HandState`]) :
//! toute incohérence (action hors tour, montant impossible, carte en double…) est localisée
//! à l'événement fautif pour que l'utilisateur la corrige.

use serde::{Deserialize, Serialize};
use stratyx_shared::{Chips, Confidence, Observed};
use thiserror::Error;

use crate::card::Card;
use crate::cardset::CardSet;
use crate::format::{FormatError, GameFormat};
use crate::game::{Action, Blinds, GameError, HandSetup, HandState, Phase, Reveal, Settlement};
use crate::range::HoleCards;

/// Un joueur servi.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeatRecord {
    /// Numéro de siège à la table (à partir de 1).
    pub seat: u8,
    pub player: String,
    /// Tapis au début de la main.
    pub stack: Observed<Chips>,
    /// Cartes connues dès la distribution (en pratique : celles du joueur analysé).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole_cards: Option<Observed<HoleCards>>,
}

/// Événement d'une main, dans l'ordre chronologique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum HandEvent {
    Action { seat: u8, action: Action },
    Board { cards: Vec<Card> },
    Show { seat: u8, cards: HoleCards },
    Muck { seat: u8 },
}

/// Contexte d'une main : joueur analysé, date, session, origine.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HandMeta {
    /// Siège du joueur analysé (le « héros »), s'il est assis à la table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_seat: Option<u8>,
    /// Début de la main, en millisecondes depuis l'époque Unix (UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played_at_ms: Option<u64>,
    /// Session de capture ou d'import à laquelle appartient la main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Salle de poker (ex. « winamax »).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Nom de la table ou identifiant du tournoi côté salle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Provenance et licence pour les archives (décision D5), ex. « mains personnelles ».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

/// Historique complet d'une main.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandRecord {
    pub id: String,
    pub format: GameFormat,
    pub blinds: Observed<Blinds>,
    pub button_seat: u8,
    pub seats: Vec<SeatRecord>,
    pub events: Vec<Observed<HandEvent>>,
    #[serde(default)]
    pub meta: HandMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecordError {
    #[error("format invalide : {0}")]
    Format(FormatError),
    #[error("{count} joueurs servis pour un format à {max} places")]
    TooManyPlayers { count: usize, max: usize },
    #[error("siège {0} en double")]
    DuplicateSeat(u8),
    #[error("le bouton (siège {0}) n'est pas un joueur servi")]
    ButtonNotSeated(u8),
    #[error("main invalide : {0}")]
    Setup(GameError),
    #[error("événement {index} : siège {seat} inconnu")]
    UnknownSeat { index: usize, seat: u8 },
    #[error("événement {index} : carte {card} déjà vue")]
    DuplicateCard { index: usize, card: Card },
    #[error("événement {index} : {source}")]
    Rule { index: usize, source: GameError },
}

/// Résultat d'une main rejouée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Outcome {
    /// Gains attribués.
    Settled(Settlement),
    /// Main terminée mais gagnant indéterminable sans inventer de cartes.
    Unresolved { reason: String },
    /// Historique incomplet (vidéo coupée, main en cours).
    Incomplete,
}

/// Élément à faire vérifier par l'utilisateur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    /// Ce qui est concerné : `"blinds"`, `"seat:3:stack"`, `"event:12"`…
    pub target: String,
    pub confidence: Confidence,
    pub video_ms: Option<u64>,
}

/// Main rejouée et validée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    pub state: HandState,
    /// Numéro de siège de chaque joueur du moteur (index → siège).
    pub seats: Vec<u8>,
    pub outcome: Outcome,
}

impl HandRecord {
    /// Rejoue l'historique dans le moteur de règles et calcule le résultat.
    pub fn replay(&self) -> Result<Replay, RecordError> {
        self.format.validate().map_err(RecordError::Format)?;
        let max = self.format.max_players();
        if self.seats.len() > max {
            return Err(RecordError::TooManyPlayers {
                count: self.seats.len(),
                max,
            });
        }
        let mut order: Vec<&SeatRecord> = self.seats.iter().collect();
        order.sort_by_key(|s| s.seat);
        if let Some(w) = order.windows(2).find(|w| w[0].seat == w[1].seat) {
            return Err(RecordError::DuplicateSeat(w[0].seat));
        }
        let seats: Vec<u8> = order.iter().map(|s| s.seat).collect();
        let button = seats
            .iter()
            .position(|s| *s == self.button_seat)
            .ok_or(RecordError::ButtonNotSeated(self.button_seat))?;
        let setup = HandSetup::new(
            order.iter().map(|s| s.stack.value).collect(),
            button,
            self.blinds.value,
        )
        .with_structure(self.format.structure);
        let mut state = HandState::new(&setup).map_err(RecordError::Setup)?;

        let mut reveals: Vec<Reveal> = order
            .iter()
            .map(|s| {
                s.hole_cards
                    .as_ref()
                    .map_or(Reveal::Unknown, |h| Reveal::Shown(h.value))
            })
            .collect();
        let mut seen = CardSet::EMPTY;
        for s in &order {
            if let Some(h) = &s.hole_cards {
                for c in h.value.cards() {
                    if !seen.insert(c) {
                        return Err(RecordError::DuplicateCard { index: 0, card: c });
                    }
                }
            }
        }

        for (index, ev) in self.events.iter().enumerate() {
            let idx_of = |seat: u8| {
                seats
                    .iter()
                    .position(|s| *s == seat)
                    .ok_or(RecordError::UnknownSeat { index, seat })
            };
            let rule = |source| RecordError::Rule { index, source };
            match &ev.value {
                HandEvent::Action { seat, action } => {
                    state.apply(idx_of(*seat)?, *action).map_err(rule)?;
                }
                HandEvent::Board { cards } => {
                    for c in cards {
                        if seen.contains(*c) {
                            return Err(RecordError::DuplicateCard { index, card: *c });
                        }
                    }
                    state.deal(cards).map_err(rule)?;
                    seen |= cards.iter().collect();
                }
                HandEvent::Show { seat, cards } => {
                    let p = idx_of(*seat)?;
                    let already_known = reveals[p] == Reveal::Shown(*cards);
                    if !already_known {
                        if let Some(c) = cards.cards().into_iter().find(|c| seen.contains(*c)) {
                            return Err(RecordError::DuplicateCard { index, card: c });
                        }
                        seen |= cards.set();
                    }
                    reveals[p] = Reveal::Shown(*cards);
                }
                HandEvent::Muck { seat } => {
                    reveals[idx_of(*seat)?] = Reveal::Mucked;
                }
            }
        }

        let outcome = match state.phase() {
            Phase::Showdown | Phase::Finished => match state.settle(&reveals) {
                Ok(s) => Outcome::Settled(s),
                Err(e) => Outcome::Unresolved {
                    reason: e.to_string(),
                },
            },
            Phase::Betting | Phase::AwaitingBoard(_) => Outcome::Incomplete,
        };
        Ok(Replay {
            state,
            seats,
            outcome,
        })
    }

    /// Détections incertaines à faire vérifier, par ordre d'apparition.
    #[must_use]
    pub fn review_items(&self, threshold: Confidence) -> Vec<ReviewItem> {
        let mut out = Vec::new();
        let mut push = |target: String, conf: Confidence, video_ms: Option<u64>| {
            out.push(ReviewItem {
                target,
                confidence: conf,
                video_ms,
            });
        };
        if self.blinds.needs_review(threshold) {
            push(
                "blinds".into(),
                self.blinds.confidence,
                self.blinds.video_ms,
            );
        }
        for s in &self.seats {
            if s.stack.needs_review(threshold) {
                push(
                    format!("seat:{}:stack", s.seat),
                    s.stack.confidence,
                    s.stack.video_ms,
                );
            }
            if let Some(h) = s.hole_cards.as_ref().filter(|h| h.needs_review(threshold)) {
                push(
                    format!("seat:{}:hole_cards", s.seat),
                    h.confidence,
                    h.video_ms,
                );
            }
        }
        for (i, ev) in self.events.iter().enumerate() {
            if ev.needs_review(threshold) {
                push(format!("event:{i}"), ev.confidence, ev.video_ms);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use stratyx_shared::Source;

    use super::*;
    use crate::card::parse_cards;

    fn ok<T>(v: T) -> Observed<T> {
        Observed::certain(v, Source::Import)
    }

    fn seat(seat: u8, name: &str, stack: u64, hole: Option<&str>) -> SeatRecord {
        SeatRecord {
            seat,
            player: name.into(),
            stack: ok(Chips(stack)),
            hole_cards: hole.map(|h| ok(h.parse().unwrap())),
        }
    }

    fn act(seat: u8, action: Action) -> Observed<HandEvent> {
        ok(HandEvent::Action { seat, action })
    }

    fn board(s: &str) -> Observed<HandEvent> {
        ok(HandEvent::Board {
            cards: parse_cards(s).unwrap(),
        })
    }

    /// Expresso : héros au siège 1 (bouton), push préflop payé par la grosse blinde.
    fn expresso_hand() -> HandRecord {
        HandRecord {
            id: "exp-0001".into(),
            meta: HandMeta {
                hero_seat: Some(1),
                ..HandMeta::default()
            },
            format: GameFormat::EXPRESSO,
            blinds: ok(Blinds::new(10, 20)),
            button_seat: 1,
            seats: vec![
                seat(1, "Hero", 500, Some("AhKh")),
                seat(2, "Villain1", 500, None),
                seat(3, "Villain2", 500, None),
            ],
            events: vec![
                act(1, Action::AllIn),
                act(2, Action::Fold),
                act(3, Action::Call),
                board("Kd7c2s"),
                board("9h"),
                board("3c"),
                ok(HandEvent::Show {
                    seat: 3,
                    cards: "QsQd".parse().unwrap(),
                }),
            ],
        }
    }

    #[test]
    fn replay_and_settle() {
        let r = expresso_hand().replay().unwrap();
        assert_eq!(r.seats, vec![1, 2, 3]);
        let Outcome::Settled(s) = r.outcome else {
            panic!("main non résolue")
        };
        assert_eq!(s.net, vec![510, -10, -500]);
    }

    #[test]
    fn hidden_cards_are_not_invented() {
        let mut h = expresso_hand();
        h.events.pop(); // la main adverse n'a pas été vue
        let r = h.replay().unwrap();
        assert!(matches!(r.outcome, Outcome::Unresolved { .. }));
    }

    #[test]
    fn incomplete_hand() {
        let mut h = expresso_hand();
        h.events.truncate(4);
        assert_eq!(h.replay().unwrap().outcome, Outcome::Incomplete);
    }

    #[test]
    fn errors_point_to_the_faulty_event() {
        let mut h = expresso_hand();
        h.events[1] = act(3, Action::Fold); // hors tour
        assert!(matches!(
            h.replay(),
            Err(RecordError::Rule { index: 1, .. })
        ));

        let mut h = expresso_hand();
        h.events[3] = board("Ah7c2s"); // As de cœur déjà chez le héros
        assert!(matches!(
            h.replay(),
            Err(RecordError::DuplicateCard { index: 3, .. })
        ));

        let mut h = expresso_hand();
        h.seats.push(seat(4, "Extra", 500, None));
        assert!(matches!(
            h.replay(),
            Err(RecordError::TooManyPlayers { .. })
        ));

        let mut h = expresso_hand();
        h.button_seat = 9;
        assert_eq!(h.replay().unwrap_err(), RecordError::ButtonNotSeated(9));
    }

    #[test]
    fn full_ring_fixed_limit_cash_game() {
        use crate::format::GameType;
        use crate::game::BettingStructure;
        let format =
            GameFormat::new(GameType::CashGame, 9, BettingStructure::FIXED_LIMIT_CASH).unwrap();
        let mut seats: Vec<SeatRecord> = (1..=9)
            .map(|i| seat(i, &format!("J{i}"), 200, None))
            .collect();
        seats[3].hole_cards = Some(ok("AhAd".parse().unwrap()));
        // Bouton siège 1 : SB 2, BB 3, UTG 4 (héros).
        let mut events = vec![act(4, Action::Raise(Chips(8)))];
        events.extend((5..=9).chain(1..=2).map(|s| act(s, Action::Fold)));
        events.push(act(3, Action::Call));
        events.push(board("Kd7c2s"));
        events.push(act(3, Action::Check));
        events.push(act(4, Action::Bet(Chips(4))));
        events.push(act(3, Action::Fold));
        let h = HandRecord {
            id: "cash-fl-9max".into(),
            meta: HandMeta::default(),
            format,
            blinds: ok(Blinds::new(2, 4)),
            button_seat: 1,
            seats,
            events,
        };
        let r = h.replay().unwrap();
        let Outcome::Settled(s) = r.outcome else {
            panic!("main non résolue")
        };
        // Le héros gagne la SB (2) et le call de la BB (8).
        assert_eq!(s.net[3], 10);

        // Une relance au mauvais montant est refusée en limite fixe.
        let mut bad = h.clone();
        bad.events[0] = act(4, Action::Raise(Chips(12)));
        assert!(matches!(
            bad.replay(),
            Err(RecordError::Rule { index: 0, .. })
        ));

        // Format incohérent.
        let mut bad = h;
        bad.format.table_size = 12;
        assert!(matches!(bad.replay(), Err(RecordError::Format(_))));
    }

    #[test]
    fn uncertain_detections_are_flagged() {
        let mut h = expresso_hand();
        h.events[2] = Observed::vision(
            HandEvent::Action {
                seat: 3,
                action: Action::Call,
            },
            Confidence::new(0.7).unwrap(),
            12_500,
        );
        let items = h.review_items(Confidence::DEFAULT_REVIEW_THRESHOLD);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, "event:2");
        assert_eq!(items[0].video_ms, Some(12_500));
    }

    #[test]
    fn json_roundtrip() {
        let h = expresso_hand();
        let json = serde_json::to_string(&h).unwrap();
        let back: HandRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
        assert!(json.contains("\"type\":\"all_in\""));
    }
}
