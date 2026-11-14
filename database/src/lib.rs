//! Persistance locale de Stratyx et formats texte des historiques de mains.
//!
//! # Base locale
//!
//! [`Database`] ouvre un **fichier redb unique** (base embarquée transactionnelle en Rust pur,
//! décision T3), organisé en cinq espaces logiques ([`Space`]) qui reprennent les bases du
//! cahier des charges (section 8) :
//!
//! | Espace | Contenu |
//! |---|---|
//! | Expresso | mains Expresso 3-Max |
//! | Tournois | mains de tournoi et sit & go NL 6-max |
//! | Hold'em | tous les autres formats de Texas Hold'em (cash game, tables pleines, PL, FL…) |
//! | Stratégie | ranges, charts, résultats de solveur (clé → blob, [`StrategyBlob`]) |
//! | Utilisateurs | sessions ([`Session`](stratyx_poker::Session)), profils, progression |
//!
//! L'espace d'une main découle de son format ([`Space::for_format`], via
//! [`GameFormat::specialist`](stratyx_poker::GameFormat::specialist)).
//!
//! **Mains.** Chaque main est stockée en JSON versionné ([`StoredHand`] : le
//! [`HandRecord`](stratyx_poker::HandRecord) pivot, son statut et son empreinte). Elle est
//! indexée par joueur, format, session, date (`meta.played_at_ms`) et statut de validation
//! ([`HandStatus`] : rejouée sans erreur, résultat non résolu, incomplète, invalide avec la
//! raison, nombre d'éléments à revoir). La recherche ([`Database::search`], [`HandQuery`])
//! combine ces filtres avec le résultat du héros, la présence d'un abattage et une pagination.
//!
//! **Déduplication.** Une [`Fingerprint`] SHA-256 couvre le contenu de jeu canonique (format,
//! blindes, sièges, tapis, cartes, événements), sans l'identifiant, les métadonnées, la
//! provenance ni la confiance : la même main capturée en vidéo puis importée en texte n'est
//! enregistrée qu'une fois.
//!
//! **Archives (décision D5).** [`Database::ingest`] et [`Database::ingest_pokerstars`]
//! exigent une [`Provenance`] (source et licence), valident chaque main par rejeu, la
//! dédupliquent et produisent un [`IngestReport`] (acceptées, doublons, rejetées avec la
//! raison), conservé dans la base.
//!
//! **RGPD.** [`Database::erase_player`] anonymise ou supprime les mains d'un joueur,
//! [`Database::delete_session`] et [`Database::delete_user`] effacent une session (avec ses
//! mains) ou un profil, [`Database::export_json`] produit un export complet.
//!
//! **Migrations.** La version du schéma ([`SCHEMA_VERSION`]) est stockée dans la base ; les
//! migrations manquantes s'appliquent à l'ouverture, dans une seule transaction.
//!
//! # Formats texte
//!
//! Le module [`text`] exporte et importe les historiques au format texte de PokerStars.
//!
//! ```
//! use stratyx_database::{Database, HandQuery, Space};
//!
//! let db = Database::in_memory()?;
//! assert_eq!(db.schema_version()?, stratyx_database::SCHEMA_VERSION);
//! let page = db.search(&HandQuery::new().space(Space::Expresso).page(0, 20))?;
//! assert_eq!(page.total, 0);
//! # Ok::<(), stratyx_database::DatabaseError>(())
//! ```

mod error;
mod fingerprint;
mod hex;
mod ingest;
mod migration;
#[cfg(feature = "nuage")]
#[allow(dead_code)] // Branché sur `Database` à l'étape b du chantier Nuage.
mod nuage_store;
mod privacy;
mod query;
mod space;
mod status;
mod store;
mod strategy;
mod tables;
pub mod text;
mod users;

pub use error::{DatabaseError, Result};
pub use fingerprint::{Fingerprint, ParseFingerprintError};
pub use ingest::{
    AcceptedHand, ArchiveBatch, DuplicateHand, IngestPolicy, IngestReport, Provenance,
    RejectReason, RejectedHand,
};
pub use migration::{AppliedMigration, SCHEMA_VERSION};
pub use privacy::{
    DatabaseExport, EXPORT_FORMAT, Erasure, ErasureReport, StrategyExport, UserExport,
};
pub use query::{HandPage, HandQuery, HeroResult, SortOrder};
pub use space::Space;
pub use status::{HandStatus, HandSummary, Validation, ValidationKind};
pub use store::{Database, HAND_FORMAT_VERSION, PutOutcome, StoredHand};
pub use strategy::{StrategyBlob, StrategyEntry};
pub use users::{PlayerAlias, ProgressEntry, SessionErasure, UserProfile};

/// Instant présent, en millisecondes depuis l'époque Unix.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
