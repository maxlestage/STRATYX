//! Règlement des pots (pots secondaires inclus) et modèle de gain : chipEV ou ICM.

use serde::{Deserialize, Serialize};
use stratyx_poker::icm::icm_equities;
use stratyx_shared::Chips;

/// Nombre de jetons par grosse blinde utilisé pour convertir les tapis vers l'ICM (entiers).
const CHIPS_PER_BB: f64 = 1_000_000.0;

/// Tapis considéré comme nul (joueur éliminé), en BB.
const BUST_EPS: f64 = 1e-9;

/// Modèle de gain des solveurs push/fold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PayoffModel {
    /// Espérance en jetons, exprimée en grosses blindes.
    ChipEv,
    /// Équité ICM (Malmuth-Harville) des tapis après la main. `payouts[0]` est le prix du
    /// premier ; `[1.0]` correspond au winner-take-all de l'Expresso classique.
    ///
    /// Les valeurs sont converties en **équivalent-BB** : équité ICM × (somme des tapis en BB
    /// / somme des prix), pour rester comparables à la chipEV (elles lui sont égales en
    /// winner-take-all).
    Icm { payouts: Vec<f64> },
}

impl PayoffModel {
    /// Vérifie la structure de prix (au moins un prix, tous finis et positifs, somme > 0).
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            PayoffModel::ChipEv => Ok(()),
            PayoffModel::Icm { payouts } => {
                if payouts.is_empty() {
                    return Err("au moins un prix est nécessaire".into());
                }
                if payouts.iter().any(|p| !p.is_finite() || *p < 0.0) {
                    return Err("les prix doivent être finis et positifs".into());
                }
                if payouts.iter().sum::<f64>() <= 0.0 {
                    return Err("la somme des prix doit être strictement positive".into());
                }
                Ok(())
            }
        }
    }

    /// Valeur de chaque joueur pour des tapis finaux donnés, relative aux tapis initiaux.
    pub(crate) fn values(&self, initial: &[f64], final_stacks: &[f64]) -> Vec<f64> {
        match self {
            PayoffModel::ChipEv => final_stacks
                .iter()
                .zip(initial)
                .map(|(f, s)| f - s)
                .collect(),
            PayoffModel::Icm { payouts } => {
                let total_chips: f64 = initial.iter().sum();
                let scale = total_chips / payouts.iter().sum::<f64>();
                let before = icm_with_busts(initial, initial, payouts);
                let after = icm_with_busts(final_stacks, initial, payouts);
                after
                    .iter()
                    .zip(&before)
                    .map(|(a, b)| (a - b) * scale)
                    .collect()
            }
        }
    }
}

/// ICM des joueurs encore en jeu ; les joueurs éliminés prennent les dernières places, le plus
/// gros tapis de départ finissant devant (partage des prix en cas d'égalité de tapis).
fn icm_with_busts(stacks: &[f64], initial: &[f64], payouts: &[f64]) -> Vec<f64> {
    let n = stacks.len();
    let mut out = vec![0.0; n];
    let survivors: Vec<usize> = (0..n).filter(|&i| stacks[i] > BUST_EPS).collect();
    if !survivors.is_empty() {
        let chips: Vec<Chips> = survivors
            .iter()
            .map(|&i| Chips(((stacks[i] * CHIPS_PER_BB).round() as u64).max(1)))
            .collect();
        let eq = icm_equities(&chips, payouts).expect("au plus 20 joueurs, tapis non nuls");
        for (k, &i) in survivors.iter().enumerate() {
            out[i] = eq[k];
        }
    }
    let mut busted: Vec<usize> = (0..n).filter(|&i| stacks[i] <= BUST_EPS).collect();
    busted.sort_by(|&a, &b| initial[b].total_cmp(&initial[a]));
    let mut place = survivors.len();
    let mut k = 0;
    while k < busted.len() {
        let mut end = k + 1;
        while end < busted.len() && initial[busted[end]] == initial[busted[k]] {
            end += 1;
        }
        let group = end - k;
        let prize = (place..place + group)
            .map(|p| payouts.get(p).copied().unwrap_or(0.0))
            .sum::<f64>()
            / group as f64;
        for &i in &busted[k..end] {
            out[i] = prize;
        }
        place += group;
        k = end;
    }
    out
}

/// Jetons récupérés par chaque joueur à l'issue de la main.
///
/// `contrib` : mise totale de chaque joueur (blindes et antes comprises) ; `active` : joueurs
/// non couchés ; `strength` : force de la main des joueurs actifs (plus grand = meilleur).
/// Les pots sont découpés par niveaux de mise (pots secondaires) ; une tranche que personne
/// d'actif ne couvre (mise non suivie) revient à ceux qui l'ont misée.
pub(crate) fn settle(contrib: &[f64], active: &[bool], strength: &[u8]) -> Vec<f64> {
    let n = contrib.len();
    let mut levels = contrib.to_vec();
    levels.sort_by(f64::total_cmp);
    levels.dedup();
    let mut won = vec![0.0; n];
    let mut prev = 0.0f64;
    for &level in &levels {
        if level <= prev {
            continue;
        }
        let part = |i: usize| contrib[i].min(level) - contrib[i].min(prev);
        let eligible: Vec<usize> = (0..n)
            .filter(|&i| active[i] && contrib[i] >= level)
            .collect();
        if eligible.is_empty() {
            for (i, w) in won.iter_mut().enumerate() {
                *w += part(i);
            }
        } else {
            let slice: f64 = (0..n).map(part).sum();
            let best = eligible.iter().map(|&i| strength[i]).max().unwrap_or(0);
            let winners: Vec<usize> = eligible
                .into_iter()
                .filter(|&i| strength[i] == best)
                .collect();
            let share = slice / winners.len() as f64;
            for i in winners {
                won[i] += share;
            }
        }
        prev = level;
    }
    won
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_pots_and_uncalled_bets() {
        // BTN 5, SB 10, BB 20 à tapis ; le BB a la meilleure main, puis le BTN.
        let won = settle(&[5.0, 10.0, 20.0], &[true; 3], &[2, 1, 3]);
        assert_eq!(won, vec![0.0, 0.0, 35.0]);
        // Le BTN gagne le pot principal (15), le SB le pot secondaire (10), le BB récupère
        // ses 10 non suivis.
        let won = settle(&[5.0, 10.0, 20.0], &[true; 3], &[3, 2, 1]);
        assert_eq!(won, vec![15.0, 10.0, 10.0]);
        // Égalité au pot principal entre BTN et BB.
        let won = settle(&[5.0, 10.0, 20.0], &[true; 3], &[3, 1, 3]);
        assert_eq!(won, vec![7.5, 0.0, 27.5]);
        // Mise non suivie : le SB se couche, le BB récupère tout.
        let won = settle(&[0.0, 0.5, 10.0], &[false, false, true], &[0, 0, 0]);
        assert_eq!(won, vec![0.0, 0.0, 10.5]);
    }

    #[test]
    fn chip_ev_and_icm_values() {
        let init = [10.0, 10.0, 10.0];
        let fin = [20.0, 0.0, 10.0];
        assert_eq!(
            PayoffModel::ChipEv.values(&init, &fin),
            vec![10.0, -10.0, 0.0]
        );
        // Winner-take-all : l'ICM en équivalent-BB coïncide avec la chipEV.
        let wta = PayoffModel::Icm { payouts: vec![1.0] };
        for (a, b) in wta.values(&init, &fin).iter().zip([10.0, -10.0, 0.0]) {
            assert!((a - b).abs() < 1e-9, "{a} {b}");
        }
        // 50/30/20 : doubler vaut moins que ce que perd l'éliminé (pression ICM).
        let icm = PayoffModel::Icm {
            payouts: vec![0.5, 0.3, 0.2],
        };
        let v = icm.values(&init, &fin);
        assert!(v[0] < 10.0 && v[1] > -10.0, "{v:?}");
        assert!((v.iter().sum::<f64>()).abs() < 1e-9);
        // Deux éliminés dans la même main : le plus gros tapis de départ finit deuxième.
        let v = icm_with_busts(&[30.0, 0.0, 0.0], &[10.0, 12.0, 8.0], &[0.5, 0.3, 0.2]);
        assert_eq!(v, vec![0.5, 0.3, 0.2]);
        let v = icm_with_busts(&[30.0, 0.0, 0.0], &[10.0, 10.0, 10.0], &[0.5, 0.3, 0.2]);
        assert_eq!(v, vec![0.5, 0.25, 0.25]);
    }

    #[test]
    fn payout_validation() {
        assert!(PayoffModel::ChipEv.validate().is_ok());
        assert!(PayoffModel::Icm { payouts: vec![] }.validate().is_err());
        assert!(
            PayoffModel::Icm {
                payouts: vec![-1.0]
            }
            .validate()
            .is_err()
        );
        assert!(PayoffModel::Icm { payouts: vec![0.0] }.validate().is_err());
        assert!(
            PayoffModel::Icm {
                payouts: vec![0.6, 0.4]
            }
            .validate()
            .is_ok()
        );
    }
}
