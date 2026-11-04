//! Détection de fuites simples : comparaison des statistiques d'un joueur (le héros) à des
//! bandes de référence configurables par format et par position.
//!
//! Une fuite n'est signalée que si la statistique atteint le niveau de fiabilité requis
//! ([`LeakConfig::min_reliability`]) et que sa fréquence observée sort de la bande. Elle est
//! dite **significative** quand tout l'intervalle de Wilson à 95 % est hors de la bande :
//! l'écart n'est alors pas attribuable à la variance de l'échantillon.
//!
//! Les bandes par défaut ([`LeakConfig::default`]) visent le no-limit 6-max (cash game et
//! tournoi). Ce sont des repères de départ, à calibrer : chaque bande est une simple donnée
//! sérialisable, remplaçable par un fichier de configuration.

use serde::{Deserialize, Serialize};
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::BettingStructure;
use stratyx_poker::position::Position;

use crate::profile::{PlayerProfile, StatKind};
use crate::stat::{Interval, Reliability, ReliabilityThresholds};

/// Structure de mise, sans ses paramètres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    NoLimit,
    PotLimit,
    FixedLimit,
}

impl From<BettingStructure> for StructureKind {
    fn from(s: BettingStructure) -> Self {
        match s {
            BettingStructure::NoLimit => StructureKind::NoLimit,
            BettingStructure::PotLimit => StructureKind::PotLimit,
            BettingStructure::FixedLimit { .. } => StructureKind::FixedLimit,
        }
    }
}

/// Formats visés par une bande ; un critère absent accepte toutes les valeurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FormatFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_type: Option<GameType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<StructureKind>,
}

impl FormatFilter {
    #[must_use]
    pub fn matches(&self, format: &GameFormat) -> bool {
        self.game_type.is_none_or(|g| g == format.game_type)
            && self.table_size.is_none_or(|t| t == format.table_size)
            && self
                .structure
                .is_none_or(|s| s == StructureKind::from(format.structure))
    }
}

/// Bande de référence d'une statistique : fréquences acceptables `[min, max]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceBand {
    pub stat: StatKind,
    #[serde(default)]
    pub format: FormatFilter,
    /// Position visée ; `None` : statistique toutes positions confondues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl ReferenceBand {
    #[must_use]
    pub fn new(
        stat: StatKind,
        format: FormatFilter,
        position: Option<Position>,
        min: f64,
        max: f64,
    ) -> ReferenceBand {
        ReferenceBand {
            stat,
            format,
            position,
            min: Some(min),
            max: Some(max),
        }
    }
}

/// Paramètres de la détection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeakConfig {
    pub bands: Vec<ReferenceBand>,
    pub thresholds: ReliabilityThresholds,
    /// Fiabilité minimale d'une statistique pour qu'une fuite soit signalée.
    pub min_reliability: Reliability,
}

impl Default for LeakConfig {
    /// Repères indicatifs pour le no-limit 6-max, fiabilité « fiable » exigée.
    fn default() -> Self {
        use Position::{BigBlind, Button, Cutoff, Hijack, SmallBlind, UnderTheGun};
        use StatKind::*;
        let six_max = FormatFilter {
            game_type: None,
            table_size: Some(6),
            structure: Some(StructureKind::NoLimit),
        };
        let band = |stat, position, min, max| ReferenceBand::new(stat, six_max, position, min, max);
        LeakConfig {
            bands: vec![
                band(Vpip, None, 0.18, 0.30),
                band(Pfr, None, 0.14, 0.25),
                band(Vpip, Some(UnderTheGun), 0.10, 0.22),
                band(Vpip, Some(Hijack), 0.13, 0.26),
                band(Vpip, Some(Cutoff), 0.20, 0.35),
                band(Vpip, Some(Button), 0.30, 0.55),
                band(Steal, Some(Cutoff), 0.20, 0.40),
                band(Steal, Some(Button), 0.35, 0.65),
                band(Steal, Some(SmallBlind), 0.25, 0.55),
                band(FoldToSteal, Some(BigBlind), 0.25, 0.60),
                band(ThreeBet, None, 0.04, 0.13),
                band(FoldToThreeBet, None, 0.35, 0.65),
                band(Limp, None, 0.0, 0.08),
                band(ColdCall, None, 0.0, 0.12),
                band(CbetFlop, None, 0.45, 0.80),
                band(FoldToCbetFlop, None, 0.30, 0.60),
                band(WentToShowdown, None, 0.22, 0.36),
                band(WonAtShowdown, None, 0.45, 0.62),
            ],
            thresholds: ReliabilityThresholds::default(),
            min_reliability: Reliability::Reliable,
        }
    }
}

impl LeakConfig {
    /// Configuration avec des bandes choisies et les seuils par défaut.
    #[must_use]
    pub fn with_bands(bands: Vec<ReferenceBand>) -> LeakConfig {
        LeakConfig {
            bands,
            ..LeakConfig::default()
        }
    }
}

/// Sens de l'écart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeakDirection {
    TooHigh,
    TooLow,
}

/// Fuite détectée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leak {
    pub stat: StatKind,
    pub format: GameFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    pub direction: LeakDirection,
    pub rate: f64,
    pub interval: Interval,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_max: Option<f64>,
    pub opportunities: u64,
    pub reliability: Reliability,
    /// L'intervalle de confiance entier est hors de la bande.
    pub significant: bool,
    /// Écart entre la fréquence observée et la borne franchie.
    pub gap: f64,
    /// Description en français.
    pub message: String,
}

/// Compare chaque format du profil aux bandes qui le concernent. Les fuites significatives
/// viennent en premier, puis les plus gros écarts.
#[must_use]
pub fn detect_leaks(profile: &PlayerProfile, config: &LeakConfig) -> Vec<Leak> {
    let mut leaks = Vec::new();
    for fmt in &profile.by_format {
        for band in config
            .bands
            .iter()
            .filter(|b| b.format.matches(&fmt.format))
        {
            let Some(line) = fmt.stats.line(band.position) else {
                continue;
            };
            let stat = line.get(band.stat);
            let reliability = stat.reliability(&config.thresholds);
            let (Some(rate), Some(interval)) = (stat.rate(), stat.wilson95()) else {
                continue;
            };
            if reliability < config.min_reliability {
                continue;
            }
            let (direction, gap, significant) = match (band.min, band.max) {
                (_, Some(max)) if rate > max => {
                    (LeakDirection::TooHigh, rate - max, interval.low > max)
                }
                (Some(min), _) if rate < min => {
                    (LeakDirection::TooLow, min - rate, interval.high < min)
                }
                _ => continue,
            };
            leaks.push(Leak {
                stat: band.stat,
                format: fmt.format,
                position: band.position,
                direction,
                rate,
                interval,
                band_min: band.min,
                band_max: band.max,
                opportunities: stat.opportunities,
                reliability,
                significant,
                gap,
                message: describe(
                    band,
                    &fmt.format,
                    direction,
                    rate,
                    interval,
                    stat.opportunities,
                ),
            });
        }
    }
    leaks.sort_by(|a, b| {
        b.significant
            .cmp(&a.significant)
            .then(b.gap.total_cmp(&a.gap))
    });
    leaks
}

fn pct(x: f64) -> String {
    format!("{:.1} %", x * 100.0).replace('.', ",")
}

fn describe(
    band: &ReferenceBand,
    format: &GameFormat,
    direction: LeakDirection,
    rate: f64,
    interval: Interval,
    opportunities: u64,
) -> String {
    let place = band.position.map_or_else(
        || "toutes positions".to_string(),
        |p| format!("position {p}"),
    );
    let side = match direction {
        LeakDirection::TooHigh => "au-dessus",
        LeakDirection::TooLow => "en dessous",
    };
    let bounds = match (band.min, band.max) {
        (Some(a), Some(b)) => format!("de {} à {}", pct(a), pct(b)),
        (Some(a), None) => format!("au moins {}", pct(a)),
        (None, Some(b)) => format!("au plus {}", pct(b)),
        (None, None) => "sans borne".to_string(),
    };
    format!(
        "{} ({}, {format}) {side} de la bande de référence : {} pour {bounds} \
         (IC 95 % {}–{}, {opportunities} opportunités)",
        band.stat.abbreviation(),
        place,
        pct(rate),
        pct(interval.low),
        pct(interval.high),
    )
}
