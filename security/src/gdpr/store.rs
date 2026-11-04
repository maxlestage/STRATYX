//! Interface avec le stockage des données personnelles, et implémentation en mémoire.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::inventory::DataCategory;
use crate::capture::{CaptureError, RoomId};

/// Personne concernée par des données.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataSubject {
    /// L'utilisateur de Stratyx (titulaire du compte).
    User {
        /// Identifiant du compte.
        user_id: String,
    },
    /// Un adversaire, identifié par son pseudonyme dans une salle.
    Opponent {
        /// Salle.
        room: RoomId,
        /// Pseudonyme, tel qu'affiché par la salle (sensible à la casse).
        pseudonym: String,
    },
}

impl DataSubject {
    /// Utilisateur `user_id`.
    pub fn user(user_id: impl Into<String>) -> Self {
        Self::User {
            user_id: user_id.into(),
        }
    }

    /// Adversaire `pseudonym` de la salle `room`.
    ///
    /// # Errors
    /// [`CaptureError::InvalidRoom`] si l'identifiant de salle est invalide.
    pub fn opponent(room: &str, pseudonym: impl Into<String>) -> Result<Self, CaptureError> {
        Ok(Self::Opponent {
            room: RoomId::new(room)?,
            pseudonym: pseudonym.into(),
        })
    }

    /// Référence pseudonymisée (SHA-256 avec séparation de domaine), pour le journal
    /// d'audit : elle permet de prouver qu'une demande a été traitée sans y recopier le
    /// pseudonyme ou l'identifiant en clair.
    #[must_use]
    pub fn reference(&self) -> SubjectRef {
        let mut hasher = Sha256::new();
        hasher.update(b"STRATYX/rgpd/sujet/v1\0");
        match self {
            Self::User { user_id } => {
                hasher.update(b"user\0");
                hasher.update(user_id.as_bytes());
            }
            Self::Opponent { room, pseudonym } => {
                hasher.update(b"opponent\0");
                hasher.update(room.as_str().as_bytes());
                hasher.update(b"\0");
                hasher.update(pseudonym.as_bytes());
            }
        }
        SubjectRef(hex(&hasher.finalize()))
    }
}

/// Référence pseudonymisée d'une personne concernée (empreinte hexadécimale).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubjectRef(String);

impl SubjectRef {
    /// Empreinte hexadécimale.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Référence d'un enregistrement stocké.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordRef {
    /// Catégorie.
    pub category: DataCategory,
    /// Identifiant dans sa catégorie.
    pub id: String,
    /// Date de référence pour la conservation (création, ou dernière observation pour un
    /// profil d'adversaire), en ms depuis l'époque Unix.
    pub reference_ms: u64,
}

/// Enregistrement exporté (droit d'accès, art. 15 ; portabilité, art. 20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedRecord {
    /// Référence de l'enregistrement.
    #[serde(flatten)]
    pub record: RecordRef,
    /// Type MIME du contenu (`application/json` de préférence).
    pub media_type: String,
    /// Contenu.
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// Résultat d'un effacement (art. 17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureReport {
    /// Enregistrements supprimés.
    pub deleted: Vec<RecordRef>,
    /// Enregistrements conservés mais dont toute trace de la personne a été retirée (main
    /// de l'utilisateur où l'adversaire apparaissait, par exemple).
    pub anonymized: Vec<RecordRef>,
}

impl ErasureReport {
    /// Nombre d'enregistrements touchés.
    #[must_use]
    pub fn total(&self) -> usize {
        self.deleted.len() + self.anonymized.len()
    }
}

/// Stockage des données personnelles, implémenté par le crate `database`.
///
/// Contrat attendu :
///
/// - [`export_subject`](Self::export_subject) renvoie **toutes** les données relatives à la
///   personne, dans un format structuré et lisible par machine ;
/// - [`erase_subject`](Self::erase_subject) supprime ce qui ne concerne que la personne et
///   anonymise ce qui est partagé (une main de l'utilisateur garde ses actions, le pseudonyme
///   de l'adversaire est remplacé) ; l'opération est définitive ;
/// - [`list_records`](Self::list_records) liste une catégorie, éventuellement limitée aux
///   enregistrements dont la date de référence est strictement antérieure à `older_than_ms` ;
/// - [`delete_records`](Self::delete_records) supprime ; une référence inconnue est ignorée.
pub trait PersonalDataStore {
    /// Erreur du stockage.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Exporte les données d'une personne.
    ///
    /// # Errors
    /// Erreur du stockage.
    fn export_subject(&self, subject: &DataSubject) -> Result<Vec<ExportedRecord>, Self::Error>;

    /// Efface les données d'une personne.
    ///
    /// # Errors
    /// Erreur du stockage.
    fn erase_subject(&mut self, subject: &DataSubject) -> Result<ErasureReport, Self::Error>;

    /// Liste les enregistrements d'une catégorie, éventuellement filtrés par âge.
    ///
    /// # Errors
    /// Erreur du stockage.
    fn list_records(
        &self,
        category: DataCategory,
        older_than_ms: Option<u64>,
    ) -> Result<Vec<RecordRef>, Self::Error>;

    /// Supprime des enregistrements ; renvoie le nombre réellement supprimés.
    ///
    /// # Errors
    /// Erreur du stockage.
    fn delete_records(&mut self, records: &[RecordRef]) -> Result<usize, Self::Error>;
}

#[derive(Debug, Clone)]
struct MemoryRecord {
    reference_ms: u64,
    subjects: BTreeSet<DataSubject>,
    media_type: String,
    data: Vec<u8>,
}

/// Stockage en mémoire, pour les tests et les prototypes.
///
/// Effacement : un enregistrement qui ne concerne que la personne est supprimé ; s'il en
/// concerne d'autres, le lien avec la personne est retiré (anonymisation).
#[derive(Debug, Clone, Default)]
pub struct InMemoryStore {
    records: BTreeMap<(DataCategory, String), MemoryRecord>,
}

impl InMemoryStore {
    /// Stockage vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute ou remplace un enregistrement.
    pub fn insert(
        &mut self,
        category: DataCategory,
        id: impl Into<String>,
        reference_ms: u64,
        subjects: impl IntoIterator<Item = DataSubject>,
        media_type: impl Into<String>,
        data: Vec<u8>,
    ) {
        self.records.insert(
            (category, id.into()),
            MemoryRecord {
                reference_ms,
                subjects: subjects.into_iter().collect(),
                media_type: media_type.into(),
                data,
            },
        );
    }

    /// Nombre d'enregistrements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Vrai si le stockage est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Vrai si l'enregistrement existe.
    #[must_use]
    pub fn contains(&self, category: DataCategory, id: &str) -> bool {
        self.records.contains_key(&(category, id.to_owned()))
    }

    /// Personnes liées à un enregistrement.
    #[must_use]
    pub fn subjects_of(&self, category: DataCategory, id: &str) -> Option<&BTreeSet<DataSubject>> {
        self.records
            .get(&(category, id.to_owned()))
            .map(|r| &r.subjects)
    }

    fn reference(key: &(DataCategory, String), record: &MemoryRecord) -> RecordRef {
        RecordRef {
            category: key.0,
            id: key.1.clone(),
            reference_ms: record.reference_ms,
        }
    }
}

impl PersonalDataStore for InMemoryStore {
    type Error = Infallible;

    fn export_subject(&self, subject: &DataSubject) -> Result<Vec<ExportedRecord>, Infallible> {
        Ok(self
            .records
            .iter()
            .filter(|(_, r)| r.subjects.contains(subject))
            .map(|(key, r)| ExportedRecord {
                record: Self::reference(key, r),
                media_type: r.media_type.clone(),
                data: r.data.clone(),
            })
            .collect())
    }

    fn erase_subject(&mut self, subject: &DataSubject) -> Result<ErasureReport, Infallible> {
        let mut report = ErasureReport::default();
        self.records.retain(|key, record| {
            if !record.subjects.remove(subject) {
                return true;
            }
            let reference = Self::reference(key, record);
            if record.subjects.is_empty() {
                report.deleted.push(reference);
                false
            } else {
                report.anonymized.push(reference);
                true
            }
        });
        Ok(report)
    }

    fn list_records(
        &self,
        category: DataCategory,
        older_than_ms: Option<u64>,
    ) -> Result<Vec<RecordRef>, Infallible> {
        Ok(self
            .records
            .iter()
            .filter(|((c, _), r)| {
                *c == category && older_than_ms.is_none_or(|limit| r.reference_ms < limit)
            })
            .map(|(key, r)| Self::reference(key, r))
            .collect())
    }

    fn delete_records(&mut self, records: &[RecordRef]) -> Result<usize, Infallible> {
        Ok(records
            .iter()
            .filter(|r| self.records.remove(&(r.category, r.id.clone())).is_some())
            .count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_references_are_stable_and_distinct() {
        let a = DataSubject::opponent("Winamax", "Villain").unwrap();
        let b = DataSubject::opponent("winamax", "Villain").unwrap();
        let c = DataSubject::opponent("winamax", "villain").unwrap();
        assert_eq!(a.reference(), b.reference(), "salle normalisée");
        assert_ne!(
            a.reference(),
            c.reference(),
            "pseudonyme sensible à la casse"
        );
        assert_ne!(
            DataSubject::user("x").reference(),
            DataSubject::user("y").reference()
        );
        assert_eq!(a.reference().as_str().len(), 64);
        assert!(!a.reference().as_str().contains("Villain"));
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"kind\":\"opponent\""));
        assert_eq!(serde_json::from_str::<DataSubject>(&json).unwrap(), a);
    }

    #[test]
    fn list_and_delete() {
        let user = DataSubject::user("u");
        let mut store = InMemoryStore::new();
        store.insert(
            DataCategory::Videos,
            "v1",
            10,
            [user.clone()],
            "video/x",
            vec![],
        );
        store.insert(
            DataCategory::Videos,
            "v2",
            20,
            [user.clone()],
            "video/x",
            vec![],
        );
        store.insert(
            DataCategory::Progress,
            "p",
            5,
            [user],
            "application/json",
            vec![],
        );
        assert_eq!(
            store
                .list_records(DataCategory::Videos, None)
                .unwrap()
                .len(),
            2
        );
        let old = store.list_records(DataCategory::Videos, Some(20)).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].id, "v1");
        assert_eq!(store.delete_records(&old).unwrap(), 1);
        assert_eq!(
            store.delete_records(&old).unwrap(),
            0,
            "référence inconnue ignorée"
        );
        assert_eq!(store.len(), 2);
    }
}
