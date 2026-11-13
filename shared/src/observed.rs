use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Niveau de confiance d'une détection, borné à `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Confidence(f32);

#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[error("confiance hors de l'intervalle [0, 1] : {0}")]
pub struct ConfidenceError(pub f32);

impl Confidence {
    /// Certitude totale : saisie ou correction manuelle, historique importé.
    pub const CERTAIN: Confidence = Confidence(1.0);

    /// Seuil par défaut en dessous duquel une détection est signalée à l'utilisateur.
    pub const DEFAULT_REVIEW_THRESHOLD: Confidence = Confidence(0.95);

    pub fn new(value: f32) -> Result<Self, ConfidenceError> {
        if (0.0..=1.0).contains(&value) {
            Ok(Confidence(value))
        } else {
            Err(ConfidenceError(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Confidence {
    type Error = ConfidenceError;
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Confidence::new(value)
    }
}

impl From<Confidence> for f32 {
    fn from(c: Confidence) -> f32 {
        c.0
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} %", self.0 * 100.0)
    }
}

/// Origine d'une donnée de main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Lue par la vision artificielle sur une image.
    Vision,
    /// Saisie ou corrigée par l'utilisateur lors de la revue.
    User,
    /// Issue d'un historique importé (fichier texte d'une salle, archive).
    Import,
    /// Déduite par le moteur à partir d'autres données observées
    /// (ex. : montant d'un call). Jamais utilisé pour des cartes privées.
    Derived,
    /// Produite par le simulateur (auto-jeu dans le moteur de règles), pour l'entraînement
    /// des IA. Jamais mélangée aux mains réelles d'un utilisateur.
    Simulation,
}

/// Valeur observée, accompagnée de sa provenance, de sa confiance et de sa position
/// dans la vidéo source (millisecondes depuis le début), si elle existe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observed<T> {
    pub value: T,
    pub source: Source,
    pub confidence: Confidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_ms: Option<u64>,
}

impl<T> Observed<T> {
    /// Valeur certaine (saisie utilisateur ou import).
    pub fn certain(value: T, source: Source) -> Self {
        Observed {
            value,
            source,
            confidence: Confidence::CERTAIN,
            video_ms: None,
        }
    }

    /// Valeur lue par la vision artificielle.
    pub fn vision(value: T, confidence: Confidence, video_ms: u64) -> Self {
        Observed {
            value,
            source: Source::Vision,
            confidence,
            video_ms: Some(video_ms),
        }
    }

    /// Vrai si la détection doit être vérifiée par l'utilisateur.
    #[must_use]
    pub fn needs_review(&self, threshold: Confidence) -> bool {
        self.source == Source::Vision && self.confidence < threshold
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Observed<U> {
        Observed {
            value: f(self.value),
            source: self.source,
            confidence: self.confidence,
            video_ms: self.video_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_bounds() {
        assert!(Confidence::new(0.5).is_ok());
        assert!(Confidence::new(1.2).is_err());
        assert!(Confidence::new(-0.1).is_err());
        assert!(Confidence::new(f32::NAN).is_err());
    }

    #[test]
    fn review_flag_only_for_uncertain_vision() {
        let low = Observed::vision(42u32, Confidence::new(0.6).unwrap(), 1_000);
        let user = Observed::certain(42u32, Source::User);
        let t = Confidence::DEFAULT_REVIEW_THRESHOLD;
        assert!(low.needs_review(t));
        assert!(!user.needs_review(t));
    }

    #[test]
    fn serde_rejects_invalid_confidence() {
        let bad = r#"{"value":1,"source":"vision","confidence":3.0}"#;
        assert!(serde_json::from_str::<Observed<u8>>(bad).is_err());
        let ok = r#"{"value":1,"source":"vision","confidence":0.9,"video_ms":12}"#;
        let o: Observed<u8> = serde_json::from_str(ok).unwrap();
        assert_eq!(o.video_ms, Some(12));
    }
}
