//! Stockage Nuage DB de la base locale (chantier Nuage, décision T14), en construction derrière
//! la fonctionnalité `nuage` : redb reste le stockage par défaut tant que toutes les opérations
//! de [`crate::Database`] n'ont pas leur équivalent ici, testé contre les deux stockages.
//!
//! Le moteur est embarqué (Rust pur, sans réseau) ; les tables sont en SQL, les valeurs passées
//! en paramètres, jamais dans le texte des requêtes.
//!
//! | Table | Contenu | redb équivalent |
//! |---|---|---|
//! | `meta` | nom → entier (version du schéma) | `stratyx/meta` |
//! | `main` | une ligne par main : espace, empreinte, date de jeu, main enregistrée et résumé (JSON) | `<espace>/hands`, `<espace>/summaries`, `stratyx/hand_locator`, `stratyx/fingerprints` |
//!
//! Fait : version du schéma, mains (écriture, lecture, résumé, empreinte, suppression,
//! comptage). Reste : index et recherche, utilisateurs, stratégie, ingestion, RGPD,
//! migrations, revalidation.

use std::path::Path;
use std::sync::Mutex;

use nuage::Embedded;
use serde_json::Value as Json;
use stratyx_poker::HandRecord;
use stratyx_shared::Confidence;

use crate::error::{DatabaseError, Result};
use crate::fingerprint::Fingerprint;
use crate::space::Space;
use crate::status::{HandSummary, analyze};
use crate::store::{HAND_FORMAT_VERSION, PutOutcome, StoredHand, decode_hand, from_json};
use crate::{SCHEMA_VERSION, now_ms};

/// Schéma : créé à l'ouverture s'il manque (une seule transaction).
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS meta (cle TEXT PRIMARY KEY, valeur BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS main (
    id TEXT PRIMARY KEY,
    espace INTEGER NOT NULL,
    empreinte TEXT NOT NULL,
    joue_le BIGINT,
    enregistre_le BIGINT NOT NULL,
    donnees TEXT NOT NULL,
    resume TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS main_espace ON main (espace);
CREATE INDEX IF NOT EXISTS main_empreinte ON main (empreinte)";

/// Une base locale ouverte dans Nuage DB.
pub(crate) struct NuageStore {
    db: Embedded,
    review_threshold: Confidence,
    /// Sérialise les écritures : une écriture lit (doublon, main existante) puis écrit, et
    /// les deux doivent voir le même état.
    writes: Mutex<()>,
}

fn nuage_err(e: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::Nuage(e.to_string())
}

impl NuageStore {
    /// Ouvre (ou crée) la base dans le dossier `dir` et y inscrit la version du schéma.
    pub(crate) fn open(dir: &Path) -> Result<NuageStore> {
        let db = Embedded::open(dir).map_err(nuage_err)?;
        let store = NuageStore {
            db,
            review_threshold: Confidence::DEFAULT_REVIEW_THRESHOLD,
            writes: Mutex::new(()),
        };
        store.sql(SCHEMA, &[])?;
        store.sql(
            "INSERT INTO meta (cle, valeur) VALUES ('schema_version', ?) \
             ON CONFLICT (cle) DO NOTHING",
            &[SCHEMA_VERSION.into()],
        )?;
        Ok(store)
    }

    /// La version du schéma inscrite dans la base.
    pub(crate) fn schema_version(&self) -> Result<u32> {
        let value = self
            .first_i64("SELECT valeur FROM meta WHERE cle = 'schema_version'", &[])?
            .ok_or_else(|| DatabaseError::Corrupted("version du schéma absente".into()))?;
        u32::try_from(value)
            .map_err(|_| DatabaseError::Corrupted(format!("version du schéma {value}")))
    }

    /// Comme [`crate::Database::put_hand`].
    pub(crate) fn put_hand(&self, record: &HandRecord) -> Result<PutOutcome> {
        if record.id.trim().is_empty() {
            return Err(DatabaseError::EmptyId("main"));
        }
        let id = record.id.as_str();
        let space = Space::for_format(record.format);
        let summary = analyze(record, space, self.review_threshold);
        let fp = summary.fingerprint;
        let _guard = self.writes.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(existing_id) = self.first_text(
            "SELECT id FROM main WHERE empreinte = ? AND id <> ? \
             ORDER BY enregistre_le, id LIMIT 1",
            &[fp.to_hex().into(), id.into()],
        )? {
            return Ok(PutOutcome::Duplicate {
                existing_id,
                fingerprint: fp,
            });
        }
        let existed = self
            .first_i64("SELECT count(*) FROM main WHERE id = ?", &[id.into()])?
            .unwrap_or(0)
            > 0;
        let stored = StoredHand {
            format_version: HAND_FORMAT_VERSION,
            space,
            fingerprint: fp,
            status: summary.status.clone(),
            batch_id: None,
            stored_at_ms: now_ms(),
            record: record.clone(),
        };
        self.sql(
            "INSERT OR REPLACE INTO main \
             (id, espace, empreinte, joue_le, enregistre_le, donnees, resume) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                id.into(),
                space.code().into(),
                fp.to_hex().into(),
                summary.played_at_ms.into(),
                stored.stored_at_ms.into(),
                serde_json::to_string(&stored)?.into(),
                serde_json::to_string(&summary)?.into(),
            ],
        )?;
        Ok(if existed {
            PutOutcome::Updated(summary)
        } else {
            PutOutcome::Inserted(summary)
        })
    }

    /// Comme [`crate::Database::get_hand`].
    pub(crate) fn get_hand(&self, id: &str) -> Result<Option<StoredHand>> {
        self.first_text("SELECT donnees FROM main WHERE id = ?", &[id.into()])?
            .map(|json| decode_hand(id, json.as_bytes()))
            .transpose()
    }

    /// Comme [`crate::Database::hand_summary`].
    pub(crate) fn hand_summary(&self, id: &str) -> Result<Option<HandSummary>> {
        self.first_text("SELECT resume FROM main WHERE id = ?", &[id.into()])?
            .map(|json| from_json(json.as_bytes()))
            .transpose()
    }

    /// Comme [`crate::Database::find_by_fingerprint`] : la première main enregistrée.
    pub(crate) fn find_by_fingerprint(&self, fingerprint: &Fingerprint) -> Result<Option<String>> {
        self.first_text(
            "SELECT id FROM main WHERE empreinte = ? ORDER BY enregistre_le, id LIMIT 1",
            &[fingerprint.to_hex().into()],
        )
    }

    /// Comme [`crate::Database::delete_hand`].
    pub(crate) fn delete_hand(&self, id: &str) -> Result<bool> {
        let _guard = self.writes.lock().unwrap_or_else(|e| e.into_inner());
        let res = self.sql("DELETE FROM main WHERE id = ?", &[id.into()])?;
        Ok(res.first().is_some_and(|r| r.rows_affected > 0))
    }

    /// Comme [`crate::Database::count_hands`].
    pub(crate) fn count_hands(&self, space: Option<Space>) -> Result<u64> {
        let n = match space {
            None => self.first_i64("SELECT count(*) FROM main", &[])?,
            Some(s) => self.first_i64(
                "SELECT count(*) FROM main WHERE espace = ?",
                &[s.code().into()],
            )?,
        };
        Ok(u64::try_from(n.unwrap_or(0)).unwrap_or(0))
    }

    /// Point de contrôle puis fermeture du moteur, pour que tout soit sur disque.
    pub(crate) fn close(self) -> Result<()> {
        let engine = self.db.engine();
        engine.checkpoint().map_err(nuage_err)?;
        engine.close().map_err(nuage_err)
    }

    fn sql(&self, sql: &str, params: &[Json]) -> Result<Vec<nuage::sql::SqlResult>> {
        self.db.sql(sql, params).map_err(nuage_err)
    }

    /// Première colonne de la première ligne, en entier.
    fn first_i64(&self, sql: &str, params: &[Json]) -> Result<Option<i64>> {
        let res = self.sql(sql, params)?;
        Ok(res
            .first()
            .and_then(|r| r.rows.first())
            .and_then(|row| row.first())
            .and_then(|v| v.as_i64()))
    }

    /// Première colonne de la première ligne, en texte.
    fn first_text(&self, sql: &str, params: &[Json]) -> Result<Option<String>> {
        let res = self.sql(sql, params)?;
        Ok(res
            .first()
            .and_then(|r| r.rows.first())
            .and_then(|row| row.first())
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::Database;
    use crate::text::{ImportOptions, import_pokerstars};

    const CASH: &str = include_str!("../tests/data/pokerstars_cash_nl6.txt");
    const TOURNOI: &str = include_str!("../tests/data/pokerstars_tournament_antes.txt");

    /// Un dossier temporaire propre au test, supprimé à la fin.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("stratyx-nuage-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hands(text: &str) -> Vec<HandRecord> {
        import_pokerstars(text, &ImportOptions::default())
            .into_iter()
            .map(|h| h.unwrap().record)
            .collect()
    }

    /// Les deux stockages, côte à côte : chaque opération doit y donner le même résultat.
    struct Both {
        redb: Database,
        nuage: NuageStore,
    }

    impl Both {
        fn put(&self, record: &HandRecord) -> PutOutcome {
            let a = self.redb.put_hand(record).unwrap();
            let b = self.nuage.put_hand(record).unwrap();
            assert_eq!(a, b, "put_hand « {} »", record.id);
            a
        }

        fn check(&self, record: &HandRecord) {
            let id = record.id.as_str();
            let (a, b) = (
                self.redb.get_hand(id).unwrap(),
                self.nuage.get_hand(id).unwrap(),
            );
            // La date d'enregistrement diffère d'un stockage à l'autre ; le reste est identique.
            assert_eq!(
                a.map(|h| StoredHand {
                    stored_at_ms: 0,
                    ..h
                }),
                b.map(|h| StoredHand {
                    stored_at_ms: 0,
                    ..h
                }),
                "get_hand « {id} »"
            );
            assert_eq!(
                self.redb.hand_summary(id).unwrap(),
                self.nuage.hand_summary(id).unwrap(),
                "hand_summary « {id} »"
            );
        }

        fn counts(&self) {
            for space in [
                None,
                Some(Space::Expresso),
                Some(Space::Tournaments),
                Some(Space::Holdem),
            ] {
                assert_eq!(
                    self.redb.count_hands(space).unwrap(),
                    self.nuage.count_hands(space).unwrap(),
                    "count_hands({space:?})"
                );
            }
        }
    }

    #[test]
    fn la_version_du_schema_survit_a_la_reouverture() {
        let dir = TempDir::new("schema");
        let store = NuageStore::open(&dir.0).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        store.close().unwrap();
        let store = NuageStore::open(&dir.0).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        store.close().unwrap();
    }

    #[test]
    fn les_mains_se_comportent_comme_avec_redb() {
        let dir = TempDir::new("mains");
        let both = Both {
            redb: Database::in_memory().unwrap(),
            nuage: NuageStore::open(&dir.0).unwrap(),
        };
        let mut all = hands(CASH);
        all.extend(hands(TOURNOI));
        assert!(all.len() > 2, "historiques d'exemple trop courts");

        // Insertion, puis lecture, résumé, empreinte et comptages identiques.
        for record in &all {
            assert!(matches!(both.put(record), PutOutcome::Inserted(_)));
        }
        for record in &all {
            both.check(record);
            let fp = both
                .redb
                .hand_summary(&record.id)
                .unwrap()
                .unwrap()
                .fingerprint;
            let found = both.redb.find_by_fingerprint(&fp).unwrap();
            assert_eq!(found.as_deref(), Some(record.id.as_str()));
            assert_eq!(found, both.nuage.find_by_fingerprint(&fp).unwrap());
        }
        both.counts();

        // Même identifiant : remplacée. Même contenu sous un autre identifiant : doublon.
        assert!(matches!(both.put(&all[0]), PutOutcome::Updated(_)));
        let mut copy = all[0].clone();
        copy.id = format!("{}-copie", copy.id);
        assert!(matches!(both.put(&copy), PutOutcome::Duplicate { .. }));
        both.check(&copy);

        // Identifiant vide refusé par les deux.
        let mut empty = all[1].clone();
        empty.id = "  ".into();
        assert!(matches!(
            both.redb.put_hand(&empty),
            Err(DatabaseError::EmptyId(_))
        ));
        assert!(matches!(
            both.nuage.put_hand(&empty),
            Err(DatabaseError::EmptyId(_))
        ));

        // Suppression : une fois, puis plus rien.
        let id = all[1].id.as_str();
        assert!(both.redb.delete_hand(id).unwrap());
        assert!(both.nuage.delete_hand(id).unwrap());
        assert!(!both.redb.delete_hand(id).unwrap());
        assert!(!both.nuage.delete_hand(id).unwrap());
        both.check(&all[1]);
        both.counts();
        let fp = both
            .redb
            .hand_summary(&all[2].id)
            .unwrap()
            .unwrap()
            .fingerprint;
        assert_eq!(
            both.redb.find_by_fingerprint(&fp).unwrap(),
            both.nuage.find_by_fingerprint(&fp).unwrap()
        );
    }

    #[test]
    fn les_mains_survivent_a_la_reouverture() {
        let dir = TempDir::new("reouverture");
        let all = hands(CASH);
        let store = NuageStore::open(&dir.0).unwrap();
        for record in &all {
            store.put_hand(record).unwrap();
        }
        store.close().unwrap();
        let store = NuageStore::open(&dir.0).unwrap();
        assert_eq!(store.count_hands(None).unwrap(), all.len() as u64);
        for record in &all {
            assert_eq!(store.get_hand(&record.id).unwrap().unwrap().record, *record);
        }
        store.close().unwrap();
    }
}
