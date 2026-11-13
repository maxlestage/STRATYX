//! Espace « Stratégie » : ranges de référence, charts, résultats de solveur et simulations,
//! stockés sous forme de blobs indexés par clé.
//!
//! Les clés sont libres ; une hiérarchie par préfixe est recommandée, par exemple
//! `charts/expresso/pushfold/15bb` ou `solveur/srp-btn-bb/flop-k72r`.

use redb::ReadableDatabase;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DatabaseError, Result};
use crate::store::{Database, from_json, to_json};
use crate::tables::{STRATEGY_DATA, STRATEGY_ENTRIES};

/// Contenu à enregistrer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyBlob {
    /// Type de contenu : « application/json », « text/x-range », « application/x-stratyx-cfr »…
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub data: Vec<u8>,
}

/// Description d'une entrée enregistrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyEntry {
    pub key: String,
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Taille des données, en octets.
    pub size: u64,
    /// SHA-256 des données (hexadécimal), vérifié à chaque lecture.
    pub sha256: String,
    pub updated_at_ms: u64,
}

fn checksum(data: &[u8]) -> String {
    crate::hex::encode(&Sha256::digest(data))
}

impl Database {
    /// Enregistre ou remplace un blob de stratégie.
    pub fn put_strategy(&self, key: &str, blob: &StrategyBlob) -> Result<StrategyEntry> {
        if key.trim().is_empty() {
            return Err(DatabaseError::EmptyId("clé de stratégie"));
        }
        let entry = StrategyEntry {
            key: key.to_string(),
            content_type: blob.content_type.clone(),
            description: blob.description.clone(),
            size: blob.data.len() as u64,
            sha256: checksum(&blob.data),
            updated_at_ms: crate::now_ms(),
        };
        let txn = self.db.begin_write()?;
        txn.open_table(STRATEGY_ENTRIES)?
            .insert(key, to_json(&entry)?.as_slice())?;
        txn.open_table(STRATEGY_DATA)?
            .insert(key, blob.data.as_slice())?;
        txn.commit()?;
        Ok(entry)
    }

    /// Blob enregistré sous cette clé (somme de contrôle vérifiée).
    pub fn get_strategy(&self, key: &str) -> Result<Option<StrategyBlob>> {
        let txn = self.db.begin_read()?;
        let entries = txn.open_table(STRATEGY_ENTRIES)?;
        let Some(entry) = entries.get(key)? else {
            return Ok(None);
        };
        let entry: StrategyEntry = from_json(entry.value())?;
        let data = txn
            .open_table(STRATEGY_DATA)?
            .get(key)?
            .map(|v| v.value().to_vec())
            .ok_or_else(|| DatabaseError::Corrupted(format!("données de « {key} » absentes")))?;
        if checksum(&data) != entry.sha256 {
            return Err(DatabaseError::Corrupted(format!(
                "somme de contrôle de « {key} » invalide"
            )));
        }
        Ok(Some(StrategyBlob {
            content_type: entry.content_type,
            description: entry.description,
            data,
        }))
    }

    /// Entrées dont la clé commence par `prefix` (toutes si vide), par clé croissante.
    pub fn strategy_entries(&self, prefix: &str) -> Result<Vec<StrategyEntry>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(STRATEGY_ENTRIES)?;
        let mut out = Vec::new();
        for item in table.range(prefix..)? {
            let (k, v) = item?;
            if !k.value().starts_with(prefix) {
                break;
            }
            out.push(from_json(v.value())?);
        }
        Ok(out)
    }

    /// Supprime un blob. Renvoie `false` s'il n'existait pas.
    pub fn delete_strategy(&self, key: &str) -> Result<bool> {
        let txn = self.db.begin_write()?;
        let existed = txn.open_table(STRATEGY_ENTRIES)?.remove(key)?.is_some();
        txn.open_table(STRATEGY_DATA)?.remove(key)?;
        txn.commit()?;
        Ok(existed)
    }
}
