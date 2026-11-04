//! Formules de base : cotes du pot, fold equity, espérance (EV), MDF.
//!
//! Convention : `pot` désigne le pot **avant** la décision analysée, mise adverse incluse
//! lorsqu'il s'agit de suivre.

/// Équité minimale pour qu'un call soit rentable : `call / (pot + call)`.
#[must_use]
pub fn required_equity(pot: f64, call: f64) -> f64 {
    if pot + call <= 0.0 {
        0.0
    } else {
        call / (pot + call)
    }
}

/// EV d'un call par rapport à un fold : `équité × (pot + call) − call`.
#[must_use]
pub fn call_ev(equity: f64, pot: f64, call: f64) -> f64 {
    equity * (pot + call) - call
}

/// Taux de fold adverse nécessaire pour qu'un bluff pur (0 % d'équité) soit rentable :
/// `mise / (pot + mise)`.
#[must_use]
pub fn bluff_break_even(pot: f64, bet: f64) -> f64 {
    if pot + bet <= 0.0 {
        0.0
    } else {
        bet / (pot + bet)
    }
}

/// Fréquence minimale de défense face à une mise : `pot / (pot + mise)`.
#[must_use]
pub fn minimum_defense_frequency(pot: f64, bet: f64) -> f64 {
    if pot + bet <= 0.0 {
        0.0
    } else {
        pot / (pot + bet)
    }
}

/// EV d'une mise (ou d'un tapis) avec fold equity, en modèle simplifié à un adversaire :
/// l'adversaire se couche avec la probabilité `fold`, sinon il paie et l'on réalise
/// `equity_when_called` sur le pot final.
#[must_use]
pub fn bet_ev(fold: f64, pot: f64, bet: f64, call: f64, equity_when_called: f64) -> f64 {
    let when_called = equity_when_called * (pot + bet + call) - bet;
    fold * pot + (1.0 - fold) * when_called
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_values() {
        // Mise d'un demi-pot : 100 dans un pot de 200 → pot à 300, il faut 25 % d'équité.
        assert!((required_equity(300.0, 100.0) - 0.25).abs() < 1e-12);
        assert!((minimum_defense_frequency(200.0, 100.0) - 2.0 / 3.0).abs() < 1e-12);
        assert!((bluff_break_even(200.0, 100.0) - 1.0 / 3.0).abs() < 1e-12);
        assert!(call_ev(0.25, 300.0, 100.0).abs() < 1e-12);
        assert!(call_ev(0.40, 300.0, 100.0) > 0.0);
        // Bluff pur au seuil de rentabilité : EV nulle.
        assert!(bet_ev(1.0 / 3.0, 200.0, 100.0, 100.0, 0.0).abs() < 1e-12);
    }
}
