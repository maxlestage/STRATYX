//! Migrations de schéma versionnées.
//!
//! La version du schéma est stockée dans la table `meta`. À l'ouverture, les migrations
//! manquantes sont appliquées dans l'ordre, **dans une seule transaction** : en cas d'échec,
//! la base reste dans sa version précédente. Une base plus récente que le logiciel est
//! refusée ([`DatabaseError::SchemaTooNew`]). Chaque migration appliquée est journalisée
//! (table `migration`).
//!
//! Pour faire évoluer le schéma : incrémenter [`SCHEMA_VERSION`] et ajouter à `MIGRATIONS` une
//! entrée qui transforme les données de la version précédente.

use serde::{Deserialize, Serialize};

use crate::error::{DatabaseError, Result};
use crate::sql::{Tx, int_at, text_at, to_u64};
use crate::store::Database;

/// Version du schéma gérée par cette version du crate.
pub const SCHEMA_VERSION: u32 = 1;

/// Clés de la table `meta`.
pub(crate) const META_SCHEMA_VERSION: &str = "schema_version";
pub(crate) const META_NEXT_BATCH: &str = "next_batch";

/// Une étape de migration, de `to - 1` vers `to`.
pub(crate) struct Migration {
    pub to: u32,
    pub description: &'static str,
    pub apply: fn(&Tx<'_>) -> Result<()>,
}

/// Migrations connues, par version croissante.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    to: 1,
    description: "schéma initial : espaces Expresso, Tournois, Hold'em, Stratégie, Utilisateurs",
    apply: create_v1,
}];

/// Migration appliquée, telle que journalisée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedMigration {
    pub version: u32,
    pub description: String,
    pub applied_at_ms: u64,
}

/// Tables présentes avant toute migration : la version du schéma et le journal.
const BOOTSTRAP: &str = "\
CREATE TABLE IF NOT EXISTS meta (cle TEXT PRIMARY KEY, valeur BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS migration (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    appliquee_le BIGINT NOT NULL
)";

/// Schéma 1. Une main est une ligne de `main` (main enregistrée et résumé en JSON, colonnes
/// indexées pour la recherche) ; `main_joueur` indexe ses joueurs ; `empreinte` désigne la
/// main qui porte chaque empreinte (déduplication).
const SCHEMA_V1: &str = "\
CREATE TABLE main (
    id TEXT PRIMARY KEY,
    espace INTEGER NOT NULL,
    empreinte TEXT NOT NULL,
    session TEXT,
    format TEXT NOT NULL,
    validation TEXT NOT NULL,
    a_revoir INTEGER NOT NULL,
    joue_le BIGINT,
    donnees TEXT NOT NULL,
    resume TEXT NOT NULL
);
CREATE INDEX main_espace ON main (espace);
CREATE INDEX main_session ON main (session);
CREATE INDEX main_format ON main (format);
CREATE INDEX main_validation ON main (validation);
CREATE INDEX main_a_revoir ON main (a_revoir);
CREATE INDEX main_joue_le ON main (joue_le);
CREATE TABLE main_joueur (cle TEXT PRIMARY KEY, main TEXT NOT NULL, joueur TEXT NOT NULL);
CREATE INDEX main_joueur_joueur ON main_joueur (joueur);
CREATE INDEX main_joueur_main ON main_joueur (main);
CREATE TABLE empreinte (empreinte TEXT PRIMARY KEY, main TEXT NOT NULL);
CREATE TABLE session (id TEXT PRIMARY KEY, debut BIGINT NOT NULL, donnees TEXT NOT NULL);
CREATE TABLE profil (id TEXT PRIMARY KEY, donnees TEXT NOT NULL);
CREATE TABLE progression (utilisateur TEXT PRIMARY KEY, donnees TEXT NOT NULL);
CREATE TABLE strategie (cle TEXT PRIMARY KEY, entree TEXT NOT NULL, donnees TEXT NOT NULL);
CREATE TABLE ingestion (id TEXT PRIMARY KEY, rapport TEXT NOT NULL)";

fn create_v1(tx: &Tx<'_>) -> Result<()> {
    tx.run(SCHEMA_V1, &[]).map(drop)
}

/// Lit un compteur de la table `meta`.
pub(crate) fn meta(tx: &Tx<'_>, key: &str) -> Result<Option<u64>> {
    tx.int("SELECT valeur FROM meta WHERE cle = ?", &[key.into()])?
        .map(to_u64)
        .transpose()
}

/// Écrit un compteur de la table `meta`.
pub(crate) fn set_meta(tx: &Tx<'_>, key: &str, value: u64) -> Result<()> {
    tx.exec(
        "INSERT OR REPLACE INTO meta (cle, valeur) VALUES (?, ?)",
        &[key.into(), value.into()],
    )
    .map(drop)
}

/// Version actuelle du schéma (0 pour une base vierge).
pub(crate) fn current_version(tx: &Tx<'_>) -> Result<u32> {
    let version = meta(tx, META_SCHEMA_VERSION)?.unwrap_or(0);
    u32::try_from(version)
        .map_err(|_| DatabaseError::Corrupted(format!("version de schéma {version}")))
}

/// Crée les tables de départ puis applique les migrations manquantes ; renvoie la version
/// finale.
pub(crate) fn migrate(db: &Database, migrations: &[Migration]) -> Result<u32> {
    db.write(|tx| tx.run(BOOTSTRAP, &[]).map(drop))?;
    db.write(|tx| {
        let target = migrations.last().map_or(0, |m| m.to);
        let found = current_version(tx)?;
        if found > target {
            return Err(DatabaseError::SchemaTooNew {
                found,
                supported: target,
            });
        }
        let mut version = found;
        for m in migrations.iter().filter(|m| m.to > found) {
            if m.to != version + 1 {
                return Err(DatabaseError::Migration {
                    version: m.to,
                    reason: format!("migration manquante depuis la version {version}"),
                });
            }
            (m.apply)(tx).map_err(|e| DatabaseError::Migration {
                version: m.to,
                reason: e.to_string(),
            })?;
            tx.exec(
                "INSERT INTO migration (version, description, appliquee_le) VALUES (?, ?, ?)",
                &[m.to.into(), m.description.into(), crate::now_ms().into()],
            )?;
            version = m.to;
        }
        if version != found {
            set_meta(tx, META_SCHEMA_VERSION, u64::from(version))?;
        }
        Ok(version)
    })
}

/// Journal des migrations appliquées, par version croissante.
pub(crate) fn log(tx: &Tx<'_>) -> Result<Vec<AppliedMigration>> {
    tx.rows(
        "SELECT version, description, appliquee_le FROM migration ORDER BY version",
        &[],
    )?
    .iter()
    .map(|row| {
        let int = |i| -> Result<u64> { to_u64(int_at(row, i)?.unwrap_or(0)) };
        Ok(AppliedMigration {
            version: u32::try_from(int(0)?)
                .map_err(|_| DatabaseError::Corrupted("version de migration".into()))?,
            description: text_at(row, 1)?,
            applied_at_ms: int(2)?,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_notes(tx: &Tx<'_>) -> Result<()> {
        tx.run(
            "CREATE TABLE notes (cle TEXT PRIMARY KEY, texte TEXT NOT NULL); \
             INSERT INTO notes (cle, texte) VALUES ('bienvenue', 'schéma 2')",
            &[],
        )
        .map(drop)
    }

    fn failing(tx: &Tx<'_>) -> Result<()> {
        tx.run("CREATE TABLE brouillon (cle TEXT PRIMARY KEY)", &[])?;
        Err(DatabaseError::Corrupted("échec simulé".into()))
    }

    fn version(db: &Database) -> u32 {
        current_version(&db.read().unwrap()).unwrap()
    }

    #[test]
    fn migrations_are_applied_in_order_and_logged() {
        let db = Database::unmigrated().unwrap();
        db.write(|tx| tx.run(BOOTSTRAP, &[]).map(drop)).unwrap();
        assert_eq!(version(&db), 0);
        assert_eq!(migrate(&db, MIGRATIONS).unwrap(), SCHEMA_VERSION);
        // Idempotent.
        assert_eq!(migrate(&db, MIGRATIONS).unwrap(), SCHEMA_VERSION);

        let v2 = [
            Migration {
                to: 1,
                description: "initial",
                apply: create_v1,
            },
            Migration {
                to: 2,
                description: "notes",
                apply: add_notes,
            },
        ];
        assert_eq!(migrate(&db, &v2).unwrap(), 2);
        let tx = db.read().unwrap();
        assert_eq!(tx.int("SELECT count(*) FROM notes", &[]).unwrap(), Some(1));
        let log = log(&tx).unwrap();
        drop(tx);
        assert_eq!(
            log.iter().map(|m| m.version).collect::<Vec<_>>(),
            vec![1, 2]
        );

        // Le logiciel « ancien » refuse la base migrée.
        assert!(matches!(
            migrate(&db, MIGRATIONS),
            Err(DatabaseError::SchemaTooNew {
                found: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn failed_migration_keeps_previous_version() {
        let db = Database::unmigrated().unwrap();
        migrate(&db, MIGRATIONS).unwrap();
        let broken = [
            Migration {
                to: 1,
                description: "initial",
                apply: create_v1,
            },
            Migration {
                to: 2,
                description: "échoue",
                apply: failing,
            },
        ];
        assert!(matches!(
            migrate(&db, &broken),
            Err(DatabaseError::Migration { version: 2, .. })
        ));
        assert_eq!(version(&db), 1);
        // La table créée par la migration ratée n'existe pas : tout a été annulé.
        assert!(
            db.read()
                .unwrap()
                .run("SELECT * FROM brouillon", &[])
                .is_err()
        );

        let gap = [Migration {
            to: 3,
            description: "trou",
            apply: add_notes,
        }];
        assert!(matches!(
            migrate(&db, &gap),
            Err(DatabaseError::Migration { version: 3, .. })
        ));
    }
}
