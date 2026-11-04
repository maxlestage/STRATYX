//! Sessions de jeu et résultats de tournois, pour les statistiques (ROI, ITM) et le
//! verrou d'analyse pendant une capture (décision D6).

use serde::{Deserialize, Serialize};

use crate::format::GameFormat;

/// Origine des mains d'une session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    /// Capture d'écran pendant le jeu.
    LiveCapture,
    /// Import d'une vidéo enregistrée.
    VideoImport,
    /// Import d'historiques texte d'une salle.
    TextImport,
    /// Parties simulées (entraînement, coaching).
    Simulation,
}

/// Résultat financier d'un tournoi, d'un sit & go ou d'une Expresso pour le joueur analysé.
/// Les montants sont en centimes de la devise indiquée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentResult {
    /// Droit d'entrée total payé (frais compris), en centimes.
    pub buy_in: u64,
    /// Part des frais dans le droit d'entrée, en centimes.
    #[serde(default)]
    pub fee: u64,
    /// Gains, en centimes (0 si éliminé hors des places payées).
    pub prize: u64,
    /// Code ISO 4217 de la devise (« EUR »).
    pub currency: String,
    /// Place finale (1 = vainqueur), si connue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<u32>,
    /// Nombre d'inscrits.
    pub entrants: u32,
    /// Multiplicateur tiré au sort (Expresso), si applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prize_multiplier: Option<u32>,
    /// Structure des prix par place, en centimes (place 1 en premier), pour l'ICM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payouts: Vec<u64>,
}

impl TournamentResult {
    /// Bénéfice net en centimes.
    #[must_use]
    pub fn profit(&self) -> i64 {
        self.prize as i64 - self.buy_in as i64
    }

    /// Vrai si le joueur a fini dans les places payées.
    #[must_use]
    pub fn in_the_money(&self) -> bool {
        self.prize > 0
    }
}

/// Une session : suite de mains jouées ou importées d'un même format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub source: SessionSource,
    pub format: GameFormat,
    /// Début, en millisecondes depuis l'époque Unix (UTC).
    pub started_at_ms: u64,
    /// Fin ; `None` tant que la session est ouverte (capture en cours).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub hand_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tournament: Option<TournamentResult>,
}

impl Session {
    /// Une session ouverte ne peut pas être analysée (décision D6).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.ended_at_ms.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profit_and_itm() {
        let r = TournamentResult {
            buy_in: 500,
            fee: 35,
            prize: 1_000,
            currency: "EUR".into(),
            finish: Some(1),
            entrants: 3,
            prize_multiplier: Some(2),
            payouts: vec![1_000],
        };
        assert_eq!(r.profit(), 500);
        assert!(r.in_the_money());
        let s = Session {
            id: "s1".into(),
            source: SessionSource::LiveCapture,
            format: GameFormat::EXPRESSO,
            started_at_ms: 0,
            ended_at_ms: None,
            hand_ids: vec![],
            tournament: Some(r),
        };
        assert!(s.is_open());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), s);
    }
}
