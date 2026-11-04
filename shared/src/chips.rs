use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// Montant en jetons, exprimé dans la plus petite unité indivisible.
///
/// Les calculs de pot et de tapis sont faits en entiers pour éviter toute erreur d'arrondi ;
/// les conversions en grosses blindes ou en pourcentages se font en `f64` à l'affichage.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Chips(pub u64);

impl Chips {
    pub const ZERO: Chips = Chips(0);

    #[must_use]
    pub const fn new(amount: u64) -> Self {
        Chips(amount)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn saturating_sub(self, rhs: Chips) -> Chips {
        Chips(self.0.saturating_sub(rhs.0))
    }

    #[must_use]
    pub const fn checked_sub(self, rhs: Chips) -> Option<Chips> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(Chips(v)),
            None => None,
        }
    }

    /// Montant exprimé en grosses blindes (pour l'affichage et les modèles).
    #[must_use]
    pub fn in_big_blinds(self, big_blind: Chips) -> f64 {
        if big_blind.is_zero() {
            return 0.0;
        }
        self.0 as f64 / big_blind.0 as f64
    }
}

impl Add for Chips {
    type Output = Chips;
    fn add(self, rhs: Chips) -> Chips {
        Chips(self.0 + rhs.0)
    }
}

impl AddAssign for Chips {
    fn add_assign(&mut self, rhs: Chips) {
        self.0 += rhs.0;
    }
}

impl Sub for Chips {
    type Output = Chips;
    fn sub(self, rhs: Chips) -> Chips {
        Chips(self.0 - rhs.0)
    }
}

impl SubAssign for Chips {
    fn sub_assign(&mut self, rhs: Chips) {
        self.0 -= rhs.0;
    }
}

impl Sum for Chips {
    fn sum<I: Iterator<Item = Chips>>(iter: I) -> Chips {
        Chips(iter.map(|c| c.0).sum())
    }
}

impl<'a> Sum<&'a Chips> for Chips {
    fn sum<I: Iterator<Item = &'a Chips>>(iter: I) -> Chips {
        Chips(iter.map(|c| c.0).sum())
    }
}

impl From<u64> for Chips {
    fn from(v: u64) -> Self {
        Chips(v)
    }
}

impl fmt::Display for Chips {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_and_bb_conversion() {
        let a = Chips(150) + Chips(50);
        assert_eq!(a, Chips(200));
        assert_eq!(a.in_big_blinds(Chips(100)), 2.0);
        assert_eq!(Chips(10).saturating_sub(Chips(20)), Chips::ZERO);
        assert_eq!(Chips(10).checked_sub(Chips(20)), None);
        let total: Chips = [Chips(1), Chips(2), Chips(3)].iter().sum();
        assert_eq!(total, Chips(6));
    }
}
