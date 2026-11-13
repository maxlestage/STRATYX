//! Moteur de calcul poker commun aux deux IA de Stratyx (Expresso 3-Max et Tournois 6-Max).
//!
//! | Module      | Rôle                                                                   |
//! |-------------|------------------------------------------------------------------------|
//! | [`card`], [`cardset`] | cartes et ensembles de cartes                                |
//! | [`eval`]    | force des mains (5 à 7 cartes)                                          |
//! | [`range`]   | mains privées, 169 classes, ranges pondérées                           |
//! | [`equity`]  | équité exacte ou Monte-Carlo, parallèle et reproductible               |
//! | [`format`](mod@format) | formats : cash game, tournoi, sit & go, Expresso ; 2 à 10 joueurs     |
//! | [`game`]    | règles du Texas Hold'em : no-limit, pot-limit, limite fixe            |
//! | [`pot`]     | pots principal/secondaires, mises non suivies, répartition             |
//! | [`history`] | historique de main avec provenance et confiance, validation par rejeu  |
//! | [`icm`]     | équité ICM (Malmuth-Harville)                                           |
//! | [`import`]  | import des historiques texte des salles (PokerStars, Winamax…)          |
//! | [`odds`]    | cotes du pot, fold equity, EV, MDF                                      |
//! | [`position`]| positions du tête-à-tête à la table pleine (2 à 10 joueurs)           |
//!
//! Variante prise en charge : **Texas Hold'em** sous toutes ses formes — no-limit, pot-limit
//! et limite fixe, cash game et tournois, du tête-à-tête à la table pleine (décision D8).

pub mod card;
pub mod cardset;
pub mod equity;
pub mod eval;
pub mod format;
pub mod game;
pub mod history;
pub mod icm;
pub mod import;
pub mod odds;
pub mod position;
pub mod pot;
pub mod range;

pub use card::{Card, Rank, Suit};
pub use cardset::CardSet;
pub use eval::{HandCategory, HandRank, evaluate};
pub use format::{GameFormat, GameType, Specialist};
pub use game::{Action, BettingStructure, Blinds, HandSetup, HandState, Street};
pub use range::{HandClass, HoleCards, Range};
