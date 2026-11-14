//! Erreurs de la couche de persistance.

use thiserror::Error;

/// Erreur renvoyée par [`Database`](crate::Database).
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// Erreur du moteur de stockage Nuage DB (dossier, transaction, requête), ou de la
    /// lecture d'une ancienne base redb à importer.
    #[error("stockage : {0}")]
    Storage(String),
    /// Donnée impossible à sérialiser ou à relire.
    #[error("sérialisation JSON : {0}")]
    Json(#[from] serde_json::Error),
    /// Erreur d'entrée-sortie lors d'un export.
    #[error("entrée-sortie : {0}")]
    Io(#[from] std::io::Error),
    /// La base a été créée par une version plus récente de Stratyx.
    #[error(
        "schéma de base en version {found}, cette version de Stratyx ne gère que la version \
         {supported} : mettez Stratyx à jour"
    )]
    SchemaTooNew { found: u32, supported: u32 },
    /// Une migration de schéma a échoué ; la base reste dans sa version précédente.
    #[error("migration vers la version {version} : {reason}")]
    Migration { version: u32, reason: String },
    /// Une main a été enregistrée dans un format plus récent que celui géré.
    #[error("main « {id} » enregistrée au format {found} (format géré : {supported})")]
    RecordTooNew {
        id: String,
        found: u32,
        supported: u32,
    },
    /// Donnée enregistrée incohérente (index, somme de contrôle…).
    #[error("donnée corrompue : {0}")]
    Corrupted(String),
    /// Identifiant vide.
    #[error("identifiant vide : {0}")]
    EmptyId(&'static str),
    /// Lot d'archives sans provenance ou sans licence (décision D5).
    #[error("lot d'archives refusé : {0} obligatoire (décision D5)")]
    MissingProvenance(&'static str),
    /// Utilisateur inconnu.
    #[error("utilisateur inconnu : « {0} »")]
    UnknownUser(String),
}

/// Raccourci pour les résultats de ce crate.
pub type Result<T, E = DatabaseError> = std::result::Result<T, E>;
