//! Stockage Nuage DB de la base locale (chantier Nuage, décision T14), en construction derrière
//! la fonctionnalité `nuage` : redb reste le stockage par défaut tant que toutes les opérations
//! de [`crate::Database`] n'ont pas leur équivalent ici, testé contre les deux stockages.
//!
//! Première pièce : ouverture du moteur embarqué (Rust pur, sans réseau) et version du schéma,
//! gardée dans une table `meta` comme la table `META` de redb.

use std::path::Path;

use nuage::Embedded;

use crate::SCHEMA_VERSION;

/// Schéma minimal : la version du schéma (une ligne par clé).
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS meta (cle TEXT PRIMARY KEY, valeur BIGINT NOT NULL)";

/// Une base locale ouverte dans Nuage DB.
pub(crate) struct NuageStore {
    db: Embedded,
}

/// Les erreurs de Nuage, en texte : le type d'erreur public de la base n'en dépend pas.
pub(crate) type Result<T> = std::result::Result<T, String>;

impl NuageStore {
    /// Ouvre (ou crée) la base dans le dossier `dir` et y inscrit la version du schéma.
    pub(crate) fn open(dir: &Path) -> Result<NuageStore> {
        let db = Embedded::open(dir).map_err(|e| e.to_string())?;
        let store = NuageStore { db };
        store.exec(SCHEMA, &[])?;
        store.exec(
            "INSERT INTO meta (cle, valeur) VALUES ('schema_version', $1) \
             ON CONFLICT (cle) DO NOTHING",
            &[SCHEMA_VERSION.into()],
        )?;
        Ok(store)
    }

    /// La version du schéma inscrite dans la base.
    pub(crate) fn schema_version(&self) -> Result<u32> {
        let res = self
            .db
            .sql("SELECT valeur FROM meta WHERE cle = 'schema_version'", &[])
            .map_err(|e| e.to_string())?;
        let value = res
            .first()
            .and_then(|r| r.rows.first())
            .and_then(|row| row.first())
            .and_then(|v| v.as_i64())
            .ok_or("version du schéma absente")?;
        u32::try_from(value).map_err(|_| format!("version du schéma invalide : {value}"))
    }

    /// Point de contrôle puis fermeture du moteur, pour que tout soit sur disque.
    pub(crate) fn close(self) -> Result<()> {
        let engine = self.db.engine();
        engine.checkpoint().map_err(|e| e.to_string())?;
        engine.close().map_err(|e| e.to_string())
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<()> {
        self.db
            .sql(sql, params)
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_version_du_schema_survit_a_la_reouverture() {
        let dir = std::env::temp_dir().join(format!("stratyx-nuage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = NuageStore::open(&dir).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        store.close().unwrap();
        let store = NuageStore::open(&dir).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        store.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
