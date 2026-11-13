//! Espace « Utilisateurs » : sessions de jeu, profils et progression.

use std::collections::BTreeMap;

use redb::{ReadableDatabase, ReadableMultimapTable, ReadableTable};
use serde::{Deserialize, Serialize};
use stratyx_poker::Session;

use crate::error::{DatabaseError, Result};
use crate::space::Space;
use crate::store::{Database, from_json, remove_hand, space_tables, to_json};
use crate::tables::{PROFILES, PROGRESS, SESSIONS};

/// Pseudonyme d'un utilisateur sur une salle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerAlias {
    /// Salle (« pokerstars », « winamax »…), si connue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub name: String,
}

/// Profil d'un utilisateur de Stratyx. Les réglages libres (`settings`) permettent d'étendre
/// le profil sans migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub display_name: String,
    /// Création, en millisecondes depuis l'époque Unix.
    pub created_at_ms: u64,
    /// Pseudonymes de l'utilisateur aux tables (pour reconnaître le héros).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<PlayerAlias>,
    /// Réglages et préférences, par clé.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, serde_json::Value>,
}

impl UserProfile {
    /// Profil sans pseudonyme ni réglage.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        created_at_ms: u64,
    ) -> UserProfile {
        UserProfile {
            id: id.into(),
            display_name: display_name.into(),
            created_at_ms,
            aliases: Vec::new(),
            settings: BTreeMap::new(),
        }
    }
}

/// Mesure de progression (exercice, quiz, statistique suivie dans le temps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressEntry {
    /// Date de la mesure, en millisecondes depuis l'époque Unix.
    pub at_ms: u64,
    /// Domaine : « pushfold », « icm », « quiz.preflop »…
    pub topic: String,
    /// Indicateur mesuré : « score », « erreurs », « ev_perdue_bb »…
    pub metric: String,
    pub value: f64,
    /// Détails libres (exercice, réponses…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Effacement d'une session (RGPD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionErasure {
    /// La fiche de session existait.
    pub session_found: bool,
    /// Mains supprimées (rattachées par `meta.session_id` ou listées dans la session).
    pub hands_deleted: usize,
}

impl Database {
    // --- Sessions -------------------------------------------------------------------

    /// Enregistre ou remplace une session.
    pub fn put_session(&self, session: &Session) -> Result<()> {
        if session.id.trim().is_empty() {
            return Err(DatabaseError::EmptyId("session"));
        }
        let txn = self.db.begin_write()?;
        txn.open_table(SESSIONS)?
            .insert(session.id.as_str(), to_json(session)?.as_slice())?;
        txn.commit()?;
        Ok(())
    }

    /// Session enregistrée sous cet identifiant.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(SESSIONS)?;
        let value = table.get(id)?;
        value.map(|v| from_json(v.value())).transpose()
    }

    /// Toutes les sessions, de la plus ancienne à la plus récente.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let txn = self.db.begin_read()?;
        let mut out: Vec<Session> = read_all(&txn.open_table(SESSIONS)?)?;
        out.sort_by(|a, b| a.started_at_ms.cmp(&b.started_at_ms).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Supprime une session **et ses mains** (droit à l'effacement).
    pub fn delete_session(&self, id: &str) -> Result<SessionErasure> {
        let txn = self.db.begin_write()?;
        let session: Option<Session> = {
            let mut table = txn.open_table(SESSIONS)?;
            let removed = table.remove(id)?;
            removed.map(|v| from_json(v.value())).transpose()?
        };
        let mut hand_ids: Vec<String> = session
            .as_ref()
            .map(|s| s.hand_ids.clone())
            .unwrap_or_default();
        for space in Space::HAND_SPACES {
            let table = txn.open_multimap_table(space_tables(space)?.by_session)?;
            for v in table.get(id)? {
                hand_ids.push(v?.value().to_string());
            }
        }
        hand_ids.sort();
        hand_ids.dedup();
        let mut hands_deleted = 0;
        for hand in &hand_ids {
            if remove_hand(&txn, hand)?.is_some() {
                hands_deleted += 1;
            }
        }
        txn.commit()?;
        Ok(SessionErasure {
            session_found: session.is_some(),
            hands_deleted,
        })
    }

    // --- Profils et progression -------------------------------------------------------

    /// Enregistre ou remplace un profil.
    pub fn put_user(&self, profile: &UserProfile) -> Result<()> {
        if profile.id.trim().is_empty() {
            return Err(DatabaseError::EmptyId("utilisateur"));
        }
        let txn = self.db.begin_write()?;
        txn.open_table(PROFILES)?
            .insert(profile.id.as_str(), to_json(profile)?.as_slice())?;
        txn.commit()?;
        Ok(())
    }

    /// Profil enregistré sous cet identifiant.
    pub fn get_user(&self, id: &str) -> Result<Option<UserProfile>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(PROFILES)?;
        let value = table.get(id)?;
        value.map(|v| from_json(v.value())).transpose()
    }

    /// Tous les profils, par identifiant.
    pub fn list_users(&self) -> Result<Vec<UserProfile>> {
        let txn = self.db.begin_read()?;
        read_all(&txn.open_table(PROFILES)?)
    }

    /// Supprime un profil et sa progression (droit à l'effacement). Les mains jouées sous
    /// ses pseudonymes s'effacent avec [`Database::erase_player`].
    pub fn delete_user(&self, id: &str) -> Result<bool> {
        let txn = self.db.begin_write()?;
        let existed = txn.open_table(PROFILES)?.remove(id)?.is_some();
        txn.open_table(PROGRESS)?.remove(id)?;
        txn.commit()?;
        Ok(existed)
    }

    /// Ajoute une mesure de progression à un utilisateur existant.
    pub fn add_progress(&self, user_id: &str, entry: ProgressEntry) -> Result<()> {
        let txn = self.db.begin_write()?;
        if txn.open_table(PROFILES)?.get(user_id)?.is_none() {
            return Err(DatabaseError::UnknownUser(user_id.to_string()));
        }
        {
            let mut table = txn.open_table(PROGRESS)?;
            let mut entries: Vec<ProgressEntry> = match table.get(user_id)? {
                Some(v) => from_json(v.value())?,
                None => Vec::new(),
            };
            entries.push(entry);
            entries.sort_by_key(|e| e.at_ms);
            table.insert(user_id, to_json(&entries)?.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Progression d'un utilisateur, par date croissante.
    pub fn progress(&self, user_id: &str) -> Result<Vec<ProgressEntry>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(PROGRESS)?;
        let value = table.get(user_id)?;
        Ok(value
            .map(|v| from_json(v.value()))
            .transpose()?
            .unwrap_or_default())
    }
}

/// Désérialise toutes les valeurs JSON d'une table.
pub(crate) fn read_all<T: serde::de::DeserializeOwned>(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for entry in table.iter()? {
        let (_, v) = entry?;
        out.push(from_json(v.value())?);
    }
    Ok(out)
}
