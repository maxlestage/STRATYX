//! Références stratégiques de Stratyx, partagées par les deux IA (Expresso 3-Max et Tournois
//! 6-Max) et par l'analyse des mains.
//!
//! | Module      | Contenu                                                                  |
//! |-------------|--------------------------------------------------------------------------|
//! | [`preflop`] | index des 169 classes, [`ClassTable`](preflop::ClassTable), matrice d'équité préflop 169 × 169 embarquée |
//! | [`pushfold`]| équilibres push/fold : tête-à-tête (chipEV) et 3-max (chipEV ou ICM), exploitabilité mesurée |
//! | [`river`]   | solveur de river CFR+ en tête-à-tête : stratégie et EV par main, exploitabilité |
//! | [`ranges`]  | ranges d'open **indicatives** 6-max et 9-max (heuristiques, priors pour estimer les ranges adverses) |
//!
//! # Reproductibilité
//!
//! Tous les calculs aléatoires utilisent ChaCha8 avec une graine fixe (décision T8) : la
//! matrice préflop (`examples/gen_preflop_matrix.rs`), la table des confrontations à trois
//! ([`pushfold::ThreeWayEquities`]) et, par conséquent, les équilibres push/fold sont
//! identiques d'une exécution à l'autre. Les solveurs (CFR+) sont déterministes.
//!
//! # Exemples
//!
//! ```text
//! cargo run --release -p stratyx-strategy --example gen_preflop_matrix   # régénère la matrice
//! cargo run --release -p stratyx-strategy --example pushfold_charts      # équilibres et temps
//! cargo run --release -p stratyx-strategy --example river_convergence    # convergence CFR+
//! ```
//!
//! ```
//! use stratyx_poker::position::Position;
//! use stratyx_strategy::preflop::{PreflopMatrix, parse_class};
//! use stratyx_strategy::pushfold::{HeadsUpConfig, SolverOptions, solve_heads_up};
//!
//! let aa = parse_class("AA").unwrap();
//! let kk = parse_class("KK").unwrap();
//! assert!((PreflopMatrix::embedded().equity(aa, kk) - 0.82).abs() < 0.01);
//!
//! let sol = solve_heads_up(
//!     &HeadsUpConfig { stack_bb: 10.0, ante_bb: 0.0 },
//!     &SolverOptions::HEADS_UP,
//! )
//! .unwrap();
//! assert!(sol.max_exploitability() < 0.01);
//! let push = sol.push_chart(Position::SmallBlind).unwrap();
//! assert!(push.frequency(aa) > 0.99);
//! println!("{push}"); // grille 13 × 13
//! ```

pub mod preflop;
pub mod pushfold;
pub mod ranges;
pub mod river;
