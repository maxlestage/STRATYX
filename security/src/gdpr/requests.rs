//! Demandes des personnes concernées : export (art. 15 et 20) et effacement (art. 17).
//!
//! Chaque demande, réussie ou non, est consignée au journal d'audit.

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::GdprError;
use super::audit::{AuditAction, AuditLog, AuditOutcome};
use super::store::{DataSubject, ErasureReport, ExportedRecord, PersonalDataStore, hex};

/// Délai légal de réponse à une demande : un mois (art. 12.3), compté ici en 30 jours.
pub const RESPONSE_DEADLINE_MS: u64 = 30 * super::retention::DAY_MS;

/// Données exportées pour une personne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectExport {
    /// Personne concernée.
    pub subject: DataSubject,
    /// Date de l'export.
    pub generated_at_ms: u64,
    /// Enregistrements.
    pub records: Vec<ExportedRecord>,
}

/// Ligne du manifeste d'export.
#[derive(Serialize)]
struct ManifestEntry<'a> {
    #[serde(flatten)]
    record: &'a ExportedRecord,
    file: String,
    size: usize,
    sha256: String,
}

/// Manifeste d'export.
#[derive(Serialize)]
struct Manifest<'a> {
    format: &'static str,
    subject: &'a DataSubject,
    generated_at_ms: u64,
    records: Vec<ManifestEntry<'a>>,
}

impl SubjectExport {
    /// Nom du manifeste dans l'export.
    pub const MANIFEST: &'static str = "manifeste.json";

    /// Fichiers de l'export, prêts à être chiffrés dans une sauvegarde
    /// ([`BackupWriter`](crate::backup::BackupWriter)) : un manifeste JSON décrivant chaque
    /// enregistrement (catégorie, identifiant, date, type, empreinte SHA-256) et un fichier
    /// par enregistrement, nommé par son rang (les identifiants ne servent jamais de chemin).
    #[must_use]
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        let names: Vec<String> = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "donnees/{:05}-{}.{}",
                    i + 1,
                    category_slug(r),
                    extension(&r.media_type)
                )
            })
            .collect();
        let manifest = Manifest {
            format: "stratyx-export-rgpd/1",
            subject: &self.subject,
            generated_at_ms: self.generated_at_ms,
            records: self
                .records
                .iter()
                .zip(&names)
                .map(|(record, file)| ManifestEntry {
                    record,
                    file: file.clone(),
                    size: record.data.len(),
                    sha256: hex(&Sha256::digest(&record.data)),
                })
                .collect(),
        };
        // Structure sans clé non textuelle : la sérialisation ne peut pas échouer.
        let manifest = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
        std::iter::once((Self::MANIFEST.to_owned(), manifest))
            .chain(
                names
                    .into_iter()
                    .zip(&self.records)
                    .map(|(name, r)| (name, r.data.clone())),
            )
            .collect()
    }
}

fn category_slug(record: &ExportedRecord) -> &'static str {
    use super::inventory::DataCategory as C;
    match record.record.category {
        C::HandHistories => "main",
        C::Videos => "video",
        C::OpponentProfiles => "profil",
        C::Statistics => "statistiques",
        C::Progress => "progression",
    }
}

fn extension(media_type: &str) -> &'static str {
    match media_type.split(';').next().unwrap_or("").trim() {
        "application/json" => "json",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "image/png" => "png",
        _ => "bin",
    }
}

/// Exporte les données d'une personne et consigne la demande.
///
/// # Errors
/// [`GdprError::Store`] (consignée comme échec).
pub fn handle_export<S: PersonalDataStore + ?Sized>(
    store: &S,
    subject: &DataSubject,
    audit: &mut AuditLog,
    now_ms: u64,
) -> Result<SubjectExport, GdprError> {
    match store.export_subject(subject) {
        Ok(records) => {
            audit.append(
                now_ms,
                AuditAction::Export,
                Some(subject.reference()),
                AuditOutcome::Completed,
                records.len() as u64,
                format!("export de {} enregistrement(s)", records.len()),
            );
            Ok(SubjectExport {
                subject: subject.clone(),
                generated_at_ms: now_ms,
                records,
            })
        }
        Err(e) => Err(record_failure(
            audit,
            AuditAction::Export,
            subject,
            now_ms,
            e,
        )),
    }
}

/// Efface les données d'une personne et consigne la demande.
///
/// # Errors
/// [`GdprError::Store`] (consignée comme échec).
pub fn handle_erasure<S: PersonalDataStore + ?Sized>(
    store: &mut S,
    subject: &DataSubject,
    audit: &mut AuditLog,
    now_ms: u64,
) -> Result<ErasureReport, GdprError> {
    match store.erase_subject(subject) {
        Ok(report) => {
            audit.append(
                now_ms,
                AuditAction::Erasure,
                Some(subject.reference()),
                AuditOutcome::Completed,
                report.total() as u64,
                format!(
                    "{} enregistrement(s) supprimé(s), {} anonymisé(s)",
                    report.deleted.len(),
                    report.anonymized.len()
                ),
            );
            Ok(report)
        }
        Err(e) => Err(record_failure(
            audit,
            AuditAction::Erasure,
            subject,
            now_ms,
            e,
        )),
    }
}

fn record_failure<E: std::error::Error + Send + Sync + 'static>(
    audit: &mut AuditLog,
    action: AuditAction,
    subject: &DataSubject,
    now_ms: u64,
    error: E,
) -> GdprError {
    let error = GdprError::store(error);
    audit.append(
        now_ms,
        action,
        Some(subject.reference()),
        AuditOutcome::Failed,
        0,
        error.to_string(),
    );
    error
}

#[cfg(test)]
mod tests {
    use super::super::inventory::DataCategory;
    use super::super::store::{InMemoryStore, RecordRef};
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn populated() -> (InMemoryStore, DataSubject, DataSubject) {
        let user = DataSubject::user("moi");
        let villain = DataSubject::opponent("winamax", "Villain").unwrap();
        let mut store = InMemoryStore::new();
        store.insert(
            DataCategory::HandHistories,
            "main-1",
            NOW,
            [user.clone(), villain.clone()],
            "application/json",
            br#"{"joueurs":["moi","Villain"]}"#.to_vec(),
        );
        store.insert(
            DataCategory::OpponentProfiles,
            "Villain",
            NOW,
            [villain.clone()],
            "application/json",
            br#"{"vpip":0.31}"#.to_vec(),
        );
        store.insert(
            DataCategory::Statistics,
            "stats",
            NOW,
            [user.clone()],
            "application/json",
            b"{}".to_vec(),
        );
        store.insert(
            DataCategory::Videos,
            "v",
            NOW,
            [user.clone()],
            "video/x-stratyx",
            vec![0; 10],
        );
        (store, user, villain)
    }

    #[test]
    fn export_lists_everything_about_the_subject() {
        let (store, user, villain) = populated();
        let mut audit = AuditLog::new();

        let export = handle_export(&store, &villain, &mut audit, NOW).unwrap();
        let ids: Vec<_> = export
            .records
            .iter()
            .map(|r| r.record.id.as_str())
            .collect();
        assert_eq!(ids, ["main-1", "Villain"]);

        let files = export.files();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].0, SubjectExport::MANIFEST);
        assert_eq!(files[1].0, "donnees/00001-main.json");
        assert_eq!(files[2].0, "donnees/00002-profil.json");
        let manifest: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(manifest["records"][1]["id"], "Villain");
        assert_eq!(manifest["records"][1]["category"], "opponent_profiles");
        assert_eq!(manifest["subject"]["pseudonym"], "Villain");

        assert_eq!(
            handle_export(&store, &user, &mut audit, NOW)
                .unwrap()
                .records
                .len(),
            3
        );
        assert_eq!(audit.entries().len(), 2);
        assert_eq!(audit.entries()[0].records, 2);
        assert_eq!(audit.entries()[0].subject, Some(villain.reference()));
        audit.verify().unwrap();
    }

    #[test]
    fn erasure_of_an_opponent_keeps_the_users_hands() {
        let (mut store, user, villain) = populated();
        let mut audit = AuditLog::new();
        let report = handle_erasure(&mut store, &villain, &mut audit, NOW).unwrap();
        let ids = |refs: &[RecordRef]| refs.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&report.deleted), ["Villain"]);
        assert_eq!(ids(&report.anonymized), ["main-1"]);
        assert!(store.contains(DataCategory::HandHistories, "main-1"));
        assert!(
            !store
                .subjects_of(DataCategory::HandHistories, "main-1")
                .unwrap()
                .contains(&villain)
        );
        assert!(
            handle_export(&store, &villain, &mut audit, NOW)
                .unwrap()
                .records
                .is_empty()
        );

        // Effacement du compte utilisateur : tout le reste disparaît.
        let report = handle_erasure(&mut store, &user, &mut audit, NOW + 1).unwrap();
        assert_eq!(report.deleted.len(), 3);
        assert!(store.is_empty());
        assert_eq!(audit.entries().len(), 3);
        assert!(
            audit.entries()[2]
                .detail
                .contains("3 enregistrement(s) supprimé(s)")
        );
        audit.verify().unwrap();
    }

    /// Stockage en panne : l'échec est consigné.
    struct Broken;

    #[derive(Debug, thiserror::Error)]
    #[error("disque indisponible")]
    struct DiskError;

    impl PersonalDataStore for Broken {
        type Error = DiskError;
        fn export_subject(&self, _: &DataSubject) -> Result<Vec<ExportedRecord>, DiskError> {
            Err(DiskError)
        }
        fn erase_subject(&mut self, _: &DataSubject) -> Result<ErasureReport, DiskError> {
            Err(DiskError)
        }
        fn list_records(
            &self,
            _: DataCategory,
            _: Option<u64>,
        ) -> Result<Vec<RecordRef>, DiskError> {
            Err(DiskError)
        }
        fn delete_records(&mut self, _: &[RecordRef]) -> Result<usize, DiskError> {
            Err(DiskError)
        }
    }

    #[test]
    fn failures_are_audited() {
        let mut audit = AuditLog::new();
        let subject = DataSubject::user("moi");
        let err = handle_erasure(&mut Broken, &subject, &mut audit, NOW).unwrap_err();
        assert!(err.to_string().contains("disque indisponible"));
        assert!(handle_export(&Broken, &subject, &mut audit, NOW).is_err());
        assert!(
            super::super::RetentionPolicy::default()
                .plan_purge(&Broken, NOW)
                .is_err()
        );
        let failed: Vec<_> = audit
            .entries()
            .iter()
            .map(|e| (e.action, e.outcome))
            .collect();
        assert_eq!(
            failed,
            [
                (AuditAction::Erasure, AuditOutcome::Failed),
                (AuditAction::Export, AuditOutcome::Failed)
            ]
        );
        assert_eq!(RESPONSE_DEADLINE_MS, 2_592_000_000);
    }
}
