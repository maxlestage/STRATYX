//! Statut de validation d'une main et résumé indexé pour la recherche.

use std::fmt;

use serde::{Deserialize, Serialize};
use stratyx_poker::game::Phase;
use stratyx_poker::history::Outcome;
use stratyx_poker::{GameFormat, HandRecord};
use stratyx_shared::Confidence;

use crate::fingerprint::Fingerprint;
use crate::space::Space;

/// Résultat de la validation par rejeu ([`HandRecord::replay`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Validation {
    /// Rejouée sans erreur, gains attribués.
    Valid,
    /// Rejouée sans erreur, mais gagnant indéterminable sans inventer de cartes.
    Unresolved { reason: String },
    /// Historique incomplet (vidéo coupée, main en cours).
    Incomplete,
    /// Rejeu refusé par le moteur de règles.
    Invalid { reason: String },
}

impl Validation {
    /// Nature du statut, sans la raison.
    #[must_use]
    pub fn kind(&self) -> ValidationKind {
        match self {
            Validation::Valid => ValidationKind::Valid,
            Validation::Unresolved { .. } => ValidationKind::Unresolved,
            Validation::Incomplete => ValidationKind::Incomplete,
            Validation::Invalid { .. } => ValidationKind::Invalid,
        }
    }
}

impl fmt::Display for Validation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Validation::Valid => f.write_str("rejouée sans erreur"),
            Validation::Unresolved { reason } => write!(f, "résultat non résolu : {reason}"),
            Validation::Incomplete => f.write_str("main incomplète"),
            Validation::Invalid { reason } => write!(f, "invalide : {reason}"),
        }
    }
}

/// Nature du statut, sans détail (filtres de recherche, index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationKind {
    Valid,
    Unresolved,
    Incomplete,
    Invalid,
}

impl ValidationKind {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            ValidationKind::Valid => "valid",
            ValidationKind::Unresolved => "unresolved",
            ValidationKind::Incomplete => "incomplete",
            ValidationKind::Invalid => "invalid",
        }
    }
}

/// Statut d'une main enregistrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandStatus {
    pub validation: Validation,
    /// Nombre de détections incertaines à faire vérifier
    /// ([`HandRecord::review_items`]).
    pub review_items: u32,
}

impl HandStatus {
    /// Vrai si la main doit être revue : détection incertaine ou main non validée.
    #[must_use]
    pub fn needs_review(&self) -> bool {
        self.review_items > 0 || self.validation != Validation::Valid
    }
}

/// Résumé d'une main, renvoyé par la recherche et indexé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandSummary {
    pub id: String,
    pub space: Space,
    pub format: GameFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Joueurs servis, dans l'ordre des sièges.
    pub players: Vec<String>,
    /// Joueur analysé, s'il est assis à la table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    /// Résultat net du héros (jetons ou centimes), si la main est résolue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_net: Option<i64>,
    /// Vrai si au moins deux joueurs sont allés jusqu'à l'abattage.
    pub showdown: bool,
    pub status: HandStatus,
    pub fingerprint: Fingerprint,
}

/// Analyse complète d'une main : statut, résumé et empreinte.
pub(crate) fn analyze(record: &HandRecord, space: Space, threshold: Confidence) -> HandSummary {
    let review_items = u32::try_from(record.review_items(threshold).len()).unwrap_or(u32::MAX);
    let replay = record.replay();
    let fingerprint = Fingerprint::with_replay(record, replay.as_ref().ok());
    let (validation, showdown, hero_net) = match &replay {
        Err(e) => (
            Validation::Invalid {
                reason: e.to_string(),
            },
            false,
            None,
        ),
        Ok(r) => {
            let showdown = r.state.phase() == Phase::Showdown;
            match &r.outcome {
                Outcome::Settled(s) => {
                    let net = record
                        .meta
                        .hero_seat
                        .and_then(|h| r.seats.iter().position(|s| *s == h))
                        .map(|i| s.net[i]);
                    (Validation::Valid, showdown, net)
                }
                Outcome::Unresolved { reason } => (
                    Validation::Unresolved {
                        reason: reason.clone(),
                    },
                    showdown,
                    None,
                ),
                Outcome::Incomplete => (Validation::Incomplete, showdown, None),
            }
        }
    };
    let mut seats: Vec<_> = record.seats.iter().collect();
    seats.sort_by_key(|s| s.seat);
    let hero = record
        .meta
        .hero_seat
        .and_then(|h| seats.iter().find(|s| s.seat == h))
        .map(|s| s.player.clone());
    HandSummary {
        id: record.id.clone(),
        space,
        format: record.format,
        played_at_ms: record.meta.played_at_ms,
        session_id: record.meta.session_id.clone(),
        site: record.meta.site.clone(),
        players: seats.iter().map(|s| s.player.clone()).collect(),
        hero,
        hero_net,
        showdown,
        status: HandStatus {
            validation,
            review_items,
        },
        fingerprint,
    }
}
