//! Import d'une ancienne base redb (versions de Stratyx antérieures à Nuage DB, décision T3).
//!
//! L'ancienne base était un fichier unique ; la nouvelle est un dossier Nuage DB. À
//! l'ouverture d'un chemin qui désigne un fichier, [`open_and_import`] le renomme
//! `<nom>.ancienne-redb`, crée la base Nuage à sa place et y recopie tout, dans une seule
//! transaction : mains (avec leur date d'enregistrement, leur lot et leur statut), empreintes,
//! sessions, profils, progression, stratégie, rapports d'ingestion et compteur de lots. En cas
//! d'échec, le dossier créé est supprimé et le fichier reprend son nom : rien n'est perdu.
//!
//! Tables de l'ancienne base (noms stables) :
//!
//! | Table | Clé → valeur |
//! |---|---|
//! | `stratyx/meta` | nom → entier (version du schéma, compteurs) |
//! | `stratyx/fingerprints` | empreinte SHA-256 → identifiant de main |
//! | `stratyx/ingestions` | identifiant de lot → rapport d'ingestion (JSON) |
//! | `<espace>/hands` | identifiant → main enregistrée (JSON versionné) |
//! | `<espace>/summaries` | identifiant → résumé (JSON) |
//! | `utilisateurs/sessions`, `profiles`, `progress` | identifiant → JSON |
//! | `strategie/entries`, `strategie/data` | clé → description (JSON), clé → octets |

use std::path::{Path, PathBuf};

/// Nom de l'ancienne base une fois importée.
#[cfg_attr(not(feature = "import-redb"), allow(dead_code))]
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".ancienne-redb");
    PathBuf::from(name)
}

#[cfg(not(feature = "import-redb"))]
pub(crate) fn open_and_import(path: &Path) -> crate::Result<crate::Database> {
    Err(crate::DatabaseError::Storage(format!(
        "{} est une ancienne base redb : l'importer demande la fonctionnalité `import-redb`",
        path.display()
    )))
}

#[cfg(feature = "import-redb")]
pub(crate) use import::open_and_import;

#[cfg(feature = "import-redb")]
mod import {
    use std::path::Path;

    use redb::{ReadableDatabase, ReadableTable, TableDefinition};
    use serde_json::Value as Json;

    use super::backup_path;
    use crate::error::{DatabaseError, Result};
    use crate::migration::{META_NEXT_BATCH, set_meta};
    use crate::space::Space;
    use crate::sql::Tx;
    use crate::status::HandSummary;
    use crate::store::{Database, decode_hand, from_json, insert_hand_row};

    type JsonTable = TableDefinition<'static, &'static str, &'static [u8]>;

    const META: TableDefinition<&str, u64> = TableDefinition::new("stratyx/meta");
    const FINGERPRINTS: TableDefinition<&[u8; 32], &str> =
        TableDefinition::new("stratyx/fingerprints");
    const INGESTIONS: JsonTable = TableDefinition::new("stratyx/ingestions");
    const SESSIONS: JsonTable = TableDefinition::new("utilisateurs/sessions");
    const PROFILES: JsonTable = TableDefinition::new("utilisateurs/profiles");
    const PROGRESS: JsonTable = TableDefinition::new("utilisateurs/progress");
    const STRATEGY_ENTRIES: JsonTable = TableDefinition::new("strategie/entries");
    const STRATEGY_DATA: JsonTable = TableDefinition::new("strategie/data");

    /// Tables `hands` et `summaries` d'un espace de mains.
    fn hand_tables(space: Space) -> (JsonTable, JsonTable) {
        match space {
            Space::Expresso => (
                TableDefinition::new("expresso/hands"),
                TableDefinition::new("expresso/summaries"),
            ),
            Space::Tournaments => (
                TableDefinition::new("tournois/hands"),
                TableDefinition::new("tournois/summaries"),
            ),
            _ => (
                TableDefinition::new("holdem/hands"),
                TableDefinition::new("holdem/summaries"),
            ),
        }
    }

    fn legacy(e: impl std::fmt::Display) -> DatabaseError {
        DatabaseError::Storage(format!("ancienne base redb : {e}"))
    }

    /// Voir le module : importe l'ancienne base `path` dans une base Nuage créée à sa place.
    pub(crate) fn open_and_import(path: &Path) -> Result<Database> {
        let backup = backup_path(path);
        if backup.exists() {
            return Err(DatabaseError::Storage(format!(
                "{} est un fichier et {} existe déjà : déplacez l'un des deux",
                path.display(),
                backup.display()
            )));
        }
        // Lecture d'abord : un fichier illisible n'est ni renommé ni remplacé.
        let old = redb::Database::open(path).map_err(legacy)?;
        std::fs::rename(path, &backup)?;
        let imported = Database::open_dir(path, nuage::EngineOptions::default())
            .and_then(|db| db.write(|tx| copy(&old, tx)).map(|()| db));
        match imported {
            Ok(db) => Ok(db),
            Err(e) => {
                let _ = std::fs::remove_dir_all(path);
                drop(old);
                let _ = std::fs::rename(&backup, path);
                Err(e)
            }
        }
    }

    /// Lit toutes les entrées d'une table JSON (absente : aucune).
    fn entries(txn: &redb::ReadTransaction, def: JsonTable) -> Result<Vec<(String, Vec<u8>)>> {
        let table = match txn.open_table(def) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(legacy(e)),
        };
        let mut out = Vec::new();
        for entry in table.iter().map_err(legacy)? {
            let (k, v) = entry.map_err(legacy)?;
            out.push((k.value().to_string(), v.value().to_vec()));
        }
        Ok(out)
    }

    fn insert(tx: &Tx<'_>, sql: &str, params: &[Json]) -> Result<()> {
        tx.exec(sql, params).map(drop)
    }

    /// Recopie tout le contenu de l'ancienne base dans la transaction Nuage.
    fn copy(old: &redb::Database, tx: &Tx<'_>) -> Result<()> {
        let txn = old.begin_read().map_err(legacy)?;
        // Empreintes d'abord : la main qui portait chaque empreinte la garde.
        if let Ok(table) = txn.open_table(FINGERPRINTS) {
            for entry in table.iter().map_err(legacy)? {
                let (fp, id) = entry.map_err(legacy)?;
                insert(
                    tx,
                    "INSERT INTO empreinte (empreinte, main) VALUES (?, ?)",
                    &[crate::hex::encode(fp.value()).into(), id.value().into()],
                )?;
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for space in Space::HAND_SPACES {
            let (hands, summaries) = hand_tables(space);
            let summaries: std::collections::BTreeMap<String, Vec<u8>> =
                entries(&txn, summaries)?.into_iter().collect();
            for (id, bytes) in entries(&txn, hands)? {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let stored = decode_hand(&id, &bytes)?;
                let summary: HandSummary = match summaries.get(&id) {
                    Some(s) => from_json(s)?,
                    None => crate::status::analyze(
                        &stored.record,
                        stored.space,
                        stratyx_shared::Confidence::DEFAULT_REVIEW_THRESHOLD,
                    ),
                };
                insert_hand_row(tx, &stored, &summary)?;
            }
        }
        for (id, json) in entries(&txn, SESSIONS)? {
            let session: stratyx_poker::Session = from_json(&json)?;
            insert(
                tx,
                "INSERT INTO session (id, debut, donnees) VALUES (?, ?, ?)",
                &[id.into(), session.started_at_ms.into(), utf8(json)?.into()],
            )?;
        }
        for (id, json) in entries(&txn, PROFILES)? {
            insert(
                tx,
                "INSERT INTO profil (id, donnees) VALUES (?, ?)",
                &[id.into(), utf8(json)?.into()],
            )?;
        }
        for (id, json) in entries(&txn, PROGRESS)? {
            insert(
                tx,
                "INSERT INTO progression (utilisateur, donnees) VALUES (?, ?)",
                &[id.into(), utf8(json)?.into()],
            )?;
        }
        let data: std::collections::BTreeMap<String, Vec<u8>> =
            entries(&txn, STRATEGY_DATA)?.into_iter().collect();
        for (key, json) in entries(&txn, STRATEGY_ENTRIES)? {
            let bytes = data.get(&key).ok_or_else(|| {
                DatabaseError::Corrupted(format!("données de « {key} » absentes"))
            })?;
            insert(
                tx,
                "INSERT INTO strategie (cle, entree, donnees) VALUES (?, ?, ?)",
                &[
                    key.as_str().into(),
                    utf8(json)?.into(),
                    crate::hex::encode(bytes).into(),
                ],
            )?;
        }
        for (id, json) in entries(&txn, INGESTIONS)? {
            insert(
                tx,
                "INSERT INTO ingestion (id, rapport) VALUES (?, ?)",
                &[id.into(), utf8(json)?.into()],
            )?;
        }
        if let Ok(meta) = txn.open_table(META)
            && let Some(n) = meta.get("next_batch").map_err(legacy)?
        {
            set_meta(tx, META_NEXT_BATCH, n.value())?;
        }
        Ok(())
    }

    fn utf8(bytes: Vec<u8>) -> Result<String> {
        String::from_utf8(bytes).map_err(|e| DatabaseError::Corrupted(e.to_string()))
    }
}

#[cfg(all(test, feature = "import-redb"))]
mod tests {
    use std::path::PathBuf;

    use redb::TableDefinition;
    use stratyx_poker::{HandRecord, Session, SessionSource};
    use stratyx_shared::Confidence;

    use super::backup_path;
    use crate::status::analyze;
    use crate::store::{HAND_FORMAT_VERSION, StoredHand, to_json};
    use crate::text::{ImportOptions, import_pokerstars};
    use crate::{
        Database, HandQuery, IngestReport, ProgressEntry, Space, StrategyBlob, StrategyEntry,
        UserProfile,
    };

    const CASH: &str = include_str!("../tests/data/pokerstars_cash_nl6.txt");
    const TOURNOI: &str = include_str!("../tests/data/pokerstars_tournament_antes.txt");

    type Json = TableDefinition<'static, &'static str, &'static [u8]>;

    /// Un chemin temporaire (fichier redb, puis dossier Nuage et copie de secours), nettoyé.
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Temp {
            let p = std::env::temp_dir().join(format!(
                "stratyx-import-{tag}-{}-{}",
                std::process::id(),
                crate::now_ms()
            ));
            let t = Temp(p);
            t.clean();
            t
        }

        fn clean(&self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(backup_path(&self.0));
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            self.clean();
        }
    }

    fn hands() -> Vec<HandRecord> {
        let mut all: Vec<HandRecord> = import_pokerstars(CASH, &ImportOptions::default())
            .into_iter()
            .chain(import_pokerstars(TOURNOI, &ImportOptions::default()))
            .map(|h| h.unwrap().record)
            .collect();
        all.truncate(6);
        all
    }

    fn put(txn: &redb::WriteTransaction, def: Json, key: &str, json: &str) {
        txn.open_table(def)
            .unwrap()
            .insert(key, json.as_bytes())
            .unwrap();
    }

    /// Écrit une base au format des versions précédentes (redb), comme elles l'écrivaient.
    fn write_legacy(path: &std::path::Path, hands: &[HandRecord]) -> (Session, UserProfile) {
        let db = redb::Database::create(path).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut meta = txn
                .open_table(TableDefinition::<&str, u64>::new("stratyx/meta"))
                .unwrap();
            meta.insert("schema_version", 1).unwrap();
            meta.insert("next_batch", 3).unwrap();
        }
        for (i, record) in hands.iter().enumerate() {
            let space = Space::for_format(record.format);
            let summary = analyze(record, space, Confidence::DEFAULT_REVIEW_THRESHOLD);
            let stored = StoredHand {
                format_version: HAND_FORMAT_VERSION,
                space,
                fingerprint: summary.fingerprint,
                status: summary.status.clone(),
                batch_id: (i == 0).then(|| "lot-000002".to_string()),
                stored_at_ms: 1_000 + i as u64,
                record: record.clone(),
            };
            let prefix = match space {
                Space::Expresso => "expresso",
                Space::Tournaments => "tournois",
                _ => "holdem",
            };
            let hands_table: Json = TableDefinition::new(match prefix {
                "expresso" => "expresso/hands",
                "tournois" => "tournois/hands",
                _ => "holdem/hands",
            });
            let summaries: Json = TableDefinition::new(match prefix {
                "expresso" => "expresso/summaries",
                "tournois" => "tournois/summaries",
                _ => "holdem/summaries",
            });
            put(&txn, hands_table, &record.id, &to_json(&stored).unwrap());
            put(&txn, summaries, &record.id, &to_json(&summary).unwrap());
            txn.open_table(TableDefinition::<&[u8; 32], &str>::new(
                "stratyx/fingerprints",
            ))
            .unwrap()
            .insert(summary.fingerprint.as_bytes(), record.id.as_str())
            .unwrap();
        }
        let session = Session {
            id: "s1".into(),
            source: SessionSource::TextImport,
            format: hands[0].format,
            started_at_ms: 5_000,
            ended_at_ms: Some(6_000),
            hand_ids: hands.iter().take(2).map(|h| h.id.clone()).collect(),
            tournament: None,
        };
        put(
            &txn,
            TableDefinition::new("utilisateurs/sessions"),
            "s1",
            &to_json(&session).unwrap(),
        );
        let profile = UserProfile::new("u1", "Max", 7);
        put(
            &txn,
            TableDefinition::new("utilisateurs/profiles"),
            "u1",
            &to_json(&profile).unwrap(),
        );
        let progress = vec![ProgressEntry {
            at_ms: 9,
            topic: "icm".into(),
            metric: "score".into(),
            value: 0.75,
            details: None,
        }];
        put(
            &txn,
            TableDefinition::new("utilisateurs/progress"),
            "u1",
            &to_json(&progress).unwrap(),
        );
        let data = b"\x00\x01range AKs".to_vec();
        let entry = StrategyEntry {
            key: "charts/test".into(),
            content_type: "application/octet-stream".into(),
            description: None,
            size: data.len() as u64,
            sha256: crate::hex::encode(&<sha2::Sha256 as sha2::Digest>::digest(&data)),
            updated_at_ms: 11,
        };
        put(
            &txn,
            TableDefinition::new("strategie/entries"),
            "charts/test",
            &to_json(&entry).unwrap(),
        );
        txn.open_table(TableDefinition::<&str, &[u8]>::new("strategie/data"))
            .unwrap()
            .insert("charts/test", data.as_slice())
            .unwrap();
        let report = serde_json::json!({
            "batch_id": "lot-000002",
            "provenance": {"source": "test", "license": "CC0"},
            "policy": {"accept_unresolved": true, "accept_incomplete": false, "accept_needing_review": false},
            "ingested_at_ms": 12,
            "accepted": [], "duplicates": [], "rejected": []
        });
        put(
            &txn,
            TableDefinition::new("stratyx/ingestions"),
            "lot-000002",
            &report.to_string(),
        );
        txn.commit().unwrap();
        (session, profile)
    }

    #[test]
    fn an_old_redb_database_is_imported_at_first_opening() {
        let path = Temp::new("ok");
        let hands = hands();
        assert!(hands.len() >= 3);
        let (session, profile) = write_legacy(&path.0, &hands);

        let db = Database::open(&path.0).unwrap();
        assert!(path.0.is_dir(), "la base Nuage remplace le fichier");
        assert!(backup_path(&path.0).is_file(), "l'ancienne base est gardée");
        let check = |db: &Database| {
            assert_eq!(db.count_hands(None).unwrap(), hands.len() as u64);
            for (i, record) in hands.iter().enumerate() {
                let stored = db.get_hand(&record.id).unwrap().unwrap();
                assert_eq!(stored.record, *record);
                assert_eq!(stored.stored_at_ms, 1_000 + i as u64);
                assert_eq!(
                    db.find_by_fingerprint(&stored.fingerprint).unwrap(),
                    Some(record.id.clone())
                );
                let found = db.search(&HandQuery::new().player(record.seats[0].player.clone()));
                assert!(found.unwrap().items.iter().any(|s| s.id == record.id));
            }
            assert_eq!(
                db.get_hand(&hands[0].id)
                    .unwrap()
                    .unwrap()
                    .batch_id
                    .as_deref(),
                Some("lot-000002")
            );
            assert_eq!(db.get_session("s1").unwrap(), Some(session.clone()));
            assert_eq!(db.get_user("u1").unwrap(), Some(profile.clone()));
            assert_eq!(db.progress("u1").unwrap().len(), 1);
            let blob: StrategyBlob = db.get_strategy("charts/test").unwrap().unwrap();
            assert_eq!(blob.data, b"\x00\x01range AKs");
            let reports: Vec<IngestReport> = db.ingestion_reports().unwrap();
            assert_eq!(reports[0].batch_id, "lot-000002");
        };
        check(&db);
        assert_eq!(db.ingestion_reports().unwrap().len(), 1);
        // Le compteur de lots continue : pas deux lots du même nom.
        let report = db
            .ingest(
                crate::ArchiveBatch::new(crate::Provenance::new("x", "CC0").unwrap(), Vec::new()),
                crate::IngestPolicy::default(),
            )
            .unwrap();
        assert_eq!(report.batch_id, "lot-000003");
        drop(db);

        // Réouverture : c'est désormais un dossier Nuage, rien n'est réimporté.
        let db = Database::open(&path.0).unwrap();
        check(&db);
        assert_eq!(db.ingestion_reports().unwrap().len(), 2);
        // Une session importée s'efface avec ses mains.
        assert_eq!(db.delete_session("s1").unwrap().hands_deleted, 2);
    }

    #[test]
    fn an_unreadable_file_is_left_untouched() {
        let path = Temp::new("illisible");
        std::fs::write(&path.0, b"ceci n'est pas une base redb").unwrap();
        assert!(Database::open(&path.0).is_err());
        assert_eq!(
            std::fs::read(&path.0).unwrap(),
            b"ceci n'est pas une base redb"
        );
        assert!(!backup_path(&path.0).exists());
    }
}
