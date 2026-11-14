//! Base locale : ouverture, enregistrement et lecture des mains.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use nuage::{Embedded, EngineOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use stratyx_poker::HandRecord;
use stratyx_shared::Confidence;

use crate::error::{DatabaseError, Result};
use crate::fingerprint::Fingerprint;
use crate::migration::{self, AppliedMigration};
use crate::query::format_key;
use crate::space::Space;
use crate::sql::{Tx, storage, to_u64};
use crate::status::{HandStatus, HandSummary, analyze};

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

/// Base de données locale de Stratyx : un dossier Nuage DB (base embarquée transactionnelle
/// en Rust pur, décision T14), organisé en cinq espaces logiques ([`Space`]).
///
/// Chaque méthode s'exécute dans sa propre transaction ACID. La base peut être partagée
/// entre fils d'exécution (`&Database` suffit, les écritures sont sérialisées).
pub struct Database {
    pub(crate) db: Embedded,
    pub(crate) review_threshold: Confidence,
    /// Sérialise les écritures (voir [`Database::write`]).
    pub(crate) writes: Mutex<()>,
    /// Dossier temporaire d'une base [`Database::in_memory`], supprimé avec elle.
    temp: Option<TempDir>,
}

/// Dossier supprimé à la fermeture de la base.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        // Tout sur disque avant de rendre le dossier (une base temporaire est supprimée
        // ensuite, champ `temp`).
        let _ = self.db.engine().close();
    }
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("review_threshold", &self.review_threshold)
            .field("temporary", &self.temp.is_some())
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
    /// Ouvre la base du dossier indiqué, la crée si besoin et applique les migrations de
    /// schéma manquantes.
    ///
    /// Si `path` est un **fichier**, c'est une ancienne base redb (versions précédentes de
    /// Stratyx) : elle est renommée `<nom>.ancienne-redb`, son contenu est importé dans une
    /// base Nuage créée à sa place, puis elle est gardée telle quelle en secours. En cas
    /// d'échec, rien n'est changé (fonctionnalité `import-redb`, active par défaut).
    pub fn open(path: impl AsRef<Path>) -> Result<Database> {
        let path = path.as_ref();
        if path.is_file() {
            return crate::legacy::open_and_import(path);
        }
        Database::open_dir(path, EngineOptions::default())
    }

    /// Base temporaire, supprimée à sa fermeture (tests, simulations).
    pub fn in_memory() -> Result<Database> {
        let db = Database::unmigrated()?;
        migration::migrate(&db, migration::MIGRATIONS)?;
        Ok(db)
    }

    /// Base temporaire sans schéma (tests des migrations).
    pub(crate) fn unmigrated() -> Result<Database> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "stratyx-base-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            crate::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let options = EngineOptions {
            sync: false,
            ..EngineOptions::default()
        };
        let db = Embedded::open_with(&dir, options).map_err(storage)?;
        Ok(Database {
            db,
            review_threshold: Confidence::DEFAULT_REVIEW_THRESHOLD,
            writes: Mutex::new(()),
            temp: Some(TempDir(dir)),
        })
    }

    pub(crate) fn open_dir(dir: &Path, options: EngineOptions) -> Result<Database> {
        let db = Database {
            db: Embedded::open_with(dir, options).map_err(storage)?,
            review_threshold: Confidence::DEFAULT_REVIEW_THRESHOLD,
            writes: Mutex::new(()),
            temp: None,
        };
        migration::migrate(&db, migration::MIGRATIONS)?;
        Ok(db)
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
        migration::current_version(&self.read()?)
    }

    /// Journal des migrations appliquées à cette base.
    pub fn migration_log(&self) -> Result<Vec<AppliedMigration>> {
        migration::log(&self.read()?)
    }

    // --- Mains ----------------------------------------------------------------------

    /// Enregistre une main de l'utilisateur, quel que soit son statut de validation (une main
    /// invalide reste consultable pour être corrigée). L'espace découle du format.
    ///
    /// Une main de même identifiant est remplacée (correction après revue). Une main dont le
    /// contenu de jeu est déjà enregistré sous un autre identifiant n'est pas écrite
    /// ([`PutOutcome::Duplicate`]).
    pub fn put_hand(&self, record: &HandRecord) -> Result<PutOutcome> {
        let outcome = self
            .write(|tx| store_hand(tx, record, self.review_threshold, None, StoreMode::Upsert))?;
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
        read_hand_in(&self.read()?, id)
    }

    /// Résumé (statut, joueurs, résultat du héros) de la main.
    pub fn hand_summary(&self, id: &str) -> Result<Option<HandSummary>> {
        self.read()?
            .text("SELECT resume FROM main WHERE id = ?", &[id.into()])?
            .map(|json| from_json(json.as_bytes()))
            .transpose()
    }

    /// Identifiant de la main enregistrée avec cette empreinte.
    pub fn find_by_fingerprint(&self, fingerprint: &Fingerprint) -> Result<Option<String>> {
        self.read()?.text(
            "SELECT main FROM empreinte WHERE empreinte = ?",
            &[fingerprint.to_hex().into()],
        )
    }

    /// Supprime une main et ses entrées d'index. Renvoie `false` si elle n'existait pas.
    pub fn delete_hand(&self, id: &str) -> Result<bool> {
        self.write(|tx| Ok(remove_hand(tx, id)?.is_some()))
    }

    /// Nombre de mains, dans un espace ou dans toute la base.
    pub fn count_hands(&self, space: Option<Space>) -> Result<u64> {
        let tx = self.read()?;
        let n = match space {
            None => tx.int("SELECT count(*) FROM main", &[])?,
            Some(s) => tx.int(
                "SELECT count(*) FROM main WHERE espace = ?",
                &[s.code().into()],
            )?,
        };
        to_u64(n.unwrap_or(0))
    }

    /// Rejoue toutes les mains enregistrées et met à jour statuts, empreintes et index (après
    /// une évolution du moteur de règles ou un changement de seuil de revue). Renvoie le
    /// nombre de mains dont le statut a changé.
    pub fn revalidate(&self) -> Result<usize> {
        self.write(|tx| {
            let mut changed = 0;
            for id in tx.texts("SELECT id FROM main ORDER BY id", &[])? {
                let Some(old) = read_hand_in(tx, &id)? else {
                    continue;
                };
                let summary = match store_hand(
                    tx,
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
            Ok(changed)
        })
    }
}

// --- Fonctions internes, dans une transaction ----------------------------------------------

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

pub(crate) fn from_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
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

pub(crate) fn read_hand_in(tx: &Tx<'_>, id: &str) -> Result<Option<StoredHand>> {
    tx.text(
        "SELECT donnees FROM main_donnees WHERE id = ?",
        &[id.into()],
    )?
    .map(|json| decode_hand(id, json.as_bytes()))
    .transpose()
}

/// Écrit la ligne d'une main et ses index (la main ne doit pas déjà exister) ; l'empreinte
/// lui revient si aucune autre main ne la porte.
pub(crate) fn insert_hand_row(tx: &Tx<'_>, stored: &StoredHand, s: &HandSummary) -> Result<()> {
    let id = s.id.as_str();
    let fp = s.fingerprint.to_hex();
    let mut params: Vec<Json> = vec![
        id.into(),
        s.space.code().into(),
        fp.clone().into(),
        s.session_id.clone().into(),
        format_key(s.format).into(),
        s.status.validation.kind().key().into(),
        u8::from(s.status.needs_review()).into(),
        s.played_at_ms.into(),
        to_json(s)?.into(),
        id.into(),
        to_json(stored)?.into(),
        fp.into(),
        id.into(),
    ];
    // Une seule requête (lue et exécutée d'un coup) : la ligne de recherche, la main complète
    // (plusieurs Ko, dans sa propre table pour que la recherche ne parcoure que des lignes
    // courtes), l'empreinte si aucune autre main ne la porte, et les joueurs.
    let mut sql = String::from(
        "INSERT INTO main (id, espace, empreinte, session, format, validation, a_revoir, \
         joue_le, resume) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?); \
         INSERT INTO main_donnees (id, donnees) VALUES (?, ?); \
         INSERT OR IGNORE INTO empreinte (empreinte, main) VALUES (?, ?)",
    );
    if !s.players.is_empty() {
        sql.push_str("; INSERT OR IGNORE INTO main_joueur (cle, main, joueur) VALUES ");
        for (i, player) in s.players.iter().enumerate() {
            sql.push_str(if i == 0 { "(?, ?, ?)" } else { ", (?, ?, ?)" });
            params.push(format!("{id}\u{1f}{player}").into());
            params.push(id.into());
            params.push(player.as_str().into());
        }
    }
    tx.exec(&sql, &params)?;
    Ok(())
}

/// Enregistre une main (et ses index) dans la transaction.
pub(crate) fn store_hand(
    tx: &Tx<'_>,
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

    if mode != StoreMode::Force
        && let Some(existing) = tx.text(
            "SELECT main FROM empreinte WHERE empreinte = ?",
            &[fp.to_hex().into()],
        )?
        && existing != id
    {
        return Ok(Stored::Duplicate {
            existing_id: existing,
            fingerprint: fp,
        });
    }

    let existed = match mode {
        StoreMode::Ingest => {
            if let Some(old) = read_hand_in(tx, id)? {
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
        StoreMode::Upsert | StoreMode::Force => remove_hand(tx, id)?.is_some(),
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
    insert_hand_row(tx, &stored, &summary)?;
    Ok(if existed {
        Stored::Updated(summary)
    } else {
        Stored::Inserted(summary)
    })
}

/// Supprime une main et ses entrées d'index ; renvoie son résumé.
pub(crate) fn remove_hand(tx: &Tx<'_>, id: &str) -> Result<Option<HandSummary>> {
    let Some(json) = tx.text("SELECT resume FROM main WHERE id = ?", &[id.into()])? else {
        return Ok(None);
    };
    let summary: HandSummary = from_json(json.as_bytes())?;
    tx.exec(
        "DELETE FROM main WHERE id = ?; \
         DELETE FROM main_donnees WHERE id = ?; \
         DELETE FROM main_joueur WHERE main = ?; \
         DELETE FROM empreinte WHERE empreinte = ? AND main = ?",
        &[
            id.into(),
            id.into(),
            id.into(),
            summary.fingerprint.to_hex().into(),
            id.into(),
        ],
    )?;
    Ok(Some(summary))
}
