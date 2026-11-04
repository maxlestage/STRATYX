//! Pots principal et secondaires, mises non suivies, et répartition des gains.

use serde::{Deserialize, Serialize};
use stratyx_shared::Chips;
use thiserror::Error;

use crate::eval::HandRank;

/// Mise totale d'un joueur sur la main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contribution {
    pub player: usize,
    /// Jetons engagés et soumis à l'égalisation (blindes, mises, antes classiques).
    pub live: Chips,
    /// Argent mort non soumis à l'égalisation (ex. : ante de grosse blinde), versé au pot
    /// principal.
    pub dead: Chips,
    pub folded: bool,
}

/// Un pot et les joueurs qui peuvent le remporter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pot {
    pub amount: Chips,
    /// Joueurs éligibles, triés.
    pub eligible: Vec<usize>,
}

/// Découpage complet des jetons engagés.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotBreakdown {
    /// Pot principal en premier, puis pots secondaires.
    pub pots: Vec<Pot>,
    /// Mise non suivie rendue à son auteur : `(joueur, montant)`.
    pub uncalled: Option<(usize, Chips)>,
}

impl PotBreakdown {
    #[must_use]
    pub fn total(&self) -> Chips {
        self.pots.iter().map(|p| p.amount).sum()
    }
}

/// Construit les pots à partir des mises de chaque joueur.
///
/// 1. La part de la plus grosse mise qui n'a été égalée par personne est rendue.
/// 2. Chaque niveau de mise distinct d'un joueur encore en jeu crée un pot, auquel sont
///    éligibles les joueurs en jeu ayant misé au moins ce niveau.
/// 3. L'argent mort rejoint le pot principal.
#[must_use]
pub fn build_pots(contributions: &[Contribution]) -> PotBreakdown {
    let mut live: Vec<u64> = contributions.iter().map(|c| c.live.get()).collect();

    // 1. Mise non suivie.
    let mut uncalled = None;
    if let Some((top_idx, &top)) = live.iter().enumerate().max_by_key(|(_, v)| **v) {
        let second = live
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != top_idx)
            .map(|(_, v)| *v)
            .max()
            .unwrap_or(0);
        if top > second {
            uncalled = Some((contributions[top_idx].player, Chips(top - second)));
            live[top_idx] = second;
        }
    }

    // 2. Niveaux des joueurs encore en jeu.
    let mut levels: Vec<u64> = contributions
        .iter()
        .zip(&live)
        .filter(|(c, v)| !c.folded && **v > 0)
        .map(|(_, v)| *v)
        .collect();
    levels.sort_unstable();
    levels.dedup();

    let in_play: Vec<usize> = {
        let mut v: Vec<usize> = contributions
            .iter()
            .filter(|c| !c.folded)
            .map(|c| c.player)
            .collect();
        v.sort_unstable();
        v
    };

    let mut pots = Vec::new();
    let mut prev = 0u64;
    for level in levels {
        let amount: u64 = live.iter().map(|v| (*v).min(level) - (*v).min(prev)).sum();
        let mut eligible: Vec<usize> = contributions
            .iter()
            .zip(&live)
            .filter(|(c, v)| !c.folded && **v >= level)
            .map(|(c, _)| c.player)
            .collect();
        eligible.sort_unstable();
        if amount > 0 {
            pots.push(Pot {
                amount: Chips(amount),
                eligible,
            });
        }
        prev = level;
    }

    // Reliquat de joueurs couchés au-delà du dernier niveau (cas limite) et argent mort.
    let leftover: u64 = live.iter().map(|v| v.saturating_sub(prev)).sum();
    let dead: u64 = contributions.iter().map(|c| c.dead.get()).sum();
    if leftover > 0 {
        match pots.last_mut() {
            Some(p) => p.amount += Chips(leftover),
            None => pots.push(Pot {
                amount: Chips(leftover),
                eligible: in_play.clone(),
            }),
        }
    }
    if dead > 0 {
        // L'argent mort revient au pot principal, ouvert à tous les joueurs en jeu.
        match pots.first_mut() {
            Some(p) if p.eligible == in_play => p.amount += Chips(dead),
            _ => pots.insert(
                0,
                Pot {
                    amount: Chips(dead),
                    eligible: in_play.clone(),
                },
            ),
        }
    }

    PotBreakdown { pots, uncalled }
}

/// Ce que l'on sait de la main d'un joueur au moment de l'abattage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "rank")]
pub enum Claim {
    /// Main montrée (ou connue : celle du joueur analysé).
    Shown(HandRank),
    /// Main jetée sans être montrée : le joueur renonce au pot.
    Mucked,
    /// Cartes non observées. Le moteur n'invente jamais de cartes : si cette information
    /// est nécessaire pour attribuer un pot, la répartition est déclarée non résolue.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AwardError {
    #[error("pot {pot} : les cartes du joueur {player} sont inconnues, gagnant indéterminable")]
    Unresolved { pot: usize, player: usize },
    #[error("pot {pot} : aucun joueur éligible n'a montré sa main")]
    NoClaimant { pot: usize },
}

/// Répartit les pots entre les gagnants.
///
/// `claims[p]` décrit la main du joueur `p`. `odd_chip_order` liste les joueurs dans
/// l'ordre de distribution des jetons indivisibles (en partant de la gauche du bouton).
/// Renvoie le gain brut de chaque joueur (hors mise non suivie).
pub fn award_pots(
    pots: &[Pot],
    claims: &[Claim],
    odd_chip_order: &[usize],
) -> Result<Vec<Chips>, AwardError> {
    let mut payouts = vec![Chips::ZERO; claims.len()];
    for (pot_idx, pot) in pots.iter().enumerate() {
        let winners: Vec<usize> = if pot.eligible.len() == 1 {
            pot.eligible.clone()
        } else {
            if let Some(&p) = pot.eligible.iter().find(|p| claims[**p] == Claim::Unknown) {
                return Err(AwardError::Unresolved {
                    pot: pot_idx,
                    player: p,
                });
            }
            let best = pot
                .eligible
                .iter()
                .filter_map(|p| match claims[*p] {
                    Claim::Shown(r) => Some(r),
                    _ => None,
                })
                .max()
                .ok_or(AwardError::NoClaimant { pot: pot_idx })?;
            pot.eligible
                .iter()
                .copied()
                .filter(|p| claims[*p] == Claim::Shown(best))
                .collect()
        };
        let share = pot.amount.get() / winners.len() as u64;
        let mut remainder = pot.amount.get() % winners.len() as u64;
        for w in &winners {
            payouts[*w] += Chips(share);
        }
        for p in odd_chip_order {
            if remainder == 0 {
                break;
            }
            if winners.contains(p) {
                payouts[*p] += Chips(1);
                remainder -= 1;
            }
        }
    }
    Ok(payouts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cardset::CardSet;
    use crate::eval::evaluate;

    fn c(player: usize, live: u64, folded: bool) -> Contribution {
        Contribution {
            player,
            live: Chips(live),
            dead: Chips::ZERO,
            folded,
        }
    }

    #[test]
    fn fold_to_big_blind_returns_uncalled() {
        let b = build_pots(&[c(0, 50, true), c(1, 100, false)]);
        assert_eq!(b.uncalled, Some((1, Chips(50))));
        assert_eq!(
            b.pots,
            vec![Pot {
                amount: Chips(100),
                eligible: vec![1]
            }]
        );
    }

    #[test]
    fn three_way_all_in_side_pots() {
        // A tapis 100, B tapis 300, C couvre et paie 300.
        let b = build_pots(&[c(0, 100, false), c(1, 300, false), c(2, 500, false)]);
        assert_eq!(b.uncalled, Some((2, Chips(200))));
        assert_eq!(
            b.pots,
            vec![
                Pot {
                    amount: Chips(300),
                    eligible: vec![0, 1, 2]
                },
                Pot {
                    amount: Chips(400),
                    eligible: vec![1, 2]
                },
            ]
        );
        assert_eq!(b.total(), Chips(700));
    }

    #[test]
    fn folded_money_stays_in_pots() {
        // D mise 200 puis se couche ; A tapis 100 ; B et C à 400.
        let b = build_pots(&[
            c(0, 100, false),
            c(1, 400, false),
            c(2, 400, false),
            c(3, 200, true),
        ]);
        assert_eq!(b.uncalled, None);
        assert_eq!(
            b.pots[0],
            Pot {
                amount: Chips(400),
                eligible: vec![0, 1, 2]
            }
        );
        assert_eq!(
            b.pots[1],
            Pot {
                amount: Chips(700),
                eligible: vec![1, 2]
            }
        );
        assert_eq!(b.total(), Chips(1100));
    }

    #[test]
    fn big_blind_ante_is_dead_money() {
        let mut bb = c(1, 100, false);
        bb.dead = Chips(100);
        let b = build_pots(&[c(0, 50, true), bb]);
        assert_eq!(b.uncalled, Some((1, Chips(50))));
        assert_eq!(
            b.pots,
            vec![Pot {
                amount: Chips(200),
                eligible: vec![1]
            }]
        );
    }

    #[test]
    fn award_with_split_and_odd_chip() {
        let pots = vec![Pot {
            amount: Chips(301),
            eligible: vec![0, 1, 2],
        }];
        let same = evaluate(CardSet::parse("AsKsQdJcTh").unwrap());
        let worse = evaluate(CardSet::parse("2s4s6d8cTh").unwrap());
        let claims = [Claim::Shown(same), Claim::Shown(same), Claim::Shown(worse)];
        // Le jeton indivisible va au premier gagnant à gauche du bouton : le joueur 1.
        let pay = award_pots(&pots, &claims, &[1, 2, 0]).unwrap();
        assert_eq!(pay, vec![Chips(150), Chips(151), Chips(0)]);
    }

    #[test]
    fn unknown_cards_are_never_invented() {
        let pots = vec![
            Pot {
                amount: Chips(300),
                eligible: vec![0, 1],
            },
            Pot {
                amount: Chips(200),
                eligible: vec![1],
            },
        ];
        let r = evaluate(CardSet::parse("AsKsQdJcTh").unwrap());
        let res = award_pots(&pots, &[Claim::Unknown, Claim::Shown(r)], &[0, 1]);
        assert_eq!(res, Err(AwardError::Unresolved { pot: 0, player: 0 }));
        // Un joueur qui jette ses cartes renonce au pot.
        let pay = award_pots(&pots, &[Claim::Mucked, Claim::Shown(r)], &[0, 1]).unwrap();
        assert_eq!(pay, vec![Chips(0), Chips(500)]);
    }
}
