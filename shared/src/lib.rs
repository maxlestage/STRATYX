//! Types transverses partagés par tous les modules Stratyx.
//!
//! - [`Chips`] : montants exprimés dans la plus petite unité (jeton de tournoi ou centime).
//! - [`Confidence`], [`Source`], [`Observed`] : toute donnée issue de la vision artificielle
//!   porte sa provenance et son niveau de confiance, afin de signaler les détections
//!   incertaines et de ne jamais présenter une valeur inférée comme observée.

mod chips;
mod observed;

pub use chips::Chips;
pub use observed::{Confidence, ConfidenceError, Observed, Source};
