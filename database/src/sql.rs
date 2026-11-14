//! Accès SQL à Nuage DB : transactions et lecture des lignes.
//!
//! Toute écriture passe par [`Database::write`] : un verrou sérialise les écritures (une
//! écriture lit, décide puis écrit, et doit voir le même état du début à la fin), puis une
//! transaction Nuage les applique d'un bloc ou pas du tout. Les lectures composées
//! ([`Database::read`]) voient un instantané cohérent. Les valeurs passent toujours en
//! paramètres `?`, jamais dans le texte des requêtes.

use std::sync::MutexGuard;

use memoire_orm::Value;
use nuage::{EmbeddedTransaction, SqlResult};
use serde_json::Value as Json;

use crate::error::{DatabaseError, Result};
use crate::store::Database;

/// Erreur du moteur Nuage, en texte.
pub(crate) fn storage(e: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::Storage(e.to_string())
}

/// Une transaction Nuage, avec le verrou d'écriture pour une écriture.
pub(crate) struct Tx<'a> {
    tx: EmbeddedTransaction,
    _writing: Option<MutexGuard<'a, ()>>,
}

impl Tx<'_> {
    /// Exécute une ou plusieurs instructions ; un résultat par instruction.
    pub(crate) fn run(&self, sql: &str, params: &[Json]) -> Result<Vec<SqlResult>> {
        self.tx.sql(sql, params).map_err(storage)
    }

    /// Exécute une écriture ; renvoie le nombre de lignes touchées.
    pub(crate) fn exec(&self, sql: &str, params: &[Json]) -> Result<u64> {
        Ok(self.run(sql, params)?.iter().map(|r| r.rows_affected).sum())
    }

    /// Lignes de la dernière instruction.
    pub(crate) fn rows(&self, sql: &str, params: &[Json]) -> Result<Vec<Vec<Value>>> {
        Ok(self
            .run(sql, params)?
            .pop()
            .map(|r| r.rows)
            .unwrap_or_default())
    }

    /// Première colonne de chaque ligne, en texte.
    pub(crate) fn texts(&self, sql: &str, params: &[Json]) -> Result<Vec<String>> {
        self.rows(sql, params)?
            .iter()
            .map(|row| text_at(row, 0))
            .collect()
    }

    /// Première colonne de la première ligne, en texte.
    pub(crate) fn text(&self, sql: &str, params: &[Json]) -> Result<Option<String>> {
        Ok(self.texts(sql, params)?.into_iter().next())
    }

    /// Première colonne de la première ligne, en entier (`NULL` : `None`).
    pub(crate) fn int(&self, sql: &str, params: &[Json]) -> Result<Option<i64>> {
        match self.rows(sql, params)?.first() {
            None => Ok(None),
            Some(row) => int_at(row, 0),
        }
    }
}

/// Colonne `i` d'une ligne, en texte.
pub(crate) fn text_at(row: &[Value], i: usize) -> Result<String> {
    row.get(i)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| DatabaseError::Corrupted(format!("colonne {i} : texte attendu")))
}

/// Colonne `i` d'une ligne, en entier (`NULL` : `None`).
pub(crate) fn int_at(row: &[Value], i: usize) -> Result<Option<i64>> {
    match row.get(i) {
        None => Err(DatabaseError::Corrupted(format!("colonne {i} absente"))),
        Some(v) if v.is_null() => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| DatabaseError::Corrupted(format!("colonne {i} : entier attendu"))),
    }
}

/// Un entier positif relu de la base.
pub(crate) fn to_u64(n: i64) -> Result<u64> {
    u64::try_from(n).map_err(|_| DatabaseError::Corrupted(format!("entier négatif {n}")))
}

impl Database {
    /// Lecture composée : un instantané cohérent, abandonné à la fin (rien n'est écrit).
    pub(crate) fn read(&self) -> Result<Tx<'_>> {
        Ok(Tx {
            tx: self.db.begin_now().map_err(storage)?,
            _writing: None,
        })
    }

    /// Écriture : `f` s'exécute dans une transaction, validée s'il réussit, annulée sinon.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Tx<'_>) -> Result<T>) -> Result<T> {
        let guard = self.writes.lock().unwrap_or_else(|e| e.into_inner());
        let tx = Tx {
            tx: self.db.begin_now().map_err(storage)?,
            _writing: Some(guard),
        };
        match f(&tx) {
            Ok(value) => {
                tx.tx.commit_now().map_err(storage)?;
                Ok(value)
            }
            Err(e) => {
                let _ = tx.tx.rollback_now();
                Err(e)
            }
        }
    }
}
