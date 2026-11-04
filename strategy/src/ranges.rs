//! Ranges de référence **indicatives** pour l'open préflop (premier relanceur) en no-limit.
//!
//! # Statut : heuristiques, pas des sorties de solveur
//!
//! Ces ranges résument les pratiques courantes à environ 100 BB de profondeur, en cash game
//! comme en début de tournoi. Elles ne sortent d'aucun solveur, ne dépendent ni des tailles
//! d'open, ni des antes, ni des adversaires, et **ne doivent pas servir de référence GTO**.
//! Elles servent de **priors** : point de départ pour estimer la range d'un adversaire avant
//! d'observer ses mains et ses statistiques (VPIP, PFR par position…).
//!
//! Propriétés garanties (testées) : chaque range est relisible par [`Range::parse`] et les
//! ranges s'emboîtent de la position la plus précoce au bouton (une main ouverte en UTG
//! l'est aussi en HJ, CO et BTN). La petite blinde, qui n'ouvre que face au BB, suit sa
//! propre logique.
//!
//! | Table | UTG | UTG+1 | UTG+2 | LJ | HJ | CO | BTN | SB |
//! |-------|-----|-------|-------|----|----|----|-----|----|
//! | 6-max | 16 % | — | — | — | 20 % | 28 % | 46 % | 35 % |
//! | 9-max | 12 % | 13 % | 14 % | 16 % | 20 % | 28 % | 46 % | 35 % |
//!
//! Positions selon [`Position::layout`] : à 6 joueurs, UTG, HJ, CO, BTN, SB, BB ; à 9, UTG,
//! UTG+1, UTG+2, LJ, HJ, CO, BTN, SB, BB. Le LJ de 9-max (cinq joueurs derrière lui) joue
//! comme l'UTG de 6-max.

use serde::{Deserialize, Serialize};
use stratyx_poker::Range;
use stratyx_poker::position::Position;

const UTG_6MAX: &str = "22+,A2s+,KTs+,QTs+,JTs,T9s,98s,87s,76s,65s,AJo+,KQo";
const HJ: &str = "22+,A2s+,K9s+,Q9s+,J9s+,T9s,98s,87s,76s,65s,54s,ATo+,KJo+,QJo";
const CO: &str = "22+,A2s+,K6s+,Q8s+,J8s+,T8s+,97s+,86s+,75s+,64s+,54s,A8o+,KTo+,QTo+,JTo";
const BTN: &str = "22+,A2s+,K2s+,Q4s+,J6s+,T6s+,96s+,85s+,74s+,63s+,53s+,43s,\
                   A2o+,K8o+,Q9o+,J9o+,T8o+,98o,87o";
const SB: &str = "22+,A2s+,K2s+,Q5s+,J7s+,T7s+,96s+,86s+,75s+,64s+,54s,A4o+,K9o+,QTo+,JTo";

const OPEN_6MAX: &[(Position, &str)] = &[
    (Position::UnderTheGun, UTG_6MAX),
    (Position::Hijack, HJ),
    (Position::Cutoff, CO),
    (Position::Button, BTN),
    (Position::SmallBlind, SB),
];

const OPEN_9MAX: &[(Position, &str)] = &[
    (Position::UnderTheGun, "55+,A9s+,KTs+,QTs+,JTs,T9s,AJo+,KQo"),
    (
        Position::UnderTheGun1,
        "44+,A8s+,KTs+,QTs+,JTs,T9s,98s,AJo+,KQo",
    ),
    (
        Position::UnderTheGun2,
        "33+,A7s+,A5s,KTs+,QTs+,JTs,T9s,98s,87s,AJo+,KQo",
    ),
    (Position::Lojack, UTG_6MAX),
    (Position::Hijack, HJ),
    (Position::Cutoff, CO),
    (Position::Button, BTN),
    (Position::SmallBlind, SB),
];

/// Range d'open de référence pour une position, en notation texte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRange {
    pub position: Position,
    /// Notation standard, relisible par [`Range::parse`].
    pub notation: String,
}

impl OpenRange {
    /// Range pondérée correspondante.
    #[must_use]
    pub fn range(&self) -> Range {
        Range::parse(&self.notation).expect("ranges de référence valides")
    }
}

/// Tableau des ranges d'open pour une taille de table, de la première position à la SB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRangeTable {
    pub table_size: usize,
    /// Nature des données : toujours `heuristic` (voir la documentation du module).
    pub source: String,
    pub ranges: Vec<OpenRange>,
}

fn entries(table_size: usize) -> Option<&'static [(Position, &'static str)]> {
    match table_size {
        6 => Some(OPEN_6MAX),
        9 => Some(OPEN_9MAX),
        _ => None,
    }
}

/// Tailles de table couvertes : 6-max et 9-max.
pub const SUPPORTED_TABLE_SIZES: [usize; 2] = [6, 9];

/// Notation de la range d'open de `position` à une table de `table_size` joueurs.
///
/// `None` pour une taille de table non couverte ou pour le BB (qui n'ouvre jamais).
#[must_use]
pub fn open_range_notation(table_size: usize, position: Position) -> Option<&'static str> {
    entries(table_size)?
        .iter()
        .find(|(p, _)| *p == position)
        .map(|(_, n)| *n)
}

/// Range d'open de `position` (voir [`open_range_notation`]).
#[must_use]
pub fn open_range(table_size: usize, position: Position) -> Option<Range> {
    open_range_notation(table_size, position)
        .map(|n| Range::parse(n).expect("ranges de référence valides"))
}

/// Tableau complet des ranges d'open pour une taille de table couverte.
#[must_use]
pub fn open_range_table(table_size: usize) -> Option<OpenRangeTable> {
    Some(OpenRangeTable {
        table_size,
        source: "heuristic".into(),
        ranges: entries(table_size)?
            .iter()
            .map(|(position, notation)| OpenRange {
                position: *position,
                notation: (*notation).to_string(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vrai si chaque combinaison de `inner` est au moins aussi fréquente dans `outer`.
    fn contains(outer: &Range, inner: &Range) -> bool {
        inner.iter().all(|(h, w)| outer.get(h) >= w)
    }

    #[test]
    fn ranges_parse_and_have_plausible_sizes() {
        let expected_6 = [
            (Position::UnderTheGun, 0.15, 0.18),
            (Position::Hijack, 0.19, 0.22),
            (Position::Cutoff, 0.26, 0.30),
            (Position::Button, 0.42, 0.50),
            (Position::SmallBlind, 0.32, 0.40),
        ];
        for (pos, lo, hi) in expected_6 {
            let f = open_range(6, pos).unwrap().fraction();
            assert!((lo..=hi).contains(&f), "6-max {pos} : {f}");
        }
        let expected_9 = [
            (Position::UnderTheGun, 0.10, 0.13),
            (Position::UnderTheGun1, 0.12, 0.14),
            (Position::UnderTheGun2, 0.13, 0.16),
            (Position::Lojack, 0.15, 0.18),
        ];
        for (pos, lo, hi) in expected_9 {
            let f = open_range(9, pos).unwrap().fraction();
            assert!((lo..=hi).contains(&f), "9-max {pos} : {f}");
        }
        assert!(open_range(6, Position::BigBlind).is_none());
        assert!(open_range(6, Position::Lojack).is_none());
        assert!(open_range(7, Position::Button).is_none());
    }

    #[test]
    fn ranges_are_nested_from_early_to_late_position() {
        for size in SUPPORTED_TABLE_SIZES {
            let table = open_range_table(size).unwrap();
            let ordered: Vec<Range> = table
                .ranges
                .iter()
                .filter(|r| r.position != Position::SmallBlind)
                .map(OpenRange::range)
                .collect();
            assert_eq!(ordered.len(), table.ranges.len() - 1);
            for pair in ordered.windows(2) {
                assert!(contains(&pair[1], &pair[0]), "{size}-max : {pair:?}");
                assert!(pair[1].fraction() > pair[0].fraction());
            }
            // L'ordre suit l'ordre de parole de la table.
            let layout = Position::layout(size).unwrap();
            let order: Vec<usize> = table
                .ranges
                .iter()
                .map(|r| {
                    (0..size)
                        .find(|&i| layout[(i + 3) % size] == r.position)
                        .unwrap()
                })
                .collect();
            assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        }
    }

    #[test]
    fn table_serde_roundtrip() {
        let table = open_range_table(9).unwrap();
        let json = serde_json::to_string(&table).unwrap();
        assert!(json.contains(r#""position":"UTG+1""#));
        assert!(json.contains(r#""source":"heuristic""#));
        let back: OpenRangeTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back, table);
        assert!(open_range_table(10).is_none());
    }
}
