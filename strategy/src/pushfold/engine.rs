//! Moteur commun des solveurs push/fold (2 ou 3 joueurs) : arbre, gains, CFR+, meilleure
//! réponse.
//!
//! # Modèle de jeu
//!
//! Chaque joueur reçoit une des 169 classes ; sa stratégie est une fréquence d'action
//! agressive (tapis ou suivi) par classe et par nœud de décision. Comme la loi des mains
//! adverses est symétrique en couleurs, toutes les combinaisons d'une classe sont
//! équivalentes et cette restriction ne coûte rien.
//!
//! La loi des classes à chaque issue tient compte des blocages **entre les joueurs encore
//! en jeu** : à l'abattage entre `a` et `b`, le poids d'un couple est le nombre de couples
//! de combinaisons compatibles `n(a, b)` ; à trois, `n(a, b, c)`. Les mains couchées sont
//! tirées indépendamment (simplification usuelle des calculateurs push/fold). Formellement,
//! c'est un jeu à types indépendants (loi a priori `|x| / 1326`) dont les gains intègrent le
//! rapport de vraisemblance ; équilibre et exploitabilité y sont donc bien définis.
//!
//! # Algorithme
//!
//! CFR+ vectorisé sur les classes, mises à jour alternées, moyenne linéaire des stratégies.
//! Chaque joueur n'agit qu'une fois par chemin : sa probabilité propre d'atteindre un nœud
//! vaut 1, ce qui simplifie la moyenne. L'exploitabilité est mesurée exactement par
//! meilleure réponse (par classe et par nœud) contre la stratégie moyenne.

use rayon::prelude::*;

use super::payoff::{PayoffModel, settle};
use super::three_way::{NUM_OUTCOMES, OUTCOMES, ThreeWayEquities};
use crate::preflop::{
    NUM_CLASSES, NUM_COMBOS, PreflopMatrix, class_at, class_combo_count, compatible_combos_idx,
};

const N: usize = NUM_CLASSES;
/// Nombre de mains possibles pour un deuxième joueur : C(50, 2).
const SECOND: f64 = 1225.0;
/// Nombre de mains possibles pour un troisième joueur : C(48, 2).
const THIRD: f64 = 1128.0;

/// Action de chaque joueur sur un chemin de l'arbre : `(nœud, action)`, ou `None` si le
/// joueur n'a pas eu à agir.
type ActionPath = Vec<Option<(usize, usize)>>;

/// Nœud de décision : action 0 = se coucher, action 1 = tapis (ou suivre un tapis).
#[derive(Debug)]
pub(crate) struct Node {
    pub player: usize,
    /// Joueurs déjà à tapis au moment de décider (dans l'ordre de parole).
    pub facing: Vec<usize>,
    /// Actions des autres joueurs menant à ce nœud : `(nœud, action)` par joueur.
    pub path: ActionPath,
    /// Issues atteignables après chaque action.
    under: [Vec<usize>; 2],
}

/// Données précalculées d'une issue pour le calcul des valeurs contrefactuelles.
#[derive(Debug)]
enum TermData {
    /// Un seul joueur en jeu : gains constants.
    Single { value: Vec<f64> },
    /// Abattage à deux (`i < j`), troisième joueur couché éventuel `k`.
    Two {
        i: usize,
        j: usize,
        /// `m[0][xi * N + xj]`, `m[1][xj * N + xi]` : poids conditionnel × gain.
        m: [Vec<f32>; 2],
        /// Pour le joueur couché : `f[xi * N + xj]` = poids joint × gain.
        folded: Option<(usize, Vec<f32>)>,
    },
    /// Abattage à trois : `x[p][(xp * N + xq) * N + xr]`, `q < r` les deux autres sièges.
    Three { x: [Vec<f32>; 3] },
}

#[derive(Debug)]
pub(crate) struct Terminal {
    /// Action de chaque joueur sur le chemin (`None` s'il n'a pas eu à agir).
    pub path: ActionPath,
    data: TermData,
}

/// Jeu push/fold complet pour des tapis donnés.
#[derive(Debug)]
pub(crate) struct Game {
    pub n: usize,
    pub nodes: Vec<Node>,
    pub terminals: Vec<Terminal>,
    prior: [f64; N],
}

/// Probabilité a priori d'une classe : `|x| / 1326`.
fn prior() -> [f64; N] {
    let mut p = [0.0; N];
    for (i, v) in p.iter_mut().enumerate() {
        *v = class_combo_count(class_at(i).expect("index")) as f64 / NUM_COMBOS as f64;
    }
    p
}

impl Game {
    /// Construit l'arbre et précalcule les gains. `stacks` et `posted` sont dans l'ordre de
    /// parole (BTN, SB, BB à trois ; SB, BB en tête-à-tête).
    pub fn new(
        stacks: &[f64],
        posted: &[f64],
        payoff: &PayoffModel,
        matrix: &PreflopMatrix,
        three_way: Option<&ThreeWayEquities>,
    ) -> Game {
        let n = stacks.len();
        let mut nodes = Vec::new();
        let mut raw_terminals = Vec::new();
        build(
            n,
            0,
            false,
            vec![false; n],
            vec![None; n],
            Vec::new(),
            &mut nodes,
            &mut raw_terminals,
        );
        for (t, (path, _)) in raw_terminals.iter().enumerate() {
            for step in path.iter().flatten() {
                nodes[step.0].under[step.1].push(t);
            }
        }
        let prior = prior();
        let terminals = raw_terminals
            .into_iter()
            .map(|(path, active)| {
                let payoffs = outcome_payoffs(stacks, posted, &active, payoff);
                let data = term_data(n, &active, &payoffs, matrix, three_way);
                Terminal { path, data }
            })
            .collect();
        Game {
            n,
            nodes,
            terminals,
            prior,
        }
    }

    /// Valeurs contrefactuelles du joueur `p` à l'issue `t` (par classe de `p`), les autres
    /// joueurs jouant `probs[nœud][action][classe]`.
    fn terminal_cv(&self, t: &Terminal, p: usize, probs: &[[Vec<f32>; 2]]) -> Vec<f64> {
        let ones = vec![1.0f32; N];
        let rho = |i: usize| -> &[f32] {
            match t.path[i] {
                Some((d, a)) => &probs[d][a],
                None => &ones,
            }
        };
        let mass = |i: usize| -> f64 {
            rho(i)
                .iter()
                .zip(&self.prior)
                .map(|(r, w)| f64::from(*r) * w)
                .sum()
        };
        match &t.data {
            TermData::Single { value } => {
                let factor: f64 = (0..self.n).filter(|&i| i != p).map(mass).product();
                vec![value[p] * factor; N]
            }
            TermData::Two { i, j, m, folded } => {
                if p == *i || p == *j {
                    let (mat, q) = if p == *i { (&m[0], *j) } else { (&m[1], *i) };
                    let factor = folded.as_ref().map_or(1.0, |(k, _)| mass(*k));
                    let rq = rho(q);
                    (0..N)
                        .map(|xp| factor * f64::from(dot(&mat[xp * N..(xp + 1) * N], rq)))
                        .collect()
                } else {
                    let (_, f) = folded.as_ref().expect("joueur couché");
                    let (ri, rj) = (rho(*i), rho(*j));
                    let v: f64 = (0..N)
                        .filter(|&xi| ri[xi] > 0.0)
                        .map(|xi| f64::from(ri[xi]) * f64::from(dot(&f[xi * N..(xi + 1) * N], rj)))
                        .sum();
                    vec![v; N]
                }
            }
            TermData::Three { x } => {
                let others: Vec<usize> = (0..3).filter(|&s| s != p).collect();
                let (rq, rr) = (rho(others[0]), rho(others[1]));
                let tensor = &x[p];
                (0..N)
                    .into_par_iter()
                    .map(|xp| {
                        let base = xp * N * N;
                        let mut acc = 0.0f64;
                        for (xq, &wq) in rq.iter().enumerate() {
                            if wq > 0.0 {
                                let row = &tensor[base + xq * N..base + (xq + 1) * N];
                                acc += f64::from(wq) * f64::from(dot(row, rr));
                            }
                        }
                        acc
                    })
                    .collect()
            }
        }
    }

    /// Valeurs contrefactuelles de toutes les issues pour le joueur `p` (`None` pour celles
    /// qui ne sont pas demandées).
    fn all_cvs(&self, p: usize, probs: &[[Vec<f32>; 2]], only_own: bool) -> Vec<Option<Vec<f64>>> {
        self.terminals
            .iter()
            .map(|t| (!only_own || t.path[p].is_some()).then(|| self.terminal_cv(t, p, probs)))
            .collect()
    }

    /// Valeurs des deux actions au nœud `d`, par classe.
    fn action_values(&self, d: usize, cvs: &[Option<Vec<f64>>]) -> [Vec<f64>; 2] {
        let mut out = [vec![0.0; N], vec![0.0; N]];
        for (a, values) in out.iter_mut().enumerate() {
            for &t in &self.nodes[d].under[a] {
                let cv = cvs[t].as_ref().expect("valeur calculée");
                for (v, c) in values.iter_mut().zip(cv) {
                    *v += c;
                }
            }
        }
        out
    }

    /// Masse a priori des actions des autres joueurs menant au nœud `d` (blocages ignorés).
    fn reach_mass(&self, d: usize, probs: &[[Vec<f32>; 2]]) -> f64 {
        self.nodes[d]
            .path
            .iter()
            .flatten()
            .map(|&(nd, a)| {
                probs[nd][a]
                    .iter()
                    .zip(&self.prior)
                    .map(|(r, w)| f64::from(*r) * w)
                    .sum::<f64>()
            })
            .product()
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Construit récursivement l'arbre push/fold.
#[allow(clippy::too_many_arguments)]
fn build(
    n: usize,
    seat: usize,
    pushed: bool,
    folded: Vec<bool>,
    path: ActionPath,
    facing: Vec<usize>,
    nodes: &mut Vec<Node>,
    terminals: &mut Vec<(ActionPath, Vec<usize>)>,
) {
    // Fin de parole, ou tout le monde s'est couché devant la grosse blinde.
    if seat == n || (!pushed && seat == n - 1) {
        let active = (0..n).filter(|&i| !folded[i]).collect();
        terminals.push((path, active));
        return;
    }
    let id = nodes.len();
    nodes.push(Node {
        player: seat,
        facing: facing.clone(),
        path: path.clone(),
        under: [Vec::new(), Vec::new()],
    });
    let mut fold_path = path.clone();
    fold_path[seat] = Some((id, 0));
    let mut fold_mask = folded.clone();
    fold_mask[seat] = true;
    build(
        n,
        seat + 1,
        pushed,
        fold_mask,
        fold_path,
        facing.clone(),
        nodes,
        terminals,
    );
    let mut push_path = path;
    push_path[seat] = Some((id, 1));
    let mut push_facing = facing;
    push_facing.push(seat);
    build(
        n,
        seat + 1,
        true,
        folded,
        push_path,
        push_facing,
        nodes,
        terminals,
    );
}

/// Gains de chaque joueur pour chaque classement possible des joueurs actifs :
/// `résultat[issue][joueur]`. Deux joueurs : (premier gagne, égalité, second gagne) ; trois :
/// ordre de [`OUTCOMES`].
fn outcome_payoffs(
    stacks: &[f64],
    posted: &[f64],
    active: &[usize],
    model: &PayoffModel,
) -> Vec<Vec<f64>> {
    let n = stacks.len();
    let is_active: Vec<bool> = (0..n).map(|i| active.contains(&i)).collect();
    let contrib: Vec<f64> = (0..n)
        .map(|i| if is_active[i] { stacks[i] } else { posted[i] })
        .collect();
    let strengths: Vec<Vec<u8>> = match active.len() {
        1 => vec![vec![0; n]],
        2 => [[2u8, 1], [1, 1], [1, 2]]
            .iter()
            .map(|s| {
                let mut v = vec![0; n];
                v[active[0]] = s[0];
                v[active[1]] = s[1];
                v
            })
            .collect(),
        3 => OUTCOMES.iter().map(|r| r.map(|x| 3 - x).to_vec()).collect(),
        _ => unreachable!("au plus trois joueurs"),
    };
    strengths
        .iter()
        .map(|s| {
            let won = settle(&contrib, &is_active, s);
            let final_stacks: Vec<f64> = (0..n).map(|i| stacks[i] - contrib[i] + won[i]).collect();
            model.values(stacks, &final_stacks)
        })
        .collect()
}

fn term_data(
    n: usize,
    active: &[usize],
    payoffs: &[Vec<f64>],
    matrix: &PreflopMatrix,
    three_way: Option<&ThreeWayEquities>,
) -> TermData {
    let size = |x: usize| class_combo_count(class_at(x).expect("index")) as f64;
    match active.len() {
        1 => TermData::Single {
            value: payoffs[0].clone(),
        },
        2 => {
            let (i, j) = (active[0], active[1]);
            let k = (0..n).find(|s| !active.contains(s));
            // Gain du joueur `p` quand `i` tient `xi` et `j` tient `xj`.
            let u = |p: usize, xi: usize, xj: usize| {
                matrix.win_idx(xi, xj) * payoffs[0][p]
                    + matrix.tie_idx(xi, xj) * payoffs[1][p]
                    + matrix.win_idx(xj, xi) * payoffs[2][p]
            };
            let mut mi = vec![0.0f32; N * N];
            let mut mj = vec![0.0f32; N * N];
            let mut f = k.map(|_| vec![0.0f32; N * N]);
            for xi in 0..N {
                for xj in 0..N {
                    let w = f64::from(compatible_combos_idx(xi, xj));
                    if w == 0.0 {
                        continue;
                    }
                    mi[xi * N + xj] = (w / (SECOND * size(xi)) * u(i, xi, xj)) as f32;
                    mj[xj * N + xi] = (w / (SECOND * size(xj)) * u(j, xi, xj)) as f32;
                    if let (Some(k), Some(f)) = (k, f.as_mut()) {
                        f[xi * N + xj] = (w / (NUM_COMBOS as f64 * SECOND) * u(k, xi, xj)) as f32;
                    }
                }
            }
            TermData::Two {
                i,
                j,
                m: [mi, mj],
                folded: k.zip(f),
            }
        }
        3 => {
            let table = three_way.expect("table des confrontations à trois requise");
            let x = [0usize, 1, 2].map(|p| {
                let others: Vec<usize> = (0..3).filter(|&s| s != p).collect();
                let (q, r) = (others[0], others[1]);
                let mut tensor = vec![0.0f32; N * N * N];
                tensor
                    .par_chunks_mut(N * N)
                    .enumerate()
                    .for_each(|(xp, block)| {
                        let norm = 1.0 / (SECOND * THIRD * size(xp));
                        let mut seat_class = [0usize; 3];
                        seat_class[p] = xp;
                        for xq in 0..N {
                            seat_class[q] = xq;
                            for xr in 0..N {
                                seat_class[r] = xr;
                                let (count, probs) =
                                    table.lookup_idx(seat_class[0], seat_class[1], seat_class[2]);
                                if count == 0 {
                                    continue;
                                }
                                let u: f64 = (0..NUM_OUTCOMES)
                                    .map(|o| f64::from(probs[o]) * payoffs[o][p])
                                    .sum();
                                block[xq * N + xr] = (f64::from(count) * norm * u) as f32;
                            }
                        }
                    });
                tensor
            });
            TermData::Three { x }
        }
        _ => unreachable!("au plus trois joueurs"),
    }
}

/// Paramètres de l'itération.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunOptions {
    pub max_iterations: u32,
    pub target: f64,
    pub check_every: u32,
}

/// Résultat brut du solveur.
#[derive(Debug)]
pub(crate) struct RunResult {
    /// Fréquence moyenne de l'action agressive, par nœud et par classe.
    pub strategy: Vec<Vec<f32>>,
    /// Valeurs (normalisées par la masse d'atteinte) des deux actions, par nœud et classe.
    pub action_ev: Vec<[Vec<f32>; 2]>,
    /// Espérance de chaque joueur à l'équilibre (par main).
    pub ev: Vec<f64>,
    /// Gain de la meilleure réponse de chaque joueur.
    pub exploitability: Vec<f64>,
    pub iterations: u32,
}

/// Vecteurs de probabilités (se coucher, agressif) de chaque nœud.
fn probs_of(strategy: &[Vec<f32>]) -> Vec<[Vec<f32>; 2]> {
    strategy
        .iter()
        .map(|s| [s.iter().map(|p| 1.0 - p).collect(), s.clone()])
        .collect()
}

impl Game {
    /// Espérance et meilleure réponse de chaque joueur contre un profil de stratégies.
    pub fn evaluate(&self, strategy: &[Vec<f32>]) -> (Vec<f64>, Vec<f64>) {
        let probs = probs_of(strategy);
        let mut ev = vec![0.0; self.n];
        let mut br = vec![0.0; self.n];
        for p in 0..self.n {
            let cvs = self.all_cvs(p, &probs, false);
            let mut per_class_ev = [0.0f64; N];
            let mut per_class_br = [0.0f64; N];
            for (t, term) in self.terminals.iter().enumerate() {
                if term.path[p].is_none() {
                    let cv = cvs[t].as_ref().expect("valeur calculée");
                    for x in 0..N {
                        per_class_ev[x] += cv[x];
                        per_class_br[x] += cv[x];
                    }
                }
            }
            for (d, node) in self.nodes.iter().enumerate() {
                if node.player != p {
                    continue;
                }
                let [v0, v1] = self.action_values(d, &cvs);
                for x in 0..N {
                    let s = f64::from(strategy[d][x]);
                    per_class_ev[x] += (1.0 - s) * v0[x] + s * v1[x];
                    per_class_br[x] += v0[x].max(v1[x]);
                }
            }
            ev[p] = per_class_ev
                .iter()
                .zip(&self.prior)
                .map(|(v, w)| v * w)
                .sum();
            br[p] = per_class_br
                .iter()
                .zip(&self.prior)
                .map(|(v, w)| v * w)
                .sum();
        }
        (ev, br)
    }

    /// Résout le jeu par CFR+ et mesure l'exploitabilité de la stratégie moyenne.
    pub fn solve(&self, options: &RunOptions) -> RunResult {
        let nodes = self.nodes.len();
        let mut current: Vec<Vec<f32>> = vec![vec![0.5; N]; nodes];
        let mut regrets: Vec<Vec<[f64; 2]>> = vec![vec![[0.0; 2]; N]; nodes];
        let mut avg_sum: Vec<Vec<f64>> = vec![vec![0.0; N]; nodes];
        let mut weight_sum = 0.0f64;
        let average = |avg_sum: &[Vec<f64>], weight_sum: f64| -> Vec<Vec<f32>> {
            avg_sum
                .iter()
                .map(|s| s.iter().map(|v| (v / weight_sum) as f32).collect())
                .collect()
        };
        let check_every = options.check_every.max(1);
        let mut iterations = 0;
        let mut last_eval: Option<(Vec<f64>, Vec<f64>)> = None;
        for t in 1..=options.max_iterations.max(1) {
            iterations = t;
            let w = f64::from(t);
            for p in 0..self.n {
                let probs = probs_of(&current);
                let cvs = self.all_cvs(p, &probs, true);
                for d in (0..nodes).filter(|&d| self.nodes[d].player == p) {
                    let [v0, v1] = self.action_values(d, &cvs);
                    for x in 0..N {
                        let s = f64::from(current[d][x]);
                        avg_sum[d][x] += w * s;
                        let v = (1.0 - s) * v0[x] + s * v1[x];
                        let r = &mut regrets[d][x];
                        r[0] = (r[0] + v0[x] - v).max(0.0);
                        r[1] = (r[1] + v1[x] - v).max(0.0);
                        let total = r[0] + r[1];
                        current[d][x] = if total > 0.0 {
                            (r[1] / total) as f32
                        } else {
                            0.5
                        };
                    }
                }
            }
            weight_sum += w;
            if t % check_every == 0 || t == options.max_iterations {
                let (ev, br) = self.evaluate(&average(&avg_sum, weight_sum));
                let worst = br
                    .iter()
                    .zip(&ev)
                    .map(|(b, e)| b - e)
                    .fold(0.0f64, f64::max);
                last_eval = Some((ev, br));
                if worst < options.target {
                    break;
                }
            }
        }
        let strategy = average(&avg_sum, weight_sum);
        let (ev, br) = last_eval.unwrap_or_else(|| self.evaluate(&strategy));
        let exploitability = br.iter().zip(&ev).map(|(b, e)| (b - e).max(0.0)).collect();
        let action_ev = self.action_evs(&strategy);
        RunResult {
            strategy,
            action_ev,
            ev,
            exploitability,
            iterations,
        }
    }

    /// Valeur de chaque action, par nœud et par classe, divisée par la masse a priori des
    /// actions adverses menant au nœud (approximation : blocages ignorés dans la masse).
    fn action_evs(&self, strategy: &[Vec<f32>]) -> Vec<[Vec<f32>; 2]> {
        let probs = probs_of(strategy);
        let per_player: Vec<Vec<Option<Vec<f64>>>> =
            (0..self.n).map(|p| self.all_cvs(p, &probs, true)).collect();
        (0..self.nodes.len())
            .map(|d| {
                let p = self.nodes[d].player;
                let mass = self.reach_mass(d, &probs);
                let [v0, v1] = self.action_values(d, &per_player[p]);
                let norm = |v: Vec<f64>| -> Vec<f32> {
                    v.iter()
                        .map(|x| if mass > 1e-12 { (x / mass) as f32 } else { 0.0 })
                        .collect()
                };
                [norm(v0), norm(v1)]
            })
            .collect()
    }
}
