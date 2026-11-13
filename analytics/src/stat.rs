//! Statistique de fréquence : occurrences, opportunités, intervalle de confiance et fiabilité.
//!
//! Chaque statistique d'un profil (VPIP, 3-bet, c-bet…) est une proportion
//! `occurrences / opportunités`. On conserve les deux compteurs, jamais le seul pourcentage :
//! les profils se cumulent ainsi main après main et l'incertitude reste mesurable.
//!
//! L'incertitude est donnée par l'**intervalle de score de Wilson** (95 % par défaut), qui
//! reste correct pour les petits échantillons et les fréquences proches de 0 ou de 1, à la
//! différence de l'approximation normale. Un **niveau de fiabilité** lisible (insuffisant,
//! indicatif, fiable) est déduit du nombre d'opportunités.

use serde::{Deserialize, Serialize};

/// Quantile de la loi normale pour un intervalle bilatéral à 95 %.
pub const Z_95: f64 = 1.959_963_984_540_054;

/// Compteurs d'une statistique de fréquence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stat {
    /// Nombre de fois où l'action a été choisie.
    pub occurrences: u64,
    /// Nombre de fois où le joueur pouvait la choisir.
    pub opportunities: u64,
}

/// Intervalle de confiance d'une fréquence, bornes dans `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub low: f64,
    pub high: f64,
}

impl Interval {
    /// Vrai si l'intervalle contient la valeur.
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        (self.low..=self.high).contains(&value)
    }
}

/// Niveau de fiabilité d'une statistique, selon la taille de l'échantillon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reliability {
    /// Trop peu d'opportunités : la valeur ne doit pas être interprétée.
    Insufficient,
    /// Tendance à confirmer.
    Indicative,
    /// Échantillon suffisant pour fonder une décision (exploitation, détection de fuite).
    Reliable,
}

impl Reliability {
    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            Reliability::Insufficient => "insuffisant",
            Reliability::Indicative => "indicatif",
            Reliability::Reliable => "fiable",
        }
    }
}

/// Seuils (en nombre d'opportunités) des niveaux de fiabilité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliabilityThresholds {
    /// À partir de ce nombre d'opportunités, la statistique est « indicative ».
    pub indicative: u64,
    /// À partir de ce nombre d'opportunités, la statistique est « fiable ».
    pub reliable: u64,
}

impl Default for ReliabilityThresholds {
    /// 30 opportunités pour une tendance, 100 pour une valeur fiable : à 100 opportunités,
    /// la demi-largeur de l'intervalle de Wilson à 95 % reste inférieure à 10 points.
    fn default() -> Self {
        ReliabilityThresholds {
            indicative: 30,
            reliable: 100,
        }
    }
}

impl ReliabilityThresholds {
    #[must_use]
    pub fn level(&self, opportunities: u64) -> Reliability {
        if opportunities >= self.reliable {
            Reliability::Reliable
        } else if opportunities >= self.indicative {
            Reliability::Indicative
        } else {
            Reliability::Insufficient
        }
    }
}

/// Vue calculée d'une statistique, prête pour l'affichage ou la sérialisation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StatSummary {
    pub occurrences: u64,
    pub opportunities: u64,
    /// Fréquence observée ; `None` sans opportunité.
    pub rate: Option<f64>,
    /// Intervalle de Wilson à 95 % ; `None` sans opportunité.
    pub interval: Option<Interval>,
    pub reliability: Reliability,
}

impl Stat {
    #[must_use]
    pub const fn new(occurrences: u64, opportunities: u64) -> Stat {
        Stat {
            occurrences,
            opportunities,
        }
    }

    /// Enregistre une main : `opportunity` si le joueur pouvait agir ainsi, `occurred` s'il
    /// l'a fait. Une occurrence sans opportunité est ignorée.
    pub fn record(&mut self, opportunity: bool, occurred: bool) {
        if opportunity {
            self.opportunities += 1;
            if occurred {
                self.occurrences += 1;
            }
        }
    }

    /// Cumule une autre statistique.
    pub fn merge(&mut self, other: Stat) {
        self.occurrences += other.occurrences;
        self.opportunities += other.opportunities;
    }

    /// Fréquence observée.
    #[must_use]
    pub fn rate(&self) -> Option<f64> {
        (self.opportunities > 0).then(|| self.occurrences as f64 / self.opportunities as f64)
    }

    /// Intervalle de score de Wilson pour le quantile `z` (1,96 pour 95 %).
    #[must_use]
    pub fn wilson(&self, z: f64) -> Option<Interval> {
        if self.opportunities == 0 {
            return None;
        }
        let n = self.opportunities as f64;
        let p = self.occurrences as f64 / n;
        let z2 = z * z;
        let denom = 1.0 + z2 / n;
        let center = (p + z2 / (2.0 * n)) / denom;
        let half = z / denom * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
        Some(Interval {
            low: (center - half).max(0.0),
            high: (center + half).min(1.0),
        })
    }

    /// Intervalle de Wilson à 95 %.
    #[must_use]
    pub fn wilson95(&self) -> Option<Interval> {
        self.wilson(Z_95)
    }

    #[must_use]
    pub fn reliability(&self, thresholds: &ReliabilityThresholds) -> Reliability {
        thresholds.level(self.opportunities)
    }

    #[must_use]
    pub fn summary(&self, thresholds: &ReliabilityThresholds) -> StatSummary {
        StatSummary {
            occurrences: self.occurrences,
            opportunities: self.opportunities,
            rate: self.rate(),
            interval: self.wilson95(),
            reliability: self.reliability(thresholds),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn wilson_reference_values() {
        // Valeurs de référence (Wilson 1927, Newcombe 1998).
        let i = Stat::new(5, 10).wilson95().unwrap();
        assert!(close(i.low, 0.2366) && close(i.high, 0.7634), "{i:?}");
        let i = Stat::new(0, 10).wilson95().unwrap();
        assert!(close(i.low, 0.0) && close(i.high, 0.2775), "{i:?}");
        let i = Stat::new(10, 10).wilson95().unwrap();
        assert!(close(i.low, 0.7225) && close(i.high, 1.0), "{i:?}");
        let i = Stat::new(81, 263).wilson95().unwrap();
        assert!(close(i.low, 0.2553) && close(i.high, 0.3662), "{i:?}");
        assert_eq!(Stat::default().wilson95(), None);
        assert_eq!(Stat::default().rate(), None);
    }

    #[test]
    fn record_and_merge() {
        let mut s = Stat::default();
        s.record(true, true);
        s.record(true, false);
        s.record(false, true); // ignoré : pas d'opportunité
        assert_eq!(s, Stat::new(1, 2));
        s.merge(Stat::new(3, 4));
        assert_eq!(s, Stat::new(4, 6));
        assert!(close(s.rate().unwrap(), 4.0 / 6.0));
    }

    #[test]
    fn reliability_levels() {
        let t = ReliabilityThresholds::default();
        assert_eq!(Stat::new(0, 29).reliability(&t), Reliability::Insufficient);
        assert_eq!(Stat::new(0, 30).reliability(&t), Reliability::Indicative);
        assert_eq!(Stat::new(0, 100).reliability(&t), Reliability::Reliable);
        assert!(Reliability::Reliable > Reliability::Indicative);
        let s = Stat::new(20, 100).summary(&t);
        assert_eq!(s.rate, Some(0.2));
        assert!(s.interval.unwrap().contains(0.2));
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"reliability\":\"reliable\""));
    }
}
