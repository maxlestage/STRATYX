//! Analyse des mains de Texas Hold'em : profils de joueurs, résultats personnels, analyse de
//! décision et détection de fuites.
//!
//! Toutes les analyses partent d'historiques [`HandRecord`] **validés par rejeu** dans le
//! moteur de règles (`stratyx_poker`). Aucune carte non observée n'est jamais inventée : ce
//! qui dépend de cartes inconnues est déclaré non calculé.
//!
//! | Module | Rôle |
//! |---|---|
//! | [`stat`] | compteurs occurrences / opportunités, intervalle de Wilson, fiabilité |
//! | [`profile`] | profils (VPIP, PFR, 3-bet, steal, c-bet, AF, WTSD…) globaux, par position et par format ; confrontations héros ↔ adversaire |
//! | [`results`] | gains en jetons et en BB, BB/100, par position et par format, courbe cumulée, gains ajustés à l'all-in |
//! | [`tournament`] | ROI, ITM, gains et multiplicateurs Expresso |
//! | [`decision`] | analyse de chaque décision : cotes, SPR, équité, EV, fold equity, ICM ; point d'extension vers les IA |
//! | [`leaks`] | fuites du héros par rapport à des bandes de référence par format et position |
//!
//! Le calcul est **incrémental** : [`Analyzer::add_hand`] (ou [`ProfileBook::add_hand`],
//! [`HeroResults::add_hand`]) intègre une main sans rien recalculer, et l'état complet se
//! sérialise pour reprendre plus tard. Toutes les sorties sont sérialisables (serde).

pub mod decision;
pub mod error;
mod hand;
pub mod leaks;
pub mod profile;
pub mod results;
pub mod stat;
pub mod tournament;

use serde::{Deserialize, Serialize};
use stratyx_poker::equity::EquityOptions;
use stratyx_poker::history::HandRecord;

pub use decision::{
    DecisionOptions, DecisionReport, DecisionSpot, ReferenceAdvice, ReferenceStrategy,
    analyze_decisions, analyze_hero_decisions,
};
pub use error::AnalyticsError;
pub use leaks::{Leak, LeakConfig, ReferenceBand, detect_leaks};
pub use profile::{HeadToHead, PlayerProfile, ProfileBook, StatKind, StatLine};
pub use results::{HeroResults, ResultsSummary};
pub use stat::{Interval, Reliability, ReliabilityThresholds, Stat, StatSummary};
pub use tournament::{TournamentLedger, TournamentTotals};

use crate::hand::ParsedHand;

/// Profils de tous les joueurs et résultats du héros, alimentés ensemble : chaque main
/// n'est rejouée qu'une fois.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Analyzer {
    profiles: ProfileBook,
    results: HeroResults,
}

impl Analyzer {
    #[must_use]
    pub fn new() -> Analyzer {
        Analyzer::default()
    }

    /// Paramètres d'équité pour les gains ajustés à l'all-in.
    #[must_use]
    pub fn with_equity_options(options: EquityOptions) -> Analyzer {
        Analyzer {
            profiles: ProfileBook::new(),
            results: HeroResults::new(options),
        }
    }

    /// Intègre une main : profils de tous les joueurs servis et, si un héros est désigné,
    /// ses confrontations et ses résultats.
    pub fn add_hand(&mut self, record: &HandRecord) -> Result<(), AnalyticsError> {
        if self.profiles.contains(&record.id) {
            return Err(AnalyticsError::DuplicateHand(record.id.clone()));
        }
        let hand = ParsedHand::parse(record, false)?;
        self.profiles.add_parsed(&hand);
        self.results.add_parsed(&hand);
        Ok(())
    }

    /// Intègre plusieurs mains ; les mains refusées sont ignorées et leurs erreurs renvoyées.
    pub fn add_hands<'a>(
        &mut self,
        records: impl IntoIterator<Item = &'a HandRecord>,
    ) -> Vec<AnalyticsError> {
        records
            .into_iter()
            .filter_map(|r| self.add_hand(r).err())
            .collect()
    }

    #[must_use]
    pub fn profiles(&self) -> &ProfileBook {
        &self.profiles
    }

    #[must_use]
    pub fn results(&self) -> &HeroResults {
        &self.results
    }

    /// Fuites du joueur `player` (en pratique, le héros).
    #[must_use]
    pub fn leaks(&self, player: &str, config: &LeakConfig) -> Vec<Leak> {
        self.profiles
            .profile(player)
            .map(|p| detect_leaks(p, config))
            .unwrap_or_default()
    }
}
