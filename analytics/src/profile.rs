//! Profils de joueurs (adversaires et héros) et historique des confrontations.
//!
//! Un profil cumule, main après main, des statistiques de fréquence ([`Stat`]) : chacune
//! stocke ses **occurrences** et ses **opportunités**. Les profils sont tenus globalement,
//! **par position**, et **par format** (puis par position dans chaque format). Le calcul est
//! incrémental : [`ProfileBook::add_hand`] met à jour les profils sans rien recalculer, et
//! le carnet entier se sérialise pour reprendre plus tard.
//!
//! # Définitions retenues
//!
//! Elles suivent les usages des logiciels de suivi (PokerTracker, Hold'em Manager). Chaque
//! statistique est comptée **au plus une fois par main et par joueur**. Un tapis incomplet
//! compte comme une relance. « Première décision » : première action volontaire du joueur
//! au préflop (poster une blinde ou une ante n'est pas une décision).
//!
//! | Statistique | Opportunité | Occurrence |
//! |---|---|---|
//! | VPIP | le joueur a pris au moins une décision préflop (les « walks » de la BB sont exclus) | call ou relance préflop |
//! | PFR | idem | relance préflop |
//! | Limp | première décision, pot non relancé, montant à suivre non nul (la BB qui peut checker est exclue ; le complément de la SB compte) | call |
//! | Cold call | première décision face à au moins une relance, sans jeton déjà engagé sur le tour (blindes exclues) | call |
//! | 3-bet | face à exactement une relance, relance autorisée | relance |
//! | Fold au 3-bet | auteur de la première relance (l'ouverture), face à exactement deux relances | fold |
//! | 4-bet | face à exactement deux relances, relance autorisée | relance |
//! | Steal | CO, BTN ou SB, première décision, personne n'est entré dans le pot, relance autorisée | relance |
//! | Fold au steal | SB ou BB, première décision face à une tentative de steal, sans suiveur entre-temps | fold |
//! | Défense de la BB | BB, première décision face à une ouverture unique (une relance, aucun limp ni suiveur), quelle que soit la position de l'ouvreur | call ou relance |
//! | C-bet flop | dernier relanceur préflop, aucune mise avant lui au flop, mise autorisée | mise |
//! | C-bet turn | auteur d'un c-bet flop non relancé, aucune mise avant lui au turn | mise |
//! | Fold au c-bet flop / turn | face au c-bet, avant toute relance | fold |
//! | AFq | actions postflop : mises, relances, calls et folds (checks exclus) | mises et relances |
//! | WTSD | a vu le flop (encore en jeu à sa distribution) et la main est allée à son terme | a participé à l'abattage |
//! | W$SD | a participé à l'abattage d'une main résolue | a remporté au moins une part de pot (mise non suivie rendue exclue) |
//!
//! Le **facteur d'agression** AF = (mises + relances) / calls postflop n'est pas une
//! proportion : il est fourni à part ([`AggressionCounts::factor`]).
//!
//! Une main incomplète (vidéo coupée) ou non résolue (cartes adverses inconnues) alimente
//! les statistiques des décisions observées, jamais celles qui dépendent de la suite : WTSD
//! exige une main terminée, W$SD une main résolue.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use stratyx_poker::format::GameFormat;
use stratyx_poker::game::{ActionKind, Street};
use stratyx_poker::history::HandRecord;
use stratyx_poker::position::Position;

use crate::error::AnalyticsError;
use crate::hand::ParsedHand;
use crate::stat::{ReliabilityThresholds, Stat, StatSummary};

/// Statistiques de fréquence suivies dans un profil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatKind {
    Vpip,
    Pfr,
    Limp,
    ColdCall,
    ThreeBet,
    FoldToThreeBet,
    FourBet,
    Steal,
    FoldToSteal,
    BigBlindDefense,
    CbetFlop,
    CbetTurn,
    FoldToCbetFlop,
    FoldToCbetTurn,
    AggressionFrequency,
    WentToShowdown,
    WonAtShowdown,
}

impl StatKind {
    /// Nombre de statistiques.
    pub const COUNT: usize = 17;

    /// Toutes les statistiques, dans l'ordre d'affichage.
    pub const ALL: [StatKind; StatKind::COUNT] = [
        StatKind::Vpip,
        StatKind::Pfr,
        StatKind::Limp,
        StatKind::ColdCall,
        StatKind::ThreeBet,
        StatKind::FoldToThreeBet,
        StatKind::FourBet,
        StatKind::Steal,
        StatKind::FoldToSteal,
        StatKind::BigBlindDefense,
        StatKind::CbetFlop,
        StatKind::CbetTurn,
        StatKind::FoldToCbetFlop,
        StatKind::FoldToCbetTurn,
        StatKind::AggressionFrequency,
        StatKind::WentToShowdown,
        StatKind::WonAtShowdown,
    ];

    /// Abréviation usuelle.
    #[must_use]
    pub const fn abbreviation(self) -> &'static str {
        match self {
            StatKind::Vpip => "VPIP",
            StatKind::Pfr => "PFR",
            StatKind::Limp => "Limp",
            StatKind::ColdCall => "CC",
            StatKind::ThreeBet => "3B",
            StatKind::FoldToThreeBet => "F3B",
            StatKind::FourBet => "4B",
            StatKind::Steal => "ATS",
            StatKind::FoldToSteal => "FST",
            StatKind::BigBlindDefense => "BBD",
            StatKind::CbetFlop => "CB-F",
            StatKind::CbetTurn => "CB-T",
            StatKind::FoldToCbetFlop => "FCB-F",
            StatKind::FoldToCbetTurn => "FCB-T",
            StatKind::AggressionFrequency => "AFq",
            StatKind::WentToShowdown => "WTSD",
            StatKind::WonAtShowdown => "W$SD",
        }
    }

    /// Nom en français.
    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            StatKind::Vpip => "Mise volontaire préflop",
            StatKind::Pfr => "Relance préflop",
            StatKind::Limp => "Limp",
            StatKind::ColdCall => "Cold call",
            StatKind::ThreeBet => "3-bet",
            StatKind::FoldToThreeBet => "Fold au 3-bet",
            StatKind::FourBet => "4-bet",
            StatKind::Steal => "Tentative de vol des blindes",
            StatKind::FoldToSteal => "Fold au vol des blindes",
            StatKind::BigBlindDefense => "Défense de la grosse blinde",
            StatKind::CbetFlop => "C-bet au flop",
            StatKind::CbetTurn => "C-bet au turn",
            StatKind::FoldToCbetFlop => "Fold au c-bet flop",
            StatKind::FoldToCbetTurn => "Fold au c-bet turn",
            StatKind::AggressionFrequency => "Fréquence d'agression postflop",
            StatKind::WentToShowdown => "Abattage après le flop",
            StatKind::WonAtShowdown => "Gain à l'abattage",
        }
    }

    /// Statistique calculée à partir d'autres compteurs (et non relevée main par main).
    #[must_use]
    pub const fn is_derived(self) -> bool {
        matches!(self, StatKind::AggressionFrequency)
    }
}

/// Actions postflop d'un joueur, pour le facteur et la fréquence d'agression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AggressionCounts {
    pub bets: u64,
    pub raises: u64,
    pub calls: u64,
    pub checks: u64,
    pub folds: u64,
}

impl AggressionCounts {
    /// Facteur d'agression AF = (mises + relances) / calls ; `None` sans call.
    #[must_use]
    pub fn factor(&self) -> Option<f64> {
        (self.calls > 0).then(|| (self.bets + self.raises) as f64 / self.calls as f64)
    }

    /// Fréquence d'agression AFq = (mises + relances) / (mises + relances + calls + folds).
    #[must_use]
    pub fn frequency(&self) -> Stat {
        let aggressive = self.bets + self.raises;
        Stat::new(aggressive, aggressive + self.calls + self.folds)
    }

    pub fn merge(&mut self, other: &AggressionCounts) {
        self.bets += other.bets;
        self.raises += other.raises;
        self.calls += other.calls;
        self.checks += other.checks;
        self.folds += other.folds;
    }

    fn record(&mut self, kind: ActionKind) {
        match kind {
            ActionKind::Bet => self.bets += 1,
            ActionKind::Raise => self.raises += 1,
            ActionKind::Call => self.calls += 1,
            ActionKind::Check => self.checks += 1,
            ActionKind::Fold => self.folds += 1,
            ActionKind::PostAnte | ActionKind::PostSmallBlind | ActionKind::PostBigBlind => {}
        }
    }
}

/// Ensemble des statistiques d'un joueur sur un échantillon de mains.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatLine {
    /// Mains servies.
    pub hands: u64,
    pub vpip: Stat,
    pub pfr: Stat,
    pub limp: Stat,
    pub cold_call: Stat,
    pub three_bet: Stat,
    pub fold_to_three_bet: Stat,
    pub four_bet: Stat,
    pub steal: Stat,
    pub fold_to_steal: Stat,
    pub big_blind_defense: Stat,
    pub cbet_flop: Stat,
    pub cbet_turn: Stat,
    pub fold_to_cbet_flop: Stat,
    pub fold_to_cbet_turn: Stat,
    pub went_to_showdown: Stat,
    pub won_at_showdown: Stat,
    /// Actions postflop (AF, AFq).
    pub postflop: AggressionCounts,
}

impl StatLine {
    /// Compteurs d'une statistique.
    #[must_use]
    pub fn get(&self, kind: StatKind) -> Stat {
        match kind {
            StatKind::AggressionFrequency => self.postflop.frequency(),
            other => *self.counter(other),
        }
    }

    fn counter(&self, kind: StatKind) -> &Stat {
        match kind {
            StatKind::Vpip => &self.vpip,
            StatKind::Pfr => &self.pfr,
            StatKind::Limp => &self.limp,
            StatKind::ColdCall => &self.cold_call,
            StatKind::ThreeBet => &self.three_bet,
            StatKind::FoldToThreeBet => &self.fold_to_three_bet,
            StatKind::FourBet => &self.four_bet,
            StatKind::Steal => &self.steal,
            StatKind::FoldToSteal => &self.fold_to_steal,
            StatKind::BigBlindDefense => &self.big_blind_defense,
            StatKind::CbetFlop => &self.cbet_flop,
            StatKind::CbetTurn => &self.cbet_turn,
            StatKind::FoldToCbetFlop => &self.fold_to_cbet_flop,
            StatKind::FoldToCbetTurn => &self.fold_to_cbet_turn,
            StatKind::WentToShowdown => &self.went_to_showdown,
            StatKind::WonAtShowdown => &self.won_at_showdown,
            StatKind::AggressionFrequency => unreachable!("statistique dérivée"),
        }
    }

    fn counter_mut(&mut self, kind: StatKind) -> Option<&mut Stat> {
        Some(match kind {
            StatKind::Vpip => &mut self.vpip,
            StatKind::Pfr => &mut self.pfr,
            StatKind::Limp => &mut self.limp,
            StatKind::ColdCall => &mut self.cold_call,
            StatKind::ThreeBet => &mut self.three_bet,
            StatKind::FoldToThreeBet => &mut self.fold_to_three_bet,
            StatKind::FourBet => &mut self.four_bet,
            StatKind::Steal => &mut self.steal,
            StatKind::FoldToSteal => &mut self.fold_to_steal,
            StatKind::BigBlindDefense => &mut self.big_blind_defense,
            StatKind::CbetFlop => &mut self.cbet_flop,
            StatKind::CbetTurn => &mut self.cbet_turn,
            StatKind::FoldToCbetFlop => &mut self.fold_to_cbet_flop,
            StatKind::FoldToCbetTurn => &mut self.fold_to_cbet_turn,
            StatKind::WentToShowdown => &mut self.went_to_showdown,
            StatKind::WonAtShowdown => &mut self.won_at_showdown,
            StatKind::AggressionFrequency => return None,
        })
    }

    /// Facteur d'agression postflop.
    #[must_use]
    pub fn aggression_factor(&self) -> Option<f64> {
        self.postflop.factor()
    }

    /// Cumule un autre échantillon.
    pub fn merge(&mut self, other: &StatLine) {
        self.hands += other.hands;
        for kind in StatKind::ALL {
            let o = other.get(kind);
            if let Some(s) = self.counter_mut(kind) {
                s.merge(o);
            }
        }
        self.postflop.merge(&other.postflop);
    }

    fn record(&mut self, facts: &PlayerHandFacts) {
        self.hands += 1;
        for kind in StatKind::ALL {
            let f = facts.flags[kind as usize];
            if let Some(s) = self.counter_mut(kind) {
                s.record(f.opportunity, f.occurred);
            }
        }
        self.postflop.merge(&facts.postflop);
    }

    /// Vue calculée (fréquences, intervalles de Wilson, fiabilité).
    #[must_use]
    pub fn report(&self, thresholds: &ReliabilityThresholds) -> StatLineReport {
        StatLineReport {
            hands: self.hands,
            stats: StatKind::ALL
                .iter()
                .map(|k| (*k, self.get(*k).summary(thresholds)))
                .collect(),
            aggression_factor: self.aggression_factor(),
        }
    }
}

/// Statistiques globales et par position.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PositionalStats {
    pub overall: StatLine,
    pub by_position: BTreeMap<Position, StatLine>,
}

impl PositionalStats {
    fn record(&mut self, facts: &PlayerHandFacts) {
        self.overall.record(facts);
        self.by_position
            .entry(facts.position)
            .or_default()
            .record(facts);
    }

    /// Statistiques d'une position (ou globales si `position` vaut `None`).
    #[must_use]
    pub fn line(&self, position: Option<Position>) -> Option<&StatLine> {
        match position {
            None => Some(&self.overall),
            Some(p) => self.by_position.get(&p),
        }
    }

    #[must_use]
    pub fn report(&self, thresholds: &ReliabilityThresholds) -> PositionalReport {
        PositionalReport {
            overall: self.overall.report(thresholds),
            by_position: self
                .by_position
                .iter()
                .map(|(p, l)| (*p, l.report(thresholds)))
                .collect(),
        }
    }
}

/// Statistiques d'un joueur pour un format donné.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatStats {
    pub format: GameFormat,
    pub stats: PositionalStats,
}

/// Profil d'un joueur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProfile {
    /// Pseudonyme à la table.
    pub player: String,
    /// Tous formats confondus.
    pub stats: PositionalStats,
    /// Détail par format (cash game 6-max, Expresso…).
    pub by_format: Vec<FormatStats>,
}

impl PlayerProfile {
    #[must_use]
    pub fn new(player: impl Into<String>) -> PlayerProfile {
        PlayerProfile {
            player: player.into(),
            stats: PositionalStats::default(),
            by_format: Vec::new(),
        }
    }

    /// Mains servies.
    #[must_use]
    pub fn hands(&self) -> u64 {
        self.stats.overall.hands
    }

    /// Statistiques d'un format, si le joueur y a été observé.
    #[must_use]
    pub fn format(&self, format: &GameFormat) -> Option<&PositionalStats> {
        self.by_format
            .iter()
            .find(|f| f.format == *format)
            .map(|f| &f.stats)
    }

    fn record(&mut self, format: GameFormat, facts: &PlayerHandFacts) {
        self.stats.record(facts);
        let idx = match self.by_format.iter().position(|f| f.format == format) {
            Some(i) => i,
            None => {
                self.by_format.push(FormatStats {
                    format,
                    stats: PositionalStats::default(),
                });
                self.by_format.len() - 1
            }
        };
        self.by_format[idx].stats.record(facts);
    }

    #[must_use]
    pub fn report(&self, thresholds: &ReliabilityThresholds) -> ProfileReport {
        ProfileReport {
            player: self.player.clone(),
            hands: self.hands(),
            stats: self.stats.report(thresholds),
            by_format: self
                .by_format
                .iter()
                .map(|f| FormatReport {
                    format: f.format,
                    stats: f.stats.report(thresholds),
                })
                .collect(),
        }
    }
}

/// Vue calculée d'un [`StatLine`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatLineReport {
    pub hands: u64,
    pub stats: BTreeMap<StatKind, StatSummary>,
    pub aggression_factor: Option<f64>,
}

/// Vue calculée d'un [`PositionalStats`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionalReport {
    pub overall: StatLineReport,
    pub by_position: BTreeMap<Position, StatLineReport>,
}

/// Vue calculée d'un format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatReport {
    pub format: GameFormat,
    pub stats: PositionalReport,
}

/// Vue calculée d'un profil, destinée à l'interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileReport {
    pub player: String,
    pub hands: u64,
    pub stats: PositionalReport,
    pub by_format: Vec<FormatReport>,
}

/// Historique des confrontations entre le héros (`meta.hero_seat` de chaque main) et un
/// adversaire.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HeadToHead {
    pub opponent: String,
    /// Mains où les deux joueurs étaient servis.
    pub hands_together: u64,
    /// Mains où les deux joueurs ont vu le flop.
    pub flops_together: u64,
    /// Mains où les deux joueurs ont participé à l'abattage.
    pub showdowns_together: u64,
    /// Mains résolues, sur lesquelles le résultat net est calculé.
    pub settled_hands: u64,
    /// Jetons gagnés par le héros aux dépens de cet adversaire (négatif : perdus). Chaque
    /// pot est réparti entre ses gagnants au prorata des mises de chaque contributeur.
    pub net_chips: i64,
    /// Même résultat en grosses blindes de chaque main.
    pub net_bb: f64,
}

/// Carnet des profils, alimenté main par main.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfileBook {
    profiles: BTreeMap<String, PlayerProfile>,
    head_to_head: BTreeMap<String, HeadToHead>,
    hand_ids: BTreeSet<String>,
}

impl ProfileBook {
    #[must_use]
    pub fn new() -> ProfileBook {
        ProfileBook::default()
    }

    /// Ajoute une main : elle est validée par rejeu, puis tous les profils des joueurs servis
    /// et les confrontations du héros sont mis à jour.
    pub fn add_hand(&mut self, record: &HandRecord) -> Result<(), AnalyticsError> {
        if self.contains(&record.id) {
            return Err(AnalyticsError::DuplicateHand(record.id.clone()));
        }
        let hand = ParsedHand::parse(record, false)?;
        self.add_parsed(&hand);
        Ok(())
    }

    /// Vrai si la main a déjà été prise en compte.
    #[must_use]
    pub fn contains(&self, hand_id: &str) -> bool {
        self.hand_ids.contains(hand_id)
    }

    /// Nombre de mains prises en compte.
    #[must_use]
    pub fn hand_count(&self) -> usize {
        self.hand_ids.len()
    }

    pub(crate) fn add_parsed(&mut self, hand: &ParsedHand<'_>) {
        let facts = hand_facts(hand);
        let format = hand.record.format;
        for (p, f) in facts.iter().enumerate() {
            let name = hand.names[p];
            self.profiles
                .entry(name.to_string())
                .or_insert_with(|| PlayerProfile::new(name))
                .record(format, f);
        }
        if let Some(h) = hand.hero {
            let bb = hand.big_blind();
            for v in (0..hand.len()).filter(|v| *v != h) {
                let e = self
                    .head_to_head
                    .entry(hand.names[v].to_string())
                    .or_insert_with(|| HeadToHead {
                        opponent: hand.names[v].to_string(),
                        ..HeadToHead::default()
                    });
                e.hands_together += 1;
                e.flops_together += u64::from(hand.saw_flop(h) && hand.saw_flop(v));
                e.showdowns_together +=
                    u64::from(hand.went_to_showdown(h) && hand.went_to_showdown(v));
                if let Some(t) = hand.transfer(v, h) {
                    e.settled_hands += 1;
                    e.net_chips += t;
                    e.net_bb += t as f64 / bb.get() as f64;
                }
            }
        }
        self.hand_ids.insert(hand.record.id.clone());
    }

    /// Profil d'un joueur.
    #[must_use]
    pub fn profile(&self, player: &str) -> Option<&PlayerProfile> {
        self.profiles.get(player)
    }

    /// Tous les profils, par ordre alphabétique.
    pub fn profiles(&self) -> impl Iterator<Item = &PlayerProfile> {
        self.profiles.values()
    }

    /// Confrontations du héros avec un adversaire.
    #[must_use]
    pub fn head_to_head(&self, opponent: &str) -> Option<&HeadToHead> {
        self.head_to_head.get(opponent)
    }

    /// Toutes les confrontations du héros.
    pub fn head_to_heads(&self) -> impl Iterator<Item = &HeadToHead> {
        self.head_to_head.values()
    }
}

/// Opportunité et occurrence d'une statistique pour une main.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Flag {
    opportunity: bool,
    occurred: bool,
}

/// Ce qu'une main apporte au profil d'un joueur.
#[derive(Debug, Clone)]
pub(crate) struct PlayerHandFacts {
    position: Position,
    flags: [Flag; StatKind::COUNT],
    postflop: AggressionCounts,
}

impl PlayerHandFacts {
    fn new(position: Position) -> PlayerHandFacts {
        PlayerHandFacts {
            position,
            flags: [Flag::default(); StatKind::COUNT],
            postflop: AggressionCounts::default(),
        }
    }

    fn mark(&mut self, kind: StatKind, occurred: bool) {
        let f = &mut self.flags[kind as usize];
        f.opportunity = true;
        f.occurred |= occurred;
    }

    fn occurred(&self, kind: StatKind) -> bool {
        self.flags[kind as usize].occurred
    }
}

/// Relevé des opportunités et occurrences de chaque joueur pour une main.
pub(crate) fn hand_facts(hand: &ParsedHand<'_>) -> Vec<PlayerHandFacts> {
    use ActionKind::{Bet, Call, Fold, Raise};
    let n = hand.len();
    let mut facts: Vec<PlayerHandFacts> = (0..n)
        .map(|p| PlayerHandFacts::new(hand.position(p)))
        .collect();

    // Préflop.
    let mut acted = vec![false; n];
    let mut raises = 0usize;
    let mut limpers = 0usize;
    let mut callers_since_raise = 0usize;
    let mut opener: Option<usize> = None;
    let mut steal_attempt = false;
    let mut preflop_aggressor: Option<usize> = None;
    // Postflop : auteur du c-bet au flop et au turn.
    let mut cbettor: [Option<usize>; 2] = [None, None];
    let mut flop_cbet_raised = false;

    for step in &hand.steps {
        let p = step.player;
        let kind = step.applied.kind;
        let f = &mut facts[p];
        if step.street == Street::Preflop {
            let first = !acted[p];
            let (call, raise, fold) = (kind == Call, kind == Raise, kind == Fold);
            f.mark(StatKind::Vpip, call || raise);
            f.mark(StatKind::Pfr, raise);
            if first && raises == 0 && !step.to_call.is_zero() {
                f.mark(StatKind::Limp, call);
            }
            if first && raises > 0 && step.street_bet.is_zero() {
                f.mark(StatKind::ColdCall, call);
            }
            let steal_seat = matches!(
                step.position,
                Position::Cutoff | Position::Button | Position::SmallBlind
            );
            if first && raises == 0 && limpers == 0 && steal_seat && step.can_raise {
                f.mark(StatKind::Steal, raise);
            }
            if first && raises == 1 && limpers == 0 && callers_since_raise == 0 {
                if step.position.is_blind() && steal_attempt {
                    f.mark(StatKind::FoldToSteal, fold);
                }
                if step.position == Position::BigBlind {
                    f.mark(StatKind::BigBlindDefense, call || raise);
                }
            }
            if raises == 1 && step.can_raise {
                f.mark(StatKind::ThreeBet, raise);
            }
            if raises == 2 && opener == Some(p) {
                f.mark(StatKind::FoldToThreeBet, fold);
            }
            if raises == 2 && step.can_raise {
                f.mark(StatKind::FourBet, raise);
            }

            acted[p] = true;
            if raise {
                raises += 1;
                if raises == 1 {
                    opener = Some(p);
                    steal_attempt = f.occurred(StatKind::Steal);
                }
                callers_since_raise = 0;
                preflop_aggressor = Some(p);
            } else if call {
                if raises == 0 {
                    limpers += 1;
                } else {
                    callers_since_raise += 1;
                }
            }
            continue;
        }

        f.postflop.record(kind);
        let (i, cbet, fold_to_cbet) = match step.street {
            Street::Flop => (0, StatKind::CbetFlop, StatKind::FoldToCbetFlop),
            Street::Turn => (1, StatKind::CbetTurn, StatKind::FoldToCbetTurn),
            _ => continue,
        };
        let candidate = if i == 0 {
            preflop_aggressor
        } else {
            cbettor[0].filter(|_| !flop_cbet_raised)
        };
        if candidate == Some(p) && step.aggressive_before == 0 && step.can_bet {
            f.mark(cbet, kind == Bet);
            if kind == Bet {
                cbettor[i] = Some(p);
            }
        }
        if let Some(c) = cbettor[i] {
            if c != p && step.aggressive_before == 1 {
                f.mark(fold_to_cbet, kind == Fold);
            }
            if i == 0 && step.aggressive_before >= 1 && kind.is_aggressive() {
                flop_cbet_raised = true;
            }
        }
    }

    // Abattage.
    let finished = hand.finished();
    for (p, f) in facts.iter_mut().enumerate() {
        let showdown = hand.went_to_showdown(p);
        if hand.saw_flop(p) && finished {
            f.mark(StatKind::WentToShowdown, showdown);
        }
        if showdown {
            if let Some(won) = hand.pot_winnings(p) {
                f.mark(StatKind::WonAtShowdown, !won.is_zero());
            }
        }
    }
    facts
}
