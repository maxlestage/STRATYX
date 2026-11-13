//! Règles du Texas Hold'em : déroulement d'une main et validation de chaque action, en
//! no-limit, pot-limit et limite fixe, de 2 à 10 joueurs.
//!
//! [`HandState`] est la référence unique des règles pour toute la plateforme : la
//! retranscription vidéo y rejoue les actions détectées pour les valider, les IA s'en servent
//! pour générer les coups légaux, et le coaching pour ses simulations.
//!
//! Règles appliquées (usages des salles en ligne, règles TDA et Robert's Rules of Poker) :
//! - **no-limit** : relance minimale égale à la dernière relance complète (au moins une grosse
//!   blinde) ; un tapis inférieur à une relance complète ne rouvre pas les enchères pour les
//!   joueurs ayant déjà agi, sauf si plusieurs tapis incomplets cumulés l'atteignent ;
//! - **pot-limit** : mêmes minimums ; relance maximale égale au pot après le call ;
//! - **limite fixe** : mises d'une petite mise (= grosse blinde) au préflop et au flop, d'une
//!   grosse mise (= deux grosses blindes) au turn et à la river ; une mise et trois relances au
//!   plus par tour (plafond levé en tête-à-tête, voir [`BettingStructure`]) ; un tapis d'au
//!   moins une demi-mise compte comme une mise complète, en dessous il ne rouvre pas les
//!   enchères et les joueurs suivants peuvent le compléter ;
//! - en tête-à-tête, le bouton est petite blinde, parle en premier préflop et en dernier ensuite ;
//! - le jeton indivisible va au premier gagnant à gauche du bouton.

use std::fmt;

use serde::{Deserialize, Serialize};
use stratyx_shared::Chips;
use thiserror::Error;

use crate::card::Card;
use crate::cardset::CardSet;
use crate::eval::{HandRank, evaluate};
use crate::position::Position;
use crate::pot::{AwardError, Claim, Contribution, PotBreakdown, award_pots, build_pots};
use crate::range::HoleCards;

/// Nombre maximal de joueurs gérés par le moteur de règles.
pub const MAX_PLAYERS: usize = 10;

/// Plafond des jetons d'une main (somme des tapis, blindes, ante) : 2⁶², bien au-delà de
/// toute partie réelle. Les montants en jeu étant bornés par la somme des tapis, les calculs
/// de pots et de relances ne peuvent pas déborder, et les résultats nets tiennent dans un
/// `i64`. Un historique corrompu peut annoncer des tapis démesurés : la main est alors
/// refusée au lieu de déborder pendant la répartition des pots.
pub const MAX_TOTAL_CHIPS: u64 = 1 << 62;

/// Tour d'enchères.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    #[must_use]
    pub const fn next(self) -> Option<Street> {
        match self {
            Street::Preflop => Some(Street::Flop),
            Street::Flop => Some(Street::Turn),
            Street::Turn => Some(Street::River),
            Street::River => None,
        }
    }

    /// Nombre de cartes distribuées au début de ce tour.
    #[must_use]
    pub const fn cards_dealt(self) -> usize {
        match self {
            Street::Preflop => 0,
            Street::Flop => 3,
            Street::Turn | Street::River => 1,
        }
    }

    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            Street::Preflop => "Préflop",
            Street::Flop => "Flop",
            Street::Turn => "Turn",
            Street::River => "River",
        }
    }
}

impl fmt::Display for Street {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name_fr())
    }
}

/// Antes de la structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "amount")]
pub enum Ante {
    #[default]
    None,
    /// Chaque joueur paie l'ante.
    Classic(Chips),
    /// La grosse blinde paie l'ante pour toute la table (argent mort).
    BigBlind(Chips),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blinds {
    pub small_blind: Chips,
    pub big_blind: Chips,
    #[serde(default)]
    pub ante: Ante,
}

impl Blinds {
    #[must_use]
    pub fn new(small_blind: u64, big_blind: u64) -> Blinds {
        Blinds {
            small_blind: Chips(small_blind),
            big_blind: Chips(big_blind),
            ante: Ante::None,
        }
    }

    #[must_use]
    pub fn with_ante(mut self, ante: Ante) -> Blinds {
        self.ante = ante;
        self
    }
}

/// Structure de mise du Texas Hold'em.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BettingStructure {
    /// No-limit : toute mise jusqu'au tapis.
    #[default]
    NoLimit,
    /// Pot-limit : mise ou relance plafonnée à la taille du pot (après le call).
    PotLimit,
    /// Limite fixe : montants imposés, nombre de mises plafonné.
    FixedLimit {
        /// Nombre maximal de mises complètes par tour (mise + relances). `None` : illimité.
        max_bets: Option<u8>,
        /// Plafond levé dès qu'il ne reste que deux joueurs dans le coup (usage des cash
        /// games). Dans tous les cas, il est levé quand seuls deux joueurs sont servis.
        uncapped_heads_up: bool,
    },
}

impl BettingStructure {
    /// Limite fixe des cash games : une mise et trois relances, plafond levé en tête-à-tête.
    pub const FIXED_LIMIT_CASH: BettingStructure = BettingStructure::FixedLimit {
        max_bets: Some(4),
        uncapped_heads_up: true,
    };

    /// Limite fixe des tournois : plafond maintenu tant que plus de deux joueurs sont servis.
    pub const FIXED_LIMIT_TOURNAMENT: BettingStructure = BettingStructure::FixedLimit {
        max_bets: Some(4),
        uncapped_heads_up: false,
    };

    /// Abréviation usuelle : `NL`, `PL`, `FL`.
    #[must_use]
    pub const fn abbreviation(self) -> &'static str {
        match self {
            BettingStructure::NoLimit => "NL",
            BettingStructure::PotLimit => "PL",
            BettingStructure::FixedLimit { .. } => "FL",
        }
    }
}

/// Situation de départ d'une main. Les joueurs sont indexés de `0` à `n - 1` dans le sens
/// des aiguilles d'une montre ; seuls les joueurs servis y figurent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandSetup {
    pub stacks: Vec<Chips>,
    pub button: usize,
    pub blinds: Blinds,
    #[serde(default)]
    pub structure: BettingStructure,
}

impl HandSetup {
    /// Main de no-limit ; voir [`HandSetup::with_structure`] pour les autres structures.
    #[must_use]
    pub fn new(stacks: Vec<Chips>, button: usize, blinds: Blinds) -> HandSetup {
        HandSetup {
            stacks,
            button,
            blinds,
            structure: BettingStructure::NoLimit,
        }
    }

    #[must_use]
    pub fn with_structure(mut self, structure: BettingStructure) -> HandSetup {
        self.structure = structure;
        self
    }
}

/// Action demandée par un joueur. Les montants de `Bet` et `Raise` sont des totaux
/// **« à »** sur le tour en cours, comme dans les historiques des salles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "to")]
pub enum Action {
    Fold,
    Check,
    Call,
    Bet(Chips),
    Raise(Chips),
    /// Tapis : converti en call, bet ou raise selon la situation.
    AllIn,
}

/// Nature d'une action une fois appliquée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    PostAnte,
    PostSmallBlind,
    PostBigBlind,
    Fold,
    Check,
    Call,
    Bet,
    Raise,
}

impl ActionKind {
    #[must_use]
    pub const fn is_voluntary(self) -> bool {
        !matches!(
            self,
            ActionKind::PostAnte | ActionKind::PostSmallBlind | ActionKind::PostBigBlind
        )
    }

    #[must_use]
    pub const fn is_aggressive(self) -> bool {
        matches!(self, ActionKind::Bet | ActionKind::Raise)
    }
}

/// Action telle qu'appliquée par le moteur, avec ses montants explicites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedAction {
    pub player: usize,
    pub street: Street,
    pub kind: ActionKind,
    /// Jetons ajoutés au pot par cette action.
    pub amount: Chips,
    /// Total engagé par le joueur sur ce tour après l'action.
    pub to: Chips,
    pub all_in: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStatus {
    Active,
    Folded,
    AllIn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerState {
    /// Tapis restant.
    pub stack: Chips,
    /// Engagé sur le tour en cours.
    pub street_bet: Chips,
    /// Engagé sur toute la main et soumis à l'égalisation.
    pub live_total: Chips,
    /// Argent mort versé (ante de grosse blinde).
    pub dead_total: Chips,
    pub status: PlayerStatus,
    needs_action: bool,
    /// Mise à suivre au moment de la dernière action volontaire sur ce tour.
    last_faced: Option<Chips>,
    /// Nombre de mises complètes du tour au moment de cette action (limite fixe).
    bets_seen: u8,
}

impl PlayerState {
    #[must_use]
    pub fn invested(&self) -> Chips {
        self.live_total + self.dead_total
    }
}

/// Étape de la main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "phase", content = "street")]
pub enum Phase {
    /// Un joueur doit agir.
    Betting,
    /// Les enchères du tour sont closes ; on attend les cartes du tour indiqué.
    AwaitingBoard(Street),
    /// River terminée (ou tapis) : abattage.
    Showdown,
    /// Tous les adversaires se sont couchés.
    Finished,
}

/// Actions légales pour le joueur qui doit parler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalActions {
    pub player: usize,
    pub fold: bool,
    pub check: bool,
    /// Montant à ajouter pour suivre.
    pub call: Option<Chips>,
    /// Bornes `(min, max)` d'une mise, en total « à ».
    pub bet: Option<(Chips, Chips)>,
    /// Bornes `(min, max)` d'une relance, en total « à ».
    pub raise: Option<(Chips, Chips)>,
}

/// Ce que l'on sait des cartes d'un joueur au moment de l'abattage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "cards")]
pub enum Reveal {
    Shown(HoleCards),
    Mucked,
    Unknown,
}

/// Résultat financier d'une main terminée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    pub pots: PotBreakdown,
    /// Total récupéré par chaque joueur (gains + mise non suivie rendue).
    pub collected: Vec<Chips>,
    /// Résultat net de chaque joueur (récupéré − investi).
    pub net: Vec<i64>,
    /// Force des mains montrées.
    pub hand_ranks: Vec<Option<HandRank>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GameError {
    #[error("situation de départ invalide : {0}")]
    InvalidSetup(String),
    #[error("aucune action attendue : la main est à l'étape {0:?}")]
    NotBetting(Phase),
    #[error("ce n'est pas au joueur {got} de parler (attendu : {expected:?})")]
    NotYourTurn { expected: Option<usize>, got: usize },
    #[error("action {action:?} illégale pour le joueur {player} : {reason}")]
    IllegalAction {
        player: usize,
        action: Action,
        reason: String,
    },
    #[error("aucune carte attendue : la main est à l'étape {0:?}")]
    NotAwaitingBoard(Phase),
    #[error("{street} : {expected} carte(s) attendue(s), {got} reçue(s)")]
    BoardSize {
        street: Street,
        expected: usize,
        got: usize,
    },
    #[error("carte déjà utilisée : {0}")]
    DuplicateCard(Card),
    #[error("la main n'est pas terminée")]
    HandNotOver,
    #[error("{0}")]
    Award(#[from] AwardError),
    #[error("une information par joueur est attendue ({expected}), {got} reçue(s)")]
    RevealCount { expected: usize, got: usize },
}

/// État complet d'une main en cours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandState {
    blinds: Blinds,
    structure: BettingStructure,
    button: usize,
    players: Vec<PlayerState>,
    street: Street,
    phase: Phase,
    board: Vec<Card>,
    current_bet: Chips,
    /// Dernière relance complète (no-limit et pot-limit).
    min_raise: Chips,
    /// Niveau sur lequel se calcule la prochaine relance (limite fixe).
    raise_base: Chips,
    /// Mises complètes sur le tour en cours (la grosse blinde compte pour une au préflop).
    bets: u8,
    to_act: Option<usize>,
    actions: Vec<AppliedAction>,
}

impl HandState {
    /// Démarre une main : antes et blindes sont postées automatiquement.
    pub fn new(setup: &HandSetup) -> Result<HandState, GameError> {
        let n = setup.stacks.len();
        let invalid = |m: &str| Err(GameError::InvalidSetup(m.to_string()));
        if !(2..=MAX_PLAYERS).contains(&n) {
            return invalid("il faut entre 2 et 10 joueurs");
        }
        if setup.button >= n {
            return invalid("bouton hors de la table");
        }
        if setup.blinds.big_blind.is_zero() {
            return invalid("la grosse blinde doit être positive");
        }
        if setup.blinds.small_blind > setup.blinds.big_blind {
            return invalid("la petite blinde dépasse la grosse blinde");
        }
        if setup.stacks.iter().any(|s| s.is_zero()) {
            return invalid("un joueur servi a un tapis nul");
        }
        let total = setup
            .stacks
            .iter()
            .try_fold(0u64, |acc, s| acc.checked_add(s.get()));
        let ante = match setup.blinds.ante {
            Ante::None => Chips::ZERO,
            Ante::Classic(a) | Ante::BigBlind(a) => a,
        };
        if total.is_none_or(|t| t > MAX_TOTAL_CHIPS)
            || setup.blinds.big_blind.get() > MAX_TOTAL_CHIPS
            || ante.get() > MAX_TOTAL_CHIPS
        {
            return invalid("montants démesurés (au-delà de 2^62 jetons)");
        }

        let players = setup
            .stacks
            .iter()
            .map(|s| PlayerState {
                stack: *s,
                street_bet: Chips::ZERO,
                live_total: Chips::ZERO,
                dead_total: Chips::ZERO,
                status: PlayerStatus::Active,
                needs_action: true,
                last_faced: None,
                bets_seen: 0,
            })
            .collect();
        if let BettingStructure::FixedLimit {
            max_bets: Some(0), ..
        } = setup.structure
        {
            return invalid("le plafond de mises doit être d'au moins une mise");
        }
        let mut state = HandState {
            blinds: setup.blinds,
            structure: setup.structure,
            button: setup.button,
            players,
            street: Street::Preflop,
            phase: Phase::Betting,
            board: Vec::new(),
            current_bet: setup.blinds.big_blind,
            min_raise: setup.blinds.big_blind,
            raise_base: setup.blinds.big_blind,
            bets: 1,
            to_act: None,
            actions: Vec::new(),
        };

        let (sb, bb) = state.blind_seats();
        match setup.blinds.ante {
            Ante::None => {}
            Ante::Classic(a) => {
                for p in 0..n {
                    state.post(p, a, ActionKind::PostAnte, false);
                }
            }
            Ante::BigBlind(a) => state.post(bb, a, ActionKind::PostAnte, true),
        }
        state.post(
            sb,
            setup.blinds.small_blind,
            ActionKind::PostSmallBlind,
            false,
        );
        state.post(bb, setup.blinds.big_blind, ActionKind::PostBigBlind, false);

        let first = if n == 2 { setup.button } else { (bb + 1) % n };
        state.advance(first);
        Ok(state)
    }

    /// Sièges (index) de la petite et de la grosse blinde.
    #[must_use]
    pub fn blind_seats(&self) -> (usize, usize) {
        let n = self.players.len();
        if n == 2 {
            (self.button, (self.button + 1) % n)
        } else {
            ((self.button + 1) % n, (self.button + 2) % n)
        }
    }

    fn post(&mut self, p: usize, amount: Chips, kind: ActionKind, dead: bool) {
        let pl = &mut self.players[p];
        let paid = amount.min(pl.stack);
        if paid.is_zero() {
            return;
        }
        pl.stack -= paid;
        if dead {
            pl.dead_total += paid;
        } else {
            pl.live_total += paid;
            if kind != ActionKind::PostAnte {
                pl.street_bet += paid;
            }
        }
        let all_in = pl.stack.is_zero();
        if all_in {
            pl.status = PlayerStatus::AllIn;
        }
        let to = pl.street_bet;
        self.actions.push(AppliedAction {
            player: p,
            street: self.street,
            kind,
            amount: paid,
            to,
            all_in,
        });
    }

    // --- Accesseurs -------------------------------------------------------------------

    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[must_use]
    pub fn street(&self) -> Street {
        self.street
    }

    #[must_use]
    pub fn board(&self) -> &[Card] {
        &self.board
    }

    #[must_use]
    pub fn to_act(&self) -> Option<usize> {
        self.to_act
    }

    #[must_use]
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    #[must_use]
    pub fn button(&self) -> usize {
        self.button
    }

    #[must_use]
    pub fn blinds(&self) -> Blinds {
        self.blinds
    }

    #[must_use]
    pub fn structure(&self) -> BettingStructure {
        self.structure
    }

    /// Mises complètes sur le tour en cours (2 = relance, 3 = 3-bet…).
    #[must_use]
    pub fn bets_this_street(&self) -> u8 {
        self.bets
    }

    /// Montant d'une mise en limite fixe sur le tour en cours : petite mise (grosse blinde)
    /// au préflop et au flop, grosse mise (deux grosses blindes) au turn et à la river.
    #[must_use]
    pub fn limit_bet_size(&self) -> Chips {
        match self.street {
            Street::Preflop | Street::Flop => self.blinds.big_blind,
            Street::Turn | Street::River => self.blinds.big_blind + self.blinds.big_blind,
        }
    }

    /// Mise à suivre sur le tour en cours.
    #[must_use]
    pub fn current_bet(&self) -> Chips {
        self.current_bet
    }

    /// Historique des actions appliquées (y compris antes et blindes).
    #[must_use]
    pub fn actions(&self) -> &[AppliedAction] {
        &self.actions
    }

    /// Total des jetons engagés au milieu de la table.
    #[must_use]
    pub fn pot_total(&self) -> Chips {
        self.players.iter().map(PlayerState::invested).sum()
    }

    /// Position du joueur (tables de 2 à 10 joueurs).
    #[must_use]
    pub fn position(&self, player: usize) -> Option<Position> {
        let n = self.players.len();
        Position::from_button_offset((player + n - self.button) % n, n)
    }

    /// Joueurs dans l'ordre de distribution des jetons indivisibles (gauche du bouton).
    #[must_use]
    pub fn odd_chip_order(&self) -> Vec<usize> {
        let n = self.players.len();
        (1..=n).map(|i| (self.button + i) % n).collect()
    }

    fn count(&self, status: PlayerStatus) -> usize {
        self.players.iter().filter(|p| p.status == status).count()
    }

    fn in_hand(&self) -> usize {
        self.players.len() - self.count(PlayerStatus::Folded)
    }

    fn needs_to_act(&self, p: usize) -> bool {
        let pl = &self.players[p];
        if pl.status != PlayerStatus::Active {
            return false;
        }
        let behind = pl.street_bet < self.current_bet;
        if self.count(PlayerStatus::Active) == 1 && !behind {
            // Plus personne ne peut miser contre lui.
            return false;
        }
        pl.needs_action || behind
    }

    /// Les enchères sont-elles ouvertes pour ce joueur (règle des tapis incomplets) ?
    fn can_raise(&self, p: usize) -> bool {
        let pl = &self.players[p];
        match (self.structure, pl.last_faced) {
            (_, None) => true,
            (BettingStructure::FixedLimit { .. }, Some(_)) => self.bets > pl.bets_seen,
            (_, Some(faced)) => self.current_bet.saturating_sub(faced) >= self.min_raise,
        }
    }

    /// Bornes `(min, max)` d'une mise ou d'une relance « à » pour le joueur, ou la raison
    /// pour laquelle il ne peut que suivre ou se coucher.
    fn aggression_bounds(&self, p: usize) -> Result<(Chips, Chips), String> {
        let pl = &self.players[p];
        let all_in_to = pl.street_bet + pl.stack;
        if all_in_to <= self.current_bet {
            return Err("tapis insuffisant pour relancer : il ne peut que suivre".into());
        }
        if !self.current_bet.is_zero() && !self.can_raise(p) {
            return Err(
                "les enchères ne sont pas rouvertes (tapis inférieur à une relance complète)"
                    .into(),
            );
        }
        let nominal_min = if self.current_bet.is_zero() {
            self.blinds.big_blind
        } else {
            self.current_bet + self.min_raise
        };
        match self.structure {
            BettingStructure::NoLimit => Ok((nominal_min.min(all_in_to), all_in_to)),
            BettingStructure::PotLimit => {
                let call = self.current_bet - pl.street_bet;
                let pot_max = self.current_bet + self.pot_total() + call;
                let max = pot_max.min(all_in_to);
                Ok((nominal_min.min(max), max))
            }
            BettingStructure::FixedLimit {
                max_bets,
                uncapped_heads_up,
            } => {
                let heads_up =
                    self.players.len() == 2 || (uncapped_heads_up && self.in_hand() == 2);
                if !heads_up && max_bets.is_some_and(|m| self.bets >= m) {
                    return Err(format!("plafond atteint : {} mises sur ce tour", self.bets));
                }
                let base = if self.current_bet.is_zero() {
                    Chips::ZERO
                } else {
                    self.raise_base
                };
                let to = (base + self.limit_bet_size()).min(all_in_to);
                Ok((to, to))
            }
        }
    }

    fn advance(&mut self, from: usize) {
        if self.in_hand() <= 1 {
            self.to_act = None;
            self.phase = Phase::Finished;
            return;
        }
        let n = self.players.len();
        if let Some(p) = (0..n)
            .map(|i| (from + i) % n)
            .find(|p| self.needs_to_act(*p))
        {
            self.to_act = Some(p);
            self.phase = Phase::Betting;
            return;
        }
        // Tour d'enchères terminé.
        self.to_act = None;
        for pl in &mut self.players {
            pl.street_bet = Chips::ZERO;
            pl.needs_action = pl.status == PlayerStatus::Active;
            pl.last_faced = None;
        }
        self.current_bet = Chips::ZERO;
        self.min_raise = self.blinds.big_blind;
        self.raise_base = Chips::ZERO;
        self.bets = 0;
        self.phase = match self.street.next() {
            Some(next) => Phase::AwaitingBoard(next),
            None => Phase::Showdown,
        };
    }

    // --- Actions ----------------------------------------------------------------------

    /// Actions légales du joueur qui doit parler.
    #[must_use]
    pub fn legal_actions(&self) -> Option<LegalActions> {
        let p = self.to_act?;
        let pl = &self.players[p];
        let behind = self.current_bet.saturating_sub(pl.street_bet);
        let call = (!behind.is_zero()).then(|| behind.min(pl.stack));
        let bounds = self.aggression_bounds(p).ok();
        let (bet, raise) = if self.current_bet.is_zero() {
            (bounds, None)
        } else {
            (None, bounds)
        };
        Some(LegalActions {
            player: p,
            fold: true,
            check: behind.is_zero(),
            call,
            bet,
            raise,
        })
    }

    /// Applique l'action d'un joueur après en avoir vérifié la légalité.
    pub fn apply(&mut self, player: usize, action: Action) -> Result<AppliedAction, GameError> {
        if self.phase != Phase::Betting {
            return Err(GameError::NotBetting(self.phase));
        }
        if self.to_act != Some(player) {
            return Err(GameError::NotYourTurn {
                expected: self.to_act,
                got: player,
            });
        }
        let illegal = |reason: String| GameError::IllegalAction {
            player,
            action,
            reason,
        };
        let pl = &self.players[player];
        let max_to = pl.street_bet + pl.stack;

        let action = match action {
            Action::AllIn if max_to <= self.current_bet => Action::Call,
            Action::AllIn if self.current_bet.is_zero() => Action::Bet(max_to),
            Action::AllIn => Action::Raise(max_to),
            other => other,
        };

        let applied = match action {
            Action::Fold => {
                self.players[player].status = PlayerStatus::Folded;
                self.record(player, ActionKind::Fold, Chips::ZERO)
            }
            Action::Check => {
                if pl.street_bet < self.current_bet {
                    return Err(illegal(format!(
                        "il reste {} à suivre",
                        self.current_bet - pl.street_bet
                    )));
                }
                self.record(player, ActionKind::Check, Chips::ZERO)
            }
            Action::Call => {
                if pl.street_bet >= self.current_bet {
                    return Err(illegal("aucune mise à suivre : utiliser check".into()));
                }
                let add = (self.current_bet - pl.street_bet).min(pl.stack);
                self.record(player, ActionKind::Call, add)
            }
            Action::Bet(to) | Action::Raise(to) => {
                let is_bet = matches!(action, Action::Bet(_));
                if is_bet && !self.current_bet.is_zero() {
                    return Err(illegal("une mise est déjà engagée : utiliser raise".into()));
                }
                if !is_bet && self.current_bet.is_zero() {
                    return Err(illegal("aucune mise à relancer : utiliser bet".into()));
                }
                let (min, max) = self.aggression_bounds(player).map_err(illegal)?;
                if to < min || to > max {
                    let reason = if min == max {
                        format!("montant imposé : {min}")
                    } else {
                        format!("montant autorisé : de {min} à {max}")
                    };
                    return Err(illegal(reason));
                }
                let kind = if is_bet {
                    ActionKind::Bet
                } else {
                    ActionKind::Raise
                };
                self.aggress(player, to, kind)
            }
            Action::AllIn => unreachable!("converti plus haut"),
        };
        self.advance((player + 1) % self.players.len());
        Ok(applied)
    }

    fn aggress(&mut self, player: usize, to: Chips, kind: ActionKind) -> AppliedAction {
        let increment = to - self.current_bet;
        let full = match self.structure {
            BettingStructure::FixedLimit { .. } => {
                let base = if self.current_bet.is_zero() {
                    Chips::ZERO
                } else {
                    self.raise_base
                };
                let unit = self.limit_bet_size();
                to >= base + unit || increment.get() * 2 >= unit.get()
            }
            _ => increment >= self.min_raise,
        };
        if full {
            if !matches!(self.structure, BettingStructure::FixedLimit { .. }) {
                self.min_raise = increment;
            }
            self.raise_base = to;
            self.bets = self.bets.saturating_add(1);
        }
        self.current_bet = to;
        for (i, pl) in self.players.iter_mut().enumerate() {
            if i != player && pl.status == PlayerStatus::Active {
                pl.needs_action = true;
            }
        }
        let add = to - self.players[player].street_bet;
        self.record(player, kind, add)
    }

    fn record(&mut self, player: usize, kind: ActionKind, add: Chips) -> AppliedAction {
        let current_bet = self.current_bet;
        let bets = self.bets;
        let pl = &mut self.players[player];
        pl.stack -= add;
        pl.street_bet += add;
        pl.live_total += add;
        pl.needs_action = false;
        pl.last_faced = Some(current_bet);
        pl.bets_seen = bets;
        let all_in = !add.is_zero() && pl.stack.is_zero();
        if all_in {
            pl.status = PlayerStatus::AllIn;
        }
        let applied = AppliedAction {
            player,
            street: self.street,
            kind,
            amount: add,
            to: pl.street_bet,
            all_in,
        };
        self.actions.push(applied);
        applied
    }

    /// Ajoute les cartes du tour attendu (3 au flop, 1 au turn et à la river).
    pub fn deal(&mut self, cards: &[Card]) -> Result<(), GameError> {
        let Phase::AwaitingBoard(street) = self.phase else {
            return Err(GameError::NotAwaitingBoard(self.phase));
        };
        if cards.len() != street.cards_dealt() {
            return Err(GameError::BoardSize {
                street,
                expected: street.cards_dealt(),
                got: cards.len(),
            });
        }
        let mut seen: CardSet = self.board.iter().collect();
        for c in cards {
            if !seen.insert(*c) {
                return Err(GameError::DuplicateCard(*c));
            }
        }
        self.board.extend_from_slice(cards);
        self.street = street;
        self.phase = Phase::Betting;
        self.advance((self.button + 1) % self.players.len());
        Ok(())
    }

    // --- Résultat ---------------------------------------------------------------------

    /// Mises de chaque joueur, pour le calcul des pots.
    #[must_use]
    pub fn contributions(&self) -> Vec<Contribution> {
        self.players
            .iter()
            .enumerate()
            .map(|(i, p)| Contribution {
                player: i,
                live: p.live_total,
                dead: p.dead_total,
                folded: p.status == PlayerStatus::Folded,
            })
            .collect()
    }

    /// Répartit les pots d'une main terminée. `reveals[p]` décrit les cartes du joueur `p`.
    pub fn settle(&self, reveals: &[Reveal]) -> Result<Settlement, GameError> {
        if !matches!(self.phase, Phase::Showdown | Phase::Finished) {
            return Err(GameError::HandNotOver);
        }
        let n = self.players.len();
        if reveals.len() != n {
            return Err(GameError::RevealCount {
                expected: n,
                got: reveals.len(),
            });
        }
        let board: CardSet = self.board.iter().collect();
        let mut hand_ranks = vec![None; n];
        let claims: Vec<Claim> = reveals
            .iter()
            .enumerate()
            .map(|(i, r)| match r {
                Reveal::Shown(h) if board.len() == 5 => {
                    let rank = evaluate(board | h.set());
                    hand_ranks[i] = Some(rank);
                    Claim::Shown(rank)
                }
                Reveal::Shown(_) | Reveal::Unknown => Claim::Unknown,
                Reveal::Mucked => Claim::Mucked,
            })
            .collect();
        let pots = build_pots(&self.contributions());
        let mut collected = award_pots(&pots.pots, &claims, &self.odd_chip_order())?;
        if let Some((p, amount)) = pots.uncalled {
            collected[p] += amount;
        }
        let net = collected
            .iter()
            .zip(&self.players)
            .map(|(c, p)| c.get() as i64 - p.invested().get() as i64)
            .collect();
        Ok(Settlement {
            pots,
            collected,
            net,
            hand_ranks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::parse_cards;

    fn setup(stacks: &[u64], button: usize, sb: u64, bb: u64) -> HandState {
        setup_with(BettingStructure::NoLimit, stacks, button, sb, bb)
    }

    fn setup_with(
        structure: BettingStructure,
        stacks: &[u64],
        button: usize,
        sb: u64,
        bb: u64,
    ) -> HandState {
        let stacks = stacks.iter().map(|s| Chips(*s)).collect();
        HandState::new(
            &HandSetup::new(stacks, button, Blinds::new(sb, bb)).with_structure(structure),
        )
        .unwrap()
    }

    fn deal(h: &mut HandState, s: &str) {
        h.deal(&parse_cards(s).unwrap()).unwrap();
    }

    fn shown(s: &str) -> Reveal {
        Reveal::Shown(s.parse().unwrap())
    }

    #[test]
    fn three_max_fold_to_big_blind() {
        // Bouton 0, SB 1, BB 2.
        let mut h = setup(&[1000, 1000, 1000], 0, 50, 100);
        assert_eq!(h.to_act(), Some(0));
        assert_eq!(h.position(0), Some(Position::Button));
        assert_eq!(h.position(2), Some(Position::BigBlind));
        h.apply(0, Action::Fold).unwrap();
        h.apply(1, Action::Fold).unwrap();
        assert_eq!(h.phase(), Phase::Finished);
        let s = h.settle(&[Reveal::Unknown; 3]).unwrap();
        assert_eq!(s.pots.uncalled, Some((2, Chips(50))));
        assert_eq!(s.net, vec![0, -50, 50]);
    }

    #[test]
    fn heads_up_action_order() {
        let mut h = setup(&[1000, 1000], 0, 50, 100);
        // Préflop : le bouton (petite blinde) parle en premier.
        assert_eq!(h.blind_seats(), (0, 1));
        assert_eq!(h.to_act(), Some(0));
        h.apply(0, Action::Call).unwrap();
        // Option de la grosse blinde.
        assert_eq!(h.to_act(), Some(1));
        h.apply(1, Action::Check).unwrap();
        assert_eq!(h.phase(), Phase::AwaitingBoard(Street::Flop));
        deal(&mut h, "Ah7c2d");
        // Postflop : la grosse blinde parle en premier.
        assert_eq!(h.to_act(), Some(1));
    }

    #[test]
    fn big_blind_option_allows_raise() {
        let mut h = setup(&[1000, 1000, 1000], 0, 50, 100);
        h.apply(0, Action::Call).unwrap();
        h.apply(1, Action::Call).unwrap();
        let legal = h.legal_actions().unwrap();
        assert_eq!(legal.player, 2);
        assert!(legal.check);
        assert_eq!(legal.raise, Some((Chips(200), Chips(1000))));
        h.apply(2, Action::Raise(Chips(300))).unwrap();
        assert_eq!(h.to_act(), Some(0));
    }

    #[test]
    fn minimum_raise_is_enforced() {
        let mut h = setup(&[1000, 1000, 1000, 1000], 0, 50, 100);
        // UTG = joueur 3 (4 joueurs : BTN 0, SB 1, BB 2).
        assert_eq!(h.to_act(), Some(3));
        assert!(h.apply(3, Action::Raise(Chips(150))).is_err());
        h.apply(3, Action::Raise(Chips(300))).unwrap();
        // Relance minimale suivante : 300 + 200.
        assert!(h.apply(0, Action::Raise(Chips(450))).is_err());
        assert!(h.apply(0, Action::Check).is_err());
        assert!(h.apply(0, Action::Bet(Chips(500))).is_err());
        h.apply(0, Action::Raise(Chips(500))).unwrap();
        assert!(h.apply(2, Action::Call).is_err(), "hors tour");
    }

    #[test]
    fn short_all_in_does_not_reopen_betting() {
        // BTN 0 (1000), SB 1 (1000), BB 2 (1000), UTG 3 (250).
        let mut h = setup(&[1000, 1000, 1000, 250], 0, 50, 100);
        h.apply(3, Action::Call).unwrap(); // UTG limpe 100
        h.apply(0, Action::Raise(Chips(200))).unwrap(); // relance complète à 200 (+100)
        h.apply(1, Action::Fold).unwrap();
        h.apply(2, Action::Call).unwrap();
        // UTG relance à tapis à 250 : +50 < 100, relance incomplète.
        let a = h.apply(3, Action::AllIn).unwrap();
        assert_eq!(a.kind, ActionKind::Raise);
        assert!(a.all_in);
        // Le bouton a déjà agi face à 200 : il ne peut que suivre ou se coucher.
        let legal = h.legal_actions().unwrap();
        assert_eq!(legal.player, 0);
        assert_eq!(legal.raise, None);
        assert_eq!(legal.call, Some(Chips(50)));
        assert!(h.apply(0, Action::Raise(Chips(400))).is_err());
        h.apply(0, Action::Call).unwrap();
        h.apply(2, Action::Call).unwrap();
        assert_eq!(h.phase(), Phase::AwaitingBoard(Street::Flop));
    }

    #[test]
    fn cumulative_short_all_ins_reopen_betting() {
        // BTN 0, SB 1, BB 2, UTG 3, HJ 4 (400), CO 5 (520).
        let mut h = setup(&[5000, 5000, 5000, 5000, 400, 520], 0, 50, 100);
        h.apply(3, Action::Raise(Chips(300))).unwrap(); // +200 : relance minimale désormais 200
        h.apply(4, Action::AllIn).unwrap(); // 400 : +100, incomplète
        h.apply(5, Action::AllIn).unwrap(); // 520 : +120, incomplète
        h.apply(0, Action::Fold).unwrap();
        h.apply(1, Action::Fold).unwrap();
        h.apply(2, Action::Fold).unwrap();
        // UTG fait face à 520 - 300 = 220 >= 200 : les enchères sont rouvertes.
        let legal = h.legal_actions().unwrap();
        assert_eq!(legal.player, 3);
        assert_eq!(legal.raise, Some((Chips(720), Chips(5000))));
    }

    #[test]
    fn all_in_runout_and_side_pots() {
        // BTN 0 (300), SB 1 (1000), BB 2 (600).
        let mut h = setup(&[300, 1000, 600], 0, 50, 100);
        h.apply(0, Action::AllIn).unwrap();
        h.apply(1, Action::AllIn).unwrap();
        h.apply(2, Action::Call).unwrap();
        // Plus personne ne peut miser : le board se déroule sans enchères.
        assert_eq!(h.phase(), Phase::AwaitingBoard(Street::Flop));
        deal(&mut h, "2c7d9h");
        assert_eq!(h.phase(), Phase::AwaitingBoard(Street::Turn));
        deal(&mut h, "Js");
        deal(&mut h, "3d");
        assert_eq!(h.phase(), Phase::Showdown);
        // BTN : brelan, SB : paire d'As, BB : paire de Rois.
        let s = h
            .settle(&[shown("7c7h"), shown("AsAh"), shown("KsKh")])
            .unwrap();
        assert_eq!(s.pots.uncalled, Some((1, Chips(400))));
        assert_eq!(s.pots.pots[0].amount, Chips(900));
        assert_eq!(s.pots.pots[1].amount, Chips(600));
        assert_eq!(s.collected, vec![Chips(900), Chips(1000), Chips(0)]);
        assert_eq!(s.net, vec![600, 0, -600]);
        assert_eq!(s.net.iter().sum::<i64>(), 0);
    }

    #[test]
    fn unknown_cards_leave_result_unresolved() {
        let mut h = setup(&[1000, 1000], 0, 50, 100);
        h.apply(0, Action::AllIn).unwrap();
        h.apply(1, Action::Call).unwrap();
        deal(&mut h, "2c7d9h");
        deal(&mut h, "Js");
        deal(&mut h, "3d");
        let r = h.settle(&[shown("AsAh"), Reveal::Unknown]);
        assert!(matches!(
            r,
            Err(GameError::Award(AwardError::Unresolved { .. }))
        ));
    }

    #[test]
    fn antes_and_short_blinds() {
        let blinds = Blinds::new(50, 100).with_ante(Ante::BigBlind(Chips(100)));
        let mut h = HandState::new(&HandSetup::new(
            vec![Chips(1000), Chips(1000), Chips(150)],
            0,
            blinds,
        ))
        .unwrap();
        // La grosse blinde paie l'ante (100) en priorité, puis 50 de blinde : tapis.
        let bb = &h.players()[2];
        assert_eq!(bb.dead_total, Chips(100));
        assert_eq!(bb.street_bet, Chips(50));
        assert_eq!(bb.status, PlayerStatus::AllIn);
        assert_eq!(h.pot_total(), Chips(200));
        h.apply(0, Action::Fold).unwrap();
        // La mise à suivre reste la grosse blinde nominale.
        assert_eq!(h.legal_actions().unwrap().call, Some(Chips(50)));
        h.apply(1, Action::Call).unwrap();
        assert_eq!(h.phase(), Phase::AwaitingBoard(Street::Flop));

        let classic = Blinds::new(50, 100).with_ante(Ante::Classic(Chips(10)));
        let h = HandState::new(&HandSetup::new(vec![Chips(1000); 3], 0, classic)).unwrap();
        assert_eq!(h.pot_total(), Chips(180));
    }

    #[test]
    fn board_dealing_errors() {
        let mut h = setup(&[1000, 1000], 0, 50, 100);
        assert!(matches!(
            h.deal(&parse_cards("AsKsQs").unwrap()),
            Err(GameError::NotAwaitingBoard(_))
        ));
        h.apply(0, Action::Call).unwrap();
        h.apply(1, Action::Check).unwrap();
        assert!(matches!(
            h.deal(&parse_cards("AsKs").unwrap()),
            Err(GameError::BoardSize { .. })
        ));
        deal(&mut h, "AsKsQs");
        h.apply(1, Action::Check).unwrap();
        h.apply(0, Action::Check).unwrap();
        assert_eq!(
            h.deal(&parse_cards("As").unwrap()),
            Err(GameError::DuplicateCard("As".parse().unwrap()))
        );
    }

    #[test]
    fn postflop_bet_and_fold_returns_uncalled() {
        let mut h = setup(&[1000, 1000, 1000], 0, 50, 100);
        h.apply(0, Action::Raise(Chips(250))).unwrap();
        h.apply(1, Action::Fold).unwrap();
        h.apply(2, Action::Call).unwrap();
        deal(&mut h, "Ah7c2d");
        assert_eq!(h.to_act(), Some(2));
        assert!(
            h.apply(2, Action::Bet(Chips(50))).is_err(),
            "sous la grosse blinde"
        );
        h.apply(2, Action::Check).unwrap();
        h.apply(0, Action::Bet(Chips(300))).unwrap();
        h.apply(2, Action::Fold).unwrap();
        assert_eq!(h.phase(), Phase::Finished);
        let s = h.settle(&[Reveal::Unknown; 3]).unwrap();
        assert_eq!(s.pots.uncalled, Some((0, Chips(300))));
        assert_eq!(s.net, vec![300, -50, -250]);
    }

    #[test]
    fn full_ring_order_and_positions() {
        // 9 joueurs, bouton 0 : SB 1, BB 2, UTG 3 … CO 8.
        let h = setup(&[1000; 9], 0, 50, 100);
        assert_eq!(h.to_act(), Some(3));
        assert_eq!(h.position(3), Some(Position::UnderTheGun));
        assert_eq!(h.position(4), Some(Position::UnderTheGun1));
        assert_eq!(h.position(6), Some(Position::Lojack));
        assert_eq!(h.position(8), Some(Position::Cutoff));
        let h = setup(&[1000; 10], 9, 50, 100);
        assert_eq!(h.to_act(), Some(2));
        assert_eq!(h.position(2), Some(Position::UnderTheGun));
        assert_eq!(h.position(5), Some(Position::UnderTheGun3));
    }

    #[test]
    fn pot_limit_bounds() {
        // Blindes 1/2, 3 joueurs : relance « pot » préflop = 2 + 3 + 2 = 7.
        let mut h = setup_with(BettingStructure::PotLimit, &[1000, 1000, 1000], 0, 1, 2);
        assert_eq!(h.legal_actions().unwrap().raise, Some((Chips(4), Chips(7))));
        assert!(h.apply(0, Action::Raise(Chips(8))).is_err());
        assert!(h.apply(0, Action::AllIn).is_err(), "tapis au-delà du pot");
        h.apply(0, Action::Raise(Chips(7))).unwrap();
        // SB : 7 + (7 + 2 + 1) + (7 - 1) = 23.
        assert_eq!(
            h.legal_actions().unwrap().raise,
            Some((Chips(12), Chips(23)))
        );
        h.apply(1, Action::Call).unwrap();
        h.apply(2, Action::Fold).unwrap();
        deal(&mut h, "Ah7c2d");
        // Pot de 16 : mise maximale 16.
        assert_eq!(h.legal_actions().unwrap().bet, Some((Chips(2), Chips(16))));
        h.apply(1, Action::Bet(Chips(8))).unwrap();
        // Exemple de référence : 8 dans 16 → call 8, pot 32, relance à 8 + 32 = 40.
        assert_eq!(
            h.legal_actions().unwrap().raise,
            Some((Chips(16), Chips(40)))
        );
    }

    #[test]
    fn fixed_limit_sizes_and_cap() {
        let fl = BettingStructure::FIXED_LIMIT_CASH;
        // Limite 2/4 : blindes 1/2, 4 joueurs, UTG = joueur 3.
        let mut h = setup_with(fl, &[1000; 4], 0, 1, 2);
        let legal = h.legal_actions().unwrap();
        assert_eq!(legal.raise, Some((Chips(4), Chips(4))));
        assert!(
            h.apply(3, Action::Raise(Chips(5))).is_err(),
            "montant imposé"
        );
        h.apply(3, Action::Raise(Chips(4))).unwrap();
        h.apply(0, Action::Raise(Chips(6))).unwrap();
        h.apply(1, Action::Raise(Chips(8))).unwrap(); // 4e mise : plafond
        assert_eq!(h.bets_this_street(), 4);
        let legal = h.legal_actions().unwrap();
        assert_eq!(legal.player, 2);
        assert_eq!(legal.raise, None);
        assert!(h.apply(2, Action::Raise(Chips(10))).is_err());
        h.apply(2, Action::Call).unwrap();
        h.apply(3, Action::Call).unwrap();
        h.apply(0, Action::Call).unwrap();
        deal(&mut h, "Ah7c2d");
        assert_eq!(h.legal_actions().unwrap().bet, Some((Chips(2), Chips(2))));
        h.apply(1, Action::Check).unwrap();
        h.apply(2, Action::Check).unwrap();
        h.apply(3, Action::Check).unwrap();
        h.apply(0, Action::Check).unwrap();
        deal(&mut h, "Ks");
        // Turn : grosse mise.
        assert_eq!(h.legal_actions().unwrap().bet, Some((Chips(4), Chips(4))));
    }

    #[test]
    fn fixed_limit_heads_up_cap() {
        // Cash game : plafond levé quand il ne reste que deux joueurs dans le coup.
        let mut h = setup_with(BettingStructure::FIXED_LIMIT_CASH, &[1000; 3], 0, 1, 2);
        h.apply(0, Action::Fold).unwrap();
        for to in [4, 6, 8, 10, 12] {
            let p = h.to_act().unwrap();
            h.apply(p, Action::Raise(Chips(to))).unwrap();
        }
        // Tournoi : plafond maintenu à trois joueurs servis.
        let mut h = setup_with(
            BettingStructure::FIXED_LIMIT_TOURNAMENT,
            &[1000; 3],
            0,
            1,
            2,
        );
        h.apply(0, Action::Fold).unwrap();
        h.apply(1, Action::Raise(Chips(4))).unwrap();
        h.apply(2, Action::Raise(Chips(6))).unwrap();
        h.apply(1, Action::Raise(Chips(8))).unwrap();
        assert_eq!(h.legal_actions().unwrap().raise, None);
    }

    #[test]
    fn fixed_limit_short_all_ins() {
        let fl = BettingStructure::FIXED_LIMIT_CASH;
        // Unité de 20 au flop (blindes 10/20). BTN 0, SB 1, BB 2 (tapis court), UTG 3.
        let mut h = setup_with(fl, &[1000, 1000, 45, 1000], 0, 10, 20);
        h.apply(3, Action::Call).unwrap();
        h.apply(0, Action::Call).unwrap();
        h.apply(1, Action::Call).unwrap();
        h.apply(2, Action::Check).unwrap();
        deal(&mut h, "Ah7c2d");
        h.apply(1, Action::Bet(Chips(20))).unwrap();
        // BB à tapis pour 25 : +5, moins d'une demi-mise → relance incomplète.
        let a = h.apply(2, Action::AllIn).unwrap();
        assert_eq!(
            (a.kind, a.to, a.all_in),
            (ActionKind::Raise, Chips(25), true)
        );
        // UTG n'a pas encore agi : il peut compléter à 40 (20 + 20).
        assert_eq!(
            h.legal_actions().unwrap().raise,
            Some((Chips(40), Chips(40)))
        );
        h.apply(3, Action::Call).unwrap();
        h.apply(0, Action::Call).unwrap();
        // La SB a misé 20 avant le tapis incomplet : elle ne peut que suivre.
        assert_eq!(h.legal_actions().unwrap().raise, None);
        assert_eq!(h.legal_actions().unwrap().call, Some(Chips(5)));

        // Tapis d'au moins une demi-mise : compte comme une relance complète.
        let mut h = setup_with(fl, &[1000, 1000, 50, 1000], 0, 10, 20);
        h.apply(3, Action::Call).unwrap();
        h.apply(0, Action::Call).unwrap();
        h.apply(1, Action::Call).unwrap();
        h.apply(2, Action::Check).unwrap();
        deal(&mut h, "Ah7c2d");
        h.apply(1, Action::Bet(Chips(20))).unwrap();
        h.apply(2, Action::AllIn).unwrap(); // 30 : +10 = demi-mise
        assert_eq!(h.bets_this_street(), 2);
        // Relance suivante : 30 + 20 = 50 ; la SB pourra relancer à son tour.
        assert_eq!(
            h.legal_actions().unwrap().raise,
            Some((Chips(50), Chips(50)))
        );
        h.apply(3, Action::Call).unwrap();
        h.apply(0, Action::Call).unwrap();
        assert_eq!(
            h.legal_actions().unwrap().raise,
            Some((Chips(50), Chips(50)))
        );
    }

    #[test]
    fn invalid_setups() {
        let bad = |stacks: Vec<u64>, button, sb, bb| {
            HandState::new(&HandSetup::new(
                stacks.into_iter().map(Chips).collect(),
                button,
                Blinds::new(sb, bb),
            ))
            .is_err()
        };
        assert!(bad(vec![100], 0, 1, 2));
        assert!(bad(vec![100, 100], 2, 1, 2));
        assert!(bad(vec![100, 100], 0, 3, 2));
        assert!(bad(vec![100, 0], 0, 1, 2));
        assert!(bad(vec![100, 100], 0, 0, 0));
        // Montants démesurés (historique corrompu) : refusés plutôt que de déborder.
        assert!(bad(vec![u64::MAX, 100], 0, 1, 2));
        assert!(bad(vec![MAX_TOTAL_CHIPS, MAX_TOTAL_CHIPS], 0, 1, 2));
        assert!(bad(vec![100, 100], 0, 1, u64::MAX));
        assert!(!bad(
            vec![MAX_TOTAL_CHIPS / 2, MAX_TOTAL_CHIPS / 2],
            0,
            1,
            2
        ));
    }
}
