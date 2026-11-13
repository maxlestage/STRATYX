//! Références stratégiques de Stratyx, partagées par les deux IA (Expresso 3-Max et Tournois
//! 6-Max) et par l'analyse des mains.
//!
//! | Module      | Contenu                                                                  |
//! |-------------|--------------------------------------------------------------------------|
//! | [`preflop`] | index des 169 classes, [`ClassTable`](preflop::ClassTable), matrice d'équité préflop 169 × 169 embarquée |
//! | [`pushfold`]| équilibres push/fold : tête-à-tête (chipEV) et 3-max (chipEV ou ICM), exploitabilité mesurée |
//! | [`river`]   | solveur de river CFR+ en tête-à-tête : stratégie et EV par main, exploitabilité |
//! | [`ranges`]  | ranges d'open **indicatives** 6-max et 9-max (heuristiques, priors pour estimer les ranges adverses) |
//! | [`classes`], [`tables`] | les 169 classes ordonnées par force (contre 1, 2 et 5 mains au hasard), ranges « top X % » |
//! | [`charts`]  | tables push/fold à 3 et à 6 joueurs (jeu fictif sur simulations, avec antes) et EV d'un spot |
//! | [`spot`]    | une situation de décision et ses caractéristiques numériques |
//! | [`mc`]      | équité Monte-Carlo contre des ranges pondérées (graine explicite) |
//! | [`villain`] | ranges probables des adversaires d'après leurs actions |
//! | [`reference`] | décision de référence d'un spot, indépendante du résultat |
//! | [`model`]   | modèles appris (softmax, perceptron multicouche), en Rust pur |
//! | [`bundle`]  | les modèles d'une IA, réunis en un fichier JSON |
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

pub mod bundle;
pub mod charts;
pub mod classes;
pub mod mc;
pub mod model;
pub mod preflop;
pub mod pushfold;
pub mod ranges;
pub mod reference;
pub mod river;
pub mod spot;
pub mod tables;
pub mod villain;

pub use bundle::ModelBundle;
pub use reference::{Basis, Inputs, Reference, classify, reference};
pub use spot::{Decision, HandFeatures, Spot, decision_features};
