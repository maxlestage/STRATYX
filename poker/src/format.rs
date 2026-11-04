//! Formats de Texas Hold'em : type de partie, taille de table, structure de mise.
//!
//! Le format détermine les règles appliquées par le moteur, la pertinence de l'ICM,
//! la base d'archives et l'IA spécialisée compétente, s'il y en a une.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::game::BettingStructure;
use crate::position::MAX_TABLE_SIZE;

/// Type de partie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameType {
    /// Partie à l'argent réel : jetons = monnaie, pas d'ICM.
    CashGame,
    /// Tournoi multi-tables.
    Tournament,
    /// Sit & go à une table.
    SitAndGo,
    /// Expresso / Spin : sit & go rapide à 3 joueurs avec multiplicateur de prix.
    Expresso,
}

impl GameType {
    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            GameType::CashGame => "Cash game",
            GameType::Tournament => "Tournoi",
            GameType::SitAndGo => "Sit & go",
            GameType::Expresso => "Expresso",
        }
    }
}

/// IA spécialisée de la plateforme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Specialist {
    /// IA Expresso 3-Max.
    Expresso3Max,
    /// IA Tournois 6-Max.
    Tournament6Max,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormatError {
    #[error("une table de Texas Hold'em compte de 2 à {MAX_TABLE_SIZE} joueurs (reçu : {0})")]
    TableSize(u8),
    #[error("une Expresso se joue à 2 ou 3 joueurs (reçu : {0})")]
    ExpressoSize(u8),
}

/// Format complet d'une partie de Texas Hold'em.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GameFormat {
    pub game_type: GameType,
    /// Nombre de places à la table (2 à 10).
    pub table_size: u8,
    #[serde(default)]
    pub structure: BettingStructure,
}

impl GameFormat {
    /// Expresso no-limit à 3 joueurs.
    pub const EXPRESSO: GameFormat = GameFormat {
        game_type: GameType::Expresso,
        table_size: 3,
        structure: BettingStructure::NoLimit,
    };

    /// Tournoi no-limit à 6 joueurs par table.
    pub const TOURNAMENT_6MAX: GameFormat = GameFormat {
        game_type: GameType::Tournament,
        table_size: 6,
        structure: BettingStructure::NoLimit,
    };

    pub fn new(
        game_type: GameType,
        table_size: u8,
        structure: BettingStructure,
    ) -> Result<GameFormat, FormatError> {
        let f = GameFormat {
            game_type,
            table_size,
            structure,
        };
        f.validate()?;
        Ok(f)
    }

    /// Vérifie la cohérence du format (utile après désérialisation).
    pub fn validate(&self) -> Result<(), FormatError> {
        if !(2..=MAX_TABLE_SIZE).contains(&usize::from(self.table_size)) {
            return Err(FormatError::TableSize(self.table_size));
        }
        if self.game_type == GameType::Expresso && self.table_size > 3 {
            return Err(FormatError::ExpressoSize(self.table_size));
        }
        Ok(())
    }

    #[must_use]
    pub const fn max_players(self) -> usize {
        self.table_size as usize
    }

    /// Vrai pour les formats à prix fixes, où la valeur des jetons suit l'ICM.
    #[must_use]
    pub const fn uses_icm(self) -> bool {
        !matches!(self.game_type, GameType::CashGame)
    }

    /// IA spécialisée compétente. Les autres formats de Texas Hold'em sont analysés par le
    /// moteur commun (équité, EV, cotes, ICM, statistiques) sans IA dédiée.
    #[must_use]
    pub fn specialist(self) -> Option<Specialist> {
        if self.structure != BettingStructure::NoLimit {
            return None;
        }
        match (self.game_type, self.table_size) {
            (GameType::Expresso, _) => Some(Specialist::Expresso3Max),
            (GameType::Tournament | GameType::SitAndGo, 6) => Some(Specialist::Tournament6Max),
            _ => None,
        }
    }
}

impl fmt::Display for GameFormat {
    /// Ex. : « Cash game NL 6-max », « Tournoi FL 9-max ».
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}-max",
            self.game_type.name_fr(),
            self.structure.abbreviation(),
            self.table_size
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialists() {
        assert_eq!(
            GameFormat::EXPRESSO.specialist(),
            Some(Specialist::Expresso3Max)
        );
        assert_eq!(
            GameFormat::TOURNAMENT_6MAX.specialist(),
            Some(Specialist::Tournament6Max)
        );
        let cash = GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap();
        assert_eq!(cash.specialist(), None);
        assert!(!cash.uses_icm());
        let mtt9 = GameFormat::new(GameType::Tournament, 9, BettingStructure::NoLimit).unwrap();
        assert_eq!(mtt9.specialist(), None);
        assert!(mtt9.uses_icm());
        let pl = GameFormat::new(GameType::Tournament, 6, BettingStructure::PotLimit).unwrap();
        assert_eq!(pl.specialist(), None);
        assert_eq!(pl.to_string(), "Tournoi PL 6-max");
    }

    #[test]
    fn validation() {
        assert!(GameFormat::new(GameType::CashGame, 1, BettingStructure::NoLimit).is_err());
        assert!(GameFormat::new(GameType::CashGame, 11, BettingStructure::NoLimit).is_err());
        assert!(GameFormat::new(GameType::Expresso, 6, BettingStructure::NoLimit).is_err());
        assert!(
            GameFormat::new(GameType::CashGame, 10, BettingStructure::FIXED_LIMIT_CASH).is_ok()
        );
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_string(&GameFormat::EXPRESSO).unwrap();
        assert_eq!(
            json,
            r#"{"game_type":"expresso","table_size":3,"structure":{"kind":"no_limit"}}"#
        );
        let fl: GameFormat = serde_json::from_str(
            r#"{"game_type":"cash_game","table_size":9,
                "structure":{"kind":"fixed_limit","max_bets":4,"uncapped_heads_up":true}}"#,
        )
        .unwrap();
        assert_eq!(fl.structure, BettingStructure::FIXED_LIMIT_CASH);
    }
}
