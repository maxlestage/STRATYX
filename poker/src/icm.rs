//! Modèle ICM (Malmuth-Harville) : valeur monétaire des tapis en tournoi.
//!
//! Calcul exact par programmation dynamique sur les sous-ensembles de joueurs déjà classés :
//! `O(2^n · n)` au pire, élagué au nombre de places payées. Exact jusqu'à 20 joueurs.

use stratyx_shared::Chips;
use thiserror::Error;

pub const MAX_ICM_PLAYERS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IcmError {
    #[error("il faut entre 1 et {MAX_ICM_PLAYERS} joueurs (reçu : {0})")]
    PlayerCount(usize),
    #[error("le joueur {0} a un tapis nul")]
    EmptyStack(usize),
}

/// Probabilités de finir à chaque place : `result[joueur][place]` (place 0 = premier).
/// Seules les `places` premières places sont calculées.
pub fn finish_probabilities(stacks: &[Chips], places: usize) -> Result<Vec<Vec<f64>>, IcmError> {
    let n = stacks.len();
    if !(1..=MAX_ICM_PLAYERS).contains(&n) {
        return Err(IcmError::PlayerCount(n));
    }
    if let Some(i) = stacks.iter().position(|s| s.is_zero()) {
        return Err(IcmError::EmptyStack(i));
    }
    let places = places.min(n);
    let s: Vec<f64> = stacks.iter().map(|c| c.get() as f64).collect();
    let total: f64 = s.iter().sum();
    let size = 1usize << n;
    let mut prob = vec![0.0f64; size];
    let mut placed_sum = vec![0.0f64; size];
    let mut finish = vec![vec![0.0f64; places]; n];
    prob[0] = 1.0;
    for mask in 0..size {
        if mask != 0 {
            let low = mask.trailing_zeros() as usize;
            placed_sum[mask] = placed_sum[mask & (mask - 1)] + s[low];
        }
        let p = prob[mask];
        let k = mask.count_ones() as usize;
        if p == 0.0 || k >= places {
            continue;
        }
        let remaining = total - placed_sum[mask];
        for j in (0..n).filter(|j| mask & (1 << j) == 0) {
            let q = p * s[j] / remaining;
            finish[j][k] += q;
            prob[mask | (1 << j)] += q;
        }
    }
    Ok(finish)
}

/// Équité ICM de chaque joueur, dans l'unité des `payouts` (montants ou fractions du
/// prize pool). `payouts[0]` est le prix du premier.
pub fn icm_equities(stacks: &[Chips], payouts: &[f64]) -> Result<Vec<f64>, IcmError> {
    let finish = finish_probabilities(stacks, payouts.len())?;
    Ok(finish
        .iter()
        .map(|places| places.iter().zip(payouts).map(|(p, v)| p * v).sum())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chips(v: &[u64]) -> Vec<Chips> {
        v.iter().map(|c| Chips(*c)).collect()
    }

    #[test]
    fn winner_takes_all_is_chip_proportional() {
        let eq = icm_equities(&chips(&[500, 300, 200]), &[1.0]).unwrap();
        for (e, x) in eq.iter().zip([0.5, 0.3, 0.2]) {
            assert!((e - x).abs() < 1e-12);
        }
    }

    #[test]
    fn known_three_player_values() {
        let eq = icm_equities(&chips(&[5000, 3000, 2000]), &[0.5, 0.3, 0.2]).unwrap();
        // Valeur calculée à la main pour le meneur : 0,383929.
        assert!((eq[0] - 0.383_928_571).abs() < 1e-6, "{}", eq[0]);
        assert!((eq.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(eq[0] > eq[1] && eq[1] > eq[2]);
    }

    #[test]
    fn equal_stacks_share_equally_and_probabilities_sum_to_one() {
        let n = 9;
        let stacks = vec![Chips(1000); n];
        let eq = icm_equities(&stacks, &[50.0, 30.0, 20.0]).unwrap();
        for e in &eq {
            assert!((e - 100.0 / n as f64).abs() < 1e-9);
        }
        let f = finish_probabilities(&chips(&[700, 200, 100]), 3).unwrap();
        for place in 0..3 {
            let s: f64 = f.iter().map(|p| p[place]).sum();
            assert!((s - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn errors() {
        assert_eq!(icm_equities(&[], &[1.0]), Err(IcmError::PlayerCount(0)));
        assert_eq!(
            icm_equities(&chips(&[10, 0]), &[1.0]),
            Err(IcmError::EmptyStack(1))
        );
    }
}
