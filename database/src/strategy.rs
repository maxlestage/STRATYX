//! Espace « Stratégie » : ranges de référence, charts, résultats de solveur et simulations,
//! stockés sous forme de blobs indexés par clé.
//!
//! Les clés sont libres ; une hiérarchie par préfixe est recommandée, par exemple
//! `charts/expresso/pushfold/15bb` ou `solveur/srp-btn-bb/flop-k72r`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DatabaseError, Result};
use crate::sql::text_at;
use crate::store::{Database, from_json, to_json};

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
        let json = to_json(&entry)?;
        self.write(|tx| {
            tx.exec(
                "INSERT OR REPLACE INTO strategie (cle, entree, donnees) VALUES (?, ?, ?)",
                &[
                    key.into(),
                    json.into(),
                    crate::hex::encode(&blob.data).into(),
                ],
            )
        })?;
        Ok(entry)
    }

    /// Blob enregistré sous cette clé (somme de contrôle vérifiée).
    pub fn get_strategy(&self, key: &str) -> Result<Option<StrategyBlob>> {
        let rows = self.read()?.rows(
            "SELECT entree, donnees FROM strategie WHERE cle = ?",
            &[key.into()],
        )?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let entry: StrategyEntry = from_json(text_at(row, 0)?.as_bytes())?;
        let data = crate::hex::decode(&text_at(row, 1)?)
            .ok_or_else(|| DatabaseError::Corrupted(format!("données de « {key} » illisibles")))?;
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
        let rows = self
            .read()?
            .rows("SELECT cle, entree FROM strategie", &[])?;
        let mut out: Vec<(String, StrategyEntry)> = Vec::new();
        for row in &rows {
            let key = text_at(row, 0)?;
            if key.starts_with(prefix) {
                out.push((key, from_json(text_at(row, 1)?.as_bytes())?));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out.into_iter().map(|(_, e)| e).collect())
    }

    /// Supprime un blob. Renvoie `false` s'il n'existait pas.
    pub fn delete_strategy(&self, key: &str) -> Result<bool> {
        self.write(|tx| Ok(tx.exec("DELETE FROM strategie WHERE cle = ?", &[key.into()])? > 0))
    }
}
