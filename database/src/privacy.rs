//! Protection des données (RGPD) : effacement d'un joueur et export complet en JSON.

use std::io::Write;

use serde::{Deserialize, Serialize};
use stratyx_poker::Session;

use crate::error::{DatabaseError, Result};
use crate::ingest::IngestReport;
use crate::migration::SCHEMA_VERSION;
use crate::sql::text_at;
use crate::store::{
    Database, StoreMode, StoredHand, decode_hand, from_json, read_hand_in, remove_hand, store_hand,
};
use crate::strategy::StrategyEntry;
use crate::users::{ProgressEntry, UserProfile, parse_all, progress_in};

/// Mode d'effacement d'un joueur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Erasure {
    /// Remplace le pseudonyme par « Anonyme (siège N) » dans chaque main : le libellé dépend
    /// du siège, il ne permet pas de relier les mains entre elles. Les mains restent
    /// exploitables (adapté aux adversaires).
    Anonymize,
    /// Supprime toutes les mains où le joueur est servi.
    DeleteHands,
}

/// Bilan d'un effacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErasureReport {
    pub hands_anonymized: usize,
    pub hands_deleted: usize,
}

/// Utilisateur exporté avec sa progression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserExport {
    pub profile: UserProfile,
    pub progress: Vec<ProgressEntry>,
}

/// Blob de stratégie exporté (données en hexadécimal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyExport {
    pub entry: StrategyEntry,
    pub data_hex: String,
}

/// Export complet de la base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseExport {
    /// Toujours « stratyx-export ».
    pub format: String,
    pub schema_version: u32,
    pub exported_at_ms: u64,
    pub hands: Vec<StoredHand>,
    pub sessions: Vec<Session>,
    pub users: Vec<UserExport>,
    pub strategy: Vec<StrategyExport>,
    pub ingestions: Vec<IngestReport>,
}

/// Valeur du champ [`DatabaseExport::format`].
pub const EXPORT_FORMAT: &str = "stratyx-export";

impl Database {
    /// Efface un joueur (droit à l'effacement) dans toutes les bases de mains, dans une seule
    /// transaction.
    pub fn erase_player(&self, player: &str, mode: Erasure) -> Result<ErasureReport> {
        if player.trim().is_empty() {
            return Err(DatabaseError::EmptyId("joueur"));
        }
        self.write(|tx| {
            let mut ids = tx.texts(
                "SELECT main FROM main_joueur WHERE joueur = ?",
                &[player.into()],
            )?;
            ids.sort();
            ids.dedup();
            let mut report = ErasureReport::default();
            for id in ids {
                match mode {
                    Erasure::DeleteHands => {
                        if remove_hand(tx, &id)?.is_some() {
                            report.hands_deleted += 1;
                        }
                    }
                    Erasure::Anonymize => {
                        let Some(stored) = read_hand_in(tx, &id)? else {
                            continue;
                        };
                        let mut record = stored.record;
                        let taken: Vec<String> =
                            record.seats.iter().map(|s| s.player.clone()).collect();
                        for seat in &mut record.seats {
                            if seat.player == player {
                                let mut label = format!("Anonyme (siège {})", seat.seat);
                                while taken.contains(&label) {
                                    label.push('*');
                                }
                                seat.player = label;
                            }
                        }
                        store_hand(
                            tx,
                            &record,
                            self.review_threshold,
                            stored.batch_id.as_deref(),
                            StoreMode::Force,
                        )?;
                        report.hands_anonymized += 1;
                    }
                }
            }
            Ok(report)
        })
    }

    /// Export complet : mains (avec statut et empreinte), sessions, utilisateurs et
    /// progression, stratégie, rapports d'ingestion.
    pub fn export(&self) -> Result<DatabaseExport> {
        let tx = self.read()?;
        let mut hands = Vec::new();
        for row in &tx.rows("SELECT id, donnees FROM main", &[])? {
            hands.push(decode_hand(&text_at(row, 0)?, text_at(row, 1)?.as_bytes())?);
        }
        hands.sort_by(|a, b| a.record.id.cmp(&b.record.id));
        let mut sessions: Vec<Session> = parse_all(tx.texts("SELECT donnees FROM session", &[])?)?;
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        let mut profiles: Vec<UserProfile> =
            parse_all(tx.texts("SELECT donnees FROM profil", &[])?)?;
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        let mut users = Vec::new();
        for profile in profiles {
            let progress = progress_in(&tx, &profile.id)?;
            users.push(UserExport { profile, progress });
        }
        let mut strategy = Vec::new();
        for row in &tx.rows("SELECT entree, donnees FROM strategie", &[])? {
            let entry: StrategyEntry = from_json(text_at(row, 0)?.as_bytes())?;
            let data_hex = text_at(row, 1)?;
            if crate::hex::decode(&data_hex).is_none() {
                return Err(DatabaseError::Corrupted(format!(
                    "données de « {} » illisibles",
                    entry.key
                )));
            }
            strategy.push(StrategyExport { entry, data_hex });
        }
        strategy.sort_by(|a, b| a.entry.key.cmp(&b.entry.key));
        let mut ingestions: Vec<IngestReport> =
            parse_all(tx.texts("SELECT rapport FROM ingestion", &[])?)?;
        ingestions.sort_by(|a, b| a.batch_id.cmp(&b.batch_id));
        Ok(DatabaseExport {
            format: EXPORT_FORMAT.to_string(),
            schema_version: SCHEMA_VERSION,
            exported_at_ms: crate::now_ms(),
            hands,
            sessions,
            users,
            strategy,
            ingestions,
        })
    }

    /// Écrit l'export complet en JSON indenté.
    pub fn export_json(&self, writer: impl Write) -> Result<()> {
        serde_json::to_writer_pretty(writer, &self.export()?)?;
        Ok(())
    }
}
