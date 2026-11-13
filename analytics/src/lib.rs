//! Analyse d'une main après la partie : chaque décision du joueur est comparée à la
//! décision de référence, **indépendamment du résultat** (cahier des charges, section 6).
//!
//! Pour une Expresso 3-Max ou un tournoi 6-Max, l'IA spécialisée fournit son équilibre
//! push/fold, son modèle de décision et son modèle de population (`ai/expresso_3max`,
//! `ai/tournament_6max`) ; les autres formats sont analysés par le moteur commun (référence
//! EV : équité, cotes, fold equity), sans IA dédiée (décision D8).
//!
//! L'analyse ne sert qu'après la partie : elle prend un historique complet (D6).

use serde::Serialize;
use stratyx_poker::game::{Action, ActionKind, HandSetup, PlayerStatus};
use stratyx_poker::history::{HandEvent, HandRecord, Outcome};
use stratyx_poker::position::Position;
use stratyx_poker::{Card, HandState, HoleCards, Specialist, Street};
use stratyx_strategy::model::Classifier;
use stratyx_strategy::pushfold::Chart;
use stratyx_strategy::{
    Basis, Decision, HandFeatures, Inputs, ModelBundle, Spot, classify, decision_features, mc,
    reference,
};
use thiserror::Error;

pub mod session;
pub mod spot;

pub use stratyx_poker::import::{ImportReport, ImportedHand, import_text};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnalysisError {
    #[error("historique invalide : {0}")]
    Invalid(String),
    #[error("les cartes du joueur analysé sont inconnues : rien à comparer")]
    NoHero,
    #[error("joueur « {0} » absent de la main")]
    UnknownPlayer(String),
}

/// Gravité d'un écart à la référence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// La décision jouée est celle de la référence.
    Good,
    /// Écart faible (moins de 0,5 BB).
    Inaccuracy,
    /// Écart net (0,5 à 3 BB).
    Mistake,
    /// Écart lourd (plus de 3 BB).
    Blunder,
}

impl Severity {
    #[must_use]
    pub fn from_loss(loss_bb: f32, same: bool) -> Severity {
        if same || loss_bb < 0.1 {
            Severity::Good
        } else if loss_bb < 0.5 {
            Severity::Inaccuracy
        } else if loss_bb < 3.0 {
            Severity::Mistake
        } else {
            Severity::Blunder
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Severity::Good => "Bon choix",
            Severity::Inaccuracy => "Imprécision",
            Severity::Mistake => "Erreur",
            Severity::Blunder => "Grosse erreur",
        }
    }
}

/// Une ligne de la main, pour la revoir.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub street: Street,
    /// Nom du joueur (vide pour une carte du board).
    pub player: String,
    pub position: String,
    pub hero: bool,
    /// « relance à 3 BB », « Flop : Qs 7h 2c »…
    pub text: String,
    /// Pot après la ligne, en grosses blindes.
    pub pot_bb: f32,
    /// Index de la décision analysée correspondante, s'il y en a une.
    pub decision: Option<usize>,
}

/// L'analyse d'une décision du joueur.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionReview {
    pub street: Street,
    pub board: Vec<String>,
    pub position: String,
    pub stack_bb: f32,
    pub effective_bb: f32,
    pub pot_bb: f32,
    pub to_call_bb: f32,
    /// Équité nécessaire pour suivre.
    pub pot_odds: f32,
    /// Le joueur fait face à une mise ou une relance volontaire (pas seulement aux blindes).
    pub facing_bet: bool,
    /// Équité contre les ranges estimées des adversaires.
    pub equity: f32,
    pub played: Decision,
    pub played_text: String,
    pub best: Decision,
    pub best_text: String,
    pub basis: Basis,
    /// EV de chaque décision (`None` : impossible), en grosses blindes.
    pub ev: [Option<f32>; 4],
    /// EV perdue par rapport à la meilleure décision.
    pub loss_bb: f32,
    pub severity: Severity,
    /// Probabilité d'équilibre de l'action agressive (push/fold).
    pub equilibrium: Option<f32>,
    pub fold_equity: f32,
    /// Prédiction du modèle de décision de l'IA : décision et confiance.
    pub model: Option<(Decision, f32)>,
    /// Ce que joue la population dans ce spot (probabilités des quatre décisions).
    pub population: Option<[f32; 4]>,
    /// Part des mains dans la range estimée de chaque adversaire (nom, part).
    pub villains: Vec<(String, f32)>,
    pub comment: String,
}

/// Le bilan de la main.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// Note sur 100 (100 : toutes les décisions sont celles de la référence).
    pub score: u8,
    pub decisions: usize,
    pub mistakes: usize,
    pub ev_lost_bb: f32,
    pub verdict: String,
    /// Les pistes de travail tirées de la main.
    pub tips: Vec<String>,
}

/// L'analyse d'une main.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandAnalysis {
    pub id: String,
    pub format: String,
    /// L'IA utilisée (« IA Expresso 3-Max »…) ou « moteur commun ».
    pub engine: String,
    pub hero: String,
    pub hero_cards: String,
    pub position: String,
    pub players: usize,
    pub big_blind: u64,
    pub stack_bb: f32,
    pub board: Vec<String>,
    pub lines: Vec<Line>,
    pub decisions: Vec<DecisionReview>,
    /// Résultat net du joueur, en grosses blindes (`None` : main non résolue).
    pub result_bb: Option<f32>,
    pub outcome: String,
    pub summary: Summary,
}

/// Réglages de l'analyse.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Tirages Monte-Carlo par calcul d'équité.
    pub samples: usize,
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            samples: 2500,
            seed: 0x5354_5241_5459_5821,
        }
    }
}

/// Les modèles de l'IA compétente pour un format.
#[must_use]
pub fn bundle_for(specialist: Option<Specialist>) -> Option<&'static ModelBundle> {
    match specialist? {
        Specialist::Expresso3Max => Some(stratyx_ai_expresso::models()),
        Specialist::Tournament6Max => Some(stratyx_ai_tournament::models()),
    }
}

fn engine_name(specialist: Option<Specialist>) -> &'static str {
    match specialist {
        Some(Specialist::Expresso3Max) => "IA Expresso 3-Max",
        Some(Specialist::Tournament6Max) => "IA Tournois 6-Max",
        None => "moteur commun",
    }
}

/// Montant en grosses blindes, à la française : « 2,5 BB ».
#[must_use]
pub fn bb(x: f32) -> String {
    let s = if (x - x.round()).abs() < 0.05 {
        format!("{x:.0}")
    } else {
        format!("{x:.1}")
    };
    format!("{} BB", s.replace('.', ","))
}

/// Pourcentage à la française : « 42,5 % ».
#[must_use]
pub fn pct(x: f32) -> String {
    format!("{:.1} %", x * 100.0).replace('.', ",")
}

fn cards_text(cards: &[Card]) -> Vec<String> {
    cards.iter().map(ToString::to_string).collect()
}

/// Le texte d'une action appliquée : « relance à 3 BB ».
fn action_text(kind: ActionKind, to_bb: f32, amount_bb: f32, all_in: bool) -> String {
    let base = match kind {
        ActionKind::Fold => "se couche".to_owned(),
        ActionKind::Check => "parle".to_owned(),
        ActionKind::Call => format!("suit {}", bb(amount_bb)),
        ActionKind::Bet => format!("mise {}", bb(to_bb)),
        ActionKind::Raise => format!("relance à {}", bb(to_bb)),
        ActionKind::PostAnte => format!("ante {}", bb(amount_bb)),
        ActionKind::PostSmallBlind => format!("petite blinde {}", bb(amount_bb)),
        ActionKind::PostBigBlind => format!("grosse blinde {}", bb(amount_bb)),
    };
    if all_in && kind.is_voluntary() && kind != ActionKind::Fold {
        format!("{base} (tapis)")
    } else {
        base
    }
}

fn decision_text(d: Decision, facing: bool, raise_to_bb: Option<f32>) -> String {
    match (d, raise_to_bb) {
        (Decision::Raise, Some(to)) => format!("{} à {}", d.label(facing), bb(to)),
        _ => d.label(facing).to_owned(),
    }
}

fn street_name(s: Street) -> &'static str {
    match s {
        Street::Preflop => "Préflop",
        Street::Flop => "Flop",
        Street::Turn => "Turn",
        Street::River => "River",
    }
}

/// Le commentaire d'une décision, en français.
fn comment(r: &DecisionReview, hole: HoleCards, engine: &str) -> String {
    let class = hole.class();
    let mut s = String::new();
    let facing = r.to_call_bb > 0.0;
    match r.basis {
        Basis::PushFold => {
            let p = r.equilibrium.unwrap_or(0.0);
            let calling = r.ev[Decision::AllIn.index()].is_none();
            s.push_str(&format!(
                "À {} effectifs en {}, l'équilibre push/fold de l'{} {} {} {} du temps.",
                bb(r.effective_bb),
                r.position,
                engine,
                if calling {
                    "suit le tapis avec"
                } else {
                    "pousse"
                },
                class,
                pct(p)
            ));
        }
        Basis::Ev => {
            s.push_str(&format!(
                "Équité estimée {} contre la range adverse",
                pct(r.equity)
            ));
            if r.facing_bet {
                s.push_str(&format!(", il en faut {} pour suivre", pct(r.pot_odds)));
            }
            s.push('.');
        }
    }
    s.push(' ');
    if r.severity == Severity::Good {
        s.push_str(&format!(
            "{} : c'est la décision de référence.",
            capitalize(&r.played_text)
        ));
    } else {
        let gain = r.loss_bb;
        let why = match (r.played, r.best) {
            (Decision::Passive, Decision::Fold) if facing => {
                format!("Suivre ne rapporte pas assez ({} d'équité pour {} requis) : se coucher économise environ {}.", pct(r.equity), pct(r.pot_odds), bb(gain))
            }
            (Decision::Fold, Decision::Passive) => {
                format!("La cote du pot justifiait de suivre ({} d'équité pour {} requis) : environ {} laissés sur la table.", pct(r.equity), pct(r.pot_odds), bb(gain))
            }
            (Decision::Passive | Decision::Fold, Decision::Raise | Decision::AllIn) => {
                format!("Plus d'agressivité était rentable ({}) : environ {} de mieux, fold equity estimée {}.", r.best_text, bb(gain), pct(r.fold_equity))
            }
            (Decision::Raise | Decision::AllIn, Decision::Fold) => {
                format!("Cette agression coûte environ {} : la référence se couche.", bb(gain))
            }
            (Decision::Raise | Decision::AllIn, Decision::Passive) => {
                format!("La référence préfère {} : la relance perd environ {} (les mains qui continuent vous dominent).", r.best_text, bb(gain))
            }
            (Decision::Raise, Decision::AllIn) => {
                "Question de taille : la référence préfère faire tapis directement, mais l'écart dépend beaucoup des mains adverses qui continuent.".to_owned()
            }
            (Decision::AllIn, Decision::Raise) => {
                format!("Question de taille : une relance plus petite ({}) obtient presque autant pour moins de risque.", r.best_text)
            }
            _ => format!("La référence préfère {} (environ {} de mieux).", r.best_text, bb(gain)),
        };
        s.push_str(&format!("{} — {}", r.severity.label(), why));
    }
    if let Some((d, conf)) = r.model {
        let agrees = d == r.best;
        s.push_str(&format!(
            " Modèle de décision : {} ({}){}.",
            d.label(facing),
            pct(conf),
            if agrees {
                ", d'accord avec la référence"
            } else {
                ", avis différent de la référence"
            }
        ));
    }
    s
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Analyse la main pour `hero` (par défaut : le joueur dont les cartes sont connues dès la
/// distribution).
pub fn analyse(
    record: &HandRecord,
    hero: Option<&str>,
    options: &Options,
) -> Result<HandAnalysis, AnalysisError> {
    let replay = record
        .replay()
        .map_err(|e| AnalysisError::Invalid(e.to_string()))?;
    let mut order: Vec<_> = record.seats.iter().collect();
    order.sort_by_key(|s| s.seat);
    let n = order.len();
    let hero_idx = match hero {
        Some(name) => order
            .iter()
            .position(|s| s.player == name)
            .ok_or_else(|| AnalysisError::UnknownPlayer(name.to_owned()))?,
        None => order
            .iter()
            .position(|s| s.hole_cards.is_some())
            .ok_or(AnalysisError::NoHero)?,
    };
    let hole = order[hero_idx]
        .hole_cards
        .as_ref()
        .map(|h| h.value)
        .or_else(|| {
            record.events.iter().find_map(|e| match &e.value {
                HandEvent::Show { seat, cards } if *seat == order[hero_idx].seat => Some(*cards),
                _ => None,
            })
        })
        .ok_or(AnalysisError::NoHero)?;

    let specialist = record.format.specialist();
    let models = bundle_for(specialist);
    let engine = engine_name(specialist);
    let chart: Option<&Chart> = models.map(|m| &m.pushfold);
    let population: Option<&Classifier> = models.map(|m| &m.population);

    let button = order
        .iter()
        .position(|s| s.seat == record.button_seat)
        .ok_or_else(|| AnalysisError::Invalid("bouton introuvable".into()))?;
    let setup = HandSetup::new(
        order.iter().map(|s| s.stack.value).collect(),
        button,
        record.blinds.value,
    )
    .with_structure(record.format.structure);
    let mut state = HandState::new(&setup).map_err(|e| AnalysisError::Invalid(e.to_string()))?;
    let big = record.blinds.value.big_blind.get().max(1);
    let to_bb = |c: u64| c as f32 / big as f32;
    let position = |i: usize| {
        Position::from_button_offset((i + n - button) % n, n)
            .map_or_else(|| "?".to_owned(), |p| p.abbreviation().to_owned())
    };

    let mut lines = Vec::new();
    let mut decisions = Vec::new();
    for (k, ev) in record.events.iter().enumerate() {
        match &ev.value {
            HandEvent::Action { seat, action } => {
                let Some(p) = order.iter().position(|s| s.seat == *seat) else {
                    continue;
                };
                let mut decision_index = None;
                if p == hero_idx && state.to_act() == Some(p) {
                    let review = review(
                        &state,
                        p,
                        hole,
                        *action,
                        chart,
                        population,
                        models,
                        &order.iter().map(|s| s.player.clone()).collect::<Vec<_>>(),
                        &position(p),
                        engine,
                        options,
                        k as u64,
                    );
                    if let Some(r) = review {
                        decision_index = Some(decisions.len());
                        decisions.push(r);
                    }
                }
                if state.apply(p, *action).is_err() {
                    break;
                }
                if let Some(a) = state.actions().last() {
                    lines.push(Line {
                        street: a.street,
                        player: order[p].player.clone(),
                        position: position(p),
                        hero: p == hero_idx,
                        text: action_text(
                            a.kind,
                            to_bb(a.to.get()),
                            to_bb(a.amount.get()),
                            a.all_in,
                        ),
                        pot_bb: to_bb(state.pot_total().get()),
                        decision: decision_index,
                    });
                }
            }
            HandEvent::Board { cards } => {
                if state.deal(cards).is_err() {
                    break;
                }
                lines.push(Line {
                    street: state.street(),
                    player: String::new(),
                    position: String::new(),
                    hero: false,
                    text: format!(
                        "{} : {}",
                        street_name(state.street()),
                        cards_text(cards).join(" ")
                    ),
                    pot_bb: to_bb(state.pot_total().get()),
                    decision: None,
                });
            }
            HandEvent::Show { seat, cards } => {
                if let Some(p) = order.iter().position(|s| s.seat == *seat) {
                    lines.push(Line {
                        street: state.street(),
                        player: order[p].player.clone(),
                        position: position(p),
                        hero: p == hero_idx,
                        text: format!("montre {cards}"),
                        pot_bb: to_bb(state.pot_total().get()),
                        decision: None,
                    });
                }
            }
            HandEvent::Muck { .. } => {}
        }
    }

    let (result_bb, outcome) = match &replay.outcome {
        Outcome::Settled(s) => {
            let net = s.net[hero_idx] as f32 / big as f32;
            let text = if net > 0.0 {
                format!(
                    "{} gagne {} (le résultat ne juge pas les décisions)",
                    order[hero_idx].player,
                    bb(net)
                )
            } else if net < 0.0 {
                format!(
                    "{} perd {} (le résultat ne juge pas les décisions)",
                    order[hero_idx].player,
                    bb(-net)
                )
            } else {
                "Résultat nul".to_owned()
            };
            (Some(net), text)
        }
        Outcome::Unresolved { reason } => (None, format!("Gagnant non déterminé : {reason}")),
        Outcome::Incomplete => (None, "Main incomplète".to_owned()),
    };
    let summary = summarize(&decisions);
    Ok(HandAnalysis {
        id: record.id.clone(),
        format: record.format.to_string(),
        engine: engine.to_owned(),
        hero: order[hero_idx].player.clone(),
        hero_cards: hole.to_string(),
        position: position(hero_idx),
        players: n,
        big_blind: big,
        stack_bb: to_bb(order[hero_idx].stack.value.get()),
        board: cards_text(replay.state.board()),
        lines,
        decisions,
        result_bb,
        outcome,
        summary,
    })
}

#[allow(clippy::too_many_arguments)]
fn review(
    state: &HandState,
    p: usize,
    hole: HoleCards,
    action: Action,
    chart: Option<&Chart>,
    population: Option<&Classifier>,
    models: Option<&ModelBundle>,
    names: &[String],
    position: &str,
    engine: &str,
    options: &Options,
    k: u64,
) -> Option<DecisionReview> {
    let seed = options.seed ^ (k << 32);
    let inputs = Inputs {
        chart,
        population,
        samples: options.samples,
        seed,
    };
    let actual_to = match action {
        Action::Bet(c) | Action::Raise(c) => Some(c.get()),
        _ => None,
    };
    let refr = reference(state, hole, &inputs, actual_to)?;
    let spot = Spot::from_state(state, p);
    let played = classify(state, action);
    let big = state.blinds().big_blind.get().max(1) as f32;
    let facing = spot.to_call_bb > 0.0;
    let model = models.map(|m| {
        let opponents = state
            .players()
            .iter()
            .enumerate()
            .filter(|(i, s)| *i != p && s.status != PlayerStatus::Folded)
            .count()
            .max(1);
        let eq = if state.street() == Street::Preflop {
            0.0
        } else {
            mc::equity(
                hole,
                &vec![stratyx_poker::Range::empty(); opponents],
                state.board(),
                400,
                seed ^ 7,
            )
        };
        let hf = HandFeatures::new(hole, state.board(), opponents, eq);
        let x = decision_features(&spot, &hf);
        let (i, conf) = m.decision.best(&x);
        (Decision::from_index(i), conf)
    });
    let population = population.map(|m| {
        let v = m.predict(&spot.context_features());
        [v[0], v[1], v[2], v[3]]
    });
    let played_to = actual_to.map(|t| t as f32 / big);
    let best_to = refr.raise_to.map(|t| t as f32 / big);
    let loss = refr.loss(played);
    let same = played == refr.best;
    // Relance contre tapis (ou l'inverse) : une question de taille, que la référence EV
    // simplifiée tranche moins sûrement. L'écart est signalé sans être une erreur.
    let sizing = matches!(played, Decision::Raise | Decision::AllIn)
        && matches!(refr.best, Decision::Raise | Decision::AllIn)
        && refr.basis == Basis::Ev;
    let severity = if sizing {
        Severity::from_loss(loss, same).min(Severity::Inaccuracy)
    } else {
        Severity::from_loss(loss, same)
    };
    let mut r = DecisionReview {
        street: state.street(),
        board: cards_text(state.board()),
        position: position.to_owned(),
        stack_bb: spot.stack_bb,
        effective_bb: spot.effective_bb,
        pot_bb: spot.pot_bb,
        to_call_bb: spot.to_call_bb,
        pot_odds: refr.pot_odds,
        facing_bet: spot.raises > 0 || (spot.street != Street::Preflop && spot.to_call_bb > 0.0),
        equity: refr.equity,
        played,
        played_text: decision_text(played, facing, played_to),
        best: refr.best,
        best_text: decision_text(refr.best, facing, best_to),
        basis: refr.basis,
        ev: refr.ev,
        loss_bb: if same {
            0.0
        } else if sizing {
            loss.min(0.49)
        } else {
            loss
        },
        severity,
        equilibrium: refr.equilibrium,
        fold_equity: refr.fold_equity,
        model,
        population,
        villains: refr
            .villain_ranges
            .iter()
            .map(|(v, f)| (names.get(*v).cloned().unwrap_or_default(), *f))
            .collect(),
        comment: String::new(),
    };
    r.comment = comment(&r, hole, engine);
    Some(r)
}

/// Le bilan et les pistes de travail.
fn summarize(decisions: &[DecisionReview]) -> Summary {
    let ev_lost: f32 = decisions.iter().map(|d| d.loss_bb).sum();
    let mistakes = decisions
        .iter()
        .filter(|d| d.severity >= Severity::Mistake)
        .count();
    // Note : 100 moins la pénalité moyenne par décision, moins l'EV perdue (plafonnée).
    let n = decisions.len().max(1) as f32;
    let penalty: f32 = decisions
        .iter()
        .map(|d| match d.severity {
            Severity::Good => 0.0,
            Severity::Inaccuracy => 8.0,
            Severity::Mistake => 25.0,
            Severity::Blunder => 45.0,
        })
        .sum();
    let score = (100.0 - penalty / n - ev_lost.min(20.0))
        .clamp(0.0, 100.0)
        .round() as u8;
    let mut tips = Vec::new();
    let count = |f: &dyn Fn(&DecisionReview) -> bool| decisions.iter().filter(|d| f(d)).count();
    if count(&|d| {
        d.severity >= Severity::Inaccuracy
            && d.played == Decision::Passive
            && d.best == Decision::Fold
            && d.to_call_bb > 0.0
    }) > 0
    {
        tips.push("Calls trop légers : comparez votre équité à la cote du pot avant de suivre, surtout face à une relance ou un tapis.".to_owned());
    }
    if count(&|d| {
        d.severity >= Severity::Inaccuracy && d.played == Decision::Fold && d.best != Decision::Fold
    }) > 0
    {
        tips.push("Folds trop serrés : avec la cote affichée, ces mains gagnent assez souvent pour continuer.".to_owned());
    }
    if count(&|d| {
        d.severity >= Severity::Inaccuracy
            && matches!(d.played, Decision::Passive | Decision::Fold)
            && matches!(d.best, Decision::Raise | Decision::AllIn)
    }) > 0
    {
        tips.push("Manque d'agressivité : miser ou relancer fait coucher des mains meilleures et protège votre équité.".to_owned());
    }
    if count(&|d| {
        d.severity >= Severity::Mistake
            && matches!(d.played, Decision::Raise | Decision::AllIn)
            && !matches!(d.best, Decision::Raise | Decision::AllIn)
    }) > 0
    {
        tips.push("Agressivité coûteuse : face à ces ranges, les mains qui continuent vous dominent trop souvent.".to_owned());
    }
    if count(&|d| {
        d.severity >= Severity::Inaccuracy
            && matches!(d.played, Decision::Raise | Decision::AllIn)
            && matches!(d.best, Decision::Raise | Decision::AllIn)
    }) > 0
    {
        tips.push("Tailles de mise : adaptez la taille à la force de la main et au tapis restant (plus gros avec une main très forte et peu de tapis derrière).".to_owned());
    }
    if count(&|d| d.basis == Basis::PushFold && d.severity >= Severity::Inaccuracy) > 0 {
        tips.push("Tapis courts : révisez les ranges push/fold de votre format (le laboratoire et l'analyse les affichent par position).".to_owned());
    }
    if tips.is_empty() && !decisions.is_empty() {
        tips.push("Rien à corriger sur cette main : continuez à analyser vos mains marquantes pour trouver vos fuites récurrentes.".to_owned());
    }
    let verdict = match (decisions.len(), mistakes, score) {
        (0, _, _) => "Aucune décision du joueur à analyser dans cette main.".to_owned(),
        (_, 0, 90..) => "Main jouée comme la référence.".to_owned(),
        (_, 0, _) => "Main bien jouée, avec quelques imprécisions.".to_owned(),
        (_, 1, _) => "Une décision coûteuse à revoir.".to_owned(),
        (_, m, _) => format!("{m} décisions coûteuses à revoir."),
    };
    Summary {
        score,
        decisions: decisions.len(),
        mistakes,
        ev_lost_bb: ev_lost,
        verdict,
        tips,
    }
}

/// Analyse toutes les mains d'un texte d'historiques.
#[must_use]
pub fn analyse_text(
    text: &str,
    hero: Option<&str>,
    options: &Options,
) -> (Vec<Result<HandAnalysis, AnalysisError>>, Vec<String>) {
    let report = import_text(text);
    let errors = report.errors.iter().map(ToString::to_string).collect();
    let analyses = report
        .hands
        .iter()
        .map(|h| analyse(&h.record, hero.or(h.hero.as_deref()), options))
        .collect();
    (analyses, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::import::{SAMPLE_POKERSTARS, SAMPLE_WINAMAX, import_hand};

    pub(crate) const BAD_CALL: &str = "PokerStars Hand #1: Tournament #2, $1+$0 USD Hold'em No Limit - Level I (10/20) - 2026/10/02 21:14:05 ET
Table '2 1' 3-max Seat #1 is the button
Seat 1: Vilain (500 in chips)
Seat 2: Hero (500 in chips)
Vilain: posts small blind 10
Hero: posts big blind 20
*** HOLE CARDS ***
Dealt to Hero [7c 2d]
Vilain: raises 480 to 500 and is all-in
Hero: calls 480 and is all-in
*** FLOP *** [As Kd 9h]
*** TURN *** [As Kd 9h] [7h]
*** RIVER *** [As Kd 9h 7h] [2s]
*** SHOW DOWN ***
Vilain: shows [Qc Jc]
Hero: shows [7c 2d]
Hero collected 1000 from pot
*** SUMMARY ***
";

    fn fast() -> Options {
        Options {
            samples: 1500,
            ..Options::default()
        }
    }

    #[test]
    fn analyses_a_winamax_expresso() {
        let h = import_hand(SAMPLE_WINAMAX).unwrap();
        let a = analyse(&h.record, h.hero.as_deref(), &fast()).unwrap();
        assert_eq!(a.engine, "IA Expresso 3-Max");
        assert_eq!(a.hero, "Hero");
        assert_eq!(a.position, "BTN");
        assert!(a.decisions.len() >= 2, "{:#?}", a.decisions);
        assert!(
            a.decisions
                .iter()
                .all(|d| d.model.is_some() && d.population.is_some())
        );
        assert!(a.result_bb.is_some_and(|r| r > 0.0));
        assert!(!a.summary.tips.is_empty());
        assert!(a.lines.iter().any(|l| l.text.starts_with("Flop")));
    }

    #[test]
    fn analyses_a_pokerstars_6max() {
        let h = import_hand(SAMPLE_POKERSTARS).unwrap();
        let a = analyse(&h.record, None, &fast()).unwrap();
        assert_eq!(a.engine, "IA Tournois 6-Max");
        assert_eq!(a.position, "BB");
        // AQo en grosse blinde à 15 BB face à une relance : le tapis est la référence.
        let d = &a.decisions[0];
        assert_eq!(d.played, Decision::AllIn);
        assert!(d.severity <= Severity::Inaccuracy, "{d:#?}");
        assert!(a.summary.score > 50);
    }

    #[test]
    fn a_bad_call_is_flagged() {
        // Tête-à-tête à 25 BB : 72o suit un tapis.
        let text = BAD_CALL;
        let (analyses, errors) = analyse_text(text, None, &fast());
        assert!(errors.is_empty(), "{errors:?}");
        let a = analyses[0].as_ref().unwrap();
        let d = &a.decisions[0];
        assert_eq!(d.played, Decision::Passive);
        assert_eq!(d.best, Decision::Fold);
        assert!(d.severity >= Severity::Mistake, "{d:#?}");
        assert!(
            a.result_bb.unwrap() > 0.0,
            "il a gagné, mais la décision reste mauvaise"
        );
        assert!(
            a.summary
                .tips
                .iter()
                .any(|t| t.contains("Calls trop légers"))
        );
    }
}
