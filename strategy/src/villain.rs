//! Les ranges probables des adversaires, déduites de leurs actions dans la main.
//!
//! Au préflop, chaque action resserre la range selon le rang préflop des mains (ouverture
//! selon le nombre de joueurs derrière, 3-bet, 4-bet, limp, call, tapis) ; après le flop,
//! selon la force de chaque combinaison sur le board (les tirages restent dans les ranges
//! de mise). C'est un modèle de population simple et explicable, pas une lecture des
//! cartes : aucune carte cachée n'est jamais utilisée.

use stratyx_poker::game::{ActionKind, AppliedAction};
use stratyx_poker::{Card, CardSet, HandState, HoleCards, Range, Street, evaluate};

use crate::preflop::{self, CLASSES, class_at};
use crate::pushfold::{Chart, Node};

/// Part des mains ouvertes en premier de parole, selon les joueurs qui restent derrière.
#[must_use]
pub fn open_fraction(behind: usize) -> f32 {
    (0.9 / (behind as f32 + 1.0) + 0.08).min(0.65)
}

/// Part des mains poussées à tapis en premier de parole, sans table d'équilibre.
#[must_use]
pub fn shove_fraction(effective_bb: f32, behind: usize) -> f32 {
    (open_fraction(behind) * (1.0 + 6.0 / effective_bb.max(1.0))).clamp(0.05, 0.9)
}

/// Part des mains qui suivent un tapis, sans table d'équilibre.
#[must_use]
pub fn call_shove_fraction(effective_bb: f32) -> f32 {
    (0.04 + 1.6 / effective_bb.max(1.0)).clamp(0.04, 0.6)
}

/// Une range par classes (préflop) : poids de chacune des 169 classes.
fn classes_to_range(w: &[f32; CLASSES]) -> Range {
    let mut r = Range::empty();
    for (i, v) in w.iter().enumerate() {
        if *v > 0.0 {
            r.set_class(class_at(i), v.min(1.0));
        }
    }
    r
}

fn band(from: f32, to: f32) -> [f32; CLASSES] {
    let mut w = [0.0f32; CLASSES];
    let mut lo = 0.0f32;
    for &c in preflop::order() {
        let hi = preflop::percentile(c);
        let overlap = (hi.min(to) - lo.max(from)).max(0.0);
        if overlap > 0.0 {
            w[c] = (overlap / (hi - lo).max(1e-9)).min(1.0);
        }
        lo = hi;
    }
    w
}

/// La range préflop d'un joueur après ses actions (`actions` : les actions volontaires du
/// préflop de toute la table, dans l'ordre).
fn preflop_weights(
    state: &HandState,
    player: usize,
    actions: &[AppliedAction],
    chart: Option<&Chart>,
) -> [f32; CLASSES] {
    let n = state.players().len();
    let bb = state.blinds().big_blind.get().max(1) as f32;
    let eff = state.players()[player].stack.get() as f32 / bb
        + state.players()[player].invested().get() as f32 / bb;
    let pre_rank = |i: usize| {
        let off = (i + n - state.button()) % n;
        if n == 2 { off } else { (off + n - 3) % n }
    };
    let mut w = [1.0f32; CLASSES];
    let mut raises = 0usize;
    let mut all_ins = 0usize;
    let mut callers_since_raise = 0usize;
    let mut first_shover: Option<usize> = None;
    for a in actions
        .iter()
        .filter(|a| a.street == Street::Preflop && a.kind.is_voluntary())
    {
        if a.player == player {
            let order = pre_rank(player);
            let behind = n - 1 - order;
            let narrow = match a.kind {
                ActionKind::Raise | ActionKind::Bet if raises == 0 && a.all_in => {
                    let f = chart
                        .filter(|c| eff <= c.max_depth() * 1.3)
                        .map(|c| c.range_fraction(Node::FirstIn, eff, order, n))
                        .unwrap_or_else(|| shove_fraction(eff, behind));
                    band(0.0, f.max(0.03))
                }
                ActionKind::Raise | ActionKind::Bet if raises == 0 => {
                    band(0.0, open_fraction(behind))
                }
                ActionKind::Raise | ActionKind::Bet if raises == 1 => {
                    band(0.0, if a.all_in { 0.14 } else { 0.09 })
                }
                ActionKind::Raise | ActionKind::Bet => band(0.0, 0.045),
                ActionKind::Call if raises == 0 => band(0.08, 0.6),
                ActionKind::Call if all_ins > 0 => {
                    let shover = first_shover.unwrap_or(0);
                    let node = if callers_since_raise > 0 {
                        Node::VsShoveAndCall { shover }
                    } else {
                        Node::VsShove { shover }
                    };
                    let f = chart
                        .filter(|c| eff <= c.max_depth() * 1.3)
                        .map(|c| c.range_fraction(node, eff, order, n))
                        .unwrap_or_else(|| call_shove_fraction(eff));
                    band(0.0, f.max(0.02))
                }
                ActionKind::Call if raises == 1 => {
                    let defend = if behind == 0 {
                        0.5
                    } else if behind == 1 {
                        0.35
                    } else {
                        0.22
                    };
                    band(0.05, defend)
                }
                ActionKind::Call => band(0.02, 0.1),
                ActionKind::Check => band(0.1, 1.0),
                _ => [1.0; CLASSES],
            };
            for (x, y) in w.iter_mut().zip(narrow) {
                *x *= y;
            }
        }
        if a.kind.is_aggressive() {
            raises += 1;
            callers_since_raise = 0;
            if a.all_in {
                all_ins += 1;
                first_shover.get_or_insert(pre_rank(a.player));
            }
        } else if a.kind == ActionKind::Call {
            callers_since_raise += 1;
        }
    }
    w
}

/// La force d'une combinaison sur un board (rang de la main, avec un bonus pour les
/// tirages), pour ordonner une range après le flop.
fn board_strength(h: HoleCards, board: CardSet) -> f32 {
    let all = board | h.set();
    let rank = evaluate(all);
    let mut s = rank.value() as f32;
    if board.len() < 5 {
        let flush_draw = stratyx_poker::Suit::ALL
            .iter()
            .any(|su| all.suit_mask(*su).count_ones() == 4);
        if flush_draw && rank.category() < stratyx_poker::HandCategory::Straight {
            // Un tirage couleur se joue comme une paire moyenne.
            // (catégorie « paire » dans les bits 20 et plus, paire de 8 environ).
            s = s.max(((1u32 << 20) + (6 << 16)) as f32);
        }
    }
    s
}

/// Resserre une range sur le board : garde les mains entre les rangs `from` et `to` (0 :
/// la plus forte), avec un poids `rest` pour les autres.
fn narrow_on_board(range: &Range, board: CardSet, from: f32, to: f32, rest: f32) -> Range {
    let mut combos: Vec<(HoleCards, f32, f32)> = range
        .iter()
        .filter(|(h, w)| *w > 0.0 && !h.set().intersects(board))
        .map(|(h, w)| (h, w, board_strength(h, board)))
        .collect();
    combos.sort_by(|a, b| b.2.total_cmp(&a.2));
    let total: f32 = combos.iter().map(|c| c.1).sum::<f32>().max(1e-9);
    let mut out = Range::empty();
    let mut acc = 0.0f32;
    for (h, w, _) in combos {
        let mid = (acc + w / 2.0) / total;
        acc += w;
        let keep = if mid >= from && mid <= to { 1.0 } else { rest };
        if keep > 0.0 {
            out.set(h, w * keep);
        }
    }
    out
}

/// La range probable de `villain` au moment présent de `state`.
#[must_use]
pub fn range_of(state: &HandState, villain: usize, chart: Option<&Chart>) -> Range {
    let actions = state.actions();
    let w = preflop_weights(state, villain, actions, chart);
    let mut range = classes_to_range(&w);
    if range.is_empty() {
        range = Range::full();
    }
    let board_cards: &[Card] = state.board();
    for street in [Street::Flop, Street::Turn, Street::River] {
        let visible = match street {
            Street::Flop => 3,
            Street::Turn => 4,
            _ => 5,
        };
        if board_cards.len() < visible {
            break;
        }
        let board: CardSet = board_cards[..visible].iter().collect();
        let mut raises = 0usize;
        for a in actions
            .iter()
            .filter(|a| a.street == street && a.kind.is_voluntary())
        {
            if a.player == villain {
                range = match a.kind {
                    ActionKind::Bet | ActionKind::Raise if a.all_in => {
                        narrow_on_board(&range, board, 0.0, 0.35, 0.12)
                    }
                    ActionKind::Bet => narrow_on_board(&range, board, 0.0, 0.55, 0.15),
                    ActionKind::Raise if raises <= 1 => {
                        narrow_on_board(&range, board, 0.0, 0.3, 0.1)
                    }
                    ActionKind::Raise => narrow_on_board(&range, board, 0.0, 0.15, 0.05),
                    ActionKind::Call if raises >= 2 => {
                        narrow_on_board(&range, board, 0.0, 0.4, 0.05)
                    }
                    ActionKind::Call => narrow_on_board(&range, board, 0.08, 0.65, 0.1),
                    ActionKind::Check => narrow_on_board(&range, board, 0.15, 1.0, 0.35),
                    _ => range,
                };
            }
            if a.kind.is_aggressive() {
                raises += 1;
            }
        }
    }
    range
}

/// Le resserrement d'une range quand le joueur continue face à une relance : garde les
/// `keep` meilleures mains (préflop par rang, ensuite par force sur le board).
#[must_use]
pub fn continuing(range: &Range, board: &[Card], keep: f32) -> Range {
    let keep = keep.clamp(0.02, 1.0);
    if board.len() >= 3 {
        return narrow_on_board(range, board.iter().collect(), 0.0, keep, 0.0);
    }
    let mut combos: Vec<(HoleCards, f32, f32)> = range
        .iter()
        .filter(|(_, w)| *w > 0.0)
        .map(|(h, w)| (h, w, preflop::score(preflop::hole_class(h))))
        .collect();
    combos.sort_by(|a, b| b.2.total_cmp(&a.2));
    let total: f32 = combos.iter().map(|c| c.1).sum::<f32>().max(1e-9);
    let mut out = Range::empty();
    let mut acc = 0.0f32;
    for (h, w, _) in combos {
        if acc / total < keep {
            out.set(h, w);
        }
        acc += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::game::{Action, Blinds, HandSetup};
    use stratyx_shared::Chips;

    #[test]
    fn ranges_narrow_with_aggression() {
        let setup = HandSetup::new(vec![Chips(5000); 6], 0, Blinds::new(50, 100));
        let mut st = HandState::new(&setup).unwrap();
        st.apply(3, Action::Raise(Chips(250))).unwrap(); // UTG ouvre
        let open = range_of(&st, 3, None).fraction();
        st.apply(4, Action::Raise(Chips(800))).unwrap(); // HJ 3-bet
        let three = range_of(&st, 4, None).fraction();
        assert!((0.15..0.3).contains(&open), "{open}");
        assert!(three < 0.1, "{three}");
        let fold = range_of(&st, 5, None).fraction();
        assert!(fold > 0.99, "pas encore parlé : range complète");
        let c = continuing(&range_of(&st, 3, None), &[], 0.3);
        assert!(c.fraction() < open * 0.4);
    }
}
