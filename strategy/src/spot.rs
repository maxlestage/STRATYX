//! Une situation de décision (« spot ») et ses caractéristiques numériques, communes à
//! l'analyse, aux simulations et aux modèles.
//!
//! [`Spot`] décrit le contexte, visible de tous (position, tapis, pot, mises, actions du
//! tour) ; [`HandFeatures`] décrit la main du joueur qui décide. Le modèle de population
//! n'utilise que le contexte (les archives ne montrent pas les cartes des adversaires), le
//! modèle de décision utilise les deux.

use serde::{Deserialize, Serialize};
use stratyx_poker::game::{ActionKind, PlayerStatus};
use stratyx_poker::{Card, CardSet, HandCategory, HandState, HoleCards, Rank, Street, evaluate};

use crate::preflop;

/// Les quatre familles de décision apprises par les modèles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Se coucher.
    Fold,
    /// Parole ou suivre.
    Passive,
    /// Miser ou relancer (sans faire tapis).
    Raise,
    /// Faire tapis.
    AllIn,
}

impl Decision {
    pub const ALL: [Decision; 4] = [
        Decision::Fold,
        Decision::Passive,
        Decision::Raise,
        Decision::AllIn,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn from_index(i: usize) -> Decision {
        match i {
            0 => Decision::Fold,
            1 => Decision::Passive,
            2 => Decision::Raise,
            _ => Decision::AllIn,
        }
    }

    /// Libellé français, selon qu'il y a une mise à suivre ou non.
    #[must_use]
    pub const fn label(self, facing_bet: bool) -> &'static str {
        match (self, facing_bet) {
            (Decision::Fold, _) => "se coucher",
            (Decision::Passive, true) => "suivre",
            (Decision::Passive, false) => "parler",
            (Decision::Raise, true) => "relancer",
            (Decision::Raise, false) => "miser",
            (Decision::AllIn, _) => "faire tapis",
        }
    }
}

/// Le contexte d'une décision, en grosses blindes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    /// Joueurs servis.
    pub players: u8,
    /// Joueurs encore dans le coup (le joueur qui décide compris).
    pub active: u8,
    /// Sièges après le bouton (0 : bouton).
    pub offset: u8,
    /// Rang dans l'ordre de parole préflop (0 : premier à parler).
    pub preflop_order: u8,
    /// Joueurs qui parlent encore après lui au préflop (hors joueurs couchés).
    pub behind: u8,
    pub street: Street,
    /// Tapis restant du joueur.
    pub stack_bb: f32,
    /// Tapis effectif : le plus petit entre le sien et le plus gros adversaire en jeu.
    pub effective_bb: f32,
    /// Pot avant la décision (mises du tour comprises).
    pub pot_bb: f32,
    /// Montant à ajouter pour suivre.
    pub to_call_bb: f32,
    /// Mises et relances du tour avant lui.
    pub raises: u8,
    /// Joueurs qui ont suivi la dernière mise du tour (ou limpé).
    pub callers: u8,
    /// Une mise du tour est un tapis.
    pub facing_all_in: bool,
    /// Il a fait la dernière mise ou relance du tour précédent.
    pub aggressor: bool,
    /// Il parle en dernier après le flop parmi les joueurs en jeu.
    pub in_position: bool,
    /// Antes dans la structure.
    pub ante: bool,
    /// Personne n'a misé volontairement avant lui au préflop.
    pub unopened: bool,
}

impl Spot {
    /// Le contexte du joueur `player` dans `state` (il doit être celui qui parle).
    #[must_use]
    pub fn from_state(state: &HandState, player: usize) -> Spot {
        let players = state.players();
        let n = players.len();
        let bb = state.blinds().big_blind.get().max(1) as f32;
        let me = &players[player];
        let offset = (player + n - state.button()) % n;
        let street = state.street();
        let actions = state.actions();

        let in_hand = |i: usize| players[i].status != PlayerStatus::Folded;
        let active = (0..n).filter(|i| in_hand(*i)).count() as u8;
        let max_other = (0..n)
            .filter(|i| *i != player && in_hand(*i))
            .map(|i| players[i].stack.get() + players[i].street_bet.get())
            .max()
            .unwrap_or(0);
        let mine = me.stack.get() + me.street_bet.get();

        let this_street: Vec<_> = actions
            .iter()
            .filter(|a| a.street == street && a.kind.is_voluntary())
            .collect();
        let raises = this_street
            .iter()
            .filter(|a| a.kind.is_aggressive())
            .count() as u8;
        let last_raise = this_street.iter().rposition(|a| a.kind.is_aggressive());
        let callers = match last_raise {
            Some(i) => this_street[i + 1..]
                .iter()
                .filter(|a| a.kind == ActionKind::Call)
                .count(),
            None => this_street
                .iter()
                .filter(|a| a.kind == ActionKind::Call)
                .count(),
        } as u8;
        let facing_all_in = this_street
            .iter()
            .any(|a| a.kind.is_aggressive() && a.all_in);
        let aggressor = previous(street).is_some_and(|prev| {
            actions
                .iter()
                .rev()
                .find(|a| a.street == prev && a.kind.is_aggressive())
                .is_some_and(|a| a.player == player)
        });
        // Ordre après le flop : le bouton parle en dernier.
        let post_rank = |i: usize| ((i + n - state.button()) % n + n - 1) % n;
        let in_position = (0..n)
            .filter(|i| in_hand(*i) && *i != player)
            .all(|i| post_rank(i) < post_rank(player));
        // Ordre préflop : la grosse blinde parle en dernier (en tête-à-tête, le bouton d'abord).
        let pre_rank = |i: usize| {
            let off = (i + n - state.button()) % n;
            if n == 2 { off } else { (off + n - 3) % n }
        };
        let preflop_order = pre_rank(player) as u8;
        let behind = (0..n)
            .filter(|i| in_hand(*i) && pre_rank(*i) > pre_rank(player))
            .count() as u8;
        let unopened = !actions.iter().any(|a| {
            a.street == Street::Preflop
                && a.kind.is_voluntary()
                && a.kind != ActionKind::Fold
                && a.kind != ActionKind::Check
        });

        Spot {
            players: n as u8,
            active,
            offset: offset as u8,
            preflop_order,
            behind,
            street,
            stack_bb: me.stack.get() as f32 / bb,
            effective_bb: mine.min(max_other) as f32 / bb,
            pot_bb: state.pot_total().get() as f32 / bb,
            to_call_bb: state
                .current_bet()
                .saturating_sub(me.street_bet)
                .get()
                .min(me.stack.get()) as f32
                / bb,
            raises,
            callers,
            facing_all_in,
            aggressor,
            in_position,
            ante: !matches!(state.blinds().ante, stratyx_poker::game::Ante::None),
            unopened,
        }
    }

    /// Cote du pot : part du pot final à gagner pour qu'un call soit rentable.
    #[must_use]
    pub fn pot_odds(&self) -> f32 {
        if self.to_call_bb <= 0.0 {
            0.0
        } else {
            self.to_call_bb / (self.pot_bb + self.to_call_bb)
        }
    }

    /// Les caractéristiques du contexte (modèle de population).
    #[must_use]
    pub fn context_features(&self) -> Vec<f32> {
        let n = f32::from(self.players.max(2));
        let street = |s: Street| f32::from(u8::from(self.street == s));
        let ln = |x: f32| (1.0 + x.max(0.0)).ln();
        vec![
            n / 10.0,
            f32::from(self.active) / n,
            f32::from(self.preflop_order) / (n - 1.0),
            f32::from(self.behind) / (n - 1.0),
            f32::from(u8::from(self.offset == 0)),
            f32::from(u8::from(self.offset == 1 && self.players > 2)),
            f32::from(u8::from(
                self.offset == 2 || (self.players == 2 && self.offset == 1),
            )),
            street(Street::Preflop),
            street(Street::Flop),
            street(Street::Turn),
            street(Street::River),
            ln(self.stack_bb) / 4.6,
            ln(self.effective_bb) / 4.6,
            ln(self.pot_bb) / 4.6,
            ln(self.to_call_bb) / 4.6,
            self.pot_odds(),
            (self.effective_bb / self.pot_bb.max(0.5)).min(20.0) / 20.0,
            f32::from(self.raises.min(4)) / 4.0,
            f32::from(self.callers.min(4)) / 4.0,
            f32::from(u8::from(self.facing_all_in)),
            f32::from(u8::from(self.aggressor)),
            f32::from(u8::from(self.in_position)),
            f32::from(u8::from(self.ante)),
            f32::from(u8::from(self.unopened)),
            f32::from(u8::from(
                self.to_call_bb >= self.stack_bb && self.to_call_bb > 0.0,
            )),
        ]
    }
}

/// Ce que l'on sait de la main du joueur qui décide.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HandFeatures {
    /// Classe préflop (0..169).
    pub class: u8,
    /// Percentile préflop (0 : AA).
    pub percentile: f32,
    /// Équité préflop contre autant de mains au hasard que d'adversaires en jeu.
    pub preflop_strength: f32,
    pub pair: bool,
    pub suited: bool,
    /// Écart entre les deux rangs (0 : paire ou connecteurs).
    pub gap: u8,
    /// Rang de la plus haute carte (2 → 0, As → 12).
    pub high: u8,
    /// Catégorie de main faite sur le board (0 : hauteur … 8 : quinte flush).
    pub category: u8,
    /// La main utilise au moins une carte privée pour sa catégorie (pas seulement le board).
    pub uses_hole: bool,
    /// Paire avec la plus haute carte du board, ou mieux.
    pub top_pair_plus: bool,
    pub flush_draw: bool,
    pub straight_draw: bool,
    pub board_paired: bool,
    /// Couleurs du board : 1 (monochrome) à 4.
    pub board_suits: u8,
    /// Équité contre des mains au hasard sur ce board (estimée), 0 au préflop.
    pub equity_random: f32,
}

impl HandFeatures {
    /// Les caractéristiques de `hole` sur `board`, contre `opponents` adversaires.
    /// `equity_random` est fournie par l'appelant (Monte-Carlo, voir [`crate::mc`]).
    #[must_use]
    pub fn new(hole: HoleCards, board: &[Card], opponents: usize, equity_random: f32) -> Self {
        let class = preflop::hole_class(hole);
        let [a, b] = hole.cards();
        let (hi, lo) = if a.rank() >= b.rank() {
            (a.rank(), b.rank())
        } else {
            (b.rank(), a.rank())
        };
        let mut f = HandFeatures {
            class: class as u8,
            percentile: preflop::percentile(class),
            preflop_strength: preflop::strength(class, opponents.max(1)),
            pair: hi == lo,
            suited: a.suit() == b.suit(),
            gap: (hi.index() - lo.index()).saturating_sub(1),
            high: hi.index(),
            category: 0,
            uses_hole: false,
            top_pair_plus: false,
            flush_draw: false,
            straight_draw: false,
            board_paired: false,
            board_suits: 0,
            equity_random,
        };
        if board.len() < 3 {
            return f;
        }
        let bset: CardSet = board.iter().collect();
        let all = bset | hole.set();
        let rank = evaluate(all);
        f.category = rank.category() as u8;
        f.uses_hole = board.len() < 5 || evaluate(bset) < rank;
        if board.len() == 5 {
            f.uses_hole = evaluate(bset) < rank;
        }
        let top = board.iter().map(|c| c.rank()).max().unwrap_or(Rank::Two);
        f.top_pair_plus = rank.category() > HandCategory::OnePair
            || (rank.category() == HandCategory::OnePair
                && (hi == top || lo == top || (f.pair && hi > top)));
        let mut ranks_on_board = [0u8; 13];
        for c in board {
            ranks_on_board[c.rank().index() as usize] += 1;
        }
        f.board_paired = ranks_on_board.iter().any(|n| *n >= 2);
        let suits: std::collections::HashSet<_> = board.iter().map(|c| c.suit()).collect();
        f.board_suits = suits.len() as u8;
        if board.len() < 5 && rank.category() < HandCategory::Flush {
            f.flush_draw = stratyx_poker::Suit::ALL
                .iter()
                .any(|s| all.suit_mask(*s).count_ones() == 4 && (a.suit() == *s || b.suit() == *s));
            f.straight_draw = rank.category() < HandCategory::Straight && straight_outs(all) >= 2;
        }
        f
    }

    /// Les caractéristiques de la main (modèle de décision).
    #[must_use]
    pub fn features(&self) -> Vec<f32> {
        let mut v = vec![
            1.0 - self.percentile,
            self.preflop_strength,
            f32::from(u8::from(self.pair)),
            f32::from(u8::from(self.suited)),
            f32::from(self.gap.min(5)) / 5.0,
            f32::from(self.high) / 12.0,
            f32::from(self.category) / 8.0,
            f32::from(u8::from(self.uses_hole)),
            f32::from(u8::from(self.top_pair_plus)),
            f32::from(u8::from(self.flush_draw)),
            f32::from(u8::from(self.straight_draw)),
            f32::from(u8::from(self.board_paired)),
            f32::from(self.board_suits) / 4.0,
            self.equity_random,
        ];
        for c in 0..9u8 {
            v.push(f32::from(u8::from(self.category == c && self.uses_hole)));
        }
        v
    }
}

/// Le tour précédent.
#[must_use]
pub const fn previous(s: Street) -> Option<Street> {
    match s {
        Street::Preflop => None,
        Street::Flop => Some(Street::Preflop),
        Street::Turn => Some(Street::Flop),
        Street::River => Some(Street::Turn),
    }
}

/// Rangs (sur 13) qui donneraient une quinte avec une carte de plus.
fn straight_outs(cards: CardSet) -> u32 {
    let mut ranks: u16 = 0;
    for s in stratyx_poker::Suit::ALL {
        ranks |= cards.suit_mask(s);
    }
    let has = |m: u16| {
        let e = (u32::from(m) << 1) | u32::from(m >> 12 & 1);
        let s = e & (e >> 1) & (e >> 2) & (e >> 3) & (e >> 4);
        s != 0
    };
    (0..13)
        .filter(|r| ranks & (1 << r) == 0 && has(ranks | (1 << r)))
        .count() as u32
}

/// Toutes les caractéristiques du modèle de décision : contexte puis main.
#[must_use]
pub fn decision_features(spot: &Spot, hand: &HandFeatures) -> Vec<f32> {
    let mut v = spot.context_features();
    v.extend(hand.features());
    // Interactions utiles à un modèle linéaire.
    let pot_odds = spot.pot_odds();
    let strength = if spot.street == Street::Preflop {
        hand.preflop_strength
    } else {
        hand.equity_random
    };
    v.push(strength - pot_odds);
    v.push((1.0 - hand.percentile) * (1.0 / (1.0 + spot.effective_bb / 10.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::game::{Action, Blinds, HandSetup};
    use stratyx_shared::Chips;

    fn hole(s: &str) -> HoleCards {
        s.parse().unwrap()
    }

    #[test]
    fn spot_from_state() {
        // 6 joueurs, bouton 0 : UTG (3) relance, HJ (4) suit, CO (5) parle.
        let setup = HandSetup::new(vec![Chips(2000); 6], 0, Blinds::new(50, 100));
        let mut st = HandState::new(&setup).unwrap();
        st.apply(3, Action::Raise(Chips(250))).unwrap();
        st.apply(4, Action::Call).unwrap();
        let s = Spot::from_state(&st, 5);
        assert_eq!(s.players, 6);
        assert_eq!(s.offset, 5);
        assert_eq!(s.preflop_order, 2);
        assert_eq!(s.behind, 3);
        assert_eq!(s.raises, 1);
        assert_eq!(s.callers, 1);
        assert!(!s.unopened);
        assert!((s.to_call_bb - 2.5).abs() < 1e-6);
        assert!((s.pot_bb - 6.5).abs() < 1e-6);
        assert!(!s.in_position, "le bouton et les blindes sont encore là");
        let f = decision_features(&s, &HandFeatures::new(hole("AhKh"), &[], 2, 0.0));
        assert_eq!(f.len(), s.context_features().len() + 23 + 2);
    }

    #[test]
    fn hand_features() {
        let board: Vec<Card> = stratyx_poker::card::parse_cards("Qh7h2c").unwrap();
        let f = HandFeatures::new(hole("AhKh"), &board, 1, 0.5);
        assert!(f.flush_draw && !f.top_pair_plus && f.category == 0);
        let f = HandFeatures::new(hole("QsJd"), &board, 1, 0.7);
        assert!(f.top_pair_plus && f.category == 1);
        let board: Vec<Card> = stratyx_poker::card::parse_cards("9c8d2h").unwrap();
        assert!(HandFeatures::new(hole("Ts7s"), &board, 1, 0.5).straight_draw);
    }
}
