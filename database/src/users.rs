//! Espace « Utilisateurs » : sessions de jeu, profils et progression.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use stratyx_poker::Session;

use crate::error::{DatabaseError, Result};
use crate::store::{Database, from_json, remove_hand, to_json};

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
        let json = to_json(session)?;
        self.write(|tx| {
            tx.exec(
                "INSERT OR REPLACE INTO session (id, debut, donnees) VALUES (?, ?, ?)",
                &[
                    session.id.as_str().into(),
                    session.started_at_ms.into(),
                    json.into(),
                ],
            )
            .map(drop)
        })
    }

    /// Session enregistrée sous cet identifiant.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.read()?
            .text("SELECT donnees FROM session WHERE id = ?", &[id.into()])?
            .map(|json| from_json(json.as_bytes()))
            .transpose()
    }

    /// Toutes les sessions, de la plus ancienne à la plus récente.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut out: Vec<Session> =
            parse_all(self.read()?.texts("SELECT donnees FROM session", &[])?)?;
        out.sort_by(|a, b| a.started_at_ms.cmp(&b.started_at_ms).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Supprime une session **et ses mains** (droit à l'effacement).
    pub fn delete_session(&self, id: &str) -> Result<SessionErasure> {
        self.write(|tx| {
            let session: Option<Session> = tx
                .text("SELECT donnees FROM session WHERE id = ?", &[id.into()])?
                .map(|json| from_json(json.as_bytes()))
                .transpose()?;
            tx.exec("DELETE FROM session WHERE id = ?", &[id.into()])?;
            let mut hand_ids: Vec<String> = session
                .as_ref()
                .map(|s| s.hand_ids.clone())
                .unwrap_or_default();
            hand_ids.extend(tx.texts("SELECT id FROM main WHERE session = ?", &[id.into()])?);
            hand_ids.sort();
            hand_ids.dedup();
            let mut hands_deleted = 0;
            for hand in &hand_ids {
                if remove_hand(tx, hand)?.is_some() {
                    hands_deleted += 1;
                }
            }
            Ok(SessionErasure {
                session_found: session.is_some(),
                hands_deleted,
            })
        })
    }

    // --- Profils et progression -------------------------------------------------------

    /// Enregistre ou remplace un profil.
    pub fn put_user(&self, profile: &UserProfile) -> Result<()> {
        if profile.id.trim().is_empty() {
            return Err(DatabaseError::EmptyId("utilisateur"));
        }
        let json = to_json(profile)?;
        self.write(|tx| {
            tx.exec(
                "INSERT OR REPLACE INTO profil (id, donnees) VALUES (?, ?)",
                &[profile.id.as_str().into(), json.into()],
            )
            .map(drop)
        })
    }

    /// Profil enregistré sous cet identifiant.
    pub fn get_user(&self, id: &str) -> Result<Option<UserProfile>> {
        self.read()?
            .text("SELECT donnees FROM profil WHERE id = ?", &[id.into()])?
            .map(|json| from_json(json.as_bytes()))
            .transpose()
    }

    /// Tous les profils, par identifiant.
    pub fn list_users(&self) -> Result<Vec<UserProfile>> {
        let mut out: Vec<UserProfile> =
            parse_all(self.read()?.texts("SELECT donnees FROM profil", &[])?)?;
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Supprime un profil et sa progression (droit à l'effacement). Les mains jouées sous
    /// ses pseudonymes s'effacent avec [`Database::erase_player`].
    pub fn delete_user(&self, id: &str) -> Result<bool> {
        self.write(|tx| {
            let existed = tx.exec("DELETE FROM profil WHERE id = ?", &[id.into()])? > 0;
            tx.exec(
                "DELETE FROM progression WHERE utilisateur = ?",
                &[id.into()],
            )?;
            Ok(existed)
        })
    }

    /// Ajoute une mesure de progression à un utilisateur existant.
    pub fn add_progress(&self, user_id: &str, entry: ProgressEntry) -> Result<()> {
        self.write(|tx| {
            let known = tx.int(
                "SELECT count(*) FROM profil WHERE id = ?",
                &[user_id.into()],
            )?;
            if known.unwrap_or(0) == 0 {
                return Err(DatabaseError::UnknownUser(user_id.to_string()));
            }
            let mut entries = progress_in(tx, user_id)?;
            entries.push(entry);
            entries.sort_by_key(|e| e.at_ms);
            tx.exec(
                "INSERT OR REPLACE INTO progression (utilisateur, donnees) VALUES (?, ?)",
                &[user_id.into(), to_json(&entries)?.into()],
            )
            .map(drop)
        })
    }

    /// Progression d'un utilisateur, par date croissante.
    pub fn progress(&self, user_id: &str) -> Result<Vec<ProgressEntry>> {
        progress_in(&self.read()?, user_id)
    }
}

/// Progression d'un utilisateur, dans une transaction.
pub(crate) fn progress_in(tx: &crate::sql::Tx<'_>, user_id: &str) -> Result<Vec<ProgressEntry>> {
    Ok(tx
        .text(
            "SELECT donnees FROM progression WHERE utilisateur = ?",
            &[user_id.into()],
        )?
        .map(|json| from_json(json.as_bytes()))
        .transpose()?
        .unwrap_or_default())
}

/// Désérialise des valeurs JSON.
pub(crate) fn parse_all<T: serde::de::DeserializeOwned>(values: Vec<String>) -> Result<Vec<T>> {
    values
        .iter()
        .map(|json| from_json(json.as_bytes()))
        .collect()
}
