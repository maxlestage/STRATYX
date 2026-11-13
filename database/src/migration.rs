//! Migrations de schéma versionnées.
//!
//! La version du schéma est stockée dans la table `stratyx/meta`. À l'ouverture, les
//! migrations manquantes sont appliquées dans l'ordre, **dans une seule transaction** : en cas
//! d'échec, la base reste dans sa version précédente. Une base plus récente que le logiciel
//! est refusée ([`DatabaseError::SchemaTooNew`]). Chaque migration appliquée est journalisée
//! (table `stratyx/migrations`).
//!
//! Pour faire évoluer le schéma : incrémenter [`SCHEMA_VERSION`] et ajouter à `MIGRATIONS` une
//! entrée qui transforme les données de la version précédente.

use redb::{ReadableDatabase, ReadableTable, WriteTransaction};
use serde::{Deserialize, Serialize};

use crate::error::{DatabaseError, Result};
use crate::space::Space;
use crate::tables::{self, META, META_SCHEMA_VERSION, MIGRATION_LOG};

/// Version du schéma gérée par cette version du crate.
pub const SCHEMA_VERSION: u32 = 1;

/// Une étape de migration, de `to - 1` vers `to`.
pub(crate) struct Migration {
    pub to: u32,
    pub description: &'static str,
    pub apply: fn(&WriteTransaction) -> Result<()>,
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

/// Crée toutes les tables du schéma 1 (redb crée une table à sa première ouverture en
/// écriture ; les lectures ultérieures ne peuvent donc pas échouer faute de table).
fn create_v1(txn: &WriteTransaction) -> Result<()> {
    txn.open_table(META)?;
    txn.open_table(MIGRATION_LOG)?;
    txn.open_table(tables::LOCATOR)?;
    txn.open_table(tables::FINGERPRINTS)?;
    txn.open_table(tables::INGESTIONS)?;
    txn.open_table(tables::SESSIONS)?;
    txn.open_table(tables::PROFILES)?;
    txn.open_table(tables::PROGRESS)?;
    txn.open_table(tables::STRATEGY_ENTRIES)?;
    txn.open_table(tables::STRATEGY_DATA)?;
    for space in Space::HAND_SPACES {
        let t = tables::hand_tables(space).expect("espace de mains");
        txn.open_table(t.hands)?;
        txn.open_table(t.summaries)?;
        txn.open_multimap_table(t.by_player)?;
        txn.open_multimap_table(t.by_session)?;
        txn.open_multimap_table(t.by_format)?;
        txn.open_multimap_table(t.by_status)?;
        txn.open_table(t.by_date)?;
    }
    Ok(())
}

/// Version actuelle du schéma (0 pour une base vierge).
pub(crate) fn current_version(db: &redb::Database) -> Result<u32> {
    let txn = db.begin_read()?;
    let table = match txn.open_table(META) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let version = table.get(META_SCHEMA_VERSION)?.map_or(0, |v| v.value());
    u32::try_from(version)
        .map_err(|_| DatabaseError::Corrupted(format!("version de schéma {version}")))
}

/// Applique les migrations manquantes et renvoie la version finale.
pub(crate) fn migrate(db: &redb::Database, migrations: &[Migration]) -> Result<u32> {
    let target = migrations.last().map_or(0, |m| m.to);
    let found = current_version(db)?;
    if found > target {
        return Err(DatabaseError::SchemaTooNew {
            found,
            supported: target,
        });
    }
    if found == target {
        return Ok(found);
    }
    let txn = db.begin_write()?;
    let mut version = found;
    for m in migrations.iter().filter(|m| m.to > found) {
        if m.to != version + 1 {
            return Err(DatabaseError::Migration {
                version: m.to,
                reason: format!("migration manquante depuis la version {version}"),
            });
        }
        (m.apply)(&txn).map_err(|e| DatabaseError::Migration {
            version: m.to,
            reason: e.to_string(),
        })?;
        let log = AppliedMigration {
            version: m.to,
            description: m.description.to_string(),
            applied_at_ms: crate::now_ms(),
        };
        txn.open_table(MIGRATION_LOG)?
            .insert(m.to, serde_json::to_vec(&log)?.as_slice())?;
        version = m.to;
    }
    txn.open_table(META)?
        .insert(META_SCHEMA_VERSION, u64::from(version))?;
    txn.commit()?;
    Ok(version)
}

/// Journal des migrations appliquées, par version croissante.
pub(crate) fn log(db: &redb::Database) -> Result<Vec<AppliedMigration>> {
    let txn = db.begin_read()?;
    let table = txn.open_table(MIGRATION_LOG)?;
    let mut out = Vec::new();
    for entry in table.iter()? {
        let (_, v) = entry?;
        out.push(serde_json::from_slice(v.value())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use redb::backends::InMemoryBackend;
    use redb::{ReadableTableMetadata, TableDefinition};

    use super::*;

    const NOTES: TableDefinition<&str, &str> = TableDefinition::new("test/notes");

    fn add_notes(txn: &WriteTransaction) -> Result<()> {
        txn.open_table(NOTES)?.insert("bienvenue", "schéma 2")?;
        Ok(())
    }

    fn failing(_: &WriteTransaction) -> Result<()> {
        Err(DatabaseError::Corrupted("échec simulé".into()))
    }

    fn memory_db() -> redb::Database {
        redb::Database::builder()
            .create_with_backend(InMemoryBackend::new())
            .unwrap()
    }

    #[test]
    fn migrations_are_applied_in_order_and_logged() {
        let db = memory_db();
        assert_eq!(current_version(&db).unwrap(), 0);
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
        let txn = db.begin_read().unwrap();
        assert_eq!(txn.open_table(NOTES).unwrap().len().unwrap(), 1);
        drop(txn);
        let log = log(&db).unwrap();
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
        let db = memory_db();
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
        assert_eq!(current_version(&db).unwrap(), 1);

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
