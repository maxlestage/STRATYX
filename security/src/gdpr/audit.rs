//! Journal d'audit horodaté et chaîné des opérations RGPD (responsabilité, art. 5.2).
//!
//! Chaque entrée contient l'empreinte SHA-256 de la précédente : modifier, supprimer ou
//! réordonner une entrée casse la chaîne ([`AuditLog::verify`]). Pour détecter aussi une
//! réécriture complète du journal, conserver ailleurs l'empreinte de tête
//! ([`AuditLog::head`]), par exemple dans les sauvegardes.
//!
//! Les personnes y figurent sous forme pseudonymisée ([`SubjectRef`]) : le journal prouve
//! qu'une demande a été traitée sans conserver le pseudonyme effacé.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::GdprError;
use super::store::{SubjectRef, hex};

/// Séparation de domaine des empreintes du journal.
const AUDIT_DOMAIN: &[u8] = b"STRATYX/audit/v1\0";
/// Empreinte précédant la première entrée.
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Opération consignée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// Export des données d'une personne (art. 15 et 20).
    Export,
    /// Effacement des données d'une personne (art. 17).
    Erasure,
    /// Purge des données arrivées au terme de leur conservation.
    RetentionPurge,
}

/// Issue de l'opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// Opération menée à terme.
    Completed,
    /// Opération en échec (le détail donne la cause).
    Failed,
}

/// Entrée du journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Numéro d'ordre, à partir de 0.
    pub sequence: u64,
    /// Horodatage (ms depuis l'époque Unix, UTC).
    pub at_ms: u64,
    /// Opération.
    pub action: AuditAction,
    /// Personne concernée, pseudonymisée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SubjectRef>,
    /// Issue.
    pub outcome: AuditOutcome,
    /// Nombre d'enregistrements concernés.
    pub records: u64,
    /// Détail lisible.
    pub detail: String,
    /// Empreinte de l'entrée précédente.
    pub previous_hash: String,
    /// Empreinte de cette entrée.
    pub hash: String,
}

/// Contenu haché d'une entrée (tout sauf `hash`), dans un ordre de champs fixe.
#[derive(Serialize)]
struct HashedFields<'a> {
    sequence: u64,
    at_ms: u64,
    action: AuditAction,
    subject: Option<&'a SubjectRef>,
    outcome: AuditOutcome,
    records: u64,
    detail: &'a str,
}

impl AuditEntry {
    fn compute_hash(&self) -> Result<String, GdprError> {
        let fields = HashedFields {
            sequence: self.sequence,
            at_ms: self.at_ms,
            action: self.action,
            subject: self.subject.as_ref(),
            outcome: self.outcome,
            records: self.records,
            detail: &self.detail,
        };
        let body =
            serde_json::to_vec(&fields).map_err(|e| GdprError::AuditFormat(e.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(AUDIT_DOMAIN);
        hasher.update(self.previous_hash.as_bytes());
        hasher.update(&body);
        Ok(hex(&hasher.finalize()))
    }
}

/// Journal d'audit en ajout seul.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    /// Journal vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute une entrée chaînée à la précédente.
    pub fn append(
        &mut self,
        at_ms: u64,
        action: AuditAction,
        subject: Option<SubjectRef>,
        outcome: AuditOutcome,
        records: u64,
        detail: impl Into<String>,
    ) -> &AuditEntry {
        let mut entry = AuditEntry {
            sequence: self.entries.len() as u64,
            at_ms,
            action,
            subject,
            outcome,
            records,
            detail: detail.into(),
            previous_hash: self.head().to_owned(),
            hash: String::new(),
        };
        // La sérialisation d'une structure sans table ni flottant ne peut pas échouer.
        entry.hash = entry.compute_hash().unwrap_or_default();
        self.entries.push(entry);
        &self.entries[self.entries.len() - 1]
    }

    /// Entrées, dans l'ordre.
    #[must_use]
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Empreinte de la dernière entrée (ou empreinte initiale si le journal est vide).
    #[must_use]
    pub fn head(&self) -> &str {
        self.entries.last().map_or(GENESIS, |e| e.hash.as_str())
    }

    /// Vérifie la numérotation et la chaîne d'empreintes.
    ///
    /// # Errors
    /// [`GdprError::AuditTampered`] à la première entrée incohérente.
    pub fn verify(&self) -> Result<(), GdprError> {
        let mut previous = GENESIS;
        for (index, entry) in self.entries.iter().enumerate() {
            let tampered = |reason| GdprError::AuditTampered {
                sequence: entry.sequence,
                reason,
            };
            if entry.sequence != index as u64 {
                return Err(tampered("numéro d'ordre inattendu"));
            }
            if entry.previous_hash != previous {
                return Err(tampered("chaînage rompu"));
            }
            if entry.compute_hash()? != entry.hash {
                return Err(tampered("contenu modifié"));
            }
            previous = &entry.hash;
        }
        Ok(())
    }

    /// Une entrée JSON par ligne.
    #[must_use]
    pub fn to_json_lines(&self) -> String {
        self.entries
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .map(|line| line + "\n")
            .collect()
    }

    /// Relit un journal produit par [`AuditLog::to_json_lines`] et vérifie sa chaîne.
    ///
    /// # Errors
    /// [`GdprError::AuditFormat`] ; [`GdprError::AuditTampered`].
    pub fn from_json_lines(text: &str) -> Result<Self, GdprError> {
        let entries = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<AuditEntry>, _>>()
            .map_err(|e| GdprError::AuditFormat(e.to_string()))?;
        let log = Self { entries };
        log.verify()?;
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::DataSubject;
    use super::*;

    fn sample() -> AuditLog {
        let mut log = AuditLog::new();
        let subject = DataSubject::opponent("winamax", "Villain")
            .unwrap()
            .reference();
        log.append(
            1_000,
            AuditAction::Export,
            Some(subject.clone()),
            AuditOutcome::Completed,
            3,
            "export",
        );
        log.append(
            2_000,
            AuditAction::Erasure,
            Some(subject),
            AuditOutcome::Completed,
            3,
            "effacement",
        );
        log.append(
            3_000,
            AuditAction::RetentionPurge,
            None,
            AuditOutcome::Failed,
            0,
            "base indisponible",
        );
        log
    }

    #[test]
    fn chain_round_trip() {
        let log = sample();
        log.verify().unwrap();
        assert_eq!(log.entries()[0].previous_hash, GENESIS);
        assert_eq!(log.entries()[1].previous_hash, log.entries()[0].hash);
        assert_eq!(log.head(), log.entries()[2].hash);
        let text = log.to_json_lines();
        assert_eq!(text.lines().count(), 3);
        assert!(!text.contains("Villain"), "aucun pseudonyme en clair");
        assert_eq!(AuditLog::from_json_lines(&text).unwrap(), log);
    }

    #[test]
    fn tampering_is_detected() {
        let log = sample();

        let mut edited = log.clone();
        edited.entries[1].records = 0;
        assert!(matches!(
            edited.verify(),
            Err(GdprError::AuditTampered { sequence: 1, .. })
        ));

        let mut removed = log.clone();
        removed.entries.remove(1);
        assert!(removed.verify().is_err());

        let mut swapped = log.clone();
        swapped.entries.swap(0, 1);
        assert!(swapped.verify().is_err());

        // Réécriture d'une entrée avec recalcul de son empreinte : la suivante ne suit plus.
        let mut rehashed = log.clone();
        rehashed.entries[0].detail = "autre".into();
        rehashed.entries[0].hash = rehashed.entries[0].compute_hash().unwrap();
        assert!(matches!(
            rehashed.verify(),
            Err(GdprError::AuditTampered { sequence: 1, .. })
        ));

        let text = log.to_json_lines().replace("effacement", "rien");
        assert!(AuditLog::from_json_lines(&text).is_err());
        assert!(AuditLog::from_json_lines("pas du json").is_err());
    }
}
