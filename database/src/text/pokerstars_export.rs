//! Export d'un [`HandRecord`] au format texte de PokerStars.

use std::fmt::Write as _;

use stratyx_poker::game::{
    ActionKind, AppliedAction, BettingStructure, Phase, PlayerStatus, Reveal, Street,
};
use stratyx_poker::history::{Outcome, Replay};
use stratyx_poker::pot::{Claim, award_pots, build_pots};
use stratyx_poker::{Card, CardSet, GameType, HandEvent, HandRecord, HoleCards, evaluate};
use stratyx_shared::Chips;

use super::amount::{currency_symbol, format_amount};
use super::datetime::format_header_date;
use super::describe::describe;
use super::{ExportError, ExportOptions, TournamentInfo};

/// Écrit une main au format PokerStars. La main doit se rejouer sans erreur (les montants
/// des suivis et des tapis sont ceux calculés par le moteur).
pub fn export_pokerstars(
    record: &HandRecord,
    options: &ExportOptions,
) -> Result<String, ExportError> {
    let replay = record.replay().map_err(ExportError::Replay)?;
    check_names(record)?;
    let w = Writer::new(record, options, &replay);
    Ok(w.write())
}

/// Écrit plusieurs mains, séparées par des lignes vides comme dans les fichiers de la salle.
pub fn export_pokerstars_many<'a>(
    records: impl IntoIterator<Item = &'a HandRecord>,
    options: &ExportOptions,
) -> Result<String, ExportError> {
    let mut out = String::new();
    for r in records {
        out.push_str(&export_pokerstars(r, options)?);
        out.push_str("\n\n");
    }
    Ok(out)
}

fn check_names(record: &HandRecord) -> Result<(), ExportError> {
    let id = &record.id;
    if id.is_empty() || id.contains(':') || id.chars().any(char::is_whitespace) {
        return Err(ExportError::HandId(id.clone()));
    }
    if let Some(t) = &record.meta.table
        && (t.contains('\n') || t.contains(" Seat #"))
    {
        return Err(ExportError::TableName(t.clone()));
    }
    let mut names: Vec<&str> = Vec::new();
    for s in &record.seats {
        let n = s.player.as_str();
        let bad = n.is_empty()
            || n.trim() != n
            || n.contains('\n')
            || n.contains(" in chips")
            || names.contains(&n);
        if bad {
            return Err(ExportError::PlayerName(n.to_string()));
        }
        names.push(n);
    }
    Ok(())
}

struct Writer<'a> {
    record: &'a HandRecord,
    options: &'a ExportOptions,
    replay: &'a Replay,
    /// Symbole monétaire ; `None` : jetons.
    symbol: Option<&'static str>,
    out: String,
}

impl<'a> Writer<'a> {
    fn new(record: &'a HandRecord, options: &'a ExportOptions, replay: &'a Replay) -> Self {
        let cash = record.format.game_type == GameType::CashGame;
        let symbol = options
            .currency
            .as_deref()
            .filter(|_| cash)
            .map(currency_symbol);
        Writer {
            record,
            options,
            replay,
            symbol,
            out: String::new(),
        }
    }

    fn amt(&self, c: Chips) -> String {
        format_amount(c, self.symbol)
    }

    fn line(&mut self, s: impl AsRef<str>) {
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    /// Pseudonyme du joueur d'index moteur `p`.
    fn name_of(&self, p: usize) -> &'a str {
        self.name_at(self.replay.seats[p])
    }

    fn name_at(&self, seat: u8) -> &'a str {
        self.record
            .seats
            .iter()
            .find(|s| s.seat == seat)
            .map_or("?", |s| s.player.as_str())
    }

    fn index_of(&self, seat: u8) -> Option<usize> {
        self.replay.seats.iter().position(|s| *s == seat)
    }

    fn header(&mut self) {
        let r = self.record;
        let b = r.blinds.value;
        let structure = match r.format.structure {
            BettingStructure::NoLimit => "No Limit",
            BettingStructure::PotLimit => "Pot Limit",
            BettingStructure::FixedLimit { .. } => "Limit",
        };
        let (lo, hi) = match r.format.structure {
            BettingStructure::FixedLimit { .. } => (b.big_blind, b.big_blind + b.big_blind),
            _ => (b.small_blind, b.big_blind),
        };
        let date = r
            .meta
            .played_at_ms
            .map(|ms| format!(" - {}", format_header_date(ms)))
            .unwrap_or_default();
        let line = if r.format.game_type == GameType::CashGame {
            let code = self
                .options
                .currency
                .as_deref()
                .map(|c| format!(" {c}"))
                .unwrap_or_default();
            format!(
                "PokerStars Hand #{}:  Hold'em {structure} ({}/{}{code}){date}",
                r.id,
                self.amt(lo),
                self.amt(hi)
            )
        } else {
            let info = self.options.tournament.clone().unwrap_or(TournamentInfo {
                id: "0".into(),
                buy_in: None,
                level: None,
            });
            let buy_in = match &info.buy_in {
                None => String::new(),
                Some(b) if b.total == 0 => "Freeroll ".to_string(),
                Some(b) => {
                    let sym = b.currency.as_deref().map(currency_symbol);
                    let code = b
                        .currency
                        .as_deref()
                        .map(|c| format!(" {c}"))
                        .unwrap_or_default();
                    let money = |v: u64| format_amount(Chips(v), Some(sym.unwrap_or("")));
                    format!(
                        "{}+{}{code} ",
                        money(b.total.saturating_sub(b.fee)),
                        money(b.fee)
                    )
                }
            };
            format!(
                "PokerStars Hand #{}: Tournament #{}, {buy_in}Hold'em {structure} - Level {} ({lo}/{hi}){date}",
                r.id,
                info.id,
                info.level.as_deref().unwrap_or("I"),
            )
        };
        self.line(line);
        self.line(format!(
            "Table '{}' {}-max Seat #{} is the button",
            r.meta.table.as_deref().unwrap_or(""),
            r.format.table_size,
            r.button_seat
        ));
    }

    fn seats_and_posts(&mut self) {
        let mut seats: Vec<_> = self.record.seats.iter().collect();
        seats.sort_by_key(|s| s.seat);
        for s in seats {
            let stack = self.amt(s.stack.value);
            self.line(format!("Seat {}: {} ({stack} in chips)", s.seat, s.player));
        }
        for a in self.replay.state.actions() {
            let verb = match a.kind {
                ActionKind::PostAnte => "posts the ante",
                ActionKind::PostSmallBlind => "posts small blind",
                ActionKind::PostBigBlind => "posts big blind",
                _ => continue,
            };
            let line = format!(
                "{}: {verb} {}{}",
                self.name_of(a.player),
                self.amt(a.amount),
                all_in(a)
            );
            self.line(line);
        }
    }

    fn hole_cards(&mut self) {
        self.line("*** HOLE CARDS ***");
        let r = self.record;
        let mut seats: Vec<_> = r.seats.iter().collect();
        seats.sort_by_key(|s| (Some(s.seat) != r.meta.hero_seat, s.seat));
        for s in seats {
            match &s.hole_cards {
                Some(h) => self.line(format!(
                    "Dealt to {} {}",
                    s.player,
                    bracket(&h.value.cards())
                )),
                None if Some(s.seat) == r.meta.hero_seat => {
                    self.line(format!("Dealt to {}", s.player));
                }
                None => {}
            }
        }
    }

    /// Cartes connues de chaque joueur à la fin de la main (comme le rejeu).
    fn reveals(&self) -> Vec<Reveal> {
        let mut reveals: Vec<Reveal> = self
            .replay
            .seats
            .iter()
            .map(|seat| {
                self.record
                    .seats
                    .iter()
                    .find(|s| s.seat == *seat)
                    .and_then(|s| s.hole_cards.as_ref())
                    .map_or(Reveal::Unknown, |h| Reveal::Shown(h.value))
            })
            .collect();
        for ev in &self.record.events {
            match &ev.value {
                HandEvent::Show { seat, cards } => {
                    if let Some(p) = self.index_of(*seat) {
                        reveals[p] = Reveal::Shown(*cards);
                    }
                }
                HandEvent::Muck { seat } => {
                    if let Some(p) = self.index_of(*seat) {
                        reveals[p] = Reveal::Mucked;
                    }
                }
                _ => {}
            }
        }
        reveals
    }

    /// Description de la main montrée, si le board est complet.
    fn description(&self, cards: HoleCards) -> Option<String> {
        let board: CardSet = self.replay.state.board().iter().collect();
        (board.len() == 5).then(|| describe(evaluate(board | cards.set())))
    }

    fn body(&mut self) {
        let voluntary: Vec<&AppliedAction> = self
            .replay
            .state
            .actions()
            .iter()
            .filter(|a| a.kind.is_voluntary())
            .collect();
        let events = &self.record.events;
        let last_action = events
            .iter()
            .rposition(|e| matches!(e.value, HandEvent::Action { .. }));
        let showdown = self.replay.state.phase() == Phase::Showdown;
        let pots = build_pots(&self.replay.state.contributions());
        let mut next = 0;
        let mut current_bet = self.record.blinds.value.big_blind;
        let mut board: Vec<Card> = Vec::new();
        let mut showdown_written = false;
        let mut uncalled_written = false;

        for (i, ev) in events.iter().enumerate() {
            let betting_over = last_action.is_none_or(|l| i > l);
            match &ev.value {
                HandEvent::Action { .. } => {
                    let Some(a) = voluntary.get(next).copied() else {
                        continue;
                    };
                    next += 1;
                    let verb = match a.kind {
                        ActionKind::Fold => "folds".to_string(),
                        ActionKind::Check => "checks".to_string(),
                        ActionKind::Call => format!("calls {}", self.amt(a.amount)),
                        ActionKind::Bet => format!("bets {}", self.amt(a.to)),
                        ActionKind::Raise => format!(
                            "raises {} to {}",
                            self.amt(a.to.saturating_sub(current_bet)),
                            self.amt(a.to)
                        ),
                        _ => continue,
                    };
                    if a.kind.is_aggressive() {
                        current_bet = a.to;
                    }
                    let line = format!("{}: {verb}{}", self.name_of(a.player), all_in(a));
                    self.line(line);
                    if Some(i) == last_action {
                        self.uncalled(&pots);
                        uncalled_written = true;
                    }
                }
                HandEvent::Board { cards } => {
                    let before = bracket(&board);
                    board.extend_from_slice(cards);
                    current_bet = Chips::ZERO;
                    let line = match board.len() {
                        3 => format!("*** FLOP *** {}", bracket(&board)),
                        4 => format!("*** TURN *** {before} {}", bracket(cards)),
                        _ => format!("*** RIVER *** {before} {}", bracket(cards)),
                    };
                    self.line(line);
                }
                HandEvent::Show { seat, cards } => {
                    if showdown && betting_over && !showdown_written {
                        self.line("*** SHOW DOWN ***");
                        showdown_written = true;
                    }
                    let desc = self
                        .description(*cards)
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default();
                    let line = format!(
                        "{}: shows {}{desc}",
                        self.name_at(*seat),
                        bracket(&cards.cards())
                    );
                    self.line(line);
                }
                HandEvent::Muck { seat } => {
                    if showdown && betting_over && !showdown_written {
                        self.line("*** SHOW DOWN ***");
                        showdown_written = true;
                    }
                    let line = format!("{}: mucks hand", self.name_at(*seat));
                    self.line(line);
                }
            }
        }
        if !uncalled_written {
            self.uncalled(&pots);
        }
        if showdown && !showdown_written {
            self.line("*** SHOW DOWN ***");
        }
        self.collected();
    }

    fn uncalled(&mut self, pots: &stratyx_poker::pot::PotBreakdown) {
        if !matches!(self.replay.state.phase(), Phase::Showdown | Phase::Finished) {
            return;
        }
        if let Some((p, amount)) = pots.uncalled {
            let line = format!(
                "Uncalled bet ({}) returned to {}",
                self.amt(amount),
                self.name_of(p)
            );
            self.line(line);
        }
    }

    /// Gains de chaque joueur, pot par pot (pots secondaires d'abord, comme la salle).
    fn pot_awards(&self) -> Option<Vec<Vec<Chips>>> {
        let Outcome::Settled(settlement) = &self.replay.outcome else {
            return None;
        };
        let board: CardSet = self.replay.state.board().iter().collect();
        let claims: Vec<Claim> = self
            .reveals()
            .iter()
            .map(|r| match r {
                Reveal::Shown(h) if board.len() == 5 => Claim::Shown(evaluate(board | h.set())),
                Reveal::Shown(_) | Reveal::Unknown => Claim::Unknown,
                Reveal::Mucked => Claim::Mucked,
            })
            .collect();
        let order = self.replay.state.odd_chip_order();
        settlement
            .pots
            .pots
            .iter()
            .map(|pot| award_pots(std::slice::from_ref(pot), &claims, &order).ok())
            .collect()
    }

    fn collected(&mut self) {
        let Some(awards) = self.pot_awards() else {
            return;
        };
        let n = awards.len();
        for (k, award) in awards.iter().enumerate().rev() {
            let label = match (n, k) {
                (1, _) => "pot".to_string(),
                (_, 0) => "main pot".to_string(),
                (2, _) => "side pot".to_string(),
                _ => format!("side pot-{k}"),
            };
            for (p, amount) in award.iter().enumerate() {
                if !amount.is_zero() {
                    let line = format!(
                        "{} collected {} from {label}",
                        self.name_of(p),
                        self.amt(*amount)
                    );
                    self.line(line);
                }
            }
        }
    }

    fn summary(&mut self) {
        self.line("*** SUMMARY ***");
        let pots = build_pots(&self.replay.state.contributions());
        let total: Chips = pots.total();
        let mut line = format!("Total pot {}", self.amt(total));
        if pots.pots.len() > 1 {
            let _ = write!(line, " Main pot {}.", self.amt(pots.pots[0].amount));
            for (k, pot) in pots.pots.iter().enumerate().skip(1) {
                let label = if pots.pots.len() == 2 {
                    "Side pot".to_string()
                } else {
                    format!("Side pot-{k}")
                };
                let _ = write!(line, " {label} {}.", self.amt(pot.amount));
            }
        }
        line.push_str(" | Rake ");
        line.push_str(&self.amt(Chips::ZERO));
        self.line(line);
        let board = self.replay.state.board().to_vec();
        if !board.is_empty() {
            self.line(format!("Board {}", bracket(&board)));
        }

        let state = &self.replay.state;
        let (sb, bb) = state.blind_seats();
        let won: Vec<Chips> = match self.pot_awards() {
            Some(awards) => (0..state.players().len())
                .map(|p| awards.iter().map(|a| a[p]).sum())
                .collect(),
            None => vec![Chips::ZERO; state.players().len()],
        };
        let reveals = self.reveals();
        let showdown = state.phase() == Phase::Showdown;
        for (p, seat) in self.replay.seats.iter().enumerate() {
            let mut line = format!("Seat {seat}: {}", self.name_of(p));
            if p == state.button() {
                line.push_str(" (button)");
            }
            if p == sb {
                line.push_str(" (small blind)");
            } else if p == bb {
                line.push_str(" (big blind)");
            }
            let player = &state.players()[p];
            if player.status == PlayerStatus::Folded {
                let fold = state
                    .actions()
                    .iter()
                    .find(|a| a.player == p && a.kind == ActionKind::Fold);
                match fold.map(|a| a.street) {
                    Some(Street::Preflop) | None => {
                        line.push_str(" folded before Flop");
                        let put_in = state.actions().iter().any(|a| {
                            a.player == p && a.kind != ActionKind::PostAnte && !a.amount.is_zero()
                        });
                        if !put_in {
                            line.push_str(" (didn't bet)");
                        }
                    }
                    Some(street) => {
                        let name = match street {
                            Street::Flop => "Flop",
                            Street::Turn => "Turn",
                            _ => "River",
                        };
                        let _ = write!(line, " folded on the {name}");
                    }
                }
            } else if showdown {
                match reveals[p] {
                    Reveal::Shown(h) => {
                        let cards = bracket(&h.cards());
                        let desc = self
                            .description(h)
                            .map(|d| format!(" with {d}"))
                            .unwrap_or_default();
                        if won[p].is_zero() {
                            let _ = write!(line, " showed {cards} and lost{desc}");
                        } else {
                            let _ = write!(
                                line,
                                " showed {cards} and won ({}){desc}",
                                self.amt(won[p])
                            );
                        }
                    }
                    Reveal::Mucked => {
                        line.push_str(" mucked");
                        // Cartes connues dès la distribution (héros) : la salle les rappelle.
                        let known = self
                            .record
                            .seats
                            .iter()
                            .find(|s| s.seat == *seat)
                            .and_then(|s| s.hole_cards.as_ref());
                        if let Some(h) = known {
                            let _ = write!(line, " {}", bracket(&h.value.cards()));
                        }
                    }
                    Reveal::Unknown => {}
                }
            } else if !won[p].is_zero() {
                let _ = write!(line, " collected ({})", self.amt(won[p]));
            }
            self.line(line);
        }
    }

    fn write(mut self) -> String {
        self.header();
        self.seats_and_posts();
        self.hole_cards();
        self.body();
        self.summary();
        self.out
    }
}

fn all_in(a: &AppliedAction) -> &'static str {
    if a.all_in { " and is all-in" } else { "" }
}

fn bracket(cards: &[Card]) -> String {
    let list: Vec<String> = cards.iter().map(ToString::to_string).collect();
    format!("[{}]", list.join(" "))
}
