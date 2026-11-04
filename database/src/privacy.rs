//! Protection des données (RGPD) : effacement d'un joueur et export complet en JSON.

use std::io::Write;

use redb::{ReadableDatabase, ReadableMultimapTable, ReadableTable};
use serde::{Deserialize, Serialize};
use stratyx_poker::Session;

use crate::error::{DatabaseError, Result};
use crate::ingest::IngestReport;
use crate::migration::SCHEMA_VERSION;
use crate::space::Space;
use crate::store::{
    Database, StoreMode, StoredHand, decode_hand, read_hand_in, remove_hand, space_tables,
    store_hand,
};
use crate::strategy::StrategyEntry;
use crate::tables::{INGESTIONS, PROFILES, PROGRESS, SESSIONS, STRATEGY_DATA, STRATEGY_ENTRIES};
use crate::users::{ProgressEntry, UserProfile, read_all};

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
        let txn = self.db.begin_write()?;
        let mut ids = Vec::new();
        for space in Space::HAND_SPACES {
            let table = txn.open_multimap_table(space_tables(space)?.by_player)?;
            for v in table.get(player)? {
                ids.push(v?.value().to_string());
            }
        }
        let mut report = ErasureReport::default();
        for id in ids {
            match mode {
                Erasure::DeleteHands => {
                    if remove_hand(&txn, &id)?.is_some() {
                        report.hands_deleted += 1;
                    }
                }
                Erasure::Anonymize => {
                    let Some(stored) = read_hand_in(&txn, &id)? else {
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
                        &txn,
                        &record,
                        self.review_threshold,
                        stored.batch_id.as_deref(),
                        StoreMode::Force,
                    )?;
                    report.hands_anonymized += 1;
                }
            }
        }
        txn.commit()?;
        Ok(report)
    }

    /// Export complet : mains (avec statut et empreinte), sessions, utilisateurs et
    /// progression, stratégie, rapports d'ingestion.
    pub fn export(&self) -> Result<DatabaseExport> {
        let txn = self.db.begin_read()?;
        let mut hands = Vec::new();
        for space in Space::HAND_SPACES {
            let table = txn.open_table(space_tables(space)?.hands)?;
            for entry in table.iter()? {
                let (k, v) = entry?;
                hands.push(decode_hand(k.value(), v.value())?);
            }
        }
        hands.sort_by(|a, b| a.record.id.cmp(&b.record.id));
        let mut sessions: Vec<Session> = read_all(&txn.open_table(SESSIONS)?)?;
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        let progress = txn.open_table(PROGRESS)?;
        let mut users = Vec::new();
        for profile in read_all::<UserProfile>(&txn.open_table(PROFILES)?)? {
            let entries = match progress.get(profile.id.as_str())? {
                Some(v) => serde_json::from_slice(v.value())?,
                None => Vec::new(),
            };
            users.push(UserExport {
                profile,
                progress: entries,
            });
        }
        let data = txn.open_table(STRATEGY_DATA)?;
        let mut strategy = Vec::new();
        for entry in read_all::<StrategyEntry>(&txn.open_table(STRATEGY_ENTRIES)?)? {
            let bytes = data.get(entry.key.as_str())?.ok_or_else(|| {
                DatabaseError::Corrupted(format!("données de « {} » absentes", entry.key))
            })?;
            strategy.push(StrategyExport {
                data_hex: crate::hex::encode(bytes.value()),
                entry,
            });
        }
        let ingestions = read_all(&txn.open_table(INGESTIONS)?)?;
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
