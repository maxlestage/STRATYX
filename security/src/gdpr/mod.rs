//! Conformité RGPD : inventaire des données, conservation, export, effacement, audit.
//!
//! - [`inventory`] : catégories de données, finalités, bases légales, conservation par
//!   défaut. Les pseudonymes des adversaires sont des données personnelles, et leurs profils
//!   un traitement fondé sur l'intérêt légitime.
//! - [`RetentionPolicy`] : durées configurables et calcul des enregistrements à purger.
//! - [`PersonalDataStore`] : contrat que le crate `database` implémentera ;
//!   [`InMemoryStore`] sert aux tests.
//! - [`handle_export`], [`handle_erasure`] : demandes des personnes, consignées dans un
//!   [`AuditLog`] horodaté et chaîné.

mod audit;
mod inventory;
mod requests;
mod retention;
mod store;

pub use audit::{AuditAction, AuditEntry, AuditLog, AuditOutcome};
pub use inventory::{CategoryInfo, DataCategory, INVENTORY, LegalBasis, Subjects, inventory};
pub use requests::{RESPONSE_DEADLINE_MS, SubjectExport, handle_erasure, handle_export};
pub use retention::{DAY_MS, PurgePlan, Retention, RetentionPolicy};
pub use store::{
    DataSubject, ErasureReport, ExportedRecord, InMemoryStore, PersonalDataStore, RecordRef,
    SubjectRef,
};

/// Erreur RGPD.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GdprError {
    /// Le stockage des données personnelles a échoué.
    #[error("échec du stockage des données personnelles : {0}")]
    Store(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Le journal d'audit a été modifié.
    #[error("journal d'audit altéré à l'entrée {sequence} : {reason}")]
    AuditTampered {
        /// Entrée fautive.
        sequence: u64,
        /// Nature de l'incohérence.
        reason: &'static str,
    },
    /// Le journal d'audit est illisible.
    #[error("journal d'audit illisible : {0}")]
    AuditFormat(String),
}

impl GdprError {
    pub(crate) fn store<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Store(Box::new(error))
    }
}
