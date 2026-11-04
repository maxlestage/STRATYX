//! Résultats de tournois, sit & go et Expresso : ROI, ITM, gains, multiplicateurs.
//!
//! Les montants ([`TournamentResult`]) sont en centimes d'une devise : les totaux sont
//! tenus **par devise**, jamais additionnés entre devises, puis détaillés par type de
//! partie quand il est connu (via [`TournamentLedger::add_session`]).
//!
//! - **ROI** = (gains − droits d'entrée) / droits d'entrée, frais compris.
//! - **ITM** = part des tournois terminés dans les places payées (gain non nul), avec son
//!   intervalle de Wilson.
//! - **Multiplicateurs** (Expresso) : fréquence de chaque multiplicateur tiré, gains et
//!   bénéfice associés, multiplicateur moyen.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use stratyx_poker::format::GameType;
use stratyx_poker::session::{Session, TournamentResult};

use crate::stat::Stat;

/// Totaux pour un multiplicateur donné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MultiplierTotals {
    pub entries: u64,
    pub buy_ins: u64,
    pub prizes: u64,
}

impl MultiplierTotals {
    #[must_use]
    pub fn profit(&self) -> i64 {
        self.prizes as i64 - self.buy_ins as i64
    }
}

/// Totaux d'un ensemble de tournois dans une devise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentTotals {
    /// Code ISO 4217.
    pub currency: String,
    /// Type de partie ; `None` : tous types confondus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_type: Option<GameType>,
    pub entries: u64,
    /// Droits d'entrée payés, frais compris (centimes).
    pub buy_ins: u64,
    /// Part des frais (centimes).
    pub fees: u64,
    /// Gains (centimes).
    pub prizes: u64,
    /// Places payées / tournois.
    pub itm: Stat,
    /// Victoires / tournois dont la place finale est connue.
    pub wins: Stat,
    /// Répartition des multiplicateurs tirés (Expresso).
    pub multipliers: BTreeMap<u32, MultiplierTotals>,
}

impl TournamentTotals {
    fn new(currency: &str, game_type: Option<GameType>) -> TournamentTotals {
        TournamentTotals {
            currency: currency.to_string(),
            game_type,
            entries: 0,
            buy_ins: 0,
            fees: 0,
            prizes: 0,
            itm: Stat::default(),
            wins: Stat::default(),
            multipliers: BTreeMap::new(),
        }
    }

    fn add(&mut self, r: &TournamentResult) {
        self.entries += 1;
        self.buy_ins += r.buy_in;
        self.fees += r.fee;
        self.prizes += r.prize;
        self.itm.record(true, r.in_the_money());
        if let Some(finish) = r.finish {
            self.wins.record(true, finish == 1);
        }
        if let Some(m) = r.prize_multiplier {
            let t = self.multipliers.entry(m).or_default();
            t.entries += 1;
            t.buy_ins += r.buy_in;
            t.prizes += r.prize;
        }
    }

    /// Bénéfice net (centimes).
    #[must_use]
    pub fn profit(&self) -> i64 {
        self.prizes as i64 - self.buy_ins as i64
    }

    /// Retour sur investissement ; `None` sans droit d'entrée payé.
    #[must_use]
    pub fn roi(&self) -> Option<f64> {
        (self.buy_ins > 0).then(|| self.profit() as f64 / self.buy_ins as f64)
    }

    /// Droit d'entrée moyen (centimes).
    #[must_use]
    pub fn average_buy_in(&self) -> Option<f64> {
        (self.entries > 0).then(|| self.buy_ins as f64 / self.entries as f64)
    }

    /// Multiplicateur moyen tiré, sur les tournois où il est connu.
    #[must_use]
    pub fn average_multiplier(&self) -> Option<f64> {
        let (n, sum) = self
            .multipliers
            .iter()
            .fold((0u64, 0u64), |(n, s), (m, t)| {
                (n + t.entries, s + u64::from(*m) * t.entries)
            });
        (n > 0).then(|| sum as f64 / n as f64)
    }

    /// Fréquence d'un multiplicateur parmi les tournois où il est connu.
    #[must_use]
    pub fn multiplier_frequency(&self, multiplier: u32) -> Stat {
        let n = self.multipliers.values().map(|t| t.entries).sum();
        Stat::new(
            self.multipliers.get(&multiplier).map_or(0, |t| t.entries),
            n,
        )
    }
}

/// Registre des tournois du héros, alimenté au fil des sessions.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TournamentLedger {
    totals: Vec<TournamentTotals>,
}

impl TournamentLedger {
    #[must_use]
    pub fn new() -> TournamentLedger {
        TournamentLedger::default()
    }

    /// Ajoute un résultat dont le type de partie est inconnu.
    pub fn add(&mut self, result: &TournamentResult) {
        self.add_with_type(result, None);
    }

    /// Ajoute le résultat d'une session de tournoi ; renvoie `false` si la session n'en
    /// contient pas (cash game).
    pub fn add_session(&mut self, session: &Session) -> bool {
        match &session.tournament {
            Some(r) => {
                self.add_with_type(r, Some(session.format.game_type));
                true
            }
            None => false,
        }
    }

    fn add_with_type(&mut self, result: &TournamentResult, game_type: Option<GameType>) {
        self.entry(&result.currency, None).add(result);
        if game_type.is_some() {
            self.entry(&result.currency, game_type).add(result);
        }
    }

    fn entry(&mut self, currency: &str, game_type: Option<GameType>) -> &mut TournamentTotals {
        let idx = match self
            .totals
            .iter()
            .position(|t| t.currency == currency && t.game_type == game_type)
        {
            Some(i) => i,
            None => {
                self.totals.push(TournamentTotals::new(currency, game_type));
                self.totals.len() - 1
            }
        };
        &mut self.totals[idx]
    }

    /// Totaux d'une devise, tous types confondus.
    #[must_use]
    pub fn totals(&self, currency: &str) -> Option<&TournamentTotals> {
        self.by_type(currency, None)
    }

    /// Totaux d'une devise pour un type de partie (`None` : tous types).
    #[must_use]
    pub fn by_type(
        &self,
        currency: &str,
        game_type: Option<GameType>,
    ) -> Option<&TournamentTotals> {
        self.totals
            .iter()
            .find(|t| t.currency == currency && t.game_type == game_type)
    }

    /// Tous les totaux : chaque devise, puis le détail par type de partie.
    #[must_use]
    pub fn all(&self) -> &[TournamentTotals] {
        &self.totals
    }
}

#[cfg(test)]
mod tests {
    use stratyx_poker::format::GameFormat;
    use stratyx_poker::session::SessionSource;

    use super::*;

    fn expresso(prize: u64, multiplier: u32, finish: u32) -> TournamentResult {
        TournamentResult {
            buy_in: 500,
            fee: 35,
            prize,
            currency: "EUR".into(),
            finish: Some(finish),
            entrants: 3,
            prize_multiplier: Some(multiplier),
            payouts: vec![500 * u64::from(multiplier)],
        }
    }

    #[test]
    fn roi_itm_and_multipliers() {
        let mut ledger = TournamentLedger::new();
        // Quatre Expresso à 5 € : gagnée ×2, perdue ×2, perdue ×2, gagnée ×10.
        for r in [
            expresso(1_000, 2, 1),
            expresso(0, 2, 2),
            expresso(0, 2, 3),
            expresso(5_000, 10, 1),
        ] {
            let session = Session {
                id: "s".into(),
                source: SessionSource::VideoImport,
                format: GameFormat::EXPRESSO,
                started_at_ms: 0,
                ended_at_ms: Some(1),
                hand_ids: vec![],
                tournament: Some(r),
            };
            assert!(ledger.add_session(&session));
        }
        // Un tournoi en dollars reste séparé.
        ledger.add(&TournamentResult {
            buy_in: 1_100,
            fee: 100,
            prize: 0,
            currency: "USD".into(),
            finish: None,
            entrants: 180,
            prize_multiplier: None,
            payouts: vec![],
        });

        let eur = ledger.totals("EUR").unwrap();
        assert_eq!(eur.entries, 4);
        assert_eq!(eur.buy_ins, 2_000);
        assert_eq!(eur.fees, 140);
        assert_eq!(eur.prizes, 6_000);
        assert_eq!(eur.profit(), 4_000);
        assert_eq!(eur.roi(), Some(2.0));
        assert_eq!(eur.itm, Stat::new(2, 4));
        assert_eq!(eur.wins, Stat::new(2, 4));
        assert_eq!(eur.average_multiplier(), Some(4.0));
        assert_eq!(eur.multiplier_frequency(2), Stat::new(3, 4));
        let x2 = eur.multipliers[&2];
        assert_eq!((x2.entries, x2.profit()), (3, -500));
        assert_eq!(eur.multipliers[&10].profit(), 4_500);
        assert_eq!(
            ledger
                .by_type("EUR", Some(GameType::Expresso))
                .unwrap()
                .entries,
            4
        );

        let usd = ledger.totals("USD").unwrap();
        assert_eq!(usd.roi(), Some(-1.0));
        assert_eq!(usd.wins, Stat::default(), "place inconnue");
        assert!(ledger.by_type("USD", Some(GameType::Tournament)).is_none());
        assert_eq!(ledger.all().len(), 3);

        let json = serde_json::to_string(&ledger).unwrap();
        assert_eq!(
            serde_json::from_str::<TournamentLedger>(&json).unwrap(),
            ledger
        );
    }
}
