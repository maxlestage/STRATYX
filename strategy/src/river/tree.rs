//! Arbre de mises de la river en tête-à-tête.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{BetSizes, Player};

/// Action à la river. Les montants sont les **contributions totales** du joueur sur la river
/// après l'action (mise ou relance « à »), dans l'unité du pot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "to", rename_all = "snake_case")]
pub enum RiverAction {
    Fold,
    Check,
    Call,
    Bet(f64),
    Raise(f64),
    /// Tapis (mise ou relance), contribution totale égale au tapis effectif.
    AllIn(f64),
}

impl fmt::Display for RiverAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RiverAction::Fold => f.write_str("parole couchée"),
            RiverAction::Check => f.write_str("parole"),
            RiverAction::Call => f.write_str("suivre"),
            RiverAction::Bet(x) => write!(f, "miser {x:.2}"),
            RiverAction::Raise(x) => write!(f, "relancer à {x:.2}"),
            RiverAction::AllIn(x) => write!(f, "tapis ({x:.2})"),
        }
    }
}

/// Nature d'un nœud de l'arbre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "player", rename_all = "snake_case")]
pub enum NodeKind {
    /// Le joueur indiqué doit agir.
    Decision(Player),
    /// Le joueur indiqué s'est couché.
    Fold(Player),
    /// Abattage.
    Showdown,
}

/// Nœud de l'arbre de la river.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiverNode {
    pub kind: NodeKind,
    /// Contributions de OOP et IP sur la river (hors pot initial).
    pub contributions: [f64; 2],
    /// Nœud parent et index de l'action qui y mène (`None` à la racine).
    pub parent: Option<(usize, usize)>,
    /// Actions possibles (vide pour une feuille), dans l'ordre de `children`.
    pub actions: Vec<RiverAction>,
    pub children: Vec<usize>,
}

/// Paramètres de construction de l'arbre.
pub(crate) struct TreeParams<'a> {
    pub pot: f64,
    pub stack: f64,
    pub sizes: &'a [BetSizes; 2],
    pub max_raises: u8,
    pub allow_all_in: bool,
}

#[derive(Clone, Copy)]
struct State {
    to_act: usize,
    contrib: [f64; 2],
    raises: u8,
    last_increment: f64,
    parent: Option<(usize, usize)>,
}

/// Construit l'arbre complet ; la racine (index 0) est la décision de OOP.
pub(crate) fn build_tree(params: &TreeParams<'_>) -> Vec<RiverNode> {
    let mut nodes = Vec::new();
    build(
        params,
        State {
            to_act: 0,
            contrib: [0.0, 0.0],
            raises: 0,
            last_increment: 0.0,
            parent: None,
        },
        &mut nodes,
    );
    nodes
}

fn leaf(nodes: &mut Vec<RiverNode>, kind: NodeKind, contrib: [f64; 2], parent: (usize, usize)) {
    nodes.push(RiverNode {
        kind,
        contributions: contrib,
        parent: Some(parent),
        actions: Vec::new(),
        children: Vec::new(),
    });
}

fn build(params: &TreeParams<'_>, state: State, nodes: &mut Vec<RiverNode>) -> usize {
    let me = state.to_act;
    let other = 1 - me;
    let id = nodes.len();
    nodes.push(RiverNode {
        kind: NodeKind::Decision(Player::from_index(me)),
        contributions: state.contrib,
        parent: state.parent,
        actions: Vec::new(),
        children: Vec::new(),
    });
    let eps = 1e-9 * params.pot.max(1.0);
    let pot_now = params.pot + state.contrib[0] + state.contrib[1];
    let to_call = state.contrib[other] - state.contrib[me];
    let my_remaining = params.stack - state.contrib[me];
    let other_remaining = params.stack - state.contrib[other];

    // Actions agressives candidates : contribution supplémentaire de `me`.
    let mut aggressive: Vec<f64> = Vec::new();
    let push_amount = |add: f64, list: &mut Vec<f64>| {
        let add = add.min(my_remaining);
        if add > to_call + eps && !list.iter().any(|a| (a - add).abs() <= eps) {
            list.push(add);
        }
    };
    if to_call <= eps {
        for f in &params.sizes[me].bets {
            push_amount(f * pot_now, &mut aggressive);
        }
        if params.allow_all_in {
            push_amount(my_remaining, &mut aggressive);
        }
    } else if state.raises < params.max_raises
        && other_remaining > eps
        && my_remaining > to_call + eps
    {
        let min_add = to_call + state.last_increment;
        for f in &params.sizes[me].raises {
            let add = (to_call + f * (pot_now + to_call)).max(min_add);
            push_amount(add, &mut aggressive);
        }
        if params.allow_all_in {
            push_amount(my_remaining, &mut aggressive);
        }
    }
    aggressive.sort_by(f64::total_cmp);

    let mut actions = Vec::new();
    let mut children = Vec::new();
    if to_call <= eps {
        // Parole : la river s'arrête si IP checke derrière OOP.
        actions.push(RiverAction::Check);
        let child = nodes.len();
        if me == 1 {
            leaf(nodes, NodeKind::Showdown, state.contrib, (id, 0));
        } else {
            build(
                params,
                State {
                    to_act: other,
                    parent: Some((id, 0)),
                    ..state
                },
                nodes,
            );
        }
        children.push(child);
    } else {
        actions.push(RiverAction::Fold);
        children.push(nodes.len());
        leaf(
            nodes,
            NodeKind::Fold(Player::from_index(me)),
            state.contrib,
            (id, 0),
        );
        actions.push(RiverAction::Call);
        let mut called = state.contrib;
        called[me] += to_call.min(my_remaining);
        children.push(nodes.len());
        leaf(nodes, NodeKind::Showdown, called, (id, 1));
    }
    for add in aggressive {
        let mut contrib = state.contrib;
        contrib[me] += add;
        let to = contrib[me];
        let all_in = (my_remaining - add).abs() <= eps;
        let action = if all_in {
            RiverAction::AllIn(to)
        } else if to_call <= eps {
            RiverAction::Bet(to)
        } else {
            RiverAction::Raise(to)
        };
        let index = actions.len();
        actions.push(action);
        let child = build(
            params,
            State {
                to_act: other,
                contrib,
                raises: if to_call <= eps { 0 } else { state.raises + 1 },
                last_increment: add - to_call,
                parent: Some((id, index)),
            },
            nodes,
        );
        children.push(child);
    }
    nodes[id].actions = actions;
    nodes[id].children = children;
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(bets: &[f64], raises: &[f64]) -> BetSizes {
        BetSizes {
            bets: bets.to_vec(),
            raises: raises.to_vec(),
        }
    }

    #[test]
    fn simple_tree_shapes() {
        let s = [sizes(&[1.0], &[]), sizes(&[], &[])];
        let tree = build_tree(&TreeParams {
            pot: 10.0,
            stack: 10.0,
            sizes: &s,
            max_raises: 0,
            allow_all_in: false,
        });
        // OOP : parole ou tapis (la mise pot vaut le tapis) ; IP : parole, ou couché/suivre.
        assert_eq!(
            tree[0].actions,
            vec![RiverAction::Check, RiverAction::AllIn(10.0)]
        );
        let after_check = &tree[tree[0].children[0]];
        assert_eq!(after_check.actions, vec![RiverAction::Check]);
        let facing = &tree[tree[0].children[1]];
        assert_eq!(facing.actions, vec![RiverAction::Fold, RiverAction::Call]);
        let called = &tree[facing.children[1]];
        assert_eq!(called.kind, NodeKind::Showdown);
        assert_eq!(called.contributions, [10.0, 10.0]);
        assert_eq!(tree.len(), 6);
    }

    #[test]
    fn raises_respect_limits_and_minimum() {
        let s = [sizes(&[0.5], &[0.1]), sizes(&[0.5], &[1.0])];
        let tree = build_tree(&TreeParams {
            pot: 10.0,
            stack: 100.0,
            sizes: &s,
            max_raises: 1,
            allow_all_in: true,
        });
        // OOP mise 5 ; IP peut relancer pot (5 + 1 × 20 = 25 à ajouter → à 25) ou tapis.
        let bet = tree[0].children[1];
        assert_eq!(tree[0].actions[1], RiverAction::Bet(5.0));
        assert_eq!(
            tree[bet].actions,
            vec![
                RiverAction::Fold,
                RiverAction::Call,
                RiverAction::Raise(25.0),
                RiverAction::AllIn(100.0)
            ]
        );
        // Plus de relance possible après une relance (max_raises = 1).
        let raised = &tree[tree[bet].children[2]];
        assert_eq!(raised.actions, vec![RiverAction::Fold, RiverAction::Call]);
        // Relance minimale : après une mise de 5, OOP relancerait au moins à 10 même avec
        // une taille de 10 % du pot.
        let after_check = tree[0].children[0];
        let ip_bet = tree[after_check].children[1];
        let oop_raise = tree[ip_bet].actions[2];
        assert_eq!(oop_raise, RiverAction::Raise(10.0));
        // Chaque nœud a autant d'enfants que d'actions et ses enfants le désignent comme parent.
        for (i, node) in tree.iter().enumerate() {
            assert_eq!(node.actions.len(), node.children.len());
            for (a, &c) in node.children.iter().enumerate() {
                assert_eq!(tree[c].parent, Some((i, a)));
            }
        }
    }
}
