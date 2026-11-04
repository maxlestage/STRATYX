//! Positions à la table (formats 2 à 6 joueurs).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Position d'un joueur pour une main donnée.
///
/// Quand la table compte moins de six joueurs, les positions les plus précoces
/// disparaissent en premier (UTG, puis HJ, puis CO). En tête-à-tête, le bouton est aussi
/// petite blinde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Position {
    #[serde(rename = "UTG")]
    UnderTheGun,
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

/// Nombre maximal de joueurs pris en charge (formats 3-max et 6-max).
pub const MAX_TABLE_SIZE: usize = 6;

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
            Position::Hijack => "HJ",
            Position::Cutoff => "CO",
            Position::Button => "BTN",
            Position::SmallBlind => "SB",
            Position::BigBlind => "BB",
        }
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
        assert_eq!(Position::layout(7), None);
        for n in 2..=MAX_TABLE_SIZE {
            let l = Position::layout(n).unwrap();
            assert_eq!(l.len(), n);
            assert!(l.contains(&Position::BigBlind));
        }
    }
}
