//! Ingestion d'archives (décision D5) : provenance et licence obligatoires, validation par
//! rejeu, déduplication, rapport détaillé.

use serde::{Deserialize, Serialize};
use stratyx_poker::HandRecord;
use thiserror::Error;

use crate::error::{DatabaseError, Result};
use crate::fingerprint::Fingerprint;
use crate::migration::{META_NEXT_BATCH, meta, set_meta};
use crate::space::Space;
use crate::status::{Validation, ValidationKind};
use crate::store::{Database, StoreMode, Stored, store_hand, to_json};
use crate::text::{self, ImportOptions};
use crate::users::parse_all;

/// Provenance d'un lot d'archives. Obligatoire : seules les mains de l'utilisateur, les
/// données sous licence vérifiée et les simulations sont admises (décision D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Origine du lot : « mains personnelles (PokerStars) », nom du fournisseur…
    pub source: String,
    /// Licence ou base légale d'utilisation : « mains de l'utilisateur », « CC BY 4.0 »,
    /// référence du contrat…
    pub license: String,
    /// Précisions libres (date d'acquisition, périmètre…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Provenance {
    /// Provenance validée (source et licence non vides).
    pub fn new(source: impl Into<String>, license: impl Into<String>) -> Result<Provenance> {
        let p = Provenance {
            source: source.into(),
            license: license.into(),
            notes: None,
        };
        p.validate()?;
        Ok(p)
    }

    /// Ajoute des précisions libres.
    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Provenance {
        self.notes = Some(notes.into());
        self
    }

    /// Vérifie que la source et la licence sont renseignées.
    pub fn validate(&self) -> Result<()> {
        if self.source.trim().is_empty() {
            return Err(DatabaseError::MissingProvenance("la source"));
        }
        if self.license.trim().is_empty() {
            return Err(DatabaseError::MissingProvenance("la licence"));
        }
        Ok(())
    }

    /// Libellé reporté dans [`HandMeta::provenance`](stratyx_poker::HandMeta) de chaque
    /// main du lot qui n'en a pas.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} (licence : {})", self.source.trim(), self.license.trim())
    }
}

/// Lot d'archives à ingérer.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveBatch {
    pub provenance: Provenance,
    pub hands: Vec<HandRecord>,
}

impl ArchiveBatch {
    /// Lot de mains d'une même provenance.
    #[must_use]
    pub fn new(provenance: Provenance, hands: Vec<HandRecord>) -> ArchiveBatch {
        ArchiveBatch { provenance, hands }
    }
}

/// Règles d'admission des mains d'un lot. Une main invalide (rejeu refusé) est toujours
/// rejetée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestPolicy {
    /// Admettre les mains dont le gagnant est indéterminable (cartes non vues). Défaut : oui,
    /// les actions restent exploitables pour les tendances.
    pub accept_unresolved: bool,
    /// Admettre les mains incomplètes. Défaut : non.
    pub accept_incomplete: bool,
    /// Admettre les mains contenant des détections incertaines. Défaut : non.
    pub accept_needing_review: bool,
}

impl Default for IngestPolicy {
    fn default() -> Self {
        IngestPolicy {
            accept_unresolved: true,
            accept_incomplete: false,
            accept_needing_review: false,
        }
    }
}

/// Raison du rejet d'une main.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum RejectReason {
    #[error("historique illisible : {0}")]
    Unreadable(String),
    #[error("identifiant de main vide")]
    EmptyId,
    #[error("rejeu refusé : {0}")]
    Invalid(String),
    #[error("main incomplète")]
    Incomplete,
    #[error("résultat non résolu : {0}")]
    Unresolved(String),
    #[error("{0} détection(s) incertaine(s) à revoir")]
    NeedsReview(u32),
    #[error("identifiant déjà attribué à une autre main (empreinte {0})")]
    IdConflict(Fingerprint),
}

/// Main admise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedHand {
    pub id: String,
    pub space: Space,
    pub fingerprint: Fingerprint,
    pub validation: ValidationKind,
}

/// Main déjà présente (même contenu de jeu).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateHand {
    pub id: String,
    pub existing_id: String,
    pub fingerprint: Fingerprint,
}

/// Main rejetée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedHand {
    /// Rang de la main dans le lot (à partir de 0).
    pub position: usize,
    /// Identifiant, s'il a pu être lu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub reason: RejectReason,
}

/// Rapport d'ingestion d'un lot, conservé dans la base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestReport {
    pub batch_id: String,
    pub provenance: Provenance,
    pub policy: IngestPolicy,
    pub ingested_at_ms: u64,
    pub accepted: Vec<AcceptedHand>,
    pub duplicates: Vec<DuplicateHand>,
    pub rejected: Vec<RejectedHand>,
}

impl IngestReport {
    /// Nombre de mains examinées.
    #[must_use]
    pub fn total(&self) -> usize {
        self.accepted.len() + self.duplicates.len() + self.rejected.len()
    }
}

/// Élément d'un lot : main lue, ou erreur de lecture (identifiant éventuel, raison).
type BatchItem = std::result::Result<HandRecord, (Option<String>, String)>;

impl Database {
    /// Ingère un lot d'archives dans une seule transaction : chaque main est validée par
    /// rejeu, dédupliquée par empreinte puis rangée dans l'espace de son format. Les mains
    /// sans provenance reçoivent celle du lot. Le rapport est enregistré
    /// ([`Database::ingestion_reports`]).
    pub fn ingest(&self, batch: ArchiveBatch, policy: IngestPolicy) -> Result<IngestReport> {
        let ArchiveBatch { provenance, hands } = batch;
        self.ingest_items(provenance, hands.into_iter().map(Ok).collect(), policy)
    }

    /// Ingère un fichier d'historiques au format texte PokerStars (voir [`text`]). Les mains
    /// illisibles figurent dans le rapport comme rejetées.
    pub fn ingest_pokerstars(
        &self,
        provenance: Provenance,
        content: &str,
        options: &ImportOptions,
        policy: IngestPolicy,
    ) -> Result<IngestReport> {
        let items = text::import_pokerstars(content, options)
            .into_iter()
            .map(|r| match r {
                Ok(h) => Ok(h.record),
                Err(e) => Err((e.hand_id().map(str::to_string), e.to_string())),
            })
            .collect();
        self.ingest_items(provenance, items, policy)
    }

    fn ingest_items(
        &self,
        provenance: Provenance,
        items: Vec<BatchItem>,
        policy: IngestPolicy,
    ) -> Result<IngestReport> {
        provenance.validate()?;
        let label = provenance.label();
        self.write(|tx| {
            let n = meta(tx, META_NEXT_BATCH)?.unwrap_or(1);
            set_meta(tx, META_NEXT_BATCH, n + 1)?;
            let batch_id = format!("lot-{n:06}");
            let mut report = IngestReport {
                batch_id: batch_id.clone(),
                provenance,
                policy,
                ingested_at_ms: crate::now_ms(),
                accepted: Vec::new(),
                duplicates: Vec::new(),
                rejected: Vec::new(),
            };
            for (position, item) in items.into_iter().enumerate() {
                let mut record = match item {
                    Ok(r) => r,
                    Err((id, reason)) => {
                        report.rejected.push(RejectedHand {
                            position,
                            id,
                            reason: RejectReason::Unreadable(reason),
                        });
                        continue;
                    }
                };
                let reported_id = Some(record.id.clone()).filter(|id| !id.trim().is_empty());
                let reject = |reason| RejectedHand {
                    position,
                    id: reported_id.clone(),
                    reason,
                };
                if record.id.trim().is_empty() {
                    report.rejected.push(reject(RejectReason::EmptyId));
                    continue;
                }
                let space = Space::for_format(record.format);
                let summary = crate::status::analyze(&record, space, self.review_threshold);
                let refusal = match &summary.status.validation {
                    Validation::Invalid { reason } => Some(RejectReason::Invalid(reason.clone())),
                    Validation::Incomplete if !policy.accept_incomplete => {
                        Some(RejectReason::Incomplete)
                    }
                    Validation::Unresolved { reason } if !policy.accept_unresolved => {
                        Some(RejectReason::Unresolved(reason.clone()))
                    }
                    _ if summary.status.review_items > 0 && !policy.accept_needing_review => {
                        Some(RejectReason::NeedsReview(summary.status.review_items))
                    }
                    _ => None,
                };
                if let Some(reason) = refusal {
                    report.rejected.push(reject(reason));
                    continue;
                }
                if record.meta.provenance.is_none() {
                    record.meta.provenance = Some(label.clone());
                }
                match store_hand(
                    tx,
                    &record,
                    self.review_threshold,
                    Some(&batch_id),
                    StoreMode::Ingest,
                )? {
                    Stored::Inserted(s) | Stored::Updated(s) => {
                        report.accepted.push(AcceptedHand {
                            id: s.id,
                            space: s.space,
                            fingerprint: s.fingerprint,
                            validation: s.status.validation.kind(),
                        })
                    }
                    Stored::Duplicate {
                        existing_id,
                        fingerprint,
                    } => report.duplicates.push(DuplicateHand {
                        id: record.id.clone(),
                        existing_id,
                        fingerprint,
                    }),
                    Stored::IdConflict { existing } => {
                        report
                            .rejected
                            .push(reject(RejectReason::IdConflict(existing)));
                    }
                }
            }
            tx.exec(
                "INSERT INTO ingestion (id, rapport) VALUES (?, ?)",
                &[batch_id.as_str().into(), to_json(&report)?.into()],
            )?;
            Ok(report)
        })
    }

    /// Rapports d'ingestion enregistrés, du plus ancien au plus récent.
    pub fn ingestion_reports(&self) -> Result<Vec<IngestReport>> {
        let mut out: Vec<IngestReport> =
            parse_all(self.read()?.texts("SELECT rapport FROM ingestion", &[])?)?;
        out.sort_by(|a, b| a.batch_id.cmp(&b.batch_id));
        Ok(out)
    }
}
