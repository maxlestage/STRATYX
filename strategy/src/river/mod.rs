//! Solveur de river CFR+ en tête-à-tête.
//!
//! Entrées ([`RiverConfig`]) : deux ranges pondérées (OOP parle en premier, IP en position),
//! board de cinq cartes, pot, tapis effectif, tailles de mise et de relance en fractions de
//! pot propres à chaque joueur, nombre maximal de relances et autorisation du tapis.
//!
//! Sorties ([`RiverSolution`]) : arbre de mises, stratégie moyenne par main et par nœud, EV
//! par main et exploitabilité mesurée par meilleure réponse exacte.
//!
//! # Conventions
//!
//! - Les montants (pot, tapis, mises) sont dans une même unité libre (BB, jetons…).
//! - Une mise de fraction `f` vaut `f × pot` ; une relance de fraction `f` ajoute, en plus du
//!   montant à suivre, `f × (pot après avoir suivi)` (relance « pot » pour `f = 1`), et au
//!   moins la relance minimale du no-limit. Une taille qui dépasse le tapis devient tapis ;
//!   les doublons sont fusionnés. Le tapis compte comme une relance pour `max_raises`.
//! - La **valeur** d'une main est son gain net depuis le début de la river, part du pot
//!   initial comprise : la somme des valeurs des deux joueurs vaut le pot (jeu à somme
//!   constante). L'EV d'une main est conditionnelle à cette main (moyenne sur les mains
//!   adverses compatibles, blocages compris).
//! - **Exploitabilité** : `(BR(OOP) + BR(IP) − pot) / 2`, où `BR(p)` est la valeur de la
//!   meilleure réponse de `p` contre la stratégie moyenne adverse ; nulle à l'équilibre.
//!
//! # Algorithme
//!
//! CFR+ vectorisé sur les combinaisons (regrets planchers à 0, mises à jour alternées,
//! moyenne linéaire). Les abattages sont évalués en temps linéaire après tri des mains par
//! force, avec correction exacte des cartes bloquées (inclusion-exclusion par carte).
//! Convergence mesurée (profil release) : jeu jouet polarisé sous 0,1 % du pot en quelques
//! centaines d'itérations ; river réaliste à plusieurs tailles sous 0,5 % du pot en moins de
//! 1 000 itérations (voir les tests du module).

mod tree;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use stratyx_poker::{CardSet, HoleCards, Range, evaluate};
use thiserror::Error;

pub use tree::{NodeKind, RiverAction, RiverNode};
use tree::{TreeParams, build_tree};

/// Joueur de la river : OOP (hors de position, parle en premier) ou IP (en position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Player {
    Oop,
    Ip,
}

impl Player {
    /// 0 pour OOP, 1 pour IP.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Player::Oop => 0,
            Player::Ip => 1,
        }
    }

    #[must_use]
    pub const fn opponent(self) -> Player {
        match self {
            Player::Oop => Player::Ip,
            Player::Ip => Player::Oop,
        }
    }

    pub(crate) const fn from_index(i: usize) -> Player {
        if i == 0 { Player::Oop } else { Player::Ip }
    }
}

/// Tailles disponibles pour un joueur, en fractions de pot.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BetSizes {
    /// Mises (personne n'a misé).
    #[serde(default)]
    pub bets: Vec<f64>,
    /// Relances (face à une mise ou une relance).
    #[serde(default)]
    pub raises: Vec<f64>,
}

/// Paramètres d'une river en tête-à-tête.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiverConfig {
    /// Board complet (cinq cartes).
    pub board: CardSet,
    pub oop_range: Range,
    pub ip_range: Range,
    /// Pot au début de la river.
    pub pot: f64,
    /// Tapis effectif au début de la river.
    pub effective_stack: f64,
    pub oop_sizes: BetSizes,
    pub ip_sizes: BetSizes,
    /// Nombre maximal de relances après la première mise.
    pub max_raises: u8,
    /// Ajoute le tapis aux options de mise et de relance.
    pub allow_all_in: bool,
}

/// Paramètres de résolution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RiverOptions {
    pub max_iterations: u32,
    /// Arrêt dès que l'exploitabilité passe sous cette fraction du pot (0,005 = 0,5 %).
    pub target_exploitability: f64,
    /// Fréquence de la mesure d'exploitabilité (en itérations).
    pub check_every: u32,
}

impl Default for RiverOptions {
    fn default() -> Self {
        RiverOptions {
            max_iterations: 2_000,
            target_exploitability: 0.002,
            check_every: 50,
        }
    }
}

/// Erreurs de paramétrage du solveur de river.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RiverError {
    #[error("le board doit compter 5 cartes (reçu : {0})")]
    Board(usize),
    #[error("pot invalide : {0} (valeur strictement positive et finie attendue)")]
    Pot(f64),
    #[error("tapis effectif invalide : {0} (valeur positive et finie attendue)")]
    Stack(f64),
    #[error("taille de mise invalide : {0} (fraction de pot strictement positive attendue)")]
    BetSize(f64),
    #[error("la range de {0:?} est vide une fois retirées les cartes du board")]
    EmptyRange(Player),
    #[error("aucune confrontation possible : les ranges se bloquent mutuellement")]
    NoMatchup,
}

/// Mains d'un joueur, compatibles avec le board.
#[derive(Debug, Clone)]
struct Hands {
    combos: Vec<HoleCards>,
    weights: Vec<f32>,
    cards: Vec<[usize; 2]>,
    strength: Vec<u32>,
    /// Index triés par force croissante.
    sorted: Vec<usize>,
    /// Index de la même combinaison chez l'adversaire (`usize::MAX` si absente).
    same: Vec<usize>,
}

impl Hands {
    fn new(range: &Range, board: CardSet) -> Hands {
        let (combos, weights): (Vec<HoleCards>, Vec<f32>) = range
            .iter()
            .filter(|(h, _)| !h.set().intersects(board))
            .unzip();
        let cards = combos
            .iter()
            .map(|h| h.cards().map(|c| usize::from(c.id())))
            .collect();
        let strength: Vec<u32> = combos
            .iter()
            .map(|h| evaluate(board | h.set()).value())
            .collect();
        let mut sorted: Vec<usize> = (0..combos.len()).collect();
        sorted.sort_by_key(|&i| strength[i]);
        Hands {
            combos,
            weights,
            cards,
            strength,
            sorted,
            same: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.combos.len()
    }
}

/// Données immuables du jeu.
#[derive(Debug, Clone)]
struct Game {
    pot: f64,
    nodes: Vec<RiverNode>,
    hands: [Hands; 2],
    /// Masse totale des confrontations compatibles : Σ w(h) w(h') sur les couples disjoints.
    norm: f64,
}

/// Tables mutables de CFR+ : regrets et cumul de la stratégie moyenne, par nœud de décision
/// (`[action × mains + main]`).
#[derive(Debug, Clone)]
struct Tables {
    regrets: Vec<Vec<f32>>,
    average: Vec<Vec<f32>>,
}

/// Mode d'évaluation d'une stratégie fixée.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Eval {
    Average,
    BestResponse,
}

impl Game {
    /// Somme des poids adverses compatibles avec chaque main de `trav`.
    fn compatible(&self, trav: usize, opp_reach: &[f32]) -> Vec<f64> {
        let opp = &self.hands[1 - trav];
        let mut total = 0.0f64;
        let mut per_card = [0.0f64; 52];
        for (i, r) in opp_reach.iter().enumerate() {
            let r = f64::from(*r);
            total += r;
            per_card[opp.cards[i][0]] += r;
            per_card[opp.cards[i][1]] += r;
        }
        let me = &self.hands[trav];
        (0..me.len())
            .map(|h| {
                let [a, b] = me.cards[h];
                let same = me.same[h];
                let back = if same == usize::MAX {
                    0.0
                } else {
                    f64::from(opp_reach[same])
                };
                total - per_card[a] - per_card[b] + back
            })
            .collect()
    }

    fn fold_values(
        &self,
        node: &RiverNode,
        folder: usize,
        trav: usize,
        opp_reach: &[f32],
    ) -> Vec<f32> {
        let payoff = if folder == trav {
            -node.contributions[trav]
        } else {
            self.pot + node.contributions[folder]
        };
        self.compatible(trav, opp_reach)
            .into_iter()
            .map(|c| (c * payoff) as f32)
            .collect()
    }

    fn showdown_values(&self, node: &RiverNode, trav: usize, opp_reach: &[f32]) -> Vec<f32> {
        let me = &self.hands[trav];
        let opp = &self.hands[1 - trav];
        let n = me.len();
        let total_pot = self.pot + node.contributions[0] + node.contributions[1];
        let cost = node.contributions[trav];
        // Passe 1 : masse adverse strictement plus faible ; passe 2 : plus faible ou égale.
        let sweep = |or_equal: bool| -> Vec<f64> {
            let mut out = vec![0.0f64; n];
            let mut total = 0.0f64;
            let mut per_card = [0.0f64; 52];
            let mut j = 0;
            for &h in &me.sorted {
                let s = me.strength[h];
                while j < opp.sorted.len() {
                    let o = opp.sorted[j];
                    let below = if or_equal {
                        opp.strength[o] <= s
                    } else {
                        opp.strength[o] < s
                    };
                    if !below {
                        break;
                    }
                    let r = f64::from(opp_reach[o]);
                    total += r;
                    per_card[opp.cards[o][0]] += r;
                    per_card[opp.cards[o][1]] += r;
                    j += 1;
                }
                let [a, b] = me.cards[h];
                let mut v = total - per_card[a] - per_card[b];
                // La même combinaison (même force) n'est comptée qu'en passe « ou égale ».
                if or_equal && me.same[h] != usize::MAX {
                    v += f64::from(opp_reach[me.same[h]]);
                }
                out[h] = v;
            }
            out
        };
        let below = sweep(false);
        let below_or_equal = sweep(true);
        let compatible = self.compatible(trav, opp_reach);
        (0..n)
            .map(|h| {
                let tie = below_or_equal[h] - below[h];
                (total_pot * (below[h] + 0.5 * tie) - cost * compatible[h]) as f32
            })
            .collect()
    }

    fn terminal_values(&self, index: usize, trav: usize, opp_reach: &[f32]) -> Option<Vec<f32>> {
        let node = &self.nodes[index];
        match node.kind {
            NodeKind::Fold(p) => Some(self.fold_values(node, p.index(), trav, opp_reach)),
            NodeKind::Showdown => Some(self.showdown_values(node, trav, opp_reach)),
            NodeKind::Decision(_) => None,
        }
    }

    /// Une itération CFR+ pour le joueur `trav` ; renvoie ses valeurs contrefactuelles.
    fn cfr(
        &self,
        tables: &mut Tables,
        index: usize,
        trav: usize,
        opp_reach: &[f32],
        weight: f32,
    ) -> Vec<f32> {
        if let Some(v) = self.terminal_values(index, trav, opp_reach) {
            return v;
        }
        let node = &self.nodes[index];
        let NodeKind::Decision(actor) = node.kind else {
            unreachable!("nœud de décision")
        };
        let player = actor.index();
        let na = node.actions.len();
        let nh = self.hands[player].len();
        if opp_reach.iter().all(|r| *r == 0.0) {
            return vec![0.0; self.hands[trav].len()];
        }
        let strategy = regret_matching(&tables.regrets[index], na, nh);
        if player == trav {
            let children: Vec<Vec<f32>> = node
                .children
                .iter()
                .map(|&c| self.cfr(tables, c, trav, opp_reach, weight))
                .collect();
            let mut values = vec![0.0f32; nh];
            for (a, cv) in children.iter().enumerate() {
                for h in 0..nh {
                    values[h] += strategy[a * nh + h] * cv[h];
                }
            }
            let regrets = &mut tables.regrets[index];
            for (a, cv) in children.iter().enumerate() {
                for h in 0..nh {
                    let r = &mut regrets[a * nh + h];
                    *r = (*r + cv[h] - values[h]).max(0.0);
                }
            }
            values
        } else {
            let mut values = vec![0.0f32; self.hands[trav].len()];
            let mut reach = vec![0.0f32; nh];
            for (a, &child) in node.children.iter().enumerate() {
                for h in 0..nh {
                    reach[h] = opp_reach[h] * strategy[a * nh + h];
                }
                let cv = self.cfr(tables, child, trav, &reach, weight);
                for (v, c) in values.iter_mut().zip(&cv) {
                    *v += c;
                }
                let avg = &mut tables.average[index];
                for h in 0..nh {
                    avg[a * nh + h] += weight * reach[h];
                }
            }
            values
        }
    }

    /// Valeurs contrefactuelles de `trav` quand les deux joueurs suivent la stratégie moyenne
    /// (`Eval::Average`) ou quand `trav` joue sa meilleure réponse (`Eval::BestResponse`).
    fn evaluate(
        &self,
        tables: &Tables,
        index: usize,
        trav: usize,
        opp_reach: &[f32],
        mode: Eval,
    ) -> Vec<f32> {
        if let Some(v) = self.terminal_values(index, trav, opp_reach) {
            return v;
        }
        let node = &self.nodes[index];
        let NodeKind::Decision(actor) = node.kind else {
            unreachable!("nœud de décision")
        };
        let player = actor.index();
        let na = node.actions.len();
        let nh = self.hands[player].len();
        let strategy = normalize(&tables.average[index], na, nh);
        if player == trav {
            let mut values = vec![
                if mode == Eval::BestResponse {
                    f32::NEG_INFINITY
                } else {
                    0.0
                };
                nh
            ];
            for (a, &child) in node.children.iter().enumerate() {
                let cv = self.evaluate(tables, child, trav, opp_reach, mode);
                for h in 0..nh {
                    values[h] = match mode {
                        Eval::BestResponse => values[h].max(cv[h]),
                        Eval::Average => values[h] + strategy[a * nh + h] * cv[h],
                    };
                }
            }
            values
        } else {
            let mut values = vec![0.0f32; self.hands[trav].len()];
            let mut reach = vec![0.0f32; nh];
            for (a, &child) in node.children.iter().enumerate() {
                for h in 0..nh {
                    reach[h] = opp_reach[h] * strategy[a * nh + h];
                }
                if reach.iter().all(|r| *r == 0.0) {
                    continue;
                }
                let cv = self.evaluate(tables, child, trav, &reach, mode);
                for (v, c) in values.iter_mut().zip(&cv) {
                    *v += c;
                }
            }
            values
        }
    }

    /// Valeur globale de `trav` (gain moyen par confrontation) à partir de ses valeurs
    /// contrefactuelles à la racine.
    fn total_value(&self, trav: usize, cfv: &[f32]) -> f64 {
        self.hands[trav]
            .weights
            .iter()
            .zip(cfv)
            .map(|(w, v)| f64::from(*w) * f64::from(*v))
            .sum::<f64>()
            / self.norm
    }
}

/// Stratégie courante par regret matching (regrets déjà positifs en CFR+).
fn regret_matching(regrets: &[f32], na: usize, nh: usize) -> Vec<f32> {
    normalize(regrets, na, nh)
}

/// Normalise des poids positifs par main ; uniforme si tous sont nuls.
fn normalize(weights: &[f32], na: usize, nh: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; na * nh];
    for h in 0..nh {
        let total: f32 = (0..na).map(|a| weights[a * nh + h]).sum();
        for a in 0..na {
            out[a * nh + h] = if total > 0.0 {
                weights[a * nh + h] / total
            } else {
                1.0 / na as f32
            };
        }
    }
    out
}

/// Solveur CFR+ d'une river en tête-à-tête.
#[derive(Debug, Clone)]
pub struct RiverSolver {
    config: RiverConfig,
    game: Game,
    tables: Tables,
    iterations: u32,
}

/// Résultat de la résolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiverSolution {
    pub config: RiverConfig,
    /// Arbre de mises ; la racine (index 0) est la décision de OOP.
    pub nodes: Vec<RiverNode>,
    /// Mains de OOP et de IP compatibles avec le board.
    pub hands: [Vec<HoleCards>; 2],
    /// Fréquence de chaque main dans sa range.
    pub weights: [Vec<f32>; 2],
    /// Stratégie moyenne par nœud de décision : `strategies[nœud][action][main]` (vide pour
    /// une feuille), mains du joueur qui agit.
    pub strategies: Vec<Vec<Vec<f32>>>,
    /// EV de chaque main (gain net depuis le début de la river, part du pot comprise).
    pub ev: [Vec<f32>; 2],
    /// Valeur moyenne de chaque joueur ; leur somme vaut le pot.
    pub value: [f64; 2],
    /// Exploitabilité, dans l'unité du pot.
    pub exploitability: f64,
    pub iterations: u32,
}

impl RiverSolver {
    /// Vérifie les paramètres et construit l'arbre.
    pub fn new(config: RiverConfig) -> Result<RiverSolver, RiverError> {
        if config.board.len() != 5 {
            return Err(RiverError::Board(config.board.len()));
        }
        if !(config.pot.is_finite() && config.pot > 0.0) {
            return Err(RiverError::Pot(config.pot));
        }
        if !(config.effective_stack.is_finite() && config.effective_stack >= 0.0) {
            return Err(RiverError::Stack(config.effective_stack));
        }
        for &f in config
            .oop_sizes
            .bets
            .iter()
            .chain(&config.oop_sizes.raises)
            .chain(&config.ip_sizes.bets)
            .chain(&config.ip_sizes.raises)
        {
            if !(f.is_finite() && f > 0.0) {
                return Err(RiverError::BetSize(f));
            }
        }
        let mut hands = [
            Hands::new(&config.oop_range, config.board),
            Hands::new(&config.ip_range, config.board),
        ];
        for p in [Player::Oop, Player::Ip] {
            if hands[p.index()].len() == 0 {
                return Err(RiverError::EmptyRange(p));
            }
        }
        for p in 0..2 {
            let other = &hands[1 - p];
            let mut index_of = vec![usize::MAX; stratyx_poker::range::NUM_COMBOS];
            for (i, h) in other.combos.iter().enumerate() {
                index_of[h.index()] = i;
            }
            let same = hands[p]
                .combos
                .iter()
                .map(|h| index_of[h.index()])
                .collect();
            hands[p].same = same;
        }
        let sizes = [config.oop_sizes.clone(), config.ip_sizes.clone()];
        let nodes = build_tree(&TreeParams {
            pot: config.pot,
            stack: config.effective_stack,
            sizes: &sizes,
            max_raises: config.max_raises,
            allow_all_in: config.allow_all_in,
        });
        let mut game = Game {
            pot: config.pot,
            nodes,
            hands,
            norm: 1.0,
        };
        let compatible = game.compatible(0, &game.hands[1].weights);
        let norm: f64 = game.hands[0]
            .weights
            .iter()
            .zip(&compatible)
            .map(|(w, c)| f64::from(*w) * c)
            .sum();
        if norm <= 0.0 {
            return Err(RiverError::NoMatchup);
        }
        game.norm = norm;
        let zeros: Vec<Vec<f32>> = game
            .nodes
            .iter()
            .map(|n| match n.kind {
                NodeKind::Decision(p) => vec![0.0; n.actions.len() * game.hands[p.index()].len()],
                _ => Vec::new(),
            })
            .collect();
        let tables = Tables {
            regrets: zeros.clone(),
            average: zeros,
        };
        Ok(RiverSolver {
            config,
            game,
            tables,
            iterations: 0,
        })
    }

    /// Effectue `count` itérations CFR+ supplémentaires.
    pub fn iterate(&mut self, count: u32) {
        for _ in 0..count {
            self.iterations += 1;
            let weight = self.iterations as f32;
            for trav in 0..2 {
                let opp_reach = self.game.hands[1 - trav].weights.clone();
                self.game.cfr(&mut self.tables, 0, trav, &opp_reach, weight);
            }
        }
    }

    /// Nombre d'itérations effectuées.
    #[must_use]
    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    /// Arbre de mises.
    #[must_use]
    pub fn nodes(&self) -> &[RiverNode] {
        &self.game.nodes
    }

    fn root_values(&self, trav: usize, mode: Eval) -> Vec<f32> {
        let opp_reach = &self.game.hands[1 - trav].weights;
        self.game.evaluate(&self.tables, 0, trav, opp_reach, mode)
    }

    /// Exploitabilité de la stratégie moyenne, dans l'unité du pot (voir le module).
    #[must_use]
    pub fn exploitability(&self) -> f64 {
        let br: f64 = (0..2)
            .map(|p| {
                self.game
                    .total_value(p, &self.root_values(p, Eval::BestResponse))
            })
            .sum();
        ((br - self.game.pot) / 2.0).max(0.0)
    }

    /// Itère jusqu'à l'objectif d'exploitabilité ou au nombre maximal d'itérations.
    pub fn run(&mut self, options: &RiverOptions) {
        let target = options.target_exploitability * self.game.pot;
        let step = options.check_every.max(1);
        while self.iterations < options.max_iterations {
            let todo = step.min(options.max_iterations - self.iterations);
            self.iterate(todo);
            if self.exploitability() < target {
                break;
            }
        }
    }

    /// Paramètres, valide puis résout.
    pub fn solve(config: RiverConfig, options: &RiverOptions) -> Result<RiverSolution, RiverError> {
        let mut solver = RiverSolver::new(config)?;
        solver.run(options);
        Ok(solver.solution())
    }

    /// Instantané de la stratégie moyenne, des EV et de l'exploitabilité.
    #[must_use]
    pub fn solution(&self) -> RiverSolution {
        let game = &self.game;
        let strategies = game
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| match n.kind {
                NodeKind::Decision(p) => {
                    let na = n.actions.len();
                    let nh = game.hands[p.index()].len();
                    let s = normalize(&self.tables.average[i], na, nh);
                    s.chunks(nh).map(<[f32]>::to_vec).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        let mut ev: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        let mut value = [0.0; 2];
        for p in 0..2 {
            let cfv = self.root_values(p, Eval::Average);
            value[p] = game.total_value(p, &cfv);
            let compatible = game.compatible(p, &game.hands[1 - p].weights);
            ev[p] = cfv
                .iter()
                .zip(&compatible)
                .map(|(v, c)| {
                    if *c > 0.0 {
                        (f64::from(*v) / c) as f32
                    } else {
                        0.0
                    }
                })
                .collect();
        }
        RiverSolution {
            config: self.config.clone(),
            nodes: game.nodes.clone(),
            hands: [game.hands[0].combos.clone(), game.hands[1].combos.clone()],
            weights: [game.hands[0].weights.clone(), game.hands[1].weights.clone()],
            strategies,
            ev,
            value,
            exploitability: self.exploitability(),
            iterations: self.iterations,
        }
    }
}

impl RiverSolution {
    /// Exploitabilité en fraction du pot.
    #[must_use]
    pub fn exploitability_fraction(&self) -> f64 {
        self.exploitability / self.config.pot
    }

    /// Index d'une main dans la liste du joueur.
    #[must_use]
    pub fn hand_index(&self, player: Player, hand: HoleCards) -> Option<usize> {
        self.hands[player.index()].iter().position(|h| *h == hand)
    }

    /// EV d'une main (voir [`RiverSolution::ev`]).
    #[must_use]
    pub fn hand_ev(&self, player: Player, hand: HoleCards) -> Option<f32> {
        self.hand_index(player, hand)
            .map(|i| self.ev[player.index()][i])
    }

    /// Stratégie d'une main au nœud `node` : `(action, fréquence)`. `None` si le nœud n'est
    /// pas une décision ou si la main n'appartient pas au joueur qui agit.
    #[must_use]
    pub fn strategy(&self, node: usize, hand: HoleCards) -> Option<Vec<(RiverAction, f32)>> {
        let n = self.nodes.get(node)?;
        let NodeKind::Decision(p) = n.kind else {
            return None;
        };
        let h = self.hand_index(p, hand)?;
        Some(
            n.actions
                .iter()
                .zip(&self.strategies[node])
                .map(|(a, s)| (*a, s[h]))
                .collect(),
        )
    }

    /// Poids de chaque main du joueur `player` au nœud `node` : fréquence dans la range
    /// multipliée par ses propres probabilités d'action le long du chemin.
    #[must_use]
    pub fn reach(&self, node: usize, player: Player) -> Vec<f32> {
        let mut reach = self.weights[player.index()].clone();
        let mut current = node;
        while let Some((parent, action)) = self.nodes[current].parent {
            if self.nodes[parent].kind == NodeKind::Decision(player) {
                for (r, s) in reach.iter_mut().zip(&self.strategies[parent][action]) {
                    *r *= s;
                }
            }
            current = parent;
        }
        reach
    }

    /// Fréquence globale de chaque action au nœud `node`, pondérée par les mains du joueur
    /// qui l'atteignent.
    #[must_use]
    pub fn action_frequencies(&self, node: usize) -> Option<Vec<(RiverAction, f64)>> {
        let n = self.nodes.get(node)?;
        let NodeKind::Decision(p) = n.kind else {
            return None;
        };
        let reach = self.reach(node, p);
        let total: f64 = reach.iter().map(|r| f64::from(*r)).sum();
        Some(
            n.actions
                .iter()
                .zip(&self.strategies[node])
                .map(|(a, s)| {
                    let f: f64 = reach
                        .iter()
                        .zip(s)
                        .map(|(r, x)| f64::from(*r) * f64::from(*x))
                        .sum();
                    (*a, if total > 0.0 { f / total } else { 0.0 })
                })
                .collect(),
        )
    }
}
