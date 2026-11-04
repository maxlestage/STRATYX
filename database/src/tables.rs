//! Définition des tables redb. Les noms sont stables : les modifier exige une migration.
//!
//! | Table | Clé → valeur |
//! |---|---|
//! | `stratyx/meta` | nom → entier (version du schéma, compteurs) |
//! | `stratyx/migrations` | version → migration appliquée (JSON) |
//! | `stratyx/hand_locator` | identifiant de main → code de l'espace |
//! | `stratyx/fingerprints` | empreinte SHA-256 → identifiant de main |
//! | `stratyx/ingestions` | identifiant de lot → rapport d'ingestion (JSON) |
//! | `<espace>/hands` | identifiant → main enregistrée (JSON versionné) |
//! | `<espace>/summaries` | identifiant → résumé (JSON) |
//! | `<espace>/by_player`, `by_session`, `by_format`, `by_status` | valeur → identifiants |
//! | `<espace>/by_date` | (date en ms, identifiant) → () |
//! | `utilisateurs/sessions`, `profiles`, `progress` | identifiant → JSON |
//! | `strategie/entries`, `strategie/data` | clé → description (JSON), clé → octets |

use redb::{MultimapTableDefinition, TableDefinition};

use crate::space::Space;

pub(crate) type JsonTable = TableDefinition<'static, &'static str, &'static [u8]>;
pub(crate) type IndexTable = MultimapTableDefinition<'static, &'static str, &'static str>;
pub(crate) type DateTable = TableDefinition<'static, (u64, &'static str), ()>;

pub(crate) const META: TableDefinition<&str, u64> = TableDefinition::new("stratyx/meta");
pub(crate) const MIGRATION_LOG: TableDefinition<u32, &[u8]> =
    TableDefinition::new("stratyx/migrations");
pub(crate) const LOCATOR: TableDefinition<&str, u8> = TableDefinition::new("stratyx/hand_locator");
pub(crate) const FINGERPRINTS: TableDefinition<&[u8; 32], &str> =
    TableDefinition::new("stratyx/fingerprints");
pub(crate) const INGESTIONS: JsonTable = TableDefinition::new("stratyx/ingestions");

pub(crate) const SESSIONS: JsonTable = TableDefinition::new("utilisateurs/sessions");
pub(crate) const PROFILES: JsonTable = TableDefinition::new("utilisateurs/profiles");
pub(crate) const PROGRESS: JsonTable = TableDefinition::new("utilisateurs/progress");

pub(crate) const STRATEGY_ENTRIES: JsonTable = TableDefinition::new("strategie/entries");
pub(crate) const STRATEGY_DATA: JsonTable = TableDefinition::new("strategie/data");

/// Clés de la table `stratyx/meta`.
pub(crate) const META_SCHEMA_VERSION: &str = "schema_version";
pub(crate) const META_NEXT_BATCH: &str = "next_batch";

/// Clé d'index regroupant les mains à revoir.
pub(crate) const STATUS_TO_REVIEW: &str = "to_review";

/// Tables d'un espace de mains.
#[derive(Clone, Copy)]
pub(crate) struct HandTables {
    pub hands: JsonTable,
    pub summaries: JsonTable,
    pub by_player: IndexTable,
    pub by_session: IndexTable,
    pub by_format: IndexTable,
    pub by_status: IndexTable,
    pub by_date: DateTable,
}

macro_rules! hand_tables {
    ($prefix:literal) => {
        HandTables {
            hands: TableDefinition::new(concat!($prefix, "/hands")),
            summaries: TableDefinition::new(concat!($prefix, "/summaries")),
            by_player: MultimapTableDefinition::new(concat!($prefix, "/by_player")),
            by_session: MultimapTableDefinition::new(concat!($prefix, "/by_session")),
            by_format: MultimapTableDefinition::new(concat!($prefix, "/by_format")),
            by_status: MultimapTableDefinition::new(concat!($prefix, "/by_status")),
            by_date: TableDefinition::new(concat!($prefix, "/by_date")),
        }
    };
}

const EXPRESSO_TABLES: HandTables = hand_tables!("expresso");
const TOURNAMENT_TABLES: HandTables = hand_tables!("tournois");
const HOLDEM_TABLES: HandTables = hand_tables!("holdem");

/// Tables de mains de l'espace, s'il en contient.
pub(crate) const fn hand_tables(space: Space) -> Option<HandTables> {
    match space {
        Space::Expresso => Some(EXPRESSO_TABLES),
        Space::Tournaments => Some(TOURNAMENT_TABLES),
        Space::Holdem => Some(HOLDEM_TABLES),
        Space::Strategy | Space::Users => None,
    }
}

#[cfg(test)]
mod tests {
    use redb::TableHandle;

    use super::*;

    #[test]
    fn table_names_are_prefixed_by_space() {
        for space in Space::HAND_SPACES {
            let t = hand_tables(space).unwrap();
            assert!(t.hands.name().starts_with(space.key()));
            assert!(t.by_date.name().starts_with(space.key()));
        }
        assert!(hand_tables(Space::Users).is_none());
        assert!(SESSIONS.name().starts_with(Space::Users.key()));
        assert!(STRATEGY_DATA.name().starts_with(Space::Strategy.key()));
    }
}
