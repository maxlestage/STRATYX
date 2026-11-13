//! Le bilan d'une session : toutes les mains d'un fichier d'historiques, leurs statistiques,
//! les fuites classées par EV perdue, les décisions les plus coûteuses et des conseils
//! concrets. Après la partie seulement (décision D6).
//!
//! [`SessionBuilder`] se remplit main par main (l'interface peut ainsi analyser une longue
//! session par petits morceaux sans bloquer la page), puis [`SessionBuilder::finish`] donne
//! le [`SessionReport`].

use serde::Serialize;
use stratyx_poker::game::{ActionKind, HandSetup};
use stratyx_poker::history::{HandEvent, HandRecord};
use stratyx_poker::{HandState, Street};
use stratyx_strategy::{Basis, Decision};

use crate::{HandAnalysis, Severity, bb, pct};

/// Statistiques du joueur sur la session (fréquences de 0 à 1).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    pub hands: usize,
    /// Mains jouées volontairement (call ou relance préflop).
    pub vpip: f32,
    /// Mains relancées préflop.
    pub pfr: f32,
    /// 3-bet quand l'occasion se présente.
    pub three_bet: f32,
    /// Facteur d'agression après le flop : (mises + relances) / calls.
    pub aggression: f32,
    /// Mains allées à l'abattage parmi celles qui ont vu le flop.
    pub went_to_showdown: f32,
    /// Tapis préflop parmi les mains jouées.
    pub preflop_all_in: f32,
}

/// Une fuite : une famille d'erreurs, son coût, et quoi faire.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Leak {
    pub title: String,
    pub advice: String,
    pub count: usize,
    pub ev_lost_bb: f32,
    /// (index de la main, index de la décision) des exemples les plus coûteux.
    pub examples: Vec<(usize, usize)>,
}

/// Une décision coûteuse de la session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Costly {
    pub hand: usize,
    pub decision: usize,
    pub hand_id: String,
    pub cards: String,
    pub street: Street,
    pub position: String,
    pub played: String,
    pub best: String,
    pub loss_bb: f32,
    pub severity: Severity,
}

/// Un repère : une statistique, sa plage habituelle pour le format, et le verdict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Benchmark {
    pub name: String,
    pub value: f32,
    pub low: f32,
    pub high: f32,
    pub advice: Option<String>,
}

/// Le bilan d'une session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionReport {
    pub hero: String,
    pub hands: usize,
    pub analysed: usize,
    /// Mains écartées (illisibles, cartes du joueur inconnues…).
    pub skipped: usize,
    pub formats: Vec<(String, usize)>,
    pub stats: Stats,
    pub benchmarks: Vec<Benchmark>,
    pub result_bb: f32,
    pub decisions: usize,
    pub by_severity: [usize; 4],
    pub ev_lost_bb: f32,
    /// EV perdue pour 100 mains.
    pub ev_lost_per_100: f32,
    pub average_score: f32,
    pub pushfold_decisions: usize,
    pub pushfold_followed: f32,
    pub leaks: Vec<Leak>,
    pub costly: Vec<Costly>,
    /// Les trois priorités de travail, dans l'ordre.
    pub priorities: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct Counters {
    hands: usize,
    vpip: usize,
    pfr: usize,
    three_bet_spots: usize,
    three_bets: usize,
    post_aggr: usize,
    post_calls: usize,
    saw_flop: usize,
    showdown: usize,
    preflop_all_in: usize,
}

/// Compte les statistiques d'une main pour le joueur `hero` (index dans l'ordre des sièges).
fn count(record: &HandRecord, hero: usize, c: &mut Counters) {
    let mut order: Vec<_> = record.seats.iter().collect();
    order.sort_by_key(|s| s.seat);
    let Some(button) = order.iter().position(|s| s.seat == record.button_seat) else {
        return;
    };
    let setup = HandSetup::new(
        order.iter().map(|s| s.stack.value).collect(),
        button,
        record.blinds.value,
    )
    .with_structure(record.format.structure);
    let Ok(mut state) = HandState::new(&setup) else {
        return;
    };
    let (mut vpip, mut pfr, mut all_in, mut saw_flop) = (false, false, false, false);
    let mut raises_before = 0usize;
    let mut spot_counted = false;
    for ev in &record.events {
        match &ev.value {
            HandEvent::Action { seat, action } => {
                let Some(p) = order.iter().position(|s| s.seat == *seat) else {
                    return;
                };
                if state.apply(p, *action).is_err() {
                    return;
                }
                let Some(a) = state.actions().last().copied() else {
                    continue;
                };
                if a.street == Street::Preflop {
                    if p == hero {
                        if raises_before == 1 && !spot_counted {
                            spot_counted = true;
                            c.three_bet_spots += 1;
                            if a.kind.is_aggressive() {
                                c.three_bets += 1;
                            }
                        }
                        if matches!(
                            a.kind,
                            ActionKind::Call | ActionKind::Raise | ActionKind::Bet
                        ) {
                            vpip = true;
                        }
                        if a.kind.is_aggressive() {
                            pfr = true;
                        }
                        if a.all_in && a.kind != ActionKind::Fold {
                            all_in = true;
                        }
                    }
                    if a.kind.is_aggressive() {
                        raises_before += 1;
                    }
                } else if p == hero {
                    if a.kind.is_aggressive() {
                        c.post_aggr += 1;
                    } else if a.kind == ActionKind::Call {
                        c.post_calls += 1;
                    }
                }
            }
            HandEvent::Board { cards } => {
                if state.deal(cards).is_err() {
                    return;
                }
                if state.street() == Street::Flop
                    && state.players()[hero].status != stratyx_poker::game::PlayerStatus::Folded
                {
                    saw_flop = true;
                }
            }
            _ => {}
        }
    }
    c.hands += 1;
    c.vpip += usize::from(vpip);
    c.pfr += usize::from(pfr);
    c.preflop_all_in += usize::from(all_in);
    if saw_flop {
        c.saw_flop += 1;
        let at_showdown = matches!(
            state.phase(),
            stratyx_poker::game::Phase::Showdown | stratyx_poker::game::Phase::Finished
        ) && state.players()[hero].status
            != stratyx_poker::game::PlayerStatus::Folded
            && state.street() == Street::River;
        c.showdown += usize::from(at_showdown);
    }
}

/// Les familles d'erreurs, avec leur conseil.
const FAMILIES: [(&str, &str); 7] = [
    (
        "Calls trop légers",
        "Avant de suivre, comparez votre équité à la cote du pot : face à une relance ou un tapis, la range adverse est forte. Coupez les calls dominés (as faibles, hauteurs dépareillées).",
    ),
    (
        "Folds trop serrés",
        "Quand la cote est bonne (petite mise, gros pot), ces mains gagnent assez souvent : continuez plus souvent, surtout en grosse blinde.",
    ),
    (
        "Manque d'agressivité",
        "Misez ou relancez vos mains fortes et vos bons tirages : vous faites coucher des mains meilleures et protégez votre équité au lieu de laisser des cartes gratuites.",
    ),
    (
        "Agressivité coûteuse",
        "Ces relances sont payées surtout par des mains qui vous dominent : gardez l'agression pour la valeur et les bluffs avec équité (tirages, bloqueurs).",
    ),
    (
        "Push/fold trop serré",
        "À tapis court, l'équilibre pousse bien plus de mains que l'instinct : revoyez les ranges de tapis par position et par profondeur (laboratoire, page Analyse).",
    ),
    (
        "Push/fold trop large",
        "Certaines poussées ou certains calls de tapis sont perdants à l'équilibre : resserrez-vous en premier de parole et face aux tapis des positions serrées.",
    ),
    (
        "Tailles de mise",
        "Adaptez la taille à la force de la main et au tapis restant : plus gros avec une main très forte et peu de tapis derrière, plus petit pour les bluffs bon marché.",
    ),
];

fn family(d: &crate::DecisionReview) -> Option<usize> {
    if d.severity == Severity::Good {
        return None;
    }
    let aggressive = |x: Decision| matches!(x, Decision::Raise | Decision::AllIn);
    Some(match (d.basis, d.played, d.best) {
        (Basis::PushFold, Decision::Fold, _) => 4,
        (Basis::PushFold, _, Decision::Fold) => 5,
        (_, p, b) if aggressive(p) && aggressive(b) => 6,
        (_, Decision::Passive, Decision::Fold) => 0,
        (_, Decision::Fold, Decision::Passive) => 1,
        (_, Decision::Passive | Decision::Fold, b) if aggressive(b) => 2,
        (_, p, _) if aggressive(p) => 3,
        _ => return None,
    })
}

/// Repères habituels (plages indicatives) selon le format le plus joué.
fn benchmarks(stats: &Stats, format: &str) -> Vec<Benchmark> {
    let expresso = format.contains("Expresso") || format.contains("3-max");
    let plages: [(&str, f32, f32, f32, &str, &str); 4] = if expresso {
        [
            (
                "VPIP",
                stats.vpip,
                0.35,
                0.60,
                "Vous jouez trop peu de mains pour un format à trois : élargissez au bouton et en petite blinde.",
                "Vous jouez trop de mains : même à trois, les mains faibles dépareillées perdent.",
            ),
            (
                "PFR",
                stats.pfr,
                0.28,
                0.50,
                "Pas assez de relances préflop : à trois, l'initiative rapporte, relancez plutôt que suivre.",
                "Beaucoup de relances : vérifiez que vos relances sont payées par des mains plus faibles.",
            ),
            (
                "3-bet",
                stats.three_bet,
                0.08,
                0.20,
                "Trop peu de 3-bets : les ouvertures à trois sont larges, défendez en relançant.",
                "3-bet très fréquent : attention aux 4-bets et tapis des joueurs serrés.",
            ),
            (
                "Agression postflop",
                stats.aggression,
                1.5,
                4.0,
                "Trop passif après le flop : misez vos mains faites et vos tirages.",
                "Très agressif après le flop : gardez des mains de parole pour équilibrer.",
            ),
        ]
    } else {
        [
            (
                "VPIP",
                stats.vpip,
                0.18,
                0.30,
                "Trop serré : ouvrez plus large en position (CO, bouton) et défendez mieux la grosse blinde.",
                "Trop large : resserrez-vous surtout en début de parole (UTG, HJ).",
            ),
            (
                "PFR",
                stats.pfr,
                0.14,
                0.25,
                "Pas assez de relances : en 6-max, entrez dans le coup en relançant plutôt qu'en suivant.",
                "Beaucoup de relances : vérifiez vos ouvertures en début de parole.",
            ),
            (
                "3-bet",
                stats.three_bet,
                0.05,
                0.11,
                "Trop peu de 3-bets face aux ouvertures tardives.",
                "3-bet très fréquent : vos 3-bets légers sont exposés aux 4-bets.",
            ),
            (
                "Agression postflop",
                stats.aggression,
                1.5,
                4.0,
                "Trop passif après le flop.",
                "Très agressif après le flop : attention aux bluffs sans équité.",
            ),
        ]
    };
    plages
        .into_iter()
        .map(|(name, value, low, high, trop_bas, trop_haut)| Benchmark {
            name: name.to_owned(),
            value,
            low,
            high,
            advice: if stats.hands < 20 {
                Some(format!(
                    "Échantillon trop petit ({} mains) pour juger : importez des sessions plus longues.",
                    stats.hands
                ))
            } else if value < low {
                Some(trop_bas.to_owned())
            } else if value > high {
                Some(trop_haut.to_owned())
            } else {
                None
            },
        })
        .collect()
}

/// Une famille d'erreurs en cours de comptage : nombre, EV perdue, exemples
/// (perte, main, décision).
type LeakAcc = (usize, f32, Vec<(f32, usize, usize)>);

/// Le bilan, construit main par main.
#[derive(Debug, Default, Clone)]
pub struct SessionBuilder {
    hero: String,
    hands: usize,
    skipped: usize,
    formats: Vec<(String, usize)>,
    counters: Counters,
    result_bb: f32,
    decisions: usize,
    by_severity: [usize; 4],
    ev_lost: f32,
    scores: Vec<f32>,
    pushfold: (usize, usize),
    leaks: Vec<LeakAcc>,
    costly: Vec<Costly>,
}

impl SessionBuilder {
    #[must_use]
    pub fn new() -> SessionBuilder {
        SessionBuilder {
            leaks: (0..FAMILIES.len()).map(|_| (0, 0.0, Vec::new())).collect(),
            ..SessionBuilder::default()
        }
    }

    /// Une main qui n'a pas pu être analysée.
    pub fn skip(&mut self) {
        self.hands += 1;
        self.skipped += 1;
    }

    /// Ajoute une main analysée (`index` : son rang dans la session).
    pub fn add(&mut self, index: usize, record: &HandRecord, a: &HandAnalysis) {
        self.hands += 1;
        if self.hero.is_empty() {
            self.hero = a.hero.clone();
        }
        match self.formats.iter_mut().find(|f| f.0 == a.format) {
            Some(f) => f.1 += 1,
            None => self.formats.push((a.format.clone(), 1)),
        }
        let mut order: Vec<_> = record.seats.iter().collect();
        order.sort_by_key(|s| s.seat);
        if let Some(h) = order.iter().position(|s| s.player == a.hero) {
            count(record, h, &mut self.counters);
        }
        self.result_bb += a.result_bb.unwrap_or(0.0);
        self.scores.push(f32::from(a.summary.score));
        for (di, d) in a.decisions.iter().enumerate() {
            self.decisions += 1;
            self.by_severity[d.severity as usize] += 1;
            self.ev_lost += d.loss_bb;
            if d.basis == Basis::PushFold {
                self.pushfold.0 += 1;
                self.pushfold.1 += usize::from(d.severity == Severity::Good);
            }
            if let Some(f) = family(d) {
                let l = &mut self.leaks[f];
                l.0 += 1;
                l.1 += d.loss_bb;
                l.2.push((d.loss_bb, index, di));
            }
            if d.severity >= Severity::Inaccuracy {
                self.costly.push(Costly {
                    hand: index,
                    decision: di,
                    hand_id: a.id.clone(),
                    cards: a.hero_cards.clone(),
                    street: d.street,
                    position: d.position.clone(),
                    played: d.played_text.clone(),
                    best: d.best_text.clone(),
                    loss_bb: d.loss_bb,
                    severity: d.severity,
                });
            }
        }
    }

    /// Le bilan final.
    #[must_use]
    pub fn finish(mut self) -> SessionReport {
        let c = &self.counters;
        let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f32 / b as f32 };
        let stats = Stats {
            hands: c.hands,
            vpip: ratio(c.vpip, c.hands),
            pfr: ratio(c.pfr, c.hands),
            three_bet: ratio(c.three_bets, c.three_bet_spots),
            aggression: if c.post_calls == 0 {
                c.post_aggr as f32
            } else {
                ratio(c.post_aggr, c.post_calls)
            },
            went_to_showdown: ratio(c.showdown, c.saw_flop),
            preflop_all_in: ratio(c.preflop_all_in, c.vpip),
        };
        self.formats.sort_by_key(|f| std::cmp::Reverse(f.1));
        let main_format = self
            .formats
            .first()
            .map(|f| f.0.clone())
            .unwrap_or_default();
        let benchmarks = benchmarks(&stats, &main_format);
        let analysed = self.hands - self.skipped;
        let mut leaks: Vec<Leak> = self
            .leaks
            .into_iter()
            .enumerate()
            .filter(|(_, l)| l.0 > 0)
            .map(|(f, (n, ev, mut ex))| {
                ex.sort_by(|a, b| b.0.total_cmp(&a.0));
                Leak {
                    title: FAMILIES[f].0.to_owned(),
                    advice: FAMILIES[f].1.to_owned(),
                    count: n,
                    ev_lost_bb: ev,
                    examples: ex.into_iter().take(3).map(|e| (e.1, e.2)).collect(),
                }
            })
            .collect();
        leaks.sort_by(|a, b| {
            b.ev_lost_bb
                .total_cmp(&a.ev_lost_bb)
                .then(b.count.cmp(&a.count))
        });
        self.costly.sort_by(|a, b| b.loss_bb.total_cmp(&a.loss_bb));
        self.costly.truncate(8);
        let stats_ok = stats.hands >= 20;
        let mut priorities: Vec<String> = leaks
            .iter()
            .take(2)
            .map(|l| {
                format!(
                    "{} ({} fois, {} perdues) : {}",
                    l.title,
                    l.count,
                    bb(l.ev_lost_bb),
                    l.advice
                )
            })
            .collect();
        priorities.extend(
            benchmarks
                .iter()
                .filter(|_| stats_ok)
                .filter_map(|b| {
                    b.advice.as_ref().map(|a| {
                        format!(
                            "{} à {} : {a}",
                            b.name,
                            if b.name.contains("Agression") {
                                format!("{:.1}", b.value).replace('.', ",")
                            } else {
                                pct(b.value)
                            }
                        )
                    })
                })
                .take(3 - priorities.len().min(3)),
        );
        if priorities.is_empty() && analysed > 0 {
            priorities.push("Aucune fuite nette sur cette session : continuez à importer vos sessions pour confirmer sur plus de mains.".to_owned());
        }
        let n = analysed.max(1) as f32;
        SessionReport {
            hero: self.hero,
            hands: self.hands,
            analysed,
            skipped: self.skipped,
            formats: self.formats,
            stats,
            benchmarks,
            result_bb: self.result_bb,
            decisions: self.decisions,
            by_severity: self.by_severity,
            ev_lost_bb: self.ev_lost,
            ev_lost_per_100: self.ev_lost / n * 100.0,
            average_score: self.scores.iter().sum::<f32>() / self.scores.len().max(1) as f32,
            pushfold_decisions: self.pushfold.0,
            pushfold_followed: ratio(self.pushfold.1, self.pushfold.0),
            leaks,
            costly: self.costly,
            priorities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, analyse};
    use stratyx_poker::import::{SAMPLE_POKERSTARS, SAMPLE_WINAMAX, import_text};

    #[test]
    fn a_session_report() {
        let text = format!(
            "{SAMPLE_POKERSTARS}\n\n{SAMPLE_WINAMAX}\n\n{}",
            crate::tests::BAD_CALL
        );
        let r = import_text(&text);
        assert_eq!(r.hands.len(), 3);
        let opts = Options {
            samples: 1200,
            ..Options::default()
        };
        let mut s = SessionBuilder::new();
        for (i, h) in r.hands.iter().enumerate() {
            match analyse(&h.record, h.hero.as_deref(), &opts) {
                Ok(a) => s.add(i, &h.record, &a),
                Err(_) => s.skip(),
            }
        }
        let rep = s.finish();
        assert_eq!(rep.analysed, 3);
        assert!(rep.decisions >= 4);
        assert!(rep.stats.vpip > 0.9, "{:?}", rep.stats);
        assert!(rep.leaks.iter().any(|l| l.title == "Calls trop légers"));
        assert_eq!(
            rep.costly[0].cards,
            "7c2d"
                .parse::<stratyx_poker::HoleCards>()
                .unwrap()
                .to_string()
        );
        assert!(!rep.priorities.is_empty());
    }
}
