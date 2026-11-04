//! Rejeu pas à pas d'un historique : contexte de chaque décision volontaire.
//!
//! L'historique est d'abord validé par [`HandRecord::replay`], puis rejoué une seconde fois
//! action par action pour capturer, juste **avant** chaque décision, le pot, le montant à
//! suivre, les tapis et les actions légales. Toutes les analyses du crate partent de là.

use stratyx_poker::card::Card;
use stratyx_poker::eval::evaluate;
use stratyx_poker::game::{
    ActionKind, AppliedAction, GameError, HandSetup, HandState, Phase, PlayerStatus, Reveal,
    Settlement,
};
use stratyx_poker::history::{HandEvent, HandRecord, Outcome, RecordError, SeatRecord};
use stratyx_poker::position::Position;
use stratyx_poker::pot::{Claim, Contribution, Pot, award_pots};
use stratyx_poker::range::HoleCards;
use stratyx_poker::{CardSet, Street};
use stratyx_shared::Chips;

use crate::error::AnalyticsError;

/// Contexte d'une décision volontaire (fold, check, call, bet, raise).
#[derive(Debug, Clone)]
pub(crate) struct Step {
    /// Index de l'événement dans `HandRecord::events`.
    pub event_index: usize,
    /// Index du joueur dans le moteur.
    pub player: usize,
    pub street: Street,
    pub position: Position,
    /// Jetons au milieu avant la décision (antes, blindes et mises du tour compris).
    pub pot: Chips,
    /// Montant à ajouter pour suivre (borné par le tapis).
    pub to_call: Chips,
    /// Mise déjà engagée par le joueur sur ce tour.
    pub street_bet: Chips,
    /// Tapis restant du joueur.
    pub stack: Chips,
    pub can_bet: bool,
    pub can_raise: bool,
    /// Nombre de mises et relances (tapis incomplets compris) déjà faites sur ce tour.
    pub aggressive_before: usize,
    /// Nombre de cartes du board au moment de la décision.
    pub board_len: usize,
    pub applied: AppliedAction,
    /// État complet avant la décision (conservé seulement pour l'analyse de décision).
    pub state: Option<HandState>,
}

/// Main validée, rejouée et décomposée en décisions.
#[derive(Debug, Clone)]
pub(crate) struct ParsedHand<'a> {
    pub record: &'a HandRecord,
    /// État final.
    pub state: HandState,
    /// Numéro de siège de chaque joueur du moteur.
    pub seats: Vec<u8>,
    /// Pseudonyme de chaque joueur du moteur.
    pub names: Vec<&'a str>,
    pub outcome: Outcome,
    /// Cartes connues de chaque joueur : distribuées au héros ou montrées à un moment de la
    /// main. Jamais inventées.
    pub known: Vec<Option<HoleCards>>,
    /// Informations d'abattage, identiques à celles utilisées par le rejeu.
    pub reveals: Vec<Reveal>,
    pub steps: Vec<Step>,
    /// Index du joueur analysé, s'il est désigné.
    pub hero: Option<usize>,
}

impl<'a> ParsedHand<'a> {
    pub fn parse(record: &'a HandRecord, keep_states: bool) -> Result<Self, AnalyticsError> {
        let invalid = |source: RecordError| AnalyticsError::InvalidRecord {
            hand_id: record.id.clone(),
            source,
        };
        let replay = record.replay().map_err(invalid)?;
        let rule =
            |index: usize| move |source: GameError| invalid(RecordError::Rule { index, source });

        let mut order: Vec<&SeatRecord> = record.seats.iter().collect();
        order.sort_by_key(|s| s.seat);
        let button = replay
            .seats
            .iter()
            .position(|s| *s == record.button_seat)
            .ok_or_else(|| invalid(RecordError::ButtonNotSeated(record.button_seat)))?;
        let setup = HandSetup::new(
            order.iter().map(|s| s.stack.value).collect(),
            button,
            record.blinds.value,
        )
        .with_structure(record.format.structure);
        let mut state = HandState::new(&setup).map_err(|e| invalid(RecordError::Setup(e)))?;

        let mut known: Vec<Option<HoleCards>> = order
            .iter()
            .map(|s| s.hole_cards.as_ref().map(|h| h.value))
            .collect();
        let mut reveals: Vec<Reveal> = known
            .iter()
            .map(|k| k.map_or(Reveal::Unknown, Reveal::Shown))
            .collect();
        let index_of = |index: usize, seat: u8| {
            replay
                .seats
                .iter()
                .position(|s| *s == seat)
                .ok_or_else(|| invalid(RecordError::UnknownSeat { index, seat }))
        };

        let mut steps = Vec::new();
        for (index, event) in record.events.iter().enumerate() {
            match &event.value {
                HandEvent::Action { seat, action } => {
                    let p = index_of(index, *seat)?;
                    let legal = state.legal_actions();
                    let pl = &state.players()[p];
                    let street = state.street();
                    let aggressive_before = state
                        .actions()
                        .iter()
                        .filter(|a| a.street == street && a.kind.is_aggressive())
                        .count();
                    let before = keep_states.then(|| state.clone());
                    let (pot, street_bet, stack) = (state.pot_total(), pl.street_bet, pl.stack);
                    let position = state
                        .position(p)
                        .expect("table de 2 à 10 joueurs validée par le rejeu");
                    let board_len = state.board().len();
                    let applied = state.apply(p, *action).map_err(rule(index))?;
                    steps.push(Step {
                        event_index: index,
                        player: p,
                        street,
                        position,
                        pot,
                        to_call: legal.and_then(|l| l.call).unwrap_or(Chips::ZERO),
                        street_bet,
                        stack,
                        can_bet: legal.is_some_and(|l| l.bet.is_some()),
                        can_raise: legal.is_some_and(|l| l.raise.is_some()),
                        aggressive_before,
                        board_len,
                        applied,
                        state: before,
                    });
                }
                HandEvent::Board { cards } => state.deal(cards).map_err(rule(index))?,
                HandEvent::Show { seat, cards } => {
                    let p = index_of(index, *seat)?;
                    known[p] = Some(*cards);
                    reveals[p] = Reveal::Shown(*cards);
                }
                HandEvent::Muck { seat } => {
                    reveals[index_of(index, *seat)?] = Reveal::Mucked;
                }
            }
        }

        debug_assert_eq!(state, replay.state, "rejeu pas à pas divergent");
        let hero = record
            .meta
            .hero_seat
            .map(|seat| {
                replay.seats.iter().position(|s| *s == seat).ok_or_else(|| {
                    AnalyticsError::SeatNotFound {
                        hand_id: record.id.clone(),
                        seat,
                    }
                })
            })
            .transpose()?;

        Ok(ParsedHand {
            record,
            state,
            names: order.iter().map(|s| s.player.as_str()).collect(),
            seats: replay.seats,
            outcome: replay.outcome,
            known,
            reveals,
            steps,
            hero,
        })
    }

    pub fn len(&self) -> usize {
        self.seats.len()
    }

    pub fn big_blind(&self) -> Chips {
        self.record.blinds.value.big_blind
    }

    pub fn settlement(&self) -> Option<&Settlement> {
        match &self.outcome {
            Outcome::Settled(s) => Some(s),
            _ => None,
        }
    }

    pub fn board(&self, len: usize) -> &[Card] {
        &self.state.board()[..len.min(self.state.board().len())]
    }

    pub fn position(&self, player: usize) -> Position {
        self.state
            .position(player)
            .expect("table de 2 à 10 joueurs validée par le rejeu")
    }

    pub fn folded(&self, player: usize) -> bool {
        self.state.players()[player].status == PlayerStatus::Folded
    }

    /// Tour au cours duquel le joueur s'est couché.
    pub fn fold_street(&self, player: usize) -> Option<Street> {
        self.steps
            .iter()
            .find(|s| s.player == player && s.applied.kind == ActionKind::Fold)
            .map(|s| s.street)
    }

    /// La main est allée jusqu'au bout (abattage ou tous les adversaires couchés).
    pub fn finished(&self) -> bool {
        matches!(self.state.phase(), Phase::Showdown | Phase::Finished)
    }

    /// Le joueur était encore en jeu à la distribution du flop.
    pub fn saw_flop(&self, player: usize) -> bool {
        self.state.board().len() >= 3
            && self.fold_street(player).is_none_or(|s| s > Street::Preflop)
    }

    /// Le joueur a participé à l'abattage.
    pub fn went_to_showdown(&self, player: usize) -> bool {
        self.state.phase() == Phase::Showdown && !self.folded(player)
    }

    /// Gains remportés dans les pots (mise non suivie rendue exclue), si la main est résolue.
    pub fn pot_winnings(&self, player: usize) -> Option<Chips> {
        let s = self.settlement()?;
        let returned = match s.pots.uncalled {
            Some((p, amount)) if p == player => amount,
            _ => Chips::ZERO,
        };
        Some(s.collected[player].saturating_sub(returned))
    }

    /// Résultat net du joueur, quand il est déterminable sans inventer de cartes : main
    /// résolue, ou joueur couché (il a perdu exactement ce qu'il avait investi).
    pub fn net(&self, player: usize) -> Option<i64> {
        if let Some(s) = self.settlement() {
            return Some(s.net[player]);
        }
        self.folded(player)
            .then(|| -(self.state.players()[player].invested().get() as i64))
    }

    /// Prétentions d'abattage, calculées comme par le moteur.
    fn claims(&self) -> Vec<Claim> {
        let board: CardSet = self.state.board().iter().collect();
        self.reveals
            .iter()
            .map(|r| match r {
                Reveal::Shown(h) if board.len() == 5 => Claim::Shown(evaluate(board | h.set())),
                Reveal::Shown(_) | Reveal::Unknown => Claim::Unknown,
                Reveal::Mucked => Claim::Mucked,
            })
            .collect()
    }

    /// Jetons transférés de `from` vers `to` au cours de la main (positif si `to` gagne).
    ///
    /// Chaque pot est réparti entre ses gagnants ; la part de chaque gagnant est prélevée sur
    /// chaque contributeur au prorata de sa mise dans ce pot. La somme des transferts d'un
    /// joueur vers tous les autres vaut son résultat net, à l'arrondi au jeton près.
    /// `None` si la main n'est pas résolue.
    pub fn transfer(&self, from: usize, to: usize) -> Option<i64> {
        let settlement = self.settlement()?;
        let pots = pot_shares(&self.state.contributions(), settlement.pots.uncalled);
        let same = pots.len() == settlement.pots.pots.len()
            && pots
                .iter()
                .zip(&settlement.pots.pots)
                .all(|((p, _), q)| p == q);
        if !same {
            return None;
        }
        let claims = self.claims();
        let order = self.state.odd_chip_order();
        let mut flow = 0.0;
        for (pot, shares) in &pots {
            let won = award_pots(std::slice::from_ref(pot), &claims, &order).ok()?;
            let amount = pot.amount.get() as f64;
            flow += (shares[from] as f64 * won[to].get() as f64
                - shares[to] as f64 * won[from].get() as f64)
                / amount;
        }
        Some(flow.round() as i64)
    }
}

/// Reconstitue les pots comme `stratyx_poker::pot::build_pots`, avec la part de chaque
/// joueur dans chacun d'eux.
fn pot_shares(
    contributions: &[Contribution],
    uncalled: Option<(usize, Chips)>,
) -> Vec<(Pot, Vec<u64>)> {
    let n = contributions.len();
    let mut live: Vec<u64> = contributions.iter().map(|c| c.live.get()).collect();
    if let Some((p, amount)) = uncalled {
        live[p] = live[p].saturating_sub(amount.get());
    }
    let folded = |i: usize| contributions[i].folded;
    let in_play: Vec<usize> = (0..n).filter(|i| !folded(*i)).collect();
    let mut levels: Vec<u64> = (0..n)
        .filter(|i| !folded(*i) && live[*i] > 0)
        .map(|i| live[i])
        .collect();
    levels.sort_unstable();
    levels.dedup();

    let mut pots: Vec<(Pot, Vec<u64>)> = Vec::new();
    let mut prev = 0u64;
    for level in levels {
        let shares: Vec<u64> = live.iter().map(|v| v.min(&level) - v.min(&prev)).collect();
        let amount: u64 = shares.iter().sum();
        if amount > 0 {
            let eligible = (0..n)
                .filter(|i| !folded(*i) && live[*i] >= level)
                .collect();
            pots.push((
                Pot {
                    amount: Chips(amount),
                    eligible,
                },
                shares,
            ));
        }
        prev = level;
    }

    let leftover: Vec<u64> = live.iter().map(|v| v.saturating_sub(prev)).collect();
    if leftover.iter().any(|v| *v > 0) {
        match pots.last_mut() {
            Some((pot, shares)) => add_shares(pot, shares, &leftover),
            None => pots.push((
                Pot {
                    amount: Chips(leftover.iter().sum()),
                    eligible: in_play.clone(),
                },
                leftover,
            )),
        }
    }
    let dead: Vec<u64> = contributions.iter().map(|c| c.dead.get()).collect();
    if dead.iter().any(|v| *v > 0) {
        match pots.first_mut() {
            Some((pot, shares)) if pot.eligible == in_play => add_shares(pot, shares, &dead),
            _ => pots.insert(
                0,
                (
                    Pot {
                        amount: Chips(dead.iter().sum()),
                        eligible: in_play,
                    },
                    dead,
                ),
            ),
        }
    }
    pots
}

fn add_shares(pot: &mut Pot, shares: &mut [u64], extra: &[u64]) {
    for (s, e) in shares.iter_mut().zip(extra) {
        *s += e;
    }
    pot.amount += Chips(extra.iter().sum());
}

#[cfg(test)]
mod tests {
    use stratyx_poker::pot::build_pots;

    use super::*;

    fn c(player: usize, live: u64, dead: u64, folded: bool) -> Contribution {
        Contribution {
            player,
            live: Chips(live),
            dead: Chips(dead),
            folded,
        }
    }

    #[test]
    fn pot_shares_match_engine_pots() {
        let cases = vec![
            vec![
                c(0, 300, 0, false),
                c(1, 1000, 0, false),
                c(2, 600, 0, false),
            ],
            vec![c(0, 50, 0, true), c(1, 100, 0, false)],
            vec![
                c(0, 500, 0, false),
                c(1, 200, 0, true),
                c(2, 500, 100, false),
            ],
            vec![c(0, 100, 0, false), c(1, 400, 0, true), c(2, 250, 0, false)],
            vec![
                c(0, 0, 100, false),
                c(1, 300, 0, false),
                c(2, 300, 0, false),
            ],
        ];
        for contribs in cases {
            let engine = build_pots(&contribs);
            let ours = pot_shares(&contribs, engine.uncalled);
            let pots: Vec<Pot> = ours.iter().map(|(p, _)| p.clone()).collect();
            assert_eq!(pots, engine.pots, "{contribs:?}");
            for (pot, shares) in &ours {
                assert_eq!(shares.iter().sum::<u64>(), pot.amount.get());
            }
        }
    }
}
