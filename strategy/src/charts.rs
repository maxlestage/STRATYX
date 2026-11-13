//! Équilibres push/fold : à tapis égaux et courts, chaque joueur fait tapis ou se couche
//! en premier de parole, puis suit ou se couche face aux tapis.
//!
//! Le solveur calcule un équilibre en espérance de jetons (chip EV, exacte pour les
//! Expresso « winner takes all ») par **jeu fictif** : à chaque itération, la meilleure
//! réponse de chaque position, situation et classe de mains est estimée par simulation
//! contre les stratégies courantes des autres joueurs, puis mélangée à la moyenne des
//! itérations précédentes. Les tirages utilisent ChaCha8 avec une graine explicite
//! (décision T8) : à graine égale, la table est identique.
//!
//! Les positions sont des **rangs préflop** (0 : premier à parler ; la grosse blinde est
//! la dernière). Une table résolue à `n` joueurs sert aussi aux tables plus courtes : une
//! position y est ramenée à celle qui a le même nombre de joueurs derrière elle.

// Les rangs préflop servent d'index à plusieurs tableaux à la fois : les boucles sur les
// rangs restent plus lisibles qu'avec des itérateurs.
#![allow(clippy::needless_range_loop)]

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use stratyx_poker::{CardSet, HoleCards, evaluate};

use crate::classes::{CLASSES, combos_of, draw, hole_class};

/// Situation d'un joueur dans un coup push/fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// Personne n'a fait tapis avant lui : tapis ou couché (parole en grosse blinde).
    FirstIn,
    /// Un tapis avant lui (du rang `shover`), sans call.
    VsShove { shover: usize },
    /// Un tapis (du rang `shover`) et au moins un call.
    VsShoveAndCall { shover: usize },
}

/// Réglages du solveur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverConfig {
    pub players: usize,
    /// Ante payée par chaque joueur, en grosses blindes.
    pub ante_bb: f32,
    /// Tapis (en grosses blindes) à résoudre.
    pub depths: Vec<f32>,
    pub iterations: usize,
    /// Tirages par position et par classe à chaque itération.
    pub samples: usize,
    pub seed: u64,
}

/// Les stratégies d'équilibre. Chaque chaîne code les 169 classes (ordre de la grille) :
/// `A` = 0 %, `B` = 5 % … `U` = 100 %.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub players: usize,
    pub ante_bb: f32,
    pub depths: Vec<f32>,
    /// `push[d][rang]` : tapis en premier de parole.
    pub push: Vec<Vec<String>>,
    /// `call[d][rang][rang du tapis]` : suivre un tapis seul (`[0]`) ou déjà suivi (`[1]`).
    pub call: Vec<Vec<Vec<[String; 2]>>>,
}

fn encode(v: &[f32]) -> String {
    v.iter()
        .map(|p| char::from(b'A' + (p.clamp(0.0, 1.0) * 20.0).round() as u8))
        .collect()
}

fn decode_at(s: &str, class: usize) -> f32 {
    s.as_bytes()
        .get(class)
        .map_or(0.0, |b| f32::from(b.saturating_sub(b'A')) / 20.0)
}

impl Chart {
    /// L'indice de la profondeur résolue la plus proche.
    fn depth_index(&self, bb: f32) -> usize {
        self.depths
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - bb).abs().total_cmp(&(b.1 - bb).abs()))
            .map_or(0, |(i, _)| i)
    }

    /// Le tapis maximal couvert par la table.
    #[must_use]
    pub fn max_depth(&self) -> f32 {
        self.depths.iter().copied().fold(0.0, f32::max)
    }

    /// Ramène un rang d'une table de `players` joueurs à celui de la table résolue, en
    /// gardant le nombre de joueurs qui parlent après lui.
    fn map_order(&self, order: usize, players: usize) -> usize {
        let behind = players.saturating_sub(order + 1);
        self.players.saturating_sub(behind + 1)
    }

    /// Probabilité (0 à 1) de l'action agressive de l'équilibre (tapis ou call).
    #[must_use]
    pub fn probability(
        &self,
        node: Node,
        effective_bb: f32,
        order: usize,
        players: usize,
        class: usize,
    ) -> f32 {
        if self.depths.is_empty() {
            return 0.0;
        }
        let d = self.depth_index(effective_bb);
        let o = self.map_order(order, players).min(self.players - 1);
        match node {
            Node::FirstIn => self.push[d].get(o).map_or(0.0, |s| decode_at(s, class)),
            Node::VsShove { shover } | Node::VsShoveAndCall { shover } => {
                let s = self.map_order(shover, players).min(o.saturating_sub(1));
                let k = usize::from(matches!(node, Node::VsShoveAndCall { .. }));
                self.call[d]
                    .get(o)
                    .and_then(|v| v.get(s))
                    .map_or(0.0, |pair| decode_at(&pair[k], class))
            }
        }
    }

    /// Part des mains jouées (pondérée par les combinaisons).
    #[must_use]
    pub fn range_fraction(
        &self,
        node: Node,
        effective_bb: f32,
        order: usize,
        players: usize,
    ) -> f32 {
        (0..CLASSES)
            .map(|c| {
                self.probability(node, effective_bb, order, players, c) * combos_of(c).len() as f32
            })
            .sum::<f32>()
            / 1326.0
    }
}

/// Les stratégies en cours de calcul : `push[rang][classe]`,
/// `call[rang][rang du tapis][k][classe]`.
struct Strategies {
    push: Vec<Vec<f32>>,
    call: Vec<Vec<[Vec<f32>; 2]>>,
}

impl Strategies {
    fn p(&self, i: usize, first: Option<usize>, in_pot: usize, class: usize) -> f32 {
        match first {
            None => self.push[i][class],
            Some(s) => self.call[i][s][usize::from(in_pot >= 2)][class],
        }
    }
}

/// Résout toutes les profondeurs de `config`. `progress` reçoit (profondeur, itération).
pub fn solve(config: &SolverConfig, mut progress: impl FnMut(f32, usize)) -> Chart {
    let mut chart = Chart {
        players: config.players,
        ante_bb: config.ante_bb,
        depths: config.depths.clone(),
        push: Vec::new(),
        call: Vec::new(),
    };
    for (di, &depth) in config.depths.iter().enumerate() {
        let s = solve_depth(config, depth, di as u64, &mut progress);
        chart.push.push(s.push.iter().map(|v| encode(v)).collect());
        chart.call.push(
            s.call
                .iter()
                .map(|by_shover| {
                    by_shover
                        .iter()
                        .map(|[a, b]| [encode(a), encode(b)])
                        .collect()
                })
                .collect(),
        );
    }
    chart
}

fn solve_depth(
    config: &SolverConfig,
    depth: f32,
    salt: u64,
    progress: &mut impl FnMut(f32, usize),
) -> Strategies {
    let n = config.players;
    // Départ : tapis avec le top 30 %, call avec le top 15 % (8 % après un call).
    let start = |share: f32| -> Vec<f32> {
        (0..CLASSES)
            .map(|c| f32::from(u8::from(crate::classes::percentile(c) <= share)))
            .collect()
    };
    let mut s = Strategies {
        push: (0..n).map(|_| start(0.3)).collect(),
        call: (0..n)
            .map(|_| (0..n).map(|_| [start(0.15), start(0.08)]).collect())
            .collect(),
    };
    for it in 0..config.iterations {
        progress(depth, it);
        let seed = config.seed ^ (salt << 40) ^ ((it as u64) << 20);
        let br = best_responses(config, depth, &s, seed);
        let w = 1.0 / (it as f32 + 2.0);
        for o in 0..n {
            for c in 0..CLASSES {
                if let Some(b) = br.push[o][c] {
                    s.push[o][c] += (b - s.push[o][c]) * w;
                }
                for sh in 0..n {
                    for k in 0..2 {
                        if let Some(b) = br.call[o][sh][k][c] {
                            s.call[o][sh][k][c] += (b - s.call[o][sh][k][c]) * w;
                        }
                    }
                }
            }
        }
    }
    s
}

type Best = Vec<Option<f32>>;

struct Responses {
    push: Vec<Best>,
    call: Vec<Vec<[Best; 2]>>,
}

/// Les mises postées (blindes et antes) de chaque rang préflop, en grosses blindes.
fn posted(n: usize, ante: f32) -> Vec<f32> {
    let mut p = vec![ante; n];
    if n == 2 {
        p[0] += 0.5; // bouton = petite blinde
        p[1] += 1.0;
    } else {
        p[n - 2] += 0.5;
        p[n - 1] += 1.0;
    }
    p
}

/// Somme des EV (agressive, fold) et nombre de tirages.
#[derive(Clone, Copy, Default)]
struct Acc {
    ev_in: f64,
    ev_out: f64,
    count: u32,
}

impl Acc {
    fn best(&self) -> Option<f32> {
        (self.count >= 8).then(|| f32::from(u8::from(self.ev_in > self.ev_out)))
    }
}

/// Une situation : `(rang du tapis, call)`.
type NodeKey = (Option<usize>, Option<usize>);

/// Les EV d'une position et d'une classe, par situation.
type Cell = (usize, usize, Vec<(NodeKey, Acc)>);

/// Les situations possibles d'un joueur : `(rang du tapis, rang du call)`.
fn nodes_of(hero: usize, n: usize) -> Vec<(Option<usize>, Option<usize>)> {
    let mut v = Vec::new();
    if hero < n - 1 {
        v.push((None, None));
    }
    for sh in 0..hero {
        v.push((Some(sh), None));
        if hero - sh >= 2 {
            v.push((Some(sh), Some(usize::MAX)));
        }
    }
    v
}

/// Les meilleures réponses de chaque position et de chaque classe (en parallèle). Chaque
/// situation est simulée directement (les joueurs d'avant reçoivent une main compatible
/// avec leur action), pour que les situations rares soient aussi bien estimées que les
/// fréquentes.
fn best_responses(config: &SolverConfig, depth: f32, s: &Strategies, seed: u64) -> Responses {
    use rayon::prelude::*;
    let n = config.players;
    let posted = posted(n, config.ante_bb);
    let stack = depth + config.ante_bb; // tapis de départ (ante comprise)
    let cells: Vec<Cell> = (0..n * CLASSES)
        .into_par_iter()
        .map(|cell| {
            let (hero, c) = (cell / CLASSES, cell % CLASSES);
            let nodes = nodes_of(hero, n);
            let per_node = (config.samples / nodes.len().max(1)).max(24);
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ ((hero as u64) << 12) ^ c as u64);
            let combos = combos_of(c);
            let accs = nodes
                .into_iter()
                .map(|node| {
                    let mut acc = Acc::default();
                    for i in 0..per_node {
                        let h = combos[i % combos.len()];
                        if let Some((ev_in, ev_out)) =
                            simulate(n, hero, h, node, s, &posted, stack, &mut rng)
                        {
                            acc.ev_in += ev_in;
                            acc.ev_out += ev_out;
                            acc.count += 1;
                        }
                    }
                    (node, acc)
                })
                .collect();
            (hero, c, accs)
        })
        .collect();
    let mut out = Responses {
        push: vec![vec![None; CLASSES]; n],
        call: (0..n)
            .map(|_| {
                (0..n)
                    .map(|_| [vec![None; CLASSES], vec![None; CLASSES]])
                    .collect()
            })
            .collect(),
    };
    for (hero, c, accs) in cells {
        for ((shover, caller), acc) in accs {
            match shover {
                None => out.push[hero][c] = acc.best(),
                Some(sh) => out.call[hero][sh][usize::from(caller.is_some())][c] = acc.best(),
            }
        }
    }
    out
}

/// Une main tirée pour un joueur, compatible avec l'action qu'il a jouée (probabilité
/// `p(classe)` de l'accepter).
fn draw_for(
    used: &mut CardSet,
    rng: &mut ChaCha8Rng,
    p: impl Fn(usize) -> f32,
) -> Option<HoleCards> {
    let mut last = None;
    for _ in 0..120 {
        let mut u = *used;
        let a = draw(&mut u, rng);
        let b = draw(&mut u, rng);
        let h = HoleCards::new(a, b)?;
        last = Some((h, u));
        if rng.random::<f32>() < p(hole_class(h)) {
            break;
        }
    }
    let (h, u) = last?;
    *used = u;
    Some(h)
}

/// Simule une main où `hero` (rang préflop) a `h` et se trouve dans la situation `node`
/// (`(rang du tapis, call)`, le call étant donné par un joueur tiré entre les deux).
/// Renvoie l'EV de l'action agressive et celle du fold, en grosses blindes.
#[allow(clippy::too_many_arguments)]
fn simulate(
    n: usize,
    hero: usize,
    h: HoleCards,
    node: (Option<usize>, Option<usize>),
    s: &Strategies,
    posted: &[f32],
    stack: f32,
    rng: &mut ChaCha8Rng,
) -> Option<(f64, f64)> {
    let mut used = h.set();
    let mut hands = [CardSet::EMPTY; 10];
    hands[hero] = h.set();
    let (shover, caller) = node;
    let caller = match (shover, caller) {
        (Some(sh), Some(_)) => Some(rng.random_range(sh + 1..hero)),
        _ => None,
    };
    let mut in_pot: Vec<usize> = Vec::with_capacity(n);
    let mut first = None;
    for i in 0..hero {
        let joins = Some(i) == shover || Some(i) == caller;
        let pot_len = in_pot.len();
        let hc = draw_for(&mut used, rng, |c| {
            let p = s.p(i, first, pot_len, c);
            if joins { p } else { 1.0 - p }
        })?;
        hands[i] = hc.set();
        if joins {
            first.get_or_insert(i);
            in_pot.push(i);
        }
    }
    first.get_or_insert(hero);
    in_pot.push(hero);
    for i in hero + 1..n {
        let a = draw(&mut used, rng);
        let b = draw(&mut used, rng);
        let hc = HoleCards::new(a, b)?;
        if rng.random::<f32>() < s.p(i, first, in_pot.len(), hole_class(hc)) {
            hands[i] = hc.set();
            in_pot.push(i);
        }
    }
    let ev_in = showdown(n, hero, &in_pot, &hands, posted, stack, &mut used, rng);
    Some((ev_in, -f64::from(posted[hero])))
}

/// Résultat (en grosses blindes) du joueur `hero` quand `in_pot` sont à tapis.
#[allow(clippy::too_many_arguments)]
fn showdown(
    n: usize,
    hero: usize,
    in_pot: &[usize],
    hands: &[CardSet; 10],
    posted: &[f32],
    stack: f32,
    used: &mut CardSet,
    rng: &mut ChaCha8Rng,
) -> f64 {
    if in_pot.len() == 1 {
        // Tout le monde se couche : il ramasse les mises postées.
        return f64::from(posted.iter().sum::<f32>() - posted[hero]);
    }
    let dead: f32 = (0..n)
        .filter(|i| !in_pot.contains(i))
        .map(|i| posted[i])
        .sum();
    let pot = f64::from(dead + stack * in_pot.len() as f32);
    let mut board = CardSet::EMPTY;
    for _ in 0..5 {
        board.insert(draw(used, rng));
    }
    let mut best = None;
    let mut winners = 0usize;
    let mut mine = None;
    for &i in in_pot {
        let r = evaluate(board | hands[i]);
        if i == hero {
            mine = Some(r);
        }
        match best {
            Some(b) if r < b => {}
            Some(b) if r == b => winners += 1,
            _ => {
                best = Some(r);
                winners = 1;
            }
        }
    }
    let share = if mine == best {
        pot / winners.max(1) as f64
    } else {
        0.0
    };
    share - f64::from(stack)
}

/// EV (en grosses blindes) de l'action agressive et du fold pour le joueur de rang
/// préflop `hero` qui a `hole`, à une table de `players` joueurs à tapis égaux
/// (`effective_bb`), quand les rangs `shoved` ont déjà fait tapis (le premier) ou suivi
/// (les suivants) : les adversaires jouent l'équilibre de la table.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn spot_ev(
    chart: &Chart,
    effective_bb: f32,
    players: usize,
    hero: usize,
    shoved: &[usize],
    hole: HoleCards,
    samples: usize,
    seed: u64,
) -> (f64, f64) {
    let n = players.clamp(2, 10);
    let hero = hero.min(n - 1);
    let posted = posted(n, chart.ante_bb);
    let stack = effective_bb + chart.ante_bb;
    let first = shoved.first().copied();
    let node_for = |in_pot: usize, first: usize| {
        if in_pot <= 1 {
            Node::VsShove { shover: first }
        } else {
            Node::VsShoveAndCall { shover: first }
        }
    };
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut total = 0.0f64;
    let mut count = 0u32;
    for _ in 0..samples.max(1) {
        let mut used = hole.set();
        let mut hands = [CardSet::EMPTY; 10];
        let mut in_pot: Vec<usize> = Vec::with_capacity(n);
        // Ceux qui ont déjà fait tapis ou suivi : une main de leur range d'équilibre.
        for (k, &o) in shoved.iter().enumerate().filter(|(_, o)| **o < n) {
            let node = match (k, first) {
                (0, _) | (_, None) => Node::FirstIn,
                (_, Some(f)) => node_for(k, f),
            };
            let mut pick = None;
            for _ in 0..60 {
                let mut u = used;
                let a = draw(&mut u, &mut rng);
                let b = draw(&mut u, &mut rng);
                let Some(h) = HoleCards::new(a, b) else {
                    continue;
                };
                pick = Some((h, u));
                if rng.random::<f32>() < chart.probability(node, effective_bb, o, n, hole_class(h))
                {
                    break;
                }
            }
            if let Some((h, u)) = pick {
                used = u;
                hands[o] = h.set();
                in_pot.push(o);
            }
        }
        hands[hero] = hole.set();
        let first = first.unwrap_or(hero);
        in_pot.push(hero);
        for o in hero + 1..n {
            let a = draw(&mut used, &mut rng);
            let b = draw(&mut used, &mut rng);
            let Some(h) = HoleCards::new(a, b) else {
                continue;
            };
            let p = chart.probability(
                node_for(in_pot.len(), first),
                effective_bb,
                o,
                n,
                hole_class(h),
            );
            if rng.random::<f32>() < p {
                hands[o] = h.set();
                in_pot.push(o);
            }
        }
        total += showdown(
            n, hero, &in_pot, &hands, &posted, stack, &mut used, &mut rng,
        );
        count += 1;
    }
    (total / f64::from(count.max(1)), -f64::from(posted[hero]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classes::class_index;
    use stratyx_poker::{HandClass, Rank};

    #[test]
    fn heads_up_push_fold_is_sensible() {
        let config = SolverConfig {
            players: 2,
            ante_bb: 0.0,
            depths: vec![10.0],
            iterations: 12,
            samples: 600,
            seed: 7,
        };
        let chart = solve(&config, |_, _| {});
        let aa = class_index(HandClass::Pair(Rank::Ace));
        let k2o = class_index(HandClass::Offsuit(Rank::King, Rank::Two));
        let s2o = class_index(HandClass::Offsuit(Rank::Seven, Rank::Two));
        let vs = Node::VsShove { shover: 0 };
        assert!(chart.probability(Node::FirstIn, 10.0, 0, 2, aa) > 0.95);
        assert!(chart.probability(vs, 10.0, 1, 2, aa) > 0.95);
        // À 10 BB en tête-à-tête, l'équilibre pousse environ 60 % des mains (K2o compris)
        // et suit avec environ 40 %.
        let push = chart.range_fraction(Node::FirstIn, 10.0, 0, 2);
        let call = chart.range_fraction(vs, 10.0, 1, 2);
        assert!((0.45..0.8).contains(&push), "push {push}");
        assert!((0.25..0.55).contains(&call), "call {call}");
        assert!(chart.probability(Node::FirstIn, 10.0, 0, 2, k2o) > 0.5);
        assert!(chart.probability(vs, 10.0, 1, 2, s2o) < 0.5);
        // L'EV suit l'équilibre : pousser AA rapporte, suivre 72o coûte.
        let (push_aa, fold) = spot_ev(&chart, 10.0, 2, 0, &[], "AsAh".parse().unwrap(), 3000, 1);
        assert!(push_aa > fold);
        let (call_72, fold) = spot_ev(&chart, 10.0, 2, 1, &[0], "7c2d".parse().unwrap(), 3000, 2);
        assert!(call_72 < fold, "{call_72} {fold}");
        let json = serde_json::to_string(&chart).unwrap();
        assert_eq!(serde_json::from_str::<Chart>(&json).unwrap(), chart);
    }
}
