//! Construction d'historiques à la main pour les tests.

#![allow(dead_code)]

use stratyx_poker::card::parse_cards;
use stratyx_poker::format::GameFormat;
use stratyx_poker::game::{Action, Blinds};
use stratyx_poker::history::{HandEvent, HandMeta, HandRecord, SeatRecord};
use stratyx_shared::{Chips, Observed, Source};

pub fn ok<T>(v: T) -> Observed<T> {
    Observed::certain(v, Source::Import)
}

pub struct HandBuilder {
    record: HandRecord,
}

impl HandBuilder {
    pub fn new(id: &str, format: GameFormat, blinds: Blinds, button_seat: u8) -> HandBuilder {
        HandBuilder {
            record: HandRecord {
                id: id.into(),
                format,
                blinds: ok(blinds),
                button_seat,
                seats: Vec::new(),
                events: Vec::new(),
                meta: HandMeta::default(),
            },
        }
    }

    pub fn seat(mut self, seat: u8, name: &str, stack: u64) -> Self {
        self.record.seats.push(SeatRecord {
            seat,
            player: name.into(),
            stack: ok(Chips(stack)),
            hole_cards: None,
        });
        self
    }

    /// Désigne le héros et lui donne ses cartes.
    pub fn hero(mut self, seat: u8, cards: &str) -> Self {
        let s = self
            .record
            .seats
            .iter_mut()
            .find(|s| s.seat == seat)
            .expect("siège du héros");
        s.hole_cards = Some(ok(cards.parse().unwrap()));
        self.record.meta.hero_seat = Some(seat);
        self
    }

    pub fn played_at(mut self, ms: u64) -> Self {
        self.record.meta.played_at_ms = Some(ms);
        self
    }

    pub fn act(mut self, seat: u8, action: Action) -> Self {
        self.record
            .events
            .push(ok(HandEvent::Action { seat, action }));
        self
    }

    pub fn fold(self, seat: u8) -> Self {
        self.act(seat, Action::Fold)
    }

    pub fn check(self, seat: u8) -> Self {
        self.act(seat, Action::Check)
    }

    pub fn call(self, seat: u8) -> Self {
        self.act(seat, Action::Call)
    }

    pub fn bet(self, seat: u8, to: u64) -> Self {
        self.act(seat, Action::Bet(Chips(to)))
    }

    pub fn raise(self, seat: u8, to: u64) -> Self {
        self.act(seat, Action::Raise(Chips(to)))
    }

    pub fn all_in(self, seat: u8) -> Self {
        self.act(seat, Action::AllIn)
    }

    pub fn board(mut self, cards: &str) -> Self {
        self.record.events.push(ok(HandEvent::Board {
            cards: parse_cards(cards).unwrap(),
        }));
        self
    }

    pub fn show(mut self, seat: u8, cards: &str) -> Self {
        self.record.events.push(ok(HandEvent::Show {
            seat,
            cards: cards.parse().unwrap(),
        }));
        self
    }

    pub fn muck(mut self, seat: u8) -> Self {
        self.record.events.push(ok(HandEvent::Muck { seat }));
        self
    }

    pub fn build(self) -> HandRecord {
        self.record
            .replay()
            .unwrap_or_else(|e| panic!("main de test invalide ({}) : {e}", self.record.id));
        self.record
    }
}

/// Table 6-max no-limit, bouton au siège 1 : BTN 1, SB 2, BB 3, UTG 4, HJ 5, CO 6.
pub fn six_max(id: &str, format: GameFormat, stack: u64) -> HandBuilder {
    HandBuilder::new(id, format, Blinds::new(50, 100), 1)
        .seat(1, "Btn", stack)
        .seat(2, "Sb", stack)
        .seat(3, "Bb", stack)
        .seat(4, "Utg", stack)
        .seat(5, "Hj", stack)
        .seat(6, "Co", stack)
}
