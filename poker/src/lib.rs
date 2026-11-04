//! Moteur de calcul poker commun aux deux IA de Stratyx (Expresso 3-Max et Tournois 6-Max).
//!
//! | Module      | Rôle                                                                   |
//! |-------------|------------------------------------------------------------------------|
//! | [`card`], [`cardset`] | cartes et ensembles de cartes                                |
//! | [`eval`]    | force des mains (5 à 7 cartes)                                          |
//! | [`range`]   | mains privées, 169 classes, ranges pondérées                           |
//! | [`equity`]  | équité exacte ou Monte-Carlo, parallèle et reproductible               |
//! | [`game`]    | règles du No-Limit Hold'em : ordre de parole, relances, tapis          |
//! | [`pot`]     | pots principal/secondaires, mises non suivies, répartition             |
//! | [`history`] | historique de main avec provenance et confiance, validation par rejeu  |
//! | [`icm`]     | équité ICM (Malmuth-Harville)                                           |
//! | [`odds`]    | cotes du pot, fold equity, EV, MDF                                      |
//! | [`position`]| positions de 2 à 6 joueurs                                              |
//!
//! Variante prise en charge : Texas Hold'em No-Limit (décision D4).

pub mod card;
pub mod cardset;
pub mod equity;
pub mod eval;
pub mod game;
pub mod history;
pub mod icm;
pub mod odds;
pub mod position;
pub mod pot;
pub mod range;

pub use card::{Card, Rank, Suit};
pub use cardset::CardSet;
pub use eval::{HandCategory, HandRank, evaluate};
pub use game::{Action, Blinds, HandSetup, HandState, Street};
pub use range::{HandClass, HoleCards, Range};
