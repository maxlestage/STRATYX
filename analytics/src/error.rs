//! Erreurs typées du crate d'analyse.

use stratyx_poker::history::RecordError;
use thiserror::Error;

/// Erreur d'analyse. Les messages sont rédigés en français pour l'affichage.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnalyticsError {
    /// L'historique ne se rejoue pas dans le moteur de règles.
    #[error("main « {hand_id} » : historique invalide ({source})")]
    InvalidRecord {
        hand_id: String,
        #[source]
        source: RecordError,
    },
    /// La main a déjà été ajoutée (calcul incrémental : chaque main ne compte qu'une fois).
    #[error("main « {0} » déjà prise en compte")]
    DuplicateHand(String),
    /// Aucun joueur analysé n'est désigné (`meta.hero_seat`).
    #[error("main « {0} » : aucun héros désigné (meta.hero_seat absent)")]
    NoHero(String),
    /// Le siège désigné n'est pas occupé par un joueur servi.
    #[error("main « {hand_id} » : le siège {seat} n'est pas occupé par un joueur servi")]
    SeatNotFound { hand_id: String, seat: u8 },
}
