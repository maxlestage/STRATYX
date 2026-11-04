//! Espaces logiques de la base : les cinq bases du cahier des charges (section 8).

use std::fmt;

use serde::{Deserialize, Serialize};
use stratyx_poker::{GameFormat, Specialist};

/// Base logique. Toutes partagent un même fichier redb ; chacune a ses propres tables,
/// préfixées par [`Space::key`].
///
/// | Espace | Base du cahier des charges | Contenu |
/// |---|---|---|
/// | [`Space::Expresso`] | Expresso | mains Expresso 3-Max (IA Expresso) |
/// | [`Space::Tournaments`] | Tournois | mains de tournoi et sit & go NL 6-max (IA Tournois) |
/// | [`Space::Holdem`] | Hold'em | tous les autres formats de Texas Hold'em |
/// | [`Space::Strategy`] | Stratégie | ranges, résultats de solveur, références (clé → blob) |
/// | [`Space::Users`] | Utilisateurs | sessions, profils, progression |
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Space {
    Expresso,
    Tournaments,
    Holdem,
    Strategy,
    Users,
}

impl Space {
    /// Les cinq espaces.
    pub const ALL: [Space; 5] = [
        Space::Expresso,
        Space::Tournaments,
        Space::Holdem,
        Space::Strategy,
        Space::Users,
    ];

    /// Les espaces qui contiennent des mains.
    pub const HAND_SPACES: [Space; 3] = [Space::Expresso, Space::Tournaments, Space::Holdem];

    /// Espace d'une main, déterminé par l'IA compétente pour son format
    /// ([`GameFormat::specialist`]) : sans IA dédiée, la main rejoint la base Hold'em.
    #[must_use]
    pub fn for_format(format: GameFormat) -> Space {
        match format.specialist() {
            Some(Specialist::Expresso3Max) => Space::Expresso,
            Some(Specialist::Tournament6Max) => Space::Tournaments,
            None => Space::Holdem,
        }
    }

    /// Vrai pour les espaces qui contiennent des mains.
    #[must_use]
    pub const fn holds_hands(self) -> bool {
        matches!(self, Space::Expresso | Space::Tournaments | Space::Holdem)
    }

    /// Préfixe stable des tables de l'espace dans le fichier.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Space::Expresso => "expresso",
            Space::Tournaments => "tournois",
            Space::Holdem => "holdem",
            Space::Strategy => "strategie",
            Space::Users => "utilisateurs",
        }
    }

    /// Nom affiché.
    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            Space::Expresso => "Expresso",
            Space::Tournaments => "Tournois",
            Space::Holdem => "Hold'em",
            Space::Strategy => "Stratégie",
            Space::Users => "Utilisateurs",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Space::Expresso => 1,
            Space::Tournaments => 2,
            Space::Holdem => 3,
            Space::Strategy => 4,
            Space::Users => 5,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Space> {
        Space::ALL.into_iter().find(|s| s.code() == code)
    }
}

impl fmt::Display for Space {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name_fr())
    }
}

#[cfg(test)]
mod tests {
    use stratyx_poker::{BettingStructure, GameType};

    use super::*;

    #[test]
    fn hands_follow_the_specialist() {
        assert_eq!(Space::for_format(GameFormat::EXPRESSO), Space::Expresso);
        assert_eq!(
            Space::for_format(GameFormat::TOURNAMENT_6MAX),
            Space::Tournaments
        );
        let sng = GameFormat::new(GameType::SitAndGo, 6, BettingStructure::NoLimit).unwrap();
        assert_eq!(Space::for_format(sng), Space::Tournaments);
        let cash = GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap();
        assert_eq!(Space::for_format(cash), Space::Holdem);
        let pl = GameFormat::new(GameType::Tournament, 6, BettingStructure::PotLimit).unwrap();
        assert_eq!(Space::for_format(pl), Space::Holdem);
    }

    #[test]
    fn codes_roundtrip() {
        for s in Space::ALL {
            assert_eq!(Space::from_code(s.code()), Some(s));
        }
        assert_eq!(Space::from_code(0), None);
        assert_eq!(
            Space::HAND_SPACES
                .iter()
                .filter(|s| s.holds_hands())
                .count(),
            3
        );
    }
}
