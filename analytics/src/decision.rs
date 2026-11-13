//! Analyse de chaque décision d'un joueur (en pratique, le héros) : pot, montant à suivre,
//! cotes du pot, équité requise, tapis effectif, SPR, équité réelle ou estimée, EV du call
//! par rapport au fold, fold equity nécessaire pour une mise ou une relance et, en tournoi,
//! la même décision vue par l'ICM.
//!
//! # Conventions
//!
//! - `pot` : jetons au milieu **avant** la décision (antes, blindes et mises du tour
//!   compris), comme dans [`stratyx_poker::odds`].
//! - Quand le joueur ne peut suivre qu'en partie (tapis insuffisant), les mises adverses
//!   au-delà de son niveau iront dans un pot secondaire qu'il ne peut pas gagner : elles
//!   sont retirées du pot gagnable pour les cotes, l'équité requise et l'EV du call.
//! - **Tapis effectif** : ce que le joueur peut encore perdre, soit le minimum entre son
//!   tapis restant et ce que l'adversaire le mieux pourvu encore en jeu peut égaler.
//!   **SPR** = tapis effectif / pot.
//! - **Équité** : contre les cartes de chaque adversaire encore en jeu si elles sont connues
//!   (montrées à un moment de la main), sinon contre la range fournie par l'appelant pour
//!   son siège, sinon contre la range par défaut (toutes les mains compatibles si aucune).
//!   Aucune carte n'est jamais inventée : les cartes inconnues restent des ranges.
//! - **EV du call** : `équité × (pot gagnable + call) − call`, en jetons, par rapport au fold.
//!   C'est l'EV immédiate : elle suppose que l'équité se réalise à l'abattage, sans action
//!   ultérieure.
//! - **Fold equity** (mises et relances) : modèle à un adversaire, celui qui peut égaler le
//!   plus de jetons parmi ceux qui ne sont pas à tapis. On donne la fréquence de fold
//!   nécessaire pour un bluff pur et, si l'équité est connue, compte tenu de l'équité
//!   quand on est payé ([`stratyx_poker::odds::bet_ev`]).
//! - **ICM** (formats à prix fixes, structure de prix fournie) : calculé pour un call qui
//!   clôt les enchères en tête-à-tête (l'adversaire est à tapis ou le call met le joueur à
//!   tapis). Les tapis pris en compte sont ceux de la table.
//!
//! La comparaison avec la décision de référence des IA passe par le trait
//! [`ReferenceStrategy`], point d'extension non implémenté dans ce crate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use stratyx_poker::card::Card;
use stratyx_poker::cardset::CardSet;
use stratyx_poker::equity::{EquityMethod, EquityOptions, equity};
use stratyx_poker::format::GameFormat;
use stratyx_poker::game::{Action, AppliedAction, HandState, PlayerState, PlayerStatus, Street};
use stratyx_poker::history::HandRecord;
use stratyx_poker::icm::icm_equities;
use stratyx_poker::odds;
use stratyx_poker::position::Position;
use stratyx_poker::range::{HoleCards, Range};
use stratyx_shared::Chips;

use crate::error::AnalyticsError;
use crate::hand::{ParsedHand, Step};

/// Paramètres de l'analyse de décision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionOptions {
    /// Paramètres du calcul d'équité (Monte-Carlo au-delà du seuil d'énumération exacte).
    pub equity: EquityOptions,
    /// Range attribuée à un adversaire, par numéro de siège, quand ses cartes sont inconnues.
    pub opponent_ranges: BTreeMap<u8, Range>,
    /// Range des autres adversaires aux cartes inconnues ; `None` : toutes les mains
    /// compatibles avec les cartes connues.
    pub default_range: Option<Range>,
    /// Utiliser les cartes adverses montrées dans la main (équité « réelle »). Sinon, seules
    /// les ranges sont utilisées (information disponible au moment de la décision).
    pub use_known_cards: bool,
    /// Prix des places restant à attribuer (première place en premier), dans une unité
    /// quelconque (centimes, fractions du prize pool). Active l'ICM en tournoi.
    pub icm_payouts: Option<Vec<f64>>,
}

impl Default for DecisionOptions {
    fn default() -> Self {
        DecisionOptions {
            equity: EquityOptions::default(),
            opponent_ranges: BTreeMap::new(),
            default_range: None,
            use_known_cards: true,
            icm_payouts: None,
        }
    }
}

/// Source de l'équité calculée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquityBasis {
    /// Cartes de tous les adversaires connues : équité réelle.
    KnownCards,
    /// Aucune carte adverse connue : équité contre des ranges.
    Ranges,
    /// Certains adversaires connus, d'autres représentés par des ranges.
    Mixed,
}

/// Équité du joueur au moment de la décision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EquityEstimate {
    pub equity: f64,
    /// Erreur-type (0 en calcul exact).
    pub std_error: f64,
    pub method: EquityMethod,
    pub basis: EquityBasis,
}

/// Adversaire encore en jeu au moment de la décision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpponentInfo {
    pub seat: u8,
    pub player: String,
    pub position: Position,
    /// Tapis restant.
    pub stack: Chips,
    /// Mise engagée sur le tour.
    pub street_bet: Chips,
    pub all_in: bool,
    /// Cartes, si elles ont été montrées dans la main et que l'option les autorise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_cards: Option<HoleCards>,
}

/// Fold equity nécessaire pour une mise ou une relance (modèle à un adversaire).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FoldEquity {
    /// Siège de l'adversaire modélisé.
    pub opponent_seat: u8,
    /// Jetons mis en jeu par la mise et qui peuvent être payés.
    pub risk: Chips,
    /// Montant que l'adversaire doit ajouter pour payer.
    pub opponent_call: Chips,
    /// Fréquence de fold nécessaire pour un bluff pur : `risque / (pot + risque)`.
    pub break_even_bluff: f64,
    /// Fréquence de fold nécessaire compte tenu de l'équité quand on est payé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_even: Option<f64>,
    /// EV (jetons) si l'adversaire paie toujours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev_when_called: Option<f64>,
}

/// Décision de call vue par l'ICM (valeurs dans l'unité des prix).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IcmAnalysis {
    /// Valeur ICM du joueur s'il se couche.
    pub fold: f64,
    /// … s'il paie et gagne le coup.
    pub call_win: f64,
    /// … s'il paie et perd le coup.
    pub call_lose: f64,
    /// Équité nécessaire pour que le call vaille le fold en ICM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_equity: Option<f64>,
    /// Prime de risque : équité requise ICM − équité requise en jetons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_premium: Option<f64>,
    /// Valeur ICM espérée du call moins celle du fold, si l'équité est connue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev_call_minus_fold: Option<f64>,
}

/// Situation soumise à une stratégie de référence : uniquement l'information disponible
/// au moment de la décision (jamais la suite de la main).
#[derive(Debug, Clone, Copy)]
pub struct DecisionSpot<'a> {
    pub format: GameFormat,
    /// État de la main juste avant la décision.
    pub state: &'a HandState,
    /// Index du joueur dans le moteur.
    pub player: usize,
    pub hole_cards: Option<HoleCards>,
}

/// Une option recommandée par la référence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceOption {
    pub action: Action,
    /// Fréquence de la stratégie de référence, dans `[0, 1]`.
    pub frequency: f64,
    /// EV de l'option, en grosses blindes, si la référence la fournit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev_bb: Option<f64>,
}

/// Conseil d'une stratégie de référence pour une décision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceAdvice {
    /// Nom de la référence (IA Expresso 3-Max, IA Tournois 6-Max, solveur…).
    pub source: String,
    pub options: Vec<ReferenceOption>,
}

/// Point d'extension : décision de référence des IA spécialisées ou d'un solveur.
///
/// Non implémenté dans ce crate ; les IA (`ai/*`) fourniront leurs implémentations. La
/// référence ne reçoit que l'information disponible au moment de la décision, afin que la
/// comparaison reste indépendante du résultat de la main.
pub trait ReferenceStrategy {
    /// Conseil pour cette situation, ou `None` si la référence ne la couvre pas.
    fn advise(&self, spot: &DecisionSpot<'_>) -> Option<ReferenceAdvice>;
}

/// Analyse d'une décision, destinée à l'interface et au coaching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionReport {
    pub hand_id: String,
    /// Index de l'événement dans l'historique.
    pub event_index: usize,
    pub seat: u8,
    pub player: String,
    pub street: Street,
    pub position: Position,
    pub board: Vec<Card>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole_cards: Option<HoleCards>,
    /// Action choisie, avec ses montants.
    pub action: AppliedAction,
    pub big_blind: Chips,
    /// Pot avant la décision.
    pub pot: Chips,
    /// Montant à ajouter pour suivre (borné par le tapis).
    pub to_call: Chips,
    /// Cotes du pot « x contre 1 » : pot gagnable / montant à suivre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pot_odds: Option<f64>,
    /// Équité minimale pour suivre : `call / (pot gagnable + call)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_equity: Option<f64>,
    pub effective_stack: Chips,
    /// Tapis effectif / pot.
    pub spr: f64,
    pub opponents: Vec<OpponentInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equity: Option<EquityEstimate>,
    /// EV du call moins EV du fold, en jetons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_ev: Option<f64>,
    /// Même valeur en grosses blindes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_ev_bb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_equity: Option<FoldEquity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icm: Option<IcmAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceAdvice>,
    /// Calculs non effectués et pourquoi.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Analyse les décisions du héros (`meta.hero_seat`).
pub fn analyze_hero_decisions(
    record: &HandRecord,
    options: &DecisionOptions,
) -> Result<Vec<DecisionReport>, AnalyticsError> {
    let seat = record
        .meta
        .hero_seat
        .ok_or_else(|| AnalyticsError::NoHero(record.id.clone()))?;
    analyze_decisions(record, seat, options, None)
}

/// Analyse chaque décision du joueur assis au siège `seat`, avec une stratégie de
/// référence facultative.
pub fn analyze_decisions(
    record: &HandRecord,
    seat: u8,
    options: &DecisionOptions,
    reference: Option<&dyn ReferenceStrategy>,
) -> Result<Vec<DecisionReport>, AnalyticsError> {
    let hand = ParsedHand::parse(record, true)?;
    let player =
        hand.seats
            .iter()
            .position(|s| *s == seat)
            .ok_or_else(|| AnalyticsError::SeatNotFound {
                hand_id: record.id.clone(),
                seat,
            })?;
    Ok(hand
        .steps
        .iter()
        .filter(|s| s.player == player)
        .map(|s| analyze_step(&hand, s, options, reference))
        .collect())
}

/// Situation d'une décision, partagée par les calculs.
struct Spot<'a, 'h> {
    hand: &'a ParsedHand<'h>,
    step: &'a Step,
    players: &'a [PlayerState],
    /// Adversaires encore en jeu (index du moteur).
    opponents: Vec<usize>,
    options: &'a DecisionOptions,
}

fn analyze_step(
    hand: &ParsedHand<'_>,
    step: &Step,
    options: &DecisionOptions,
    reference: Option<&dyn ReferenceStrategy>,
) -> DecisionReport {
    let state = step.state.as_ref().expect("états conservés pour l'analyse");
    let p = step.player;
    let players = state.players();
    let spot = Spot {
        hand,
        step,
        players,
        opponents: (0..hand.len())
            .filter(|o| *o != p && players[*o].status != PlayerStatus::Folded)
            .collect(),
        options,
    };
    let bb = hand.big_blind();
    let mut notes = Vec::new();

    let opponent_infos: Vec<OpponentInfo> = spot
        .opponents
        .iter()
        .map(|o| OpponentInfo {
            seat: hand.seats[*o],
            player: hand.names[*o].to_string(),
            position: hand.position(*o),
            stack: players[*o].stack,
            street_bet: players[*o].street_bet,
            all_in: players[*o].status == PlayerStatus::AllIn,
            known_cards: spot.known(*o),
        })
        .collect();

    let winnable = spot.winnable_pot();
    let facing = !step.to_call.is_zero();
    let call = step.to_call.get() as f64;
    let pot_odds = facing.then(|| winnable.get() as f64 / call);
    let required_equity = facing.then(|| odds::required_equity(winnable.get() as f64, call));

    let reach = spot
        .opponents
        .iter()
        .map(|o| (players[*o].street_bet + players[*o].stack).saturating_sub(step.street_bet))
        .max()
        .unwrap_or(Chips::ZERO);
    let effective_stack = step.stack.min(reach);
    let spr = effective_stack.get() as f64 / step.pot.get() as f64;

    let hole_cards = hand.known[p];
    let board = hand.board(step.board_len).to_vec();
    let estimate = match hole_cards {
        Some(hole) => spot.equity(hole, &board, &mut notes),
        None => {
            notes.push("cartes du joueur inconnues : équité non calculée".into());
            None
        }
    };
    let eq = estimate.map(|e| e.equity);
    let call_ev = match (facing, eq) {
        (true, Some(e)) => Some(odds::call_ev(e, winnable.get() as f64, call)),
        _ => None,
    };
    let fold_equity = step
        .applied
        .kind
        .is_aggressive()
        .then(|| spot.fold_equity(eq))
        .flatten();
    let icm = spot.icm(required_equity, eq, &mut notes);

    let reference = reference.and_then(|r| {
        r.advise(&DecisionSpot {
            format: hand.record.format,
            state,
            player: p,
            hole_cards,
        })
    });

    DecisionReport {
        hand_id: hand.record.id.clone(),
        event_index: step.event_index,
        seat: hand.seats[p],
        player: hand.names[p].to_string(),
        street: step.street,
        position: step.position,
        board,
        hole_cards,
        action: step.applied,
        big_blind: bb,
        pot: step.pot,
        to_call: step.to_call,
        pot_odds,
        required_equity,
        effective_stack,
        spr,
        opponents: opponent_infos,
        equity: estimate,
        call_ev,
        call_ev_bb: call_ev.map(|v| v / bb.get() as f64),
        fold_equity,
        icm,
        reference,
        notes,
    }
}

impl Spot<'_, '_> {
    /// Cartes d'un adversaire, si elles sont connues et que l'option les autorise.
    fn known(&self, o: usize) -> Option<HoleCards> {
        self.hand.known[o].filter(|_| self.options.use_known_cards)
    }

    /// Pot gagnable en suivant : les mises adverses au-delà du niveau que le joueur peut
    /// atteindre iront dans un pot secondaire.
    fn winnable_pot(&self) -> Chips {
        let level = self.step.street_bet + self.step.to_call;
        let excess: Chips = self
            .opponents
            .iter()
            .map(|o| self.players[*o].street_bet.saturating_sub(level))
            .sum();
        self.step.pot.saturating_sub(excess)
    }

    /// Équité du joueur contre les adversaires encore en jeu.
    fn equity(
        &self,
        hole: HoleCards,
        board: &[Card],
        notes: &mut Vec<String>,
    ) -> Option<EquityEstimate> {
        let options = self.options;
        let mut ranges = vec![Range::single(hole)];
        let mut known = 0usize;
        for o in &self.opponents {
            match self.known(*o) {
                Some(h) => {
                    known += 1;
                    ranges.push(Range::single(h));
                }
                None => ranges.push(
                    options
                        .opponent_ranges
                        .get(&self.hand.seats[*o])
                        .or(options.default_range.as_ref())
                        .cloned()
                        .unwrap_or_else(Range::full),
                ),
            }
        }
        let basis = match known {
            0 => EquityBasis::Ranges,
            k if k == self.opponents.len() => EquityBasis::KnownCards,
            _ => EquityBasis::Mixed,
        };
        let board: CardSet = board.iter().collect();
        match equity(&ranges, board, CardSet::EMPTY, &options.equity) {
            Ok(r) => Some(EquityEstimate {
                equity: r.players[0].equity,
                std_error: r.players[0].std_error,
                method: r.method,
                basis,
            }),
            Err(e) => {
                notes.push(format!("équité non calculée : {e}"));
                None
            }
        }
    }

    /// Fold equity nécessaire pour la mise ou la relance effectuée.
    fn fold_equity(&self, eq: Option<f64>) -> Option<FoldEquity> {
        let players = self.players;
        let o = *self
            .opponents
            .iter()
            .filter(|o| players[**o].status == PlayerStatus::Active)
            .max_by_key(|o| players[**o].street_bet + players[**o].stack)?;
        let opp = &players[o];
        let step = self.step;
        let matched_to = step.applied.to.min(opp.street_bet + opp.stack);
        let risk = matched_to.saturating_sub(step.street_bet);
        let opponent_call = matched_to.saturating_sub(opp.street_bet);
        let pot = step.pot.get() as f64;
        let (r, c) = (risk.get() as f64, opponent_call.get() as f64);
        let when_called = eq.map(|e| e * (pot + r + c) - r);
        let break_even = when_called.map(|w| if w >= 0.0 { 0.0 } else { -w / (pot - w) });
        Some(FoldEquity {
            opponent_seat: self.hand.seats[o],
            risk,
            opponent_call,
            break_even_bluff: odds::bluff_break_even(pot, r),
            break_even,
            ev_when_called: when_called,
        })
    }

    /// Le call face à une mise, vu par l'ICM, quand il clôt les enchères en tête-à-tête.
    fn icm(
        &self,
        chip_required: Option<f64>,
        eq: Option<f64>,
        notes: &mut Vec<String>,
    ) -> Option<IcmAnalysis> {
        let payouts = self.options.icm_payouts.as_deref()?;
        let step = self.step;
        if step.to_call.is_zero() {
            return None;
        }
        if !self.hand.record.format.uses_icm() {
            notes.push("ICM non pertinent en cash game".into());
            return None;
        }
        if payouts.is_empty() {
            notes.push("ICM non calculé : structure de prix vide".into());
            return None;
        }
        let [o] = self.opponents[..] else {
            notes.push("ICM calculé seulement pour un call en tête-à-tête".into());
            return None;
        };
        let p = step.player;
        let closes = self.players[o].stack.is_zero() || step.to_call >= step.stack;
        if !closes {
            notes.push("ICM calculé seulement pour un call qui clôt les enchères (tapis)".into());
            return None;
        }
        let pot = step.pot;
        let level = step.street_bet + step.to_call;
        let excess = self.players[o].street_bet.saturating_sub(level);
        let base: Vec<Chips> = self.players.iter().map(|pl| pl.stack).collect();
        let scenario = |hero: Chips, opp: Chips| {
            let mut s = base.clone();
            s[p] = hero;
            s[o] = opp;
            icm_value(&s, p, payouts)
        };
        let stack = step.stack;
        let fold = scenario(stack, base[o] + pot)?;
        let call_win = scenario(stack + pot.saturating_sub(excess), base[o] + excess)?;
        let call_lose = scenario(stack - step.to_call, base[o] + pot + step.to_call)?;
        let spread = call_win - call_lose;
        let required_equity = (spread > 0.0).then(|| (fold - call_lose) / spread);
        Some(IcmAnalysis {
            fold,
            call_win,
            call_lose,
            required_equity,
            risk_premium: required_equity.zip(chip_required).map(|(i, c)| i - c),
            ev_call_minus_fold: eq.map(|e| e * call_win + (1.0 - e) * call_lose - fold),
        })
    }
}

/// Valeur ICM du joueur `who` pour des tapis donnés (tapis nul : joueur éliminé).
fn icm_value(stacks: &[Chips], who: usize, payouts: &[f64]) -> Option<f64> {
    let alive: Vec<usize> = (0..stacks.len())
        .filter(|i| !stacks[*i].is_zero())
        .collect();
    match alive.iter().position(|i| *i == who) {
        Some(idx) => {
            let alive_stacks: Vec<Chips> = alive.iter().map(|i| stacks[*i]).collect();
            icm_equities(&alive_stacks, payouts).ok().map(|e| e[idx])
        }
        // Éliminé : il prend la place qui suit celles des survivants.
        None => Some(payouts.get(alive.len()).copied().unwrap_or(0.0)),
    }
}
