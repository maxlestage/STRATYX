//! Formats texte des historiques de mains des salles.
//!
//! # PokerStars
//!
//! [`export_pokerstars`] écrit un [`HandRecord`](stratyx_poker::HandRecord) au format texte
//! de PokerStars et [`import_pokerstars`] le relit (un fichier peut contenir plusieurs mains).
//! Sont pris en charge : cash game (montants en centimes de la devise de l'en-tête, ou jetons fictifs) et
//! tournoi ; no-limit, pot-limit et limite fixe ; tables de 2 à 10 joueurs ; antes classiques
//! et antes de grosse blinde ; tapis et pots secondaires ; mises non suivies ; mains montrées
//! ou jetées à l'abattage ; héros (« Dealt to »).
//!
//! Chaque main importée est **validée par rejeu** dans le moteur de règles, puis confrontée
//! au texte : montants des blindes, des antes, des suivis, drapeaux de tapis et mise non
//! suivie doivent concorder, sinon la main est refusée avec le numéro de ligne fautif. Les
//! gains annoncés (« collected ») sont comparés au partage calculé par le moteur ; un écart
//! (râteau, jackpot…) produit un avertissement. Toutes les observations importées portent la
//! provenance [`Source::Import`](stratyx_shared::Source::Import) et une confiance certaine.
//!
//! **Aucune carte inconnue n'est inventée** : à l'export, seules les cartes connues
//! (cartes du héros, mains montrées) sont écrites ; une main non résolue n'a pas de ligne de
//! gains.
//!
//! Conventions et limites :
//! - les dates sont à la seconde ; l'export écrit l'heure UTC suivie de l'heure de l'Est
//!   entre crochets, comme PokerStars ;
//! - PokerStars ne distingue pas tournoi, sit & go et Spin & Go : le type est fourni par
//!   [`ImportOptions::tournament_type`] ;
//! - la limite fixe importée suit l'usage de la salle (plafond levé en tête-à-tête en cash
//!   game, maintenu en tournoi) ; l'en-tête d'une limite fixe donne la petite et la grosse
//!   mise, la petite blinde étant lue sur sa ligne ;
//! - le râteau est lu ([`ImportedHand::rake`]) mais n'entre pas dans le `HandRecord` ;
//! - non pris en charge (refusés avec un message explicite) : straddles, blindes mortes ou
//!   postées à l'arrivée, bouton mort, « run it twice », cash-out ;
//! - une main dont le héros n'est pas renseigné mais dont des cartes sont connues est relue
//!   avec, pour héros, le premier joueur « Dealt to ».
//!
//! # Autres salles
//!
//! Les formats des autres salles (Winamax notamment, salle visée en priorité) ne sont **pas
//! encore** pris en charge : leur écriture exacte (en-têtes, devises, tournois Expresso,
//! libellés des actions) doit être établie à partir de **vrais historiques**, qu'il faut
//! collecter avant d'écrire les parseurs correspondants.

mod amount;
mod datetime;
mod describe;
mod pokerstars_export;
mod pokerstars_import;

use std::fmt;

use serde::{Deserialize, Serialize};
use stratyx_poker::GameType;
use stratyx_poker::history::RecordError;
use stratyx_shared::Chips;
use thiserror::Error;

pub use pokerstars_export::{export_pokerstars, export_pokerstars_many};
pub use pokerstars_import::{import_pokerstars, import_pokerstars_hand};

/// Identifiant de salle reporté dans `meta.site` des mains importées.
pub const POKERSTARS_SITE: &str = "pokerstars";

/// Droit d'entrée d'un tournoi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyIn {
    /// Montant total payé, frais compris, en centimes (0 : freeroll).
    pub total: u64,
    /// Part des frais, en centimes.
    pub fee: u64,
    /// Code ISO 4217 de la devise, si indiqué.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

/// Informations d'en-tête d'un tournoi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentInfo {
    /// Numéro du tournoi chez la salle.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buy_in: Option<BuyIn>,
    /// Niveau de blindes tel qu'affiché (« V », « XII »).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

/// Options d'export.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Devise d'un cash game (code ISO 4217) : les montants du `HandRecord` sont alors des
    /// centimes. Absente : partie en jetons fictifs.
    pub currency: Option<String>,
    /// En-tête d'un tournoi (numéro, droit d'entrée, niveau). Absent : tournoi n° 0.
    pub tournament: Option<TournamentInfo>,
}

/// Options d'import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOptions {
    /// Type attribué aux mains de tournoi (PokerStars ne distingue pas tournoi, sit & go et
    /// Spin & Go). [`GameType::CashGame`] est traité comme [`GameType::Tournament`].
    pub tournament_type: GameType,
    /// Session à laquelle rattacher les mains (`meta.session_id`).
    pub session_id: Option<String>,
    /// Provenance reportée dans `meta.provenance`.
    pub provenance: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            tournament_type: GameType::Tournament,
            session_id: None,
            provenance: None,
        }
    }
}

/// Main importée.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedHand {
    pub record: stratyx_poker::HandRecord,
    /// Devise de l'en-tête (code ISO 4217) ; les montants sont alors en centimes.
    pub currency: Option<String>,
    pub tournament: Option<TournamentInfo>,
    /// Râteau prélevé par la salle, s'il est indiqué.
    pub rake: Option<Chips>,
    /// Écarts non bloquants (gains annoncés différents du partage calculé…).
    pub warnings: Vec<String>,
}

/// Nature d'une erreur d'import.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImportErrorKind {
    #[error("aucune main PokerStars trouvée")]
    NoHand,
    #[error("{0} mains trouvées, une seule attendue")]
    SeveralHands(usize),
    #[error("en-tête invalide : {0}")]
    Header(String),
    #[error("jeu non pris en charge : « {0} » (Texas Hold'em uniquement)")]
    UnsupportedGame(String),
    #[error("{0} : non pris en charge par le moteur de règles")]
    Unsupported(String),
    #[error("ligne illisible : « {0} »")]
    Syntax(String),
    #[error("montant invalide : « {0} »")]
    InvalidAmount(String),
    #[error("cartes invalides : « {0} »")]
    InvalidCards(String),
    #[error("joueur inconnu : « {0} »")]
    UnknownPlayer(String),
    #[error("{0} absente")]
    Missing(&'static str),
    #[error("{what} : {found} dans l'historique, {expected} selon le moteur de règles")]
    AmountMismatch {
        what: String,
        expected: Chips,
        found: Chips,
    },
    #[error("incohérence avec le moteur de règles : {0}")]
    Inconsistent(String),
    #[error("rejeu refusé : {0}")]
    Replay(RecordError),
}

/// Erreur d'import, localisée dans le texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    /// Identifiant de la main, si l'en-tête a pu être lu.
    pub hand_id: Option<String>,
    /// Numéro de ligne dans le texte importé (à partir de 1 ; 0 si sans objet).
    pub line: usize,
    pub kind: ImportErrorKind,
}

impl ImportError {
    /// Identifiant de la main en cause, si l'en-tête a pu être lu.
    #[must_use]
    pub fn hand_id(&self) -> Option<&str> {
        self.hand_id.as_deref()
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = &self.hand_id {
            write!(f, "main {id}, ")?;
        }
        if self.line > 0 {
            write!(f, "ligne {} : ", self.line)?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ImportErrorKind::Replay(e) => Some(e),
            _ => None,
        }
    }
}

/// Erreur d'export.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportError {
    #[error("main non exportable, le rejeu échoue : {0}")]
    Replay(RecordError),
    #[error("identifiant de main non exportable : « {0} » (ni espace, ni deux-points)")]
    HandId(String),
    #[error("pseudonyme non exportable : « {0} »")]
    PlayerName(String),
    #[error("nom de table non exportable : « {0} »")]
    TableName(String),
}
