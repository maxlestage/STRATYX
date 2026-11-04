//! Équilibres push/fold : tête-à-tête (chipEV) et 3-max (chipEV ou ICM).
//!
//! Chaque joueur n'a que deux options : tapis (ou suivre un tapis) et se coucher. Les
//! stratégies sont des fréquences par classe de main (169), affichables en grille 13 × 13
//! ([`PushFoldChart`]).
//!
//! | Fonction              | Jeu                                                          |
//! |-----------------------|--------------------------------------------------------------|
//! | [`solve_heads_up`]    | SB pousse ou se couche, BB suit ou se couche ; chipEV        |
//! | [`solve_three_max`]   | BTN, SB, BB (ordre de parole 3-max), sur-call à trois inclus ; chipEV ou ICM |
//!
//! # Algorithme et garanties
//!
//! CFR+ vectorisé sur les 169 classes, pondérées par leurs combinaisons et par les blocages
//! entre joueurs en jeu (voir le moteur, module privé `engine`). Les équités à deux joueurs
//! viennent de la matrice préflop embarquée ([`PreflopMatrix::embedded`]) ; celles à trois
//! joueurs de [`ThreeWayEquities`] (Monte-Carlo à graine fixe, en cache). Tout est
//! déterministe.
//!
//! L'**exploitabilité** est mesurée exactement par meilleure réponse : pour chaque joueur,
//! gain de la meilleure réponse contre la stratégie moyenne des autres, en BB par main
//! (équivalent-BB en ICM, voir [`PayoffModel::Icm`]).
//!
//! | Jeu | Exigence testée | Cible par défaut | Mesures (release, 4 cœurs virtuels) |
//! |-----|-----------------|------------------|-------------------------------------|
//! | Tête-à-tête, 1 à 25 BB | < 0,01 BB par joueur | < 10⁻⁴ BB | 100 à 250 itérations, < 0,02 s par profondeur |
//! | 3-max, chipEV ou ICM | < 0,05 BB par joueur | < 10⁻⁴ BB | 100 à 200 itérations, ≈ 0,5 s par profondeur |
//!
//! À trois joueurs s'ajoute, une fois par processus, le calcul de la table des
//! confrontations à trois ([`ThreeWayEquities::shared`], ≈ 11 s avec 4 096 boards) : une
//! première profondeur coûte donc ≈ 12 s, les suivantes ≈ 0,5 s. CFR+ ne garantit pas la
//! convergence vers un équilibre de Nash à trois joueurs ; elle est donc **vérifiée** à
//! chaque résolution par la mesure d'exploitabilité ci-dessus (10⁻⁵ BB atteint en ≈ 500
//! itérations sur les configurations testées).
//!
//! # Convention des tapis
//!
//! Les tapis sont donnés **avant** de poster blindes et antes, en grosses blindes ; la petite
//! blinde vaut 0,5 BB et chaque joueur paie l'ante (`ante_bb`, éventuellement nulle). En
//! tête-à-tête, le tapis est le tapis effectif des deux joueurs, et la petite blinde (qui est
//! aussi le bouton) est notée [`Position::SmallBlind`].

mod engine;
mod payoff;
#[cfg(test)]
mod tests;
mod three_way;

use std::fmt;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use stratyx_poker::position::Position;
use stratyx_poker::{HandClass, Range};
use thiserror::Error;

pub use payoff::PayoffModel;
pub use three_way::{NUM_OUTCOMES, NUM_TRIPLES, OUTCOMES, ThreeWayEquities, ThreeWayOptions};

use crate::preflop::{ClassTable, PreflopMatrix, classes};
use engine::{Game, RunOptions};

/// Fréquences d'une action (tapis ou suivi) par classe de main.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PushFoldChart {
    frequencies: ClassTable,
}

impl PushFoldChart {
    #[must_use]
    pub fn new(frequencies: ClassTable) -> PushFoldChart {
        PushFoldChart { frequencies }
    }

    /// Fréquence de l'action pour la classe, dans `[0, 1]`.
    #[must_use]
    pub fn frequency(&self, class: HandClass) -> f64 {
        f64::from(self.frequencies.get(class))
    }

    #[must_use]
    pub fn frequencies(&self) -> &ClassTable {
        &self.frequencies
    }

    /// Part des 1 326 combinaisons jouées (fréquences pondérées par les combinaisons).
    #[must_use]
    pub fn fraction(&self) -> f64 {
        self.frequencies.combo_weighted_mean()
    }

    /// Range pondérée correspondante (fréquence de chaque combinaison).
    #[must_use]
    pub fn to_range(&self) -> Range {
        let mut r = Range::empty();
        for (class, f) in self.frequencies.iter() {
            if f > 0.0 {
                r.set_class(class, f);
            }
        }
        r
    }

    /// Fréquences en grille 13 × 13 (voir [`HandClass::grid_position`]).
    #[must_use]
    pub fn grid(&self) -> [[f32; 13]; 13] {
        self.frequencies.grid()
    }

    /// Classes jouées au moins à la fréquence `threshold`.
    #[must_use]
    pub fn classes_at_least(&self, threshold: f64) -> Vec<HandClass> {
        classes()
            .filter(|c| self.frequency(*c) >= threshold)
            .collect()
    }
}

impl fmt::Display for PushFoldChart {
    /// Grille 13 × 13 en pourcentages ; « . » pour une classe jamais jouée.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const RANKS: &str = "AKQJT98765432";
        write!(f, "  ")?;
        for r in RANKS.chars() {
            write!(f, "{r:>5}")?;
        }
        for (row, values) in self.grid().iter().enumerate() {
            write!(f, "\n{} ", RANKS.as_bytes()[row] as char)?;
            for v in values {
                let pct = (f64::from(*v) * 100.0).round();
                if pct <= 0.0 {
                    write!(f, "{:>5}", ".")?;
                } else {
                    write!(f, "{pct:>5}")?;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for PushFoldChart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PushFoldChart({:.1} %)", 100.0 * self.fraction())
    }
}

/// Nature de l'action agressive d'un nœud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotAction {
    /// Personne n'est à tapis : pousser ou se coucher.
    Push,
    /// Au moins un joueur est à tapis : suivre ou se coucher.
    Call,
}

/// Un nœud de décision de l'équilibre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushFoldSpot {
    pub position: Position,
    /// Joueurs déjà à tapis, dans l'ordre de parole (vide : personne n'a poussé).
    pub facing: Vec<Position>,
    pub action: SpotAction,
    /// Fréquence de l'action agressive (tapis ou suivi) par classe.
    pub chart: PushFoldChart,
    /// EV de se coucher, par classe, dans l'unité de la solution, relative au tapis de début
    /// de main. Hors nœud racine, valeur contrefactuelle divisée par la probabilité a priori
    /// d'atteindre le nœud (approximation : blocages ignorés dans cette probabilité).
    pub ev_fold: ClassTable,
    /// EV de l'action agressive, même convention.
    pub ev_aggressive: ClassTable,
}

/// Équilibre push/fold calculé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushFoldSolution {
    /// Joueurs dans l'ordre de parole.
    pub positions: Vec<Position>,
    /// Tapis avant blindes et antes, en BB, dans l'ordre de parole.
    pub stacks_bb: Vec<f64>,
    pub ante_bb: f64,
    pub payoff: PayoffModel,
    pub spots: Vec<PushFoldSpot>,
    /// Espérance de chaque joueur par main à l'équilibre (BB ou équivalent-BB).
    pub ev: Vec<f64>,
    /// Gain de la meilleure réponse de chaque joueur contre la solution (même unité).
    pub exploitability: Vec<f64>,
    pub iterations: u32,
}

impl PushFoldSolution {
    /// Nœud d'un joueur face aux joueurs déjà à tapis `facing` (ordre de parole).
    #[must_use]
    pub fn spot(&self, position: Position, facing: &[Position]) -> Option<&PushFoldSpot> {
        self.spots
            .iter()
            .find(|s| s.position == position && s.facing == facing)
    }

    /// Range de tapis d'un joueur quand personne n'a poussé avant lui.
    #[must_use]
    pub fn push_chart(&self, position: Position) -> Option<&PushFoldChart> {
        self.spot(position, &[]).map(|s| &s.chart)
    }

    /// Range de suivi d'un joueur face aux tapis de `facing`.
    #[must_use]
    pub fn call_chart(&self, position: Position, facing: &[Position]) -> Option<&PushFoldChart> {
        if facing.is_empty() {
            return None;
        }
        self.spot(position, facing).map(|s| &s.chart)
    }

    /// Plus grand gain de meilleure réponse parmi les joueurs.
    #[must_use]
    pub fn max_exploitability(&self) -> f64 {
        self.exploitability.iter().copied().fold(0.0, f64::max)
    }
}

/// Erreurs de paramétrage des solveurs push/fold.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PushFoldError {
    #[error(
        "tapis invalide pour {position} : {stack} BB (au moins {minimum} BB, blinde et ante comprises)"
    )]
    Stack {
        position: Position,
        stack: f64,
        minimum: f64,
    },
    #[error("ante invalide : {0} BB (valeur positive et finie attendue)")]
    Ante(f64),
    #[error("structure de prix invalide : {0}")]
    Payouts(String),
}

/// Paramètres de l'itération CFR+.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SolverOptions {
    /// Nombre maximal d'itérations.
    pub max_iterations: u32,
    /// Arrêt dès que le gain de meilleure réponse de chaque joueur passe sous ce seuil
    /// (BB ou équivalent-BB par main).
    pub target_exploitability: f64,
    /// Fréquence de la mesure d'exploitabilité (en itérations).
    pub check_every: u32,
}

impl SolverOptions {
    /// Réglages par défaut en tête-à-tête.
    pub const HEADS_UP: SolverOptions = SolverOptions {
        max_iterations: 20_000,
        target_exploitability: 1e-4,
        check_every: 50,
    };

    /// Réglages par défaut à trois joueurs.
    pub const THREE_MAX: SolverOptions = SolverOptions {
        max_iterations: 4_000,
        target_exploitability: 1e-4,
        check_every: 50,
    };
}

impl Default for SolverOptions {
    fn default() -> Self {
        SolverOptions::HEADS_UP
    }
}

/// Paramètres d'un push/fold en tête-à-tête.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeadsUpConfig {
    /// Tapis effectif avant blindes et antes, en BB (au moins 1 BB + ante).
    pub stack_bb: f64,
    /// Ante payée par chaque joueur, en BB (0 sans ante).
    #[serde(default)]
    pub ante_bb: f64,
}

/// Paramètres d'un push/fold à trois joueurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreeMaxConfig {
    /// Tapis de BTN, SB et BB avant blindes et antes, en BB.
    pub stacks_bb: [f64; 3],
    /// Ante payée par chaque joueur, en BB (0 sans ante).
    #[serde(default)]
    pub ante_bb: f64,
    pub payoff: PayoffModel,
}

fn check_ante(ante: f64) -> Result<(), PushFoldError> {
    if ante.is_finite() && ante >= 0.0 {
        Ok(())
    } else {
        Err(PushFoldError::Ante(ante))
    }
}

/// Vérifie les tapis et renvoie les montants postés (blinde + ante) dans l'ordre de parole.
fn posted_amounts(
    positions: &[Position],
    stacks: &[f64],
    ante: f64,
) -> Result<Vec<f64>, PushFoldError> {
    positions
        .iter()
        .zip(stacks)
        .map(|(&position, &stack)| {
            let blind = match position {
                Position::SmallBlind => 0.5,
                Position::BigBlind => 1.0,
                _ => 0.0,
            };
            let minimum = blind + ante;
            if !stack.is_finite() || stack < minimum || stack <= 0.0 {
                Err(PushFoldError::Stack {
                    position,
                    stack,
                    minimum,
                })
            } else {
                Ok(minimum)
            }
        })
        .collect()
}

fn solve_game(
    positions: Vec<Position>,
    stacks: Vec<f64>,
    ante: f64,
    payoff: PayoffModel,
    three_way: Option<&ThreeWayEquities>,
    options: &SolverOptions,
) -> Result<PushFoldSolution, PushFoldError> {
    check_ante(ante)?;
    payoff.validate().map_err(PushFoldError::Payouts)?;
    let posted = posted_amounts(&positions, &stacks, ante)?;
    let game = Game::new(
        &stacks,
        &posted,
        &payoff,
        PreflopMatrix::embedded(),
        three_way,
    );
    let run = game.solve(&RunOptions {
        max_iterations: options.max_iterations,
        target: options.target_exploitability,
        check_every: options.check_every,
    });
    let spots = game
        .nodes
        .iter()
        .enumerate()
        .map(|(d, node)| {
            let table = |v: &[f32]| ClassTable::from_values(v.to_vec()).expect("169 valeurs");
            PushFoldSpot {
                position: positions[node.player],
                facing: node.facing.iter().map(|&s| positions[s]).collect(),
                action: if node.facing.is_empty() {
                    SpotAction::Push
                } else {
                    SpotAction::Call
                },
                chart: PushFoldChart::new(table(&run.strategy[d])),
                ev_fold: table(&run.action_ev[d][0]),
                ev_aggressive: table(&run.action_ev[d][1]),
            }
        })
        .collect();
    Ok(PushFoldSolution {
        positions,
        stacks_bb: stacks,
        ante_bb: ante,
        payoff,
        spots,
        ev: run.ev,
        exploitability: run.exploitability,
        iterations: run.iterations,
    })
}

/// Équilibre push/fold en tête-à-tête, en chipEV.
///
/// Nœuds de la solution : SB face à personne ([`SpotAction::Push`]) et BB face au SB
/// ([`SpotAction::Call`]).
pub fn solve_heads_up(
    config: &HeadsUpConfig,
    options: &SolverOptions,
) -> Result<PushFoldSolution, PushFoldError> {
    solve_game(
        vec![Position::SmallBlind, Position::BigBlind],
        vec![config.stack_bb; 2],
        config.ante_bb,
        PayoffModel::ChipEv,
        None,
        options,
    )
}

/// Résout plusieurs profondeurs de tapis en parallèle (même ante), dans l'ordre donné.
pub fn solve_heads_up_table(
    stacks_bb: &[f64],
    ante_bb: f64,
    options: &SolverOptions,
) -> Result<Vec<PushFoldSolution>, PushFoldError> {
    stacks_bb
        .par_iter()
        .map(|&stack_bb| solve_heads_up(&HeadsUpConfig { stack_bb, ante_bb }, options))
        .collect()
}

/// Équilibre push/fold à trois joueurs (BTN, SB, BB), en chipEV ou en ICM.
///
/// Nœuds : BTN pousse ; SB pousse (BTN couché) ou suit le BTN ; BB suit le SB, suit le BTN,
/// ou sur-suit (BTN et SB à tapis). `three_way` fournit les classements à trois
/// ([`ThreeWayEquities::shared`] convient dans la plupart des cas). La résolution alloue
/// ≈ 60 Mo de tables temporaires (gains de l'abattage à trois pour chaque triplet ordonné).
pub fn solve_three_max(
    config: &ThreeMaxConfig,
    three_way: &ThreeWayEquities,
    options: &SolverOptions,
) -> Result<PushFoldSolution, PushFoldError> {
    solve_game(
        vec![Position::Button, Position::SmallBlind, Position::BigBlind],
        config.stacks_bb.to_vec(),
        config.ante_bb,
        config.payoff.clone(),
        Some(three_way),
        options,
    )
}
