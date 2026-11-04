//! Positions à la table, du tête-à-tête à la table pleine (2 à 10 joueurs).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Position d'un joueur pour une main donnée, dans l'ordre de parole préflop.
///
/// Jusqu'à 6 joueurs, les positions les plus précoces disparaissent en premier (UTG, puis HJ,
/// puis CO). Au-delà, le premier à parler est toujours UTG et les places intermédiaires
/// s'ajoutent : LJ (7 joueurs), UTG+1 (8), UTG+2 (9), UTG+3 (10). En tête-à-tête, le bouton
/// est aussi petite blinde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Position {
    #[serde(rename = "UTG")]
    UnderTheGun,
    #[serde(rename = "UTG+1")]
    UnderTheGun1,
    #[serde(rename = "UTG+2")]
    UnderTheGun2,
    #[serde(rename = "UTG+3")]
    UnderTheGun3,
    #[serde(rename = "LJ")]
    Lojack,
    #[serde(rename = "HJ")]
    Hijack,
    #[serde(rename = "CO")]
    Cutoff,
    #[serde(rename = "BTN")]
    Button,
    #[serde(rename = "SB")]
    SmallBlind,
    #[serde(rename = "BB")]
    BigBlind,
}

/// Nombre maximal de joueurs à une table de Texas Hold'em.
pub const MAX_TABLE_SIZE: usize = 10;

impl Position {
    /// Positions dans l'ordre des sièges en partant du bouton, pour `n` joueurs.
    #[must_use]
    pub const fn layout(n: usize) -> Option<&'static [Position]> {
        use Position::*;
        match n {
            2 => Some(&[Button, BigBlind]),
            3 => Some(&[Button, SmallBlind, BigBlind]),
            4 => Some(&[Button, SmallBlind, BigBlind, Cutoff]),
            5 => Some(&[Button, SmallBlind, BigBlind, Hijack, Cutoff]),
            6 => Some(&[Button, SmallBlind, BigBlind, UnderTheGun, Hijack, Cutoff]),
            7 => Some(&[
                Button,
                SmallBlind,
                BigBlind,
                UnderTheGun,
                Lojack,
                Hijack,
                Cutoff,
            ]),
            8 => Some(&[
                Button,
                SmallBlind,
                BigBlind,
                UnderTheGun,
                UnderTheGun1,
                Lojack,
                Hijack,
                Cutoff,
            ]),
            9 => Some(&[
                Button,
                SmallBlind,
                BigBlind,
                UnderTheGun,
                UnderTheGun1,
                UnderTheGun2,
                Lojack,
                Hijack,
                Cutoff,
            ]),
            10 => Some(&[
                Button,
                SmallBlind,
                BigBlind,
                UnderTheGun,
                UnderTheGun1,
                UnderTheGun2,
                UnderTheGun3,
                Lojack,
                Hijack,
                Cutoff,
            ]),
            _ => None,
        }
    }

    /// Position du joueur situé `offset` sièges après le bouton, à une table de `n` joueurs.
    #[must_use]
    pub fn from_button_offset(offset: usize, n: usize) -> Option<Position> {
        Position::layout(n).map(|l| l[offset % n])
    }

    #[must_use]
    pub const fn abbreviation(self) -> &'static str {
        match self {
            Position::UnderTheGun => "UTG",
            Position::UnderTheGun1 => "UTG+1",
            Position::UnderTheGun2 => "UTG+2",
            Position::UnderTheGun3 => "UTG+3",
            Position::Lojack => "LJ",
            Position::Hijack => "HJ",
            Position::Cutoff => "CO",
            Position::Button => "BTN",
            Position::SmallBlind => "SB",
            Position::BigBlind => "BB",
        }
    }

    /// Vrai pour les blindes.
    #[must_use]
    pub const fn is_blind(self) -> bool {
        matches!(self, Position::SmallBlind | Position::BigBlind)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.abbreviation())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(Position::from_button_offset(0, 2), Some(Position::Button));
        assert_eq!(Position::from_button_offset(1, 2), Some(Position::BigBlind));
        assert_eq!(
            Position::from_button_offset(3, 6),
            Some(Position::UnderTheGun)
        );
        assert_eq!(Position::from_button_offset(2, 3), Some(Position::BigBlind));
        assert_eq!(Position::layout(1), None);
        assert_eq!(Position::layout(11), None);
        for n in 2..=MAX_TABLE_SIZE {
            let l = Position::layout(n).unwrap();
            assert_eq!(l.len(), n);
            assert!(l.contains(&Position::BigBlind));
            // Positions toutes distinctes.
            let mut v = l.to_vec();
            v.sort();
            v.dedup();
            assert_eq!(v.len(), n);
        }
        assert_eq!(
            Position::from_button_offset(3, 9),
            Some(Position::UnderTheGun)
        );
        assert_eq!(Position::from_button_offset(6, 9), Some(Position::Lojack));
        assert_eq!(
            Position::from_button_offset(6, 10),
            Some(Position::UnderTheGun3)
        );
    }
}
