//! Base locale : ouverture, enregistrement et lecture des mains.

use std::fmt;
use std::path::Path;

use redb::backends::InMemoryBackend;
use redb::{ReadableDatabase, ReadableTable, WriteTransaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use stratyx_poker::HandRecord;
use stratyx_shared::Confidence;

use crate::error::{DatabaseError, Result};
use crate::fingerprint::Fingerprint;
use crate::migration::{self, AppliedMigration};
use crate::query::format_key;
use crate::space::Space;
use crate::status::{HandStatus, HandSummary, analyze};
use crate::tables::{self, FINGERPRINTS, HandTables, LOCATOR, STATUS_TO_REVIEW};

/// Version du format d'enregistrement d'une main ([`StoredHand`]).
pub const HAND_FORMAT_VERSION: u32 = 1;

/// Main telle qu'enregistrée : historique pivot ([`HandRecord`]) et informations calculées.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredHand {
    /// Version du format d'enregistrement ([`HAND_FORMAT_VERSION`]).
    pub format_version: u32,
    pub space: Space,
    pub fingerprint: Fingerprint,
    pub status: HandStatus,
    /// Lot d'archives d'origine, pour une main ingérée (décision D5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    /// Date d'enregistrement, en millisecondes depuis l'époque Unix.
    pub stored_at_ms: u64,
    pub record: HandRecord,
}

/// Résultat de [`Database::put_hand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutOutcome {
    /// Nouvelle main enregistrée.
    Inserted(HandSummary),
    /// Main existante (même identifiant) remplacée, par exemple après correction.
    Updated(HandSummary),
    /// Contenu de jeu identique à une main déjà enregistrée sous un autre identifiant :
    /// rien n'est écrit.
    Duplicate {
        existing_id: String,
        fingerprint: Fingerprint,
    },
}

/// Base de données locale de Stratyx : un fichier redb unique, organisé en cinq espaces
/// logiques ([`Space`]).
///
/// Chaque méthode s'exécute dans sa propre transaction ACID. La base peut être partagée
/// entre fils d'exécution (`&Database` suffit, les écritures sont sérialisées par redb).
pub struct Database {
    pub(crate) db: redb::Database,
    pub(crate) review_threshold: Confidence,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("review_threshold", &self.review_threshold)
            .finish_non_exhaustive()
    }
}

/// Mode d'écriture d'une main.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreMode {
    /// Remplace la main de même identifiant ; refuse un doublon d'une autre main.
    Upsert,
    /// Archive : ne remplace jamais une main existante.
    Ingest,
    /// Réécriture interne (anonymisation, revalidation) : pas de contrôle de doublon.
    Force,
}

/// Résultat interne d'une écriture.
#[derive(Debug)]
pub(crate) enum Stored {
    Inserted(HandSummary),
    Updated(HandSummary),
    Duplicate {
        existing_id: String,
        fingerprint: Fingerprint,
    },
    IdConflict {
        existing: Fingerprint,
    },
}

impl Database {
    /// Ouvre la base du fichier indiqué, la crée si besoin et applique les migrations de
    /// schéma manquantes.
    pub fn open(path: impl AsRef<Path>) -> Result<Database> {
        Database::init(redb::Database::create(path)?)
    }

    /// Base en mémoire, sans fichier (tests, simulations).
    pub fn in_memory() -> Result<Database> {
        Database::init(redb::Database::builder().create_with_backend(InMemoryBackend::new())?)
    }

    fn init(db: redb::Database) -> Result<Database> {
        migration::migrate(&db, migration::MIGRATIONS)?;
        Ok(Database {
            db,
            review_threshold: Confidence::DEFAULT_REVIEW_THRESHOLD,
        })
    }

    /// Seuil de confiance sous lequel une détection de la vision est à revoir
    /// (par défaut [`Confidence::DEFAULT_REVIEW_THRESHOLD`]). S'applique aux écritures
    /// suivantes ; [`Database::revalidate`] l'applique aux mains déjà enregistrées.
    #[must_use]
    pub fn with_review_threshold(mut self, threshold: Confidence) -> Database {
        self.review_threshold = threshold;
        self
    }

    /// Version du schéma de la base.
    pub fn schema_version(&self) -> Result<u32> {
        migration::current_version(&self.db)
    }

    /// Journal des migrations appliquées à cette base.
    pub fn migration_log(&self) -> Result<Vec<AppliedMigration>> {
        migration::log(&self.db)
    }

    // --- Mains ----------------------------------------------------------------------

    /// Enregistre une main de l'utilisateur, quel que soit son statut de validation (une main
    /// invalide reste consultable pour être corrigée). L'espace découle du format.
    ///
    /// Une main de même identifiant est remplacée (correction après revue). Une main dont le
    /// contenu de jeu est déjà enregistré sous un autre identifiant n'est pas écrite
    /// ([`PutOutcome::Duplicate`]).
    pub fn put_hand(&self, record: &HandRecord) -> Result<PutOutcome> {
        let txn = self.db.begin_write()?;
        let outcome = store_hand(&txn, record, self.review_threshold, None, StoreMode::Upsert)?;
        txn.commit()?;
        Ok(match outcome {
            Stored::Inserted(s) => PutOutcome::Inserted(s),
            Stored::Updated(s) => PutOutcome::Updated(s),
            Stored::Duplicate {
                existing_id,
                fingerprint,
            } => PutOutcome::Duplicate {
                existing_id,
                fingerprint,
            },
            Stored::IdConflict { .. } => unreachable!("mode Upsert"),
        })
    }

    /// Main enregistrée sous cet identifiant.
    pub fn get_hand(&self, id: &str) -> Result<Option<StoredHand>> {
        let txn = self.db.begin_read()?;
        let Some(space) = locate(&txn.open_table(LOCATOR)?, id)? else {
            return Ok(None);
        };
        let t = space_tables(space)?;
        let Some(bytes) = txn.open_table(t.hands)?.get(id)? else {
            return Err(DatabaseError::Corrupted(format!(
                "main « {id} » référencée mais absente"
            )));
        };
        decode_hand(id, bytes.value()).map(Some)
    }

    /// Résumé (statut, joueurs, résultat du héros) de la main.
    pub fn hand_summary(&self, id: &str) -> Result<Option<HandSummary>> {
        let txn = self.db.begin_read()?;
        let Some(space) = locate(&txn.open_table(LOCATOR)?, id)? else {
            return Ok(None);
        };
        let t = space_tables(space)?;
        let summary = txn.open_table(t.summaries)?.get(id)?;
        summary.map(|v| from_json(v.value())).transpose()
    }

    /// Identifiant de la main enregistrée avec cette empreinte.
    pub fn find_by_fingerprint(&self, fingerprint: &Fingerprint) -> Result<Option<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(FINGERPRINTS)?;
        Ok(table
            .get(fingerprint.as_bytes())?
            .map(|v| v.value().to_string()))
    }

    /// Supprime une main et ses entrées d'index. Renvoie `false` si elle n'existait pas.
    pub fn delete_hand(&self, id: &str) -> Result<bool> {
        let txn = self.db.begin_write()?;
        let removed = remove_hand(&txn, id)?.is_some();
        txn.commit()?;
        Ok(removed)
    }

    /// Nombre de mains, dans un espace ou dans toute la base.
    pub fn count_hands(&self, space: Option<Space>) -> Result<u64> {
        let txn = self.db.begin_read()?;
        let mut total = 0;
        for s in Space::HAND_SPACES {
            if space.is_none_or(|x| x == s) {
                total +=
                    redb::ReadableTableMetadata::len(&txn.open_table(space_tables(s)?.hands)?)?;
            }
        }
        Ok(total)
    }

    /// Rejoue toutes les mains enregistrées et met à jour statuts, empreintes et index (après
    /// une évolution du moteur de règles ou un changement de seuil de revue). Renvoie le
    /// nombre de mains dont le statut a changé.
    pub fn revalidate(&self) -> Result<usize> {
        let txn = self.db.begin_write()?;
        let mut changed = 0;
        for space in Space::HAND_SPACES {
            let ids: Vec<String> = {
                let table = txn.open_table(space_tables(space)?.hands)?;
                table
                    .iter()?
                    .map(|e| e.map(|(k, _)| k.value().to_string()))
                    .collect::<Result<_, _>>()?
            };
            for id in ids {
                let Some(old) = read_hand_in(&txn, &id)? else {
                    continue;
                };
                let summary = match store_hand(
                    &txn,
                    &old.record,
                    self.review_threshold,
                    old.batch_id.as_deref(),
                    StoreMode::Force,
                )? {
                    Stored::Inserted(s) | Stored::Updated(s) => s,
                    _ => continue,
                };
                if summary.status != old.status {
                    changed += 1;
                }
            }
        }
        txn.commit()?;
        Ok(changed)
    }
}

// --- Fonctions internes, dans une transaction d'écriture ----------------------------------

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

pub(crate) fn from_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}

pub(crate) fn space_tables(space: Space) -> Result<HandTables> {
    tables::hand_tables(space).ok_or_else(|| {
        DatabaseError::Corrupted(format!("l'espace {space} ne contient pas de mains"))
    })
}

pub(crate) fn locate(
    table: &impl ReadableTable<&'static str, u8>,
    id: &str,
) -> Result<Option<Space>> {
    match table.get(id)? {
        None => Ok(None),
        Some(code) => Space::from_code(code.value())
            .map(Some)
            .ok_or_else(|| DatabaseError::Corrupted(format!("code d'espace {}", code.value()))),
    }
}

pub(crate) fn decode_hand(id: &str, bytes: &[u8]) -> Result<StoredHand> {
    #[derive(Deserialize)]
    struct Version {
        format_version: u32,
    }
    let v: Version = from_json(bytes)?;
    if v.format_version > HAND_FORMAT_VERSION {
        return Err(DatabaseError::RecordTooNew {
            id: id.to_string(),
            found: v.format_version,
            supported: HAND_FORMAT_VERSION,
        });
    }
    from_json(bytes)
}

pub(crate) fn read_hand_in(txn: &WriteTransaction, id: &str) -> Result<Option<StoredHand>> {
    let Some(space) = locate(&txn.open_table(LOCATOR)?, id)? else {
        return Ok(None);
    };
    let table = txn.open_table(space_tables(space)?.hands)?;
    let bytes = table.get(id)?;
    bytes.map(|b| decode_hand(id, b.value())).transpose()
}

/// Enregistre une main (et ses index) dans la transaction.
pub(crate) fn store_hand(
    txn: &WriteTransaction,
    record: &HandRecord,
    threshold: Confidence,
    batch_id: Option<&str>,
    mode: StoreMode,
) -> Result<Stored> {
    if record.id.trim().is_empty() {
        return Err(DatabaseError::EmptyId("main"));
    }
    let id = record.id.as_str();
    let space = Space::for_format(record.format);
    let summary = analyze(record, space, threshold);
    let fp = summary.fingerprint;

    if mode != StoreMode::Force {
        let fingerprints = txn.open_table(FINGERPRINTS)?;
        if let Some(existing) = fingerprints.get(fp.as_bytes())? {
            let existing = existing.value();
            if existing != id {
                return Ok(Stored::Duplicate {
                    existing_id: existing.to_string(),
                    fingerprint: fp,
                });
            }
        }
    }

    let existed = match mode {
        StoreMode::Ingest => {
            if let Some(old) = read_hand_in(txn, id)? {
                return Ok(if old.fingerprint == fp {
                    Stored::Duplicate {
                        existing_id: id.to_string(),
                        fingerprint: fp,
                    }
                } else {
                    Stored::IdConflict {
                        existing: old.fingerprint,
                    }
                });
            }
            false
        }
        StoreMode::Upsert | StoreMode::Force => remove_hand(txn, id)?.is_some(),
    };

    let stored = StoredHand {
        format_version: HAND_FORMAT_VERSION,
        space,
        fingerprint: fp,
        status: summary.status.clone(),
        batch_id: batch_id.map(str::to_string),
        stored_at_ms: crate::now_ms(),
        record: record.clone(),
    };
    let t = space_tables(space)?;
    txn.open_table(t.hands)?
        .insert(id, to_json(&stored)?.as_slice())?;
    txn.open_table(t.summaries)?
        .insert(id, to_json(&summary)?.as_slice())?;
    index(txn, &t, &summary, true)?;
    txn.open_table(LOCATOR)?.insert(id, space.code())?;
    let mut fingerprints = txn.open_table(FINGERPRINTS)?;
    if fingerprints.get(fp.as_bytes())?.is_none() {
        fingerprints.insert(fp.as_bytes(), id)?;
    }
    Ok(if existed {
        Stored::Updated(summary)
    } else {
        Stored::Inserted(summary)
    })
}

/// Supprime une main et ses entrées d'index ; renvoie son résumé.
pub(crate) fn remove_hand(txn: &WriteTransaction, id: &str) -> Result<Option<HandSummary>> {
    let space = {
        let mut locator = txn.open_table(LOCATOR)?;
        let Some(code) = locator.remove(id)? else {
            return Ok(None);
        };
        let code = code.value();
        Space::from_code(code)
            .ok_or_else(|| DatabaseError::Corrupted(format!("code d'espace {code}")))?
    };
    let t = space_tables(space)?;
    txn.open_table(t.hands)?.remove(id)?;
    let summary: HandSummary = {
        let mut summaries = txn.open_table(t.summaries)?;
        let bytes = summaries.remove(id)?.ok_or_else(|| {
            DatabaseError::Corrupted(format!("résumé de la main « {id} » absent"))
        })?;
        from_json(bytes.value())?
    };
    index(txn, &t, &summary, false)?;
    let mut fingerprints = txn.open_table(FINGERPRINTS)?;
    let owned = fingerprints
        .get(summary.fingerprint.as_bytes())?
        .is_some_and(|v| v.value() == id);
    if owned {
        fingerprints.remove(summary.fingerprint.as_bytes())?;
    }
    Ok(Some(summary))
}

/// Ajoute (`insert = true`) ou retire les entrées d'index d'une main.
fn index(txn: &WriteTransaction, t: &HandTables, s: &HandSummary, insert: bool) -> Result<()> {
    let id = s.id.as_str();
    let mut entries: Vec<(redb::MultimapTableDefinition<&str, &str>, String)> =
        s.players.iter().map(|p| (t.by_player, p.clone())).collect();
    if let Some(session) = &s.session_id {
        entries.push((t.by_session, session.clone()));
    }
    entries.push((t.by_format, format_key(s.format)));
    entries.push((t.by_status, s.status.validation.kind().key().to_string()));
    if s.status.needs_review() {
        entries.push((t.by_status, STATUS_TO_REVIEW.to_string()));
    }
    for (def, key) in entries {
        let mut table = txn.open_multimap_table(def)?;
        if insert {
            table.insert(key.as_str(), id)?;
        } else {
            table.remove(key.as_str(), id)?;
        }
    }
    if let Some(at) = s.played_at_ms {
        let mut dates = txn.open_table(t.by_date)?;
        if insert {
            dates.insert((at, id), ())?;
        } else {
            dates.remove((at, id))?;
        }
    }
    Ok(())
}
