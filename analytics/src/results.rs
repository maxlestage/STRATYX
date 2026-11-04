//! Résultats personnels du héros : gains en jetons et en grosses blindes, BB/100, détail par
//! position et par format, courbe cumulée, gains ajustés à l'all-in.
//!
//! # Résultat d'une main
//!
//! Le résultat net du héros est connu si la main est résolue, ou s'il s'est couché : il a
//! alors perdu exactement ce qu'il avait investi, quel que soit le vainqueur. Sinon (cartes
//! adverses inconnues à l'abattage, vidéo coupée), la main est comptée à part comme « non
//! résolue » et n'entre dans aucun total.
//!
//! # Gains ajustés à l'all-in
//!
//! Quand les enchères se ferment avant la river avec le héros encore en jeu (tapis payé), le
//! résultat réel est remplacé par son espérance : pour chaque pot auquel le héros est
//! éligible, son équité (calcul exact) sur le board connu au moment du tapis, multipliée par
//! le montant du pot, plus la mise non suivie rendue, moins sa mise totale. Ce calcul exige
//! les cartes de **tous** les joueurs encore en jeu : sans elles, l'ajustement n'est pas
//! calculé (le résultat réel est conservé et la main est comptée dans
//! [`ResultsSummary::all_in_unknown_cards`]) ; aucune carte n'est jamais inventée.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use stratyx_poker::cardset::CardSet;
use stratyx_poker::equity::{EquityOptions, equity};
use stratyx_poker::format::GameFormat;
use stratyx_poker::game::{Phase, Street};
use stratyx_poker::history::HandRecord;
use stratyx_poker::position::Position;
use stratyx_poker::pot::build_pots;
use stratyx_poker::range::{HoleCards, Range};
use stratyx_shared::Chips;

use crate::error::AnalyticsError;
use crate::hand::ParsedHand;

/// Équité du héros dans un pot, au moment du tapis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PotEquity {
    pub amount: Chips,
    /// Nombre de joueurs éligibles.
    pub players: usize,
    pub equity: f64,
}

/// Espérance du héros quand les enchères se ferment avant la river.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllInEv {
    /// Dernier tour d'enchères (board connu au moment du tapis).
    pub street: Street,
    /// Équité dans chaque pot auquel le héros est éligible (principal en premier).
    pub pots: Vec<PotEquity>,
    /// Résultat net espéré, en jetons.
    pub expected_net: f64,
}

/// Résultat du héros sur une main.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandResult {
    pub hand_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played_at_ms: Option<u64>,
    pub format: GameFormat,
    pub position: Position,
    pub big_blind: Chips,
    /// Résultat réel, en jetons.
    pub net_chips: i64,
    /// Espérance à l'all-in, si elle a pu être calculée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_in: Option<AllInEv>,
}

impl HandResult {
    #[must_use]
    pub fn net_bb(&self) -> f64 {
        self.net_chips as f64 / self.big_blind.get() as f64
    }

    /// Résultat ajusté à l'all-in, en jetons (le réel hors situation d'all-in).
    #[must_use]
    pub fn adjusted_net(&self) -> f64 {
        self.all_in
            .as_ref()
            .map_or(self.net_chips as f64, |a| a.expected_net)
    }

    #[must_use]
    pub fn adjusted_net_bb(&self) -> f64 {
        self.adjusted_net() / self.big_blind.get() as f64
    }
}

/// Totaux sur un ensemble de mains.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ResultLine {
    /// Mains au résultat connu.
    pub hands: u64,
    pub net_chips: i64,
    pub net_bb: f64,
    /// Total ajusté à l'all-in (jetons).
    pub adjusted_net_chips: f64,
    pub adjusted_net_bb: f64,
    /// Mains dont le résultat a été remplacé par l'espérance à l'all-in.
    pub all_in_adjusted_hands: u64,
}

impl ResultLine {
    /// Grosses blindes gagnées pour 100 mains.
    #[must_use]
    pub fn bb_per_100(&self) -> Option<f64> {
        (self.hands > 0).then(|| self.net_bb * 100.0 / self.hands as f64)
    }

    /// BB/100 ajustées à l'all-in.
    #[must_use]
    pub fn adjusted_bb_per_100(&self) -> Option<f64> {
        (self.hands > 0).then(|| self.adjusted_net_bb * 100.0 / self.hands as f64)
    }

    fn add(&mut self, r: &HandResult) {
        self.hands += 1;
        self.net_chips += r.net_chips;
        self.net_bb += r.net_bb();
        self.adjusted_net_chips += r.adjusted_net();
        self.adjusted_net_bb += r.adjusted_net_bb();
        self.all_in_adjusted_hands += u64::from(r.all_in.is_some());
    }
}

/// Résultats d'un format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatResults {
    pub format: GameFormat,
    pub overall: ResultLine,
    pub by_position: BTreeMap<Position, ResultLine>,
}

/// Point de la courbe de gains cumulés.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub hand_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played_at_ms: Option<u64>,
    pub net_chips: i64,
    pub cumulative_chips: i64,
    pub cumulative_bb: f64,
    pub cumulative_adjusted_bb: f64,
}

/// Synthèse des résultats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultsSummary {
    pub overall: ResultLine,
    pub by_position: BTreeMap<Position, ResultLine>,
    pub by_format: Vec<FormatResults>,
    /// Gains cumulés dans l'ordre chronologique (`meta.played_at_ms`) ; les mains sans date
    /// suivent, dans leur ordre d'ajout.
    pub curve: Vec<CurvePoint>,
    /// Mains dont le résultat du héros est indéterminable sans inventer de cartes.
    pub unresolved_hands: u64,
    /// Situations d'all-in sans les cartes de tous les adversaires : non ajustées.
    pub all_in_unknown_cards: u64,
}

/// Résultats du héros, alimentés main par main.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroResults {
    hands: Vec<HandResult>,
    unresolved_hands: u64,
    all_in_unknown_cards: u64,
    hand_ids: BTreeSet<String>,
    /// Paramètres du calcul d'équité à l'all-in (exact dans la pratique : mains connues).
    pub equity: EquityOptions,
}

impl Default for HeroResults {
    fn default() -> Self {
        HeroResults::new(EquityOptions::default())
    }
}

/// Situation d'all-in d'une main.
enum AllIn {
    None,
    UnknownCards,
    Ev(AllInEv),
}

impl HeroResults {
    #[must_use]
    pub fn new(equity: EquityOptions) -> HeroResults {
        HeroResults {
            hands: Vec::new(),
            unresolved_hands: 0,
            all_in_unknown_cards: 0,
            hand_ids: BTreeSet::new(),
            equity,
        }
    }

    /// Ajoute une main du héros (désigné par `meta.hero_seat`).
    pub fn add_hand(&mut self, record: &HandRecord) -> Result<(), AnalyticsError> {
        if self.hand_ids.contains(&record.id) {
            return Err(AnalyticsError::DuplicateHand(record.id.clone()));
        }
        let hand = ParsedHand::parse(record, false)?;
        if hand.hero.is_none() {
            return Err(AnalyticsError::NoHero(record.id.clone()));
        }
        self.add_parsed(&hand);
        Ok(())
    }

    /// Vrai si la main a déjà été prise en compte.
    #[must_use]
    pub fn contains(&self, hand_id: &str) -> bool {
        self.hand_ids.contains(hand_id)
    }

    /// Mains au résultat connu, dans l'ordre d'ajout.
    #[must_use]
    pub fn hands(&self) -> &[HandResult] {
        &self.hands
    }

    pub(crate) fn add_parsed(&mut self, hand: &ParsedHand<'_>) {
        let Some(hero) = hand.hero else { return };
        self.hand_ids.insert(hand.record.id.clone());
        let Some(net) = hand.net(hero) else {
            self.unresolved_hands += 1;
            return;
        };
        let all_in = match all_in_ev(hand, hero, &self.equity) {
            AllIn::None => None,
            AllIn::UnknownCards => {
                self.all_in_unknown_cards += 1;
                None
            }
            AllIn::Ev(ev) => Some(ev),
        };
        self.hands.push(HandResult {
            hand_id: hand.record.id.clone(),
            played_at_ms: hand.record.meta.played_at_ms,
            format: hand.record.format,
            position: hand.position(hero),
            big_blind: hand.big_blind(),
            net_chips: net,
            all_in,
        });
    }

    /// Synthèse : totaux, détail par position et par format, courbe cumulée.
    #[must_use]
    pub fn summary(&self) -> ResultsSummary {
        let mut overall = ResultLine::default();
        let mut by_position: BTreeMap<Position, ResultLine> = BTreeMap::new();
        let mut by_format: Vec<FormatResults> = Vec::new();
        for r in &self.hands {
            overall.add(r);
            by_position.entry(r.position).or_default().add(r);
            let idx = match by_format.iter().position(|f| f.format == r.format) {
                Some(i) => i,
                None => {
                    by_format.push(FormatResults {
                        format: r.format,
                        overall: ResultLine::default(),
                        by_position: BTreeMap::new(),
                    });
                    by_format.len() - 1
                }
            };
            let f = &mut by_format[idx];
            f.overall.add(r);
            f.by_position.entry(r.position).or_default().add(r);
        }

        let mut order: Vec<usize> = (0..self.hands.len()).collect();
        order.sort_by_key(|i| {
            let t = self.hands[*i].played_at_ms;
            (t.is_none(), t, *i)
        });
        let mut curve = Vec::with_capacity(order.len());
        let (mut chips, mut bb, mut adjusted) = (0i64, 0.0f64, 0.0f64);
        for i in order {
            let r = &self.hands[i];
            chips += r.net_chips;
            bb += r.net_bb();
            adjusted += r.adjusted_net_bb();
            curve.push(CurvePoint {
                hand_id: r.hand_id.clone(),
                played_at_ms: r.played_at_ms,
                net_chips: r.net_chips,
                cumulative_chips: chips,
                cumulative_bb: bb,
                cumulative_adjusted_bb: adjusted,
            });
        }

        ResultsSummary {
            overall,
            by_position,
            by_format,
            curve,
            unresolved_hands: self.unresolved_hands,
            all_in_unknown_cards: self.all_in_unknown_cards,
        }
    }
}

/// Espérance du héros si les enchères se sont fermées avant la river.
fn all_in_ev(hand: &ParsedHand<'_>, hero: usize, options: &EquityOptions) -> AllIn {
    let state = &hand.state;
    if state.phase() != Phase::Showdown || hand.folded(hero) {
        return AllIn::None;
    }
    let (street, board_len) = hand
        .steps
        .last()
        .map_or((Street::Preflop, 0), |s| (s.street, s.board_len));
    if board_len >= 5 {
        return AllIn::None;
    }
    let in_hand: Vec<usize> = (0..hand.len()).filter(|p| !hand.folded(*p)).collect();
    let Some(holes) = in_hand
        .iter()
        .map(|p| hand.known[*p])
        .collect::<Option<Vec<HoleCards>>>()
    else {
        return AllIn::UnknownCards;
    };
    let hole_of = |p: usize| holes[in_hand.iter().position(|q| *q == p).expect("en jeu")];
    let board: CardSet = hand.board(board_len).iter().collect();
    let breakdown = build_pots(&state.contributions());

    let mut expected = 0.0;
    let mut pots = Vec::new();
    for pot in breakdown.pots.iter().filter(|p| p.eligible.contains(&hero)) {
        let eq = if pot.eligible.len() == 1 {
            1.0
        } else {
            let ranges: Vec<Range> = pot
                .eligible
                .iter()
                .map(|p| Range::single(hole_of(*p)))
                .collect();
            let dead: CardSet = in_hand
                .iter()
                .filter(|p| !pot.eligible.contains(p))
                .fold(CardSet::EMPTY, |acc, p| acc | hole_of(*p).set());
            let me = pot
                .eligible
                .iter()
                .position(|p| *p == hero)
                .expect("héros éligible");
            match equity(&ranges, board, dead, options) {
                Ok(r) => r.players[me].equity,
                Err(_) => return AllIn::UnknownCards,
            }
        };
        expected += eq * pot.amount.get() as f64;
        pots.push(PotEquity {
            amount: pot.amount,
            players: pot.eligible.len(),
            equity: eq,
        });
    }
    if let Some((p, amount)) = breakdown.uncalled
        && p == hero
    {
        expected += amount.get() as f64;
    }
    let invested = state.players()[hero].invested().get() as f64;
    AllIn::Ev(AllInEv {
        street,
        pots,
        expected_net: expected - invested,
    })
}
