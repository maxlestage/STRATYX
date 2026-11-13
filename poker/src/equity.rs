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

/// Sommes pondérées d'un joueur.
#[derive(Debug, Clone, Copy, Default)]
struct PlayerAcc {
    equity: f64,
    equity_sq: f64,
    win: f64,
    tie: f64,
}

/// Accumulateur pondéré, un [`PlayerAcc`] par joueur.
#[derive(Clone)]
struct Acc {
    weight: f64,
    players: Vec<PlayerAcc>,
    deals: u64,
}

impl Acc {
    fn new(n: usize) -> Acc {
        Acc {
            weight: 0.0,
            players: vec![PlayerAcc::default(); n],
            deals: 0,
        }
    }

    fn merge(mut self, o: Acc) -> Acc {
        self.weight += o.weight;
        self.deals += o.deals;
        for (p, q) in self.players.iter_mut().zip(&o.players) {
            p.equity += q.equity;
            p.equity_sq += q.equity_sq;
            p.win += q.win;
            p.tie += q.tie;
        }
        self
    }

    /// Abattage : `hands` contient les deux cartes de chaque joueur, `board` est complet.
    ///
    /// Chemin chaud de l'énumération exacte (un appel par board) : les mains arrivent déjà
    /// converties en [`CardSet`], on repère le gagnant pendant l'évaluation et, quand il est
    /// seul (le cas courant), on ne met à jour que lui.
    #[inline]
    fn showdown(&mut self, hands: &[CardSet], board: CardSet, weight: f64) {
        let mut ranks = [0u32; MAX_PLAYERS];
        let mut best = 0u32;
        let mut winners = 0usize;
        let mut winner = 0usize;
        for (i, (hand, rank)) in hands.iter().zip(&mut ranks).enumerate() {
            let r = evaluate(board | *hand).value();
            *rank = r;
            if winners == 0 || r > best {
                best = r;
                winners = 1;
                winner = i;
            } else if r == best {
                winners += 1;
            }
        }
        if winners == 1 {
            let p = &mut self.players[winner];
            p.equity += weight;
            p.equity_sq += weight;
            p.win += weight;
        } else {
            let share = 1.0 / winners as f64;
            for (p, rank) in self.players.iter_mut().zip(&ranks) {
                if *rank == best {
                    p.equity += weight * share;
                    p.equity_sq += weight * share * share;
                    p.tie += weight;
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
        let players = self
            .players
            .iter()
            .map(|p| {
                let mean = p.equity / w;
                let std_error = match method {
                    EquityMethod::Exact => 0.0,
                    EquityMethod::MonteCarlo => {
                        let var = (p.equity_sq / w - mean * mean).max(0.0);
                        (var / self.deals as f64).sqrt()
                    }
                };
                PlayerEquity {
                    equity: mean,
                    win: p.win / w,
                    tie: p.tie / w,
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

/// Les cartes de `set`, chacune sous forme d'ensemble à une carte, dans l'ordre de
/// [`CardSet::iter`].
fn singletons(set: CardSet) -> Vec<CardSet> {
    set.iter().map(CardSet::from_card).collect()
}

/// Appelle `f` pour chaque union de `k` éléments de `cards` (des ensembles à une carte).
fn for_each_combination(cards: &[CardSet], k: usize, mut f: impl FnMut(CardSet)) {
    fn rec(cards: &[CardSet], k: usize, start: usize, acc: CardSet, f: &mut impl FnMut(CardSet)) {
        // Dernier niveau en boucle simple : c'est là que passe chaque board.
        if k == 1 {
            for c in &cards[start..] {
                f(acc | *c);
            }
            return;
        }
        for i in start..=cards.len() - k {
            rec(cards, k - 1, i + 1, acc | cards[i], f);
        }
    }
    if k == 0 {
        f(CardSet::EMPTY);
    } else if k <= cards.len() {
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
    let acc = exact_tasks(combos, known, to_deal)
        .par_iter()
        .map(|task| {
            let mut acc = Acc::new(n);
            let mut holes = task.holes.clone();
            if holes.len() < n {
                ctx.rec(holes.len(), task.used, task.weight, &mut holes, &mut acc);
            } else {
                let deck = singletons(!task.used);
                ExactCtx::leaf(
                    &holes,
                    board | task.extra,
                    &deck[task.start..],
                    task.to_deal,
                    task.weight,
                    &mut acc,
                );
            }
            acc
        })
        .reduce(|| Acc::new(n), Acc::merge);
    acc.finish(EquityMethod::Exact)
}

/// Nombre de tâches visé pour répartir l'énumération exacte sur tous les cœurs.
const EXACT_TASKS: usize = 256;

/// Une part de l'énumération exacte : les mains des premiers joueurs, et éventuellement les
/// premières cartes du board à venir quand toutes les mains sont déjà fixées.
#[derive(Debug, Clone)]
struct ExactTask {
    holes: Vec<HoleCards>,
    used: CardSet,
    weight: f64,
    /// Cartes du board déjà fixées par le découpage, en plus du board connu.
    extra: CardSet,
    /// Indice, dans le paquet restant (`!used`), de la première carte encore tirable.
    start: usize,
    /// Cartes du board encore à distribuer.
    to_deal: usize,
}

/// Découpe l'énumération exacte en au moins [`EXACT_TASKS`] tâches quand c'est possible.
///
/// Paralléliser sur les seules mains du premier joueur laissait un unique cœur au travail
/// dès que celui-ci avait une main précise (« AhKh contre … », le cas le plus courant) : on
/// fixe donc les mains joueur après joueur, puis, si cela ne suffit pas, les premières
/// cartes du board, jusqu'à obtenir assez de tâches pour occuper tous les cœurs.
fn exact_tasks(combos: &[Vec<(HoleCards, f64)>], known: CardSet, to_deal: usize) -> Vec<ExactTask> {
    let mut tasks = vec![ExactTask {
        holes: Vec::with_capacity(combos.len()),
        used: known,
        weight: 1.0,
        extra: CardSet::EMPTY,
        start: 0,
        to_deal,
    }];
    for list in combos {
        if tasks.len() >= EXACT_TASKS {
            return tasks;
        }
        tasks = tasks
            .into_iter()
            .flat_map(|task| {
                list.iter()
                    .filter(move |(h, _)| !h.set().intersects(task.used))
                    .map(move |(h, w)| {
                        let mut next = task.clone();
                        next.holes.push(*h);
                        next.used |= h.set();
                        next.weight *= w;
                        next
                    })
            })
            .collect();
    }
    // Toutes les mains sont fixées : on découpe les tirages du board par leurs premières
    // cartes (les combinaisons sont énumérées dans l'ordre du paquet).
    while tasks.len() < EXACT_TASKS && tasks.first().is_some_and(|t| t.to_deal > 0) {
        tasks = tasks
            .into_iter()
            .flat_map(|task| {
                let deck = singletons(!task.used);
                (task.start..=deck.len() - task.to_deal)
                    .map(|i| ExactTask {
                        extra: task.extra | deck[i],
                        start: i + 1,
                        to_deal: task.to_deal - 1,
                        ..task.clone()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    tasks
}

struct ExactCtx<'a> {
    combos: &'a [Vec<(HoleCards, f64)>],
    board: CardSet,
    to_deal: usize,
}

impl ExactCtx<'_> {
    /// Abattages de toutes les façons de compléter `board` avec `to_deal` cartes de `deck`.
    fn leaf(
        holes: &[HoleCards],
        board: CardSet,
        deck: &[CardSet],
        to_deal: usize,
        weight: f64,
        acc: &mut Acc,
    ) {
        let mut sets = [CardSet::EMPTY; MAX_PLAYERS];
        for (set, h) in sets.iter_mut().zip(holes) {
            *set = h.set();
        }
        let hands = &sets[..holes.len()];
        if to_deal == 0 {
            acc.showdown(hands, board, weight);
        } else {
            for_each_combination(deck, to_deal, |extra| {
                acc.showdown(hands, board | extra, weight);
            });
        }
    }

    fn rec(
        &self,
        depth: usize,
        used: CardSet,
        weight: f64,
        holes: &mut Vec<HoleCards>,
        acc: &mut Acc,
    ) {
        if depth == self.combos.len() {
            let deck = singletons(!used);
            Self::leaf(holes, self.board, &deck, self.to_deal, weight, acc);
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
///
/// Les combinaisons sont gardées sous forme de [`CardSet`], seule forme qu'utilise le
/// Monte-Carlo.
struct WeightedCombos {
    combos: Vec<CardSet>,
    cumulative: Vec<f64>,
    /// Tous les poids valent 1 (cas courant : range sans pondération). Les cumuls sont alors
    /// les entiers 1, 2, 3… représentés exactement, et la recherche dichotomique se réduit à
    /// la partie entière du tirage : même indice, donc mêmes résultats, sans la recherche.
    unit: bool,
}

impl WeightedCombos {
    fn new(list: &[(HoleCards, f64)]) -> WeightedCombos {
        let mut total = 0.0;
        let cumulative: Vec<f64> = list
            .iter()
            .map(|(_, w)| {
                total += w;
                total
            })
            .collect();
        let unit = list.iter().all(|(_, w)| *w == 1.0);
        WeightedCombos {
            combos: list.iter().map(|(h, _)| h.set()).collect(),
            cumulative,
            unit,
        }
    }

    #[inline]
    fn sample(&self, rng: &mut impl Rng) -> CardSet {
        let total = *self.cumulative.last().expect("liste non vide");
        let x = rng.random::<f64>() * total;
        let i = if self.unit {
            // Nombre de cumuls k ≤ x parmi 1, 2, 3… : la partie entière de x (x ≥ 0).
            x as usize
        } else {
            self.cumulative.partition_point(|c| *c <= x)
        };
        self.combos[i.min(self.combos.len() - 1)]
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
    let deck = singletons(!known);
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
            let mut hands = [CardSet::EMPTY; MAX_PLAYERS];
            let mut attempts = 0u64;
            while acc.deals < target {
                attempts += 1;
                // Garde-fou : ranges qui se bloquent (presque) toujours.
                if attempts > 1_000 * target && acc.deals == 0 {
                    break;
                }
                // Rejet de la distribution complète en cas de conflit : on échantillonne
                // ainsi exactement la loi jointe des ranges pondérées.
                let mut used = CardSet::EMPTY;
                let mut ok = true;
                for (s, hand) in samplers.iter().zip(&mut hands) {
                    let h = s.sample(&mut rng);
                    if h.intersects(used) {
                        ok = false;
                        break;
                    }
                    used |= h;
                    *hand = h;
                }
                if !ok {
                    continue;
                }
                let mut full = board;
                let mut dealt = 0;
                while dealt < to_deal {
                    let c = deck[rng.random_range(0..deck.len())];
                    if !c.intersects(used | full) {
                        full |= c;
                        dealt += 1;
                    }
                }
                acc.showdown(&hands[..n], full, 1.0);
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
    fn exact_split_matches_sequential_enumeration() {
        // Le découpage en tâches (mains, puis premières cartes du board) doit parcourir
        // chaque distribution exactement une fois : on le compare à l'énumération séquentielle.
        let cases = [
            (vec![r("AhKh"), r("QsQd")], cs("2c7d9s")),
            (vec![r("AhKh"), r("QsQd"), r("JcTc")], cs("2c7d9s")),
            (vec![r("AhKh"), r("QQ+,AKo:0.5")], cs("Qs7h2h")),
            (vec![r("KK+"), r("AKs"), r("98s")], cs("9h8h2c3d")),
            (vec![r("AsAh"), r("KdKc")], cs("2c7d9sTh4s")),
        ];
        for (ranges, board) in cases {
            let combos: Vec<Vec<(HoleCards, f64)>> = ranges
                .iter()
                .map(|r| {
                    r.iter()
                        .filter(|(h, _)| !h.set().intersects(board))
                        .map(|(h, w)| (h, f64::from(w)))
                        .collect()
                })
                .collect();
            let to_deal = 5 - board.len();
            let split = exact(&combos, board, board, to_deal).unwrap();
            let ctx = ExactCtx {
                combos: &combos,
                board,
                to_deal,
            };
            let mut acc = Acc::new(combos.len());
            ctx.rec(0, board, 1.0, &mut Vec::new(), &mut acc);
            let sequential = acc.finish(EquityMethod::Exact).unwrap();
            assert_eq!(split.deals, sequential.deals);
            for (a, b) in split.players.iter().zip(&sequential.players) {
                assert!((a.equity - b.equity).abs() < 1e-12, "{a:?} {b:?}");
                assert!((a.win - b.win).abs() < 1e-12, "{a:?} {b:?}");
                assert!((a.tie - b.tie).abs() < 1e-12, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn exact_tasks_fill_the_cores() {
        // Deux mains précises avant le flop : le travail est découpé sur le board.
        let combos = vec![
            vec![("AhKh".parse::<HoleCards>().unwrap(), 1.0)],
            vec![("QsQd".parse::<HoleCards>().unwrap(), 1.0)],
        ];
        let tasks = exact_tasks(&combos, CardSet::EMPTY, 5);
        assert!(tasks.len() >= EXACT_TASKS, "{}", tasks.len());
        let total: u64 = tasks
            .iter()
            .map(|t| binomial((48 - t.start) as u64, t.to_deal as u64))
            .sum();
        assert_eq!(total, 1_712_304);
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
    fn unit_weights_shortcut_matches_binary_search() {
        // Le raccourci des poids unitaires doit choisir exactement la même combinaison que la
        // recherche dichotomique, tirage par tirage (sinon les résultats changeraient).
        for range in ["random", "AhKh", "QQ+,AK", "22+,A2s+,KTo+"] {
            let list: Vec<(HoleCards, f64)> =
                r(range).iter().map(|(h, w)| (h, f64::from(w))).collect();
            let fast = WeightedCombos::new(&list);
            assert!(fast.unit, "{range}");
            let slow = WeightedCombos {
                unit: false,
                combos: fast.combos.clone(),
                cumulative: fast.cumulative.clone(),
            };
            let mut a = ChaCha8Rng::seed_from_u64(11);
            let mut b = ChaCha8Rng::seed_from_u64(11);
            for _ in 0..100_000 {
                assert_eq!(fast.sample(&mut a), slow.sample(&mut b), "{range}");
            }
        }
        let weighted: Vec<(HoleCards, f64)> = r("QQ+,AKo:0.5")
            .iter()
            .map(|(h, w)| (h, f64::from(w)))
            .collect();
        assert!(!WeightedCombos::new(&weighted).unit);
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
