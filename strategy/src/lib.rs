//! Références stratégiques partagées par les deux IA (Expresso 3-Max, Tournois 6-Max) et
//! par l'analyse des mains.
//!
//! | Module | Rôle |
//! |---|---|
//! | [`preflop`], [`tables`] | les 169 classes de mains de départ, leur force, les ranges « top X % » |
//! | [`spot`] | une situation de décision et ses caractéristiques numériques |
//! | [`mc`] | équité Monte-Carlo contre des ranges pondérées (graine explicite) |
//! | [`villain`] | ranges probables des adversaires d'après leurs actions |
//! | [`pushfold`] | équilibres push/fold (jeu fictif sur simulations) |
//! | [`reference`] | décision de référence d'un spot, indépendante du résultat |
//! | [`model`] | modèles appris (softmax, perceptron multicouche), en Rust pur |
//! | [`bundle`] | les modèles d'une IA, réunis en un fichier JSON |

pub mod bundle;
pub mod mc;
pub mod model;
pub mod preflop;
pub mod pushfold;
pub mod reference;
pub mod spot;
pub mod tables;
pub mod villain;

pub use bundle::ModelBundle;
pub use reference::{Basis, Inputs, Reference, classify, reference};
pub use spot::{Decision, HandFeatures, Spot, decision_features};
