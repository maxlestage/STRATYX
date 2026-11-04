//! Calcul d'équité entre ranges, avec board et cartes mortes.
//!
//! - **Énumération exacte** lorsque le volume de calcul le permet (mains précises, flop, turn…).
//! - **Monte-Carlo** sinon : échantillonnage parallèle (Rayon) et **déterministe** — à graine
//!   égale, le résultat est identique quel que soit le nombre de cœurs, ce qui rend les
//!   analyses et les évaluations des IA reproductibles.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::card::Card;
use crate::cardset::CardSet;
use crate::eval::evaluate;
use crate::range::{HoleCards, Range};

/// Nombre maximal de joueurs dans un calcul d'équité.
pub const MAX_PLAYERS: usize = 10;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EquityError {
    #[error("il faut entre 2 et {MAX_PLAYERS} joueurs (reçu : {0})")]
    PlayerCount(usize),
    #[error("le board doit contenir 0, 3, 4 ou 5 cartes (reçu : {0})")]
    BoardSize(usize),
    #[error("le board et les cartes mortes se chevauchent")]
    BoardDeadOverlap,
    #[error("la range du joueur {0} est vide une fois retirées les cartes connues")]
    EmptyRange(usize),
    #[error("aucune distribution compatible : les ranges se bloquent mutuellement")]
    NoValidDeal,
}

/// Paramètres du calcul.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EquityOptions {
    /// Au-delà de ce nombre d'évaluations, on bascule en Monte-Carlo.
    pub max_exact_evaluations: u64,
    /// Nombre de tirages en Monte-Carlo.
    pub samples: u64,
    /// Graine du générateur pseudo-aléatoire (reproductibilité).
    pub seed: u64,
}

impl Default for EquityOptions {
    fn default() -> Self {
        EquityOptions {
            max_exact_evaluations: 30_000_000,
            samples: 400_000,
            seed: 0x0053_5452_4154_5958, // « STRATYX »
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquityMethod {
    Exact,
    MonteCarlo,
}

/// Résultat pour un joueur. Toutes les valeurs sont des probabilités dans `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerEquity {
    /// Part du pot attendue (victoires + fractions des partages).
    pub equity: f64,
    /// Probabilité de gagner seul.
    pub win: f64,
    /// Probabilité de partager le pot.
    pub tie: f64,
    /// Erreur-type de l'équité (0 en calcul exact).
    pub std_error: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquityResult {
    pub players: Vec<PlayerEquity>,
    pub method: EquityMethod,
    /// Nombre de distributions évaluées (exact) ou de tirages acceptés (Monte-Carlo).
    pub deals: u64,
}

/// Accumulateur pondéré par joueur.
#[derive(Clone)]
struct Acc {
    weight: f64,
    equity: Vec<f64>,
    equity_sq: Vec<f64>,
    win: Vec<f64>,
    tie: Vec<f64>,
    deals: u64,
}

impl Acc {
    fn new(n: usize) -> Acc {
        Acc {
            weight: 0.0,
            equity: vec![0.0; n],
            equity_sq: vec![0.0; n],
            win: vec![0.0; n],
            tie: vec![0.0; n],
            deals: 0,
        }
    }

    fn merge(mut self, o: Acc) -> Acc {
        self.weight += o.weight;
        self.deals += o.deals;
        for i in 0..self.equity.len() {
            self.equity[i] += o.equity[i];
            self.equity_sq[i] += o.equity_sq[i];
            self.win[i] += o.win[i];
            self.tie[i] += o.tie[i];
        }
        self
    }

    /// Abattage : `holes` contient la main de chaque joueur, `board` est complet.
    #[inline]
    fn showdown(&mut self, holes: &[HoleCards], board: CardSet, weight: f64) {
        let mut ranks = [0u32; MAX_PLAYERS];
        let mut best = 0u32;
        for (i, h) in holes.iter().enumerate() {
            let r = evaluate(board | h.set()).value();
            ranks[i] = r;
            best = best.max(r);
        }
        let winners = ranks[..holes.len()].iter().filter(|r| **r == best).count();
        let share = 1.0 / winners as f64;
        for (i, rank) in ranks.iter().enumerate().take(holes.len()) {
            if *rank == best {
                self.equity[i] += weight * share;
                self.equity_sq[i] += weight * share * share;
                if winners == 1 {
                    self.win[i] += weight;
                } else {
                    self.tie[i] += weight;
                }
            }
        }
        self.weight += weight;
        self.deals += 1;
    }

    fn finish(self, method: EquityMethod) -> Result<EquityResult, EquityError> {
        if self.weight <= 0.0 {
            return Err(EquityError::NoValidDeal);
        }
        let w = self.weight;
        let players = (0..self.equity.len())
            .map(|i| {
                let mean = self.equity[i] / w;
                let std_error = match method {
                    EquityMethod::Exact => 0.0,
                    EquityMethod::MonteCarlo => {
                        let var = (self.equity_sq[i] / w - mean * mean).max(0.0);
                        (var / self.deals as f64).sqrt()
                    }
                };
                PlayerEquity {
                    equity: mean,
                    win: self.win[i] / w,
                    tie: self.tie[i] / w,
                    std_error,
                }
            })
            .collect();
        Ok(EquityResult {
            players,
            method,
            deals: self.deals,
        })
    }
}

/// Appelle `f` pour chaque sous-ensemble de `k` cartes de `cards`.
fn for_each_combination(cards: &[Card], k: usize, mut f: impl FnMut(CardSet)) {
    fn rec(cards: &[Card], k: usize, start: usize, acc: CardSet, f: &mut impl FnMut(CardSet)) {
        if k == 0 {
            f(acc);
            return;
        }
        for i in start..=cards.len() - k {
            rec(cards, k - 1, i + 1, acc | CardSet::from_card(cards[i]), f);
        }
    }
    if k <= cards.len() {
        rec(cards, k, 0, CardSet::EMPTY, &mut f);
    }
}

fn binomial(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    (0..k).fold(1u64, |acc, i| acc * (n - i) / (i + 1))
}

/// Calcule l'équité de chaque range face aux autres.
pub fn equity(
    ranges: &[Range],
    board: CardSet,
    dead: CardSet,
    options: &EquityOptions,
) -> Result<EquityResult, EquityError> {
    let n = ranges.len();
    if !(2..=MAX_PLAYERS).contains(&n) {
        return Err(EquityError::PlayerCount(n));
    }
    if !matches!(board.len(), 0 | 3 | 4 | 5) {
        return Err(EquityError::BoardSize(board.len()));
    }
    if board.intersects(dead) {
        return Err(EquityError::BoardDeadOverlap);
    }
    let known = board | dead;
    let combos: Vec<Vec<(HoleCards, f64)>> = ranges
        .iter()
        .map(|r| {
            r.iter()
                .filter(|(h, _)| !h.set().intersects(known))
                .map(|(h, w)| (h, f64::from(w)))
                .collect()
        })
        .collect();
    if let Some(i) = combos.iter().position(Vec::is_empty) {
        return Err(EquityError::EmptyRange(i));
    }

    let to_deal = 5 - board.len();
    let remaining = (52 - known.len() - 2 * n) as u64;
    let tuples = combos
        .iter()
        .try_fold(1u64, |acc, c| acc.checked_mul(c.len() as u64));
    let work = tuples
        .and_then(|t| t.checked_mul(binomial(remaining, to_deal as u64)))
        .and_then(|t| t.checked_mul(n as u64));
    match work {
        Some(w) if w <= options.max_exact_evaluations => exact(&combos, board, known, to_deal),
        _ => monte_carlo(&combos, board, known, to_deal, options),
    }
}

fn exact(
    combos: &[Vec<(HoleCards, f64)>],
    board: CardSet,
    known: CardSet,
    to_deal: usize,
) -> Result<EquityResult, EquityError> {
    let n = combos.len();
    let ctx = ExactCtx {
        combos,
        board,
        to_deal,
    };
    let acc = combos[0]
        .par_iter()
        .map(|(h, w)| {
            let mut acc = Acc::new(n);
            let mut holes = vec![*h];
            ctx.rec(1, known | h.set(), *w, &mut holes, &mut acc);
            acc
        })
        .reduce(|| Acc::new(n), Acc::merge);
    acc.finish(EquityMethod::Exact)
}

struct ExactCtx<'a> {
    combos: &'a [Vec<(HoleCards, f64)>],
    board: CardSet,
    to_deal: usize,
}

impl ExactCtx<'_> {
    fn rec(
        &self,
        depth: usize,
        used: CardSet,
        weight: f64,
        holes: &mut Vec<HoleCards>,
        acc: &mut Acc,
    ) {
        if depth == self.combos.len() {
            if self.to_deal == 0 {
                acc.showdown(holes, self.board, weight);
            } else {
                let deck: Vec<Card> = (!used).iter().collect();
                for_each_combination(&deck, self.to_deal, |extra| {
                    acc.showdown(holes, self.board | extra, weight);
                });
            }
            return;
        }
        for (h, w) in &self.combos[depth] {
            if h.set().intersects(used) {
                continue;
            }
            holes.push(*h);
            self.rec(depth + 1, used | h.set(), weight * w, holes, acc);
            holes.pop();
        }
    }
}

/// Tirage pondéré dans une liste de combinaisons (recherche dichotomique sur les cumuls).
struct WeightedCombos {
    combos: Vec<HoleCards>,
    cumulative: Vec<f64>,
}

impl WeightedCombos {
    fn new(list: &[(HoleCards, f64)]) -> WeightedCombos {
        let mut total = 0.0;
        let cumulative = list
            .iter()
            .map(|(_, w)| {
                total += w;
                total
            })
            .collect();
        WeightedCombos {
            combos: list.iter().map(|(h, _)| *h).collect(),
            cumulative,
        }
    }

    fn sample(&self, rng: &mut impl Rng) -> HoleCards {
        let total = *self.cumulative.last().expect("liste non vide");
        let x = rng.random::<f64>() * total;
        let i = self
            .cumulative
            .partition_point(|c| *c <= x)
            .min(self.combos.len() - 1);
        self.combos[i]
    }
}

fn monte_carlo(
    combos: &[Vec<(HoleCards, f64)>],
    board: CardSet,
    known: CardSet,
    to_deal: usize,
    options: &EquityOptions,
) -> Result<EquityResult, EquityError> {
    const CHUNK: u64 = 4096;
    let n = combos.len();
    let samplers: Vec<WeightedCombos> = combos.iter().map(|c| WeightedCombos::new(c)).collect();
    let deck: Vec<Card> = (!known).iter().collect();
    let chunks = options.samples.div_ceil(CHUNK).max(1);

    let acc = (0..chunks)
        .into_par_iter()
        .map(|chunk| {
            let mut rng = ChaCha8Rng::seed_from_u64(options.seed);
            rng.set_stream(chunk);
            let target = CHUNK
                .min(options.samples.saturating_sub(chunk * CHUNK))
                .max(1);
            let mut acc = Acc::new(n);
            let mut holes = Vec::with_capacity(n);
            let mut attempts = 0u64;
            while acc.deals < target {
                attempts += 1;
                // Garde-fou : ranges qui se bloquent (presque) toujours.
                if attempts > 1_000 * target && acc.deals == 0 {
                    break;
                }
                // Rejet de la distribution complète en cas de conflit : on échantillonne
                // ainsi exactement la loi jointe des ranges pondérées.
                holes.clear();
                let mut used = CardSet::EMPTY;
                let mut ok = true;
                for s in &samplers {
                    let h = s.sample(&mut rng);
                    if h.set().intersects(used) {
                        ok = false;
                        break;
                    }
                    used |= h.set();
                    holes.push(h);
                }
                if !ok {
                    continue;
                }
                let mut full = board;
                let mut dealt = 0;
                while dealt < to_deal {
                    let c = deck[rng.random_range(0..deck.len())];
                    if !used.contains(c) && !full.contains(c) {
                        full.insert(c);
                        dealt += 1;
                    }
                }
                acc.showdown(&holes, full, 1.0);
            }
            acc
        })
        .reduce(|| Acc::new(n), Acc::merge);
    acc.finish(EquityMethod::MonteCarlo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Range {
        Range::parse(s).unwrap()
    }

    fn cs(s: &str) -> CardSet {
        CardSet::parse(s).unwrap()
    }

    fn sum_equity(res: &EquityResult) -> f64 {
        res.players.iter().map(|p| p.equity).sum()
    }

    #[test]
    fn river_is_deterministic() {
        // Brelan de Rois contre paire d'As : aucune carte à venir.
        let res = equity(
            &[r("KsKh"), r("AsAh")],
            cs("Kd7c2h9s3d"),
            CardSet::EMPTY,
            &EquityOptions::default(),
        )
        .unwrap();
        assert_eq!(res.method, EquityMethod::Exact);
        assert_eq!(res.players[0].equity, 1.0);
        assert_eq!(res.players[1].equity, 0.0);
    }

    #[test]
    fn board_plays_is_a_split() {
        let res = equity(
            &[r("2c3d"), r("2h3s")],
            cs("AsKsQsJsTs"),
            CardSet::EMPTY,
            &EquityOptions::default(),
        )
        .unwrap();
        assert_eq!(res.players[0].tie, 1.0);
        assert!((res.players[0].equity - 0.5).abs() < 1e-12);
    }

    #[test]
    fn preflop_exact_aa_vs_kk() {
        // Énumération des 1 712 304 boards : AA domine KK à ~82 %.
        let res = equity(
            &[r("AsAh"), r("KdKc")],
            CardSet::EMPTY,
            CardSet::EMPTY,
            &EquityOptions::default(),
        )
        .unwrap();
        assert_eq!(res.method, EquityMethod::Exact);
        assert_eq!(res.deals, 1_712_304);
        let aa = res.players[0].equity;
        assert!((0.81..0.83).contains(&aa), "{aa}");
        assert!((sum_equity(&res) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn monte_carlo_agrees_with_exact() {
        let ranges = [r("AhKh"), r("QQ")];
        let board = cs("Qs7h2h");
        let exact = equity(&ranges, board, CardSet::EMPTY, &EquityOptions::default()).unwrap();
        assert_eq!(exact.method, EquityMethod::Exact);
        let opts = EquityOptions {
            max_exact_evaluations: 0,
            samples: 200_000,
            ..Default::default()
        };
        let mc = equity(&ranges, board, CardSet::EMPTY, &opts).unwrap();
        assert_eq!(mc.method, EquityMethod::MonteCarlo);
        let diff = (mc.players[0].equity - exact.players[0].equity).abs();
        assert!(diff < 5.0 * mc.players[0].std_error + 1e-3, "écart {diff}");
    }

    #[test]
    fn monte_carlo_is_reproducible() {
        let ranges = [r("22+,A2s+,KTo+"), r("random"), r("QQ+,AK")];
        let opts = EquityOptions {
            samples: 50_000,
            ..Default::default()
        };
        let a = equity(&ranges, CardSet::EMPTY, CardSet::EMPTY, &opts).unwrap();
        let b = equity(&ranges, CardSet::EMPTY, CardSet::EMPTY, &opts).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.method, EquityMethod::MonteCarlo);
        assert!((sum_equity(&a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn symmetric_ranges_are_even() {
        let opts = EquityOptions {
            samples: 400_000,
            ..Default::default()
        };
        let res = equity(
            &[r("random"), r("random")],
            CardSet::EMPTY,
            CardSet::EMPTY,
            &opts,
        )
        .unwrap();
        assert!((res.players[0].equity - 0.5).abs() < 0.005);
    }

    #[test]
    fn input_errors() {
        let o = EquityOptions::default();
        assert_eq!(
            equity(&[r("AA")], CardSet::EMPTY, CardSet::EMPTY, &o),
            Err(EquityError::PlayerCount(1))
        );
        assert_eq!(
            equity(&[r("AA"), r("KK")], cs("As"), CardSet::EMPTY, &o),
            Err(EquityError::BoardSize(1))
        );
        assert_eq!(
            equity(&[r("AsAh"), r("KK")], cs("As7c2d"), CardSet::EMPTY, &o),
            Err(EquityError::EmptyRange(0))
        );
        assert_eq!(
            equity(&[r("AsAh"), r("AsAd")], CardSet::EMPTY, cs("Ad"), &o),
            Err(EquityError::EmptyRange(1))
        );
        assert_eq!(
            equity(&[r("AsAh"), r("AsAh")], CardSet::EMPTY, CardSet::EMPTY, &o),
            Err(EquityError::NoValidDeal)
        );
    }
}
