//! Analyse de décision : cotes, SPR, équité, EV du call, fold equity, ICM, référence.

mod common;

use common::HandBuilder;
use stratyx_analytics::decision::{EquityBasis, ReferenceOption};
use stratyx_analytics::{
    AnalyticsError, DecisionOptions, DecisionReport, DecisionSpot, ReferenceAdvice,
    ReferenceStrategy, analyze_decisions, analyze_hero_decisions,
};
use stratyx_poker::HandRecord;
use stratyx_poker::equity::EquityMethod;
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::{Action, ActionKind, BettingStructure, Blinds, Street};
use stratyx_poker::icm::icm_equities;
use stratyx_poker::odds;
use stratyx_poker::position::Position;
use stratyx_poker::range::Range;
use stratyx_shared::Chips;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn hu_cash() -> GameFormat {
    GameFormat::new(GameType::CashGame, 2, BettingStructure::NoLimit).unwrap()
}

/// Tête-à-tête : le héros (BB, 99) suit une relance, touche un brelan au flop et paie
/// deux mises ; l'adversaire montre QQ.
fn set_over_overpair() -> HandRecord {
    HandBuilder::new("set", hu_cash(), Blinds::new(50, 100), 1)
        .seat(1, "Villain", 10_000)
        .seat(2, "Hero", 10_000)
        .hero(2, "9c9s")
        .raise(1, 250)
        .call(2)
        .board("2c7d9h")
        .check(2)
        .bet(1, 200)
        .call(2)
        .board("Js")
        .check(2)
        .check(1)
        .board("3d")
        .check(2)
        .bet(1, 300)
        .call(2)
        .show(1, "QhQd")
        .build()
}

#[test]
fn pot_odds_spr_and_call_ev_with_known_cards() {
    let reports =
        analyze_hero_decisions(&set_over_overpair(), &DecisionOptions::default()).unwrap();
    let kinds: Vec<(Street, ActionKind)> =
        reports.iter().map(|r| (r.street, r.action.kind)).collect();
    use ActionKind::{Call, Check};
    assert_eq!(
        kinds,
        vec![
            (Street::Preflop, Call),
            (Street::Flop, Check),
            (Street::Flop, Call),
            (Street::Turn, Check),
            (Street::River, Check),
            (Street::River, Call),
        ]
    );

    // Préflop : 150 à suivre dans un pot de 350.
    let pre = &reports[0];
    assert_eq!((pre.pot, pre.to_call), (Chips(350), Chips(150)));
    assert_eq!(pre.position, Position::BigBlind);
    assert!(close(pre.pot_odds.unwrap(), 350.0 / 150.0));
    assert!(close(pre.required_equity.unwrap(), 0.3));
    assert_eq!(pre.effective_stack, Chips(9_900));
    assert!(close(pre.spr, 9_900.0 / 350.0));
    let eq = pre.equity.unwrap();
    assert_eq!(
        (eq.basis, eq.method),
        (EquityBasis::KnownCards, EquityMethod::Exact)
    );
    assert!((0.15..0.22).contains(&eq.equity), "{}", eq.equity);
    assert!(close(pre.call_ev.unwrap(), eq.equity * 500.0 - 150.0));
    assert!(
        pre.call_ev.unwrap() < 0.0,
        "call préflop perdant contre QQ connues"
    );

    // Check : rien à suivre.
    let check = &reports[1];
    assert_eq!((check.pot, check.to_call), (Chips(500), Chips::ZERO));
    assert_eq!(check.pot_odds, None);
    assert_eq!(check.call_ev, None);
    assert_eq!(check.fold_equity, None);
    assert_eq!(check.board.len(), 3);

    // River : 300 à suivre dans 1 200 avec la meilleure main.
    let river = reports.last().unwrap();
    assert_eq!((river.pot, river.to_call), (Chips(1_200), Chips(300)));
    assert!(close(river.pot_odds.unwrap(), 4.0));
    assert!(close(river.required_equity.unwrap(), 0.2));
    assert!(close(river.equity.unwrap().equity, 1.0));
    assert!(close(river.call_ev.unwrap(), 1_200.0));
    assert!(close(river.call_ev_bb.unwrap(), 12.0));
    assert_eq!(river.effective_stack, Chips(9_550));
    assert!(close(river.spr, 9_550.0 / 1_200.0));
    assert_eq!(river.opponents.len(), 1);
    assert_eq!(
        river.opponents[0].known_cards,
        Some("QhQd".parse().unwrap())
    );
    assert!(river.notes.is_empty(), "{:?}", river.notes);

    let json = serde_json::to_string(&reports).unwrap();
    let back: Vec<DecisionReport> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), 6);
    assert_eq!(back[5].action, river.action);
}

#[test]
fn equity_against_ranges_when_cards_are_ignored() {
    let hand = set_over_overpair();
    let mut options = DecisionOptions {
        use_known_cards: false,
        ..DecisionOptions::default()
    };
    options
        .opponent_ranges
        .insert(1, Range::parse("QQ+,AK").unwrap());
    let river = analyze_hero_decisions(&hand, &options)
        .unwrap()
        .pop()
        .unwrap();
    let eq = river.equity.unwrap();
    assert_eq!(
        (eq.basis, eq.method),
        (EquityBasis::Ranges, EquityMethod::Exact)
    );
    assert!(
        close(eq.equity, 1.0),
        "aucune main de la range ne bat le brelan"
    );
    assert_eq!(river.opponents[0].known_cards, None);

    // Range par défaut : toutes les mains compatibles (quinte T8, brelan de valets…).
    let options = DecisionOptions {
        use_known_cards: false,
        ..DecisionOptions::default()
    };
    let river = analyze_hero_decisions(&hand, &options)
        .unwrap()
        .pop()
        .unwrap();
    let eq = river.equity.unwrap().equity;
    assert!((0.9..1.0).contains(&eq), "{eq}");
    assert_eq!(
        river.call_ev.unwrap() > 0.0,
        eq > river.required_equity.unwrap()
    );

    // Préflop contre une range complète : Monte-Carlo, avec erreur-type.
    let mut options = DecisionOptions {
        use_known_cards: false,
        ..DecisionOptions::default()
    };
    options.equity.samples = 20_000;
    let pre = analyze_hero_decisions(&hand, &options).unwrap().remove(0);
    let eq = pre.equity.unwrap();
    assert_eq!(eq.method, EquityMethod::MonteCarlo);
    assert!(eq.std_error > 0.0);
    assert!(
        (0.66..0.75).contains(&eq.equity),
        "99 contre une main au hasard : {}",
        eq.equity
    );
}

#[test]
fn fold_equity_for_bets_and_raises() {
    let hand = set_over_overpair();
    let reports = analyze_decisions(&hand, 1, &DecisionOptions::default(), None).unwrap();
    assert_eq!(reports[0].action.kind, ActionKind::Raise);

    // Relance préflop à 250 depuis la petite blinde : 200 risqués, 150 à payer en face.
    let raise = &reports[0];
    assert_eq!((raise.pot, raise.to_call), (Chips(150), Chips(50)));
    let fe = raise.fold_equity.unwrap();
    assert_eq!(
        (fe.risk, fe.opponent_call, fe.opponent_seat),
        (Chips(200), Chips(150), 2)
    );
    assert!(close(fe.break_even_bluff, 200.0 / 350.0));

    // Mise de 200 au flop dans 500 avec QQ contre le brelan connu.
    let bet = reports
        .iter()
        .find(|r| r.street == Street::Flop && r.action.kind == ActionKind::Bet)
        .unwrap();
    assert_eq!(bet.pot, Chips(500));
    let fe = bet.fold_equity.unwrap();
    assert_eq!((fe.risk, fe.opponent_call), (Chips(200), Chips(200)));
    assert!(close(fe.break_even_bluff, 200.0 / 700.0));
    let e = bet.equity.unwrap().equity;
    assert!(e < 0.1, "{e}");
    let needed = fe.break_even.unwrap();
    // Un peu d'équité quand on est payé : il faut moins de folds qu'un bluff pur.
    assert!(needed > 0.0 && needed < fe.break_even_bluff, "{needed}");
    assert!(close(fe.ev_when_called.unwrap(), e * 900.0 - 200.0));
    // Au seuil, l'EV de la mise est nulle.
    assert!(odds::bet_ev(needed, 500.0, 200.0, 200.0, e).abs() < 1e-9);
}

/// Expresso : le bouton pousse 500, la SB se couche, le héros (BB, 300) doit décider.
fn push_spot(format: GameFormat) -> HandRecord {
    HandBuilder::new("push", format, Blinds::new(10, 20), 1)
        .seat(1, "Villain", 500)
        .seat(2, "Sb", 700)
        .seat(3, "Hero", 300)
        .hero(3, "AhKh")
        .all_in(1)
        .fold(2)
        .call(3)
        .board("2c7d9s")
        .board("Js")
        .board("3d")
        .show(1, "QcQd")
        .build()
}

#[test]
fn icm_call_against_a_shove() {
    // Gagnant unique : l'ICM est proportionnel aux jetons, la prime de risque est nulle.
    let options = DecisionOptions {
        icm_payouts: Some(vec![1.0]),
        ..DecisionOptions::default()
    };
    let call = analyze_hero_decisions(&push_spot(GameFormat::EXPRESSO), &options)
        .unwrap()
        .pop()
        .unwrap();
    // 530 au milieu, 280 à suivre ; 200 de la poussée ne peuvent pas être gagnés.
    assert_eq!((call.pot, call.to_call), (Chips(530), Chips(280)));
    assert!(close(call.required_equity.unwrap(), 280.0 / 610.0));
    assert!(close(call.pot_odds.unwrap(), 330.0 / 280.0));
    let icm = call.icm.as_ref().unwrap();
    assert!(close(icm.fold, 280.0 / 1_500.0));
    assert!(close(icm.call_win, 610.0 / 1_500.0));
    assert!(close(icm.call_lose, 0.0));
    assert!(close(icm.required_equity.unwrap(), 280.0 / 610.0));
    assert!(icm.risk_premium.unwrap().abs() < 1e-9);
    let e = call.equity.unwrap().equity;
    assert!(close(
        icm.ev_call_minus_fold.unwrap(),
        e * 610.0 / 1_500.0 - 280.0 / 1_500.0
    ));

    // Sit & go à la bulle (deux places payées) : la prime de risque devient positive.
    let sng = GameFormat::new(GameType::SitAndGo, 3, BettingStructure::NoLimit).unwrap();
    let payouts = vec![0.65, 0.35];
    let options = DecisionOptions {
        icm_payouts: Some(payouts.clone()),
        ..DecisionOptions::default()
    };
    let call = analyze_hero_decisions(&push_spot(sng), &options)
        .unwrap()
        .pop()
        .unwrap();
    let icm = call.icm.unwrap();
    let chips = |v: [u64; 3]| v.map(Chips).to_vec();
    let fold = icm_equities(&chips([530, 690, 280]), &payouts).unwrap()[2];
    let win = icm_equities(&chips([200, 690, 610]), &payouts).unwrap()[2];
    assert!(close(icm.fold, fold));
    assert!(close(icm.call_win, win));
    assert!(close(icm.call_lose, 0.0), "éliminé hors des places payées");
    assert!(close(icm.required_equity.unwrap(), fold / win));
    assert!(icm.risk_premium.unwrap() > 0.05, "{:?}", icm.risk_premium);

    // En cash game, l'ICM n'a pas de sens.
    let cash3 = GameFormat::new(GameType::CashGame, 3, BettingStructure::NoLimit).unwrap();
    let call = analyze_hero_decisions(&push_spot(cash3), &options)
        .unwrap()
        .pop()
        .unwrap();
    assert!(call.icm.is_none());
    assert!(
        call.notes.iter().any(|n| n.contains("cash game")),
        "{:?}",
        call.notes
    );
}

struct AlwaysFold;

impl ReferenceStrategy for AlwaysFold {
    fn advise(&self, spot: &DecisionSpot<'_>) -> Option<ReferenceAdvice> {
        // La référence voit la situation avant la décision, jamais la suite.
        assert_eq!(spot.state.to_act(), Some(spot.player));
        (spot.state.street() == Street::Preflop).then(|| ReferenceAdvice {
            source: "test".into(),
            options: vec![ReferenceOption {
                action: Action::Fold,
                frequency: 1.0,
                ev_bb: Some(0.0),
            }],
        })
    }
}

#[test]
fn reference_strategy_extension_point() {
    let reports = analyze_decisions(
        &set_over_overpair(),
        2,
        &DecisionOptions::default(),
        Some(&AlwaysFold),
    )
    .unwrap();
    assert_eq!(reports[0].reference.as_ref().unwrap().source, "test");
    assert!(reports[1..].iter().all(|r| r.reference.is_none()));
}

#[test]
fn unknown_cards_and_errors() {
    let hand = set_over_overpair();
    // Les cartes du joueur au siège 1 sont connues (montrées) ; sans elles, pas d'équité.
    let mut hidden = hand.clone();
    hidden.events.pop();
    let reports = analyze_decisions(&hidden, 1, &DecisionOptions::default(), None).unwrap();
    assert!(
        reports
            .iter()
            .all(|r| r.equity.is_none() && r.call_ev.is_none())
    );
    assert!(reports[0].notes[0].contains("inconnues"));
    assert!(reports[0].fold_equity.unwrap().break_even.is_none());

    assert!(matches!(
        analyze_decisions(&hand, 7, &DecisionOptions::default(), None),
        Err(AnalyticsError::SeatNotFound { seat: 7, .. })
    ));
    let mut anonymous = hand;
    anonymous.meta.hero_seat = None;
    assert!(matches!(
        analyze_hero_decisions(&anonymous, &DecisionOptions::default()),
        Err(AnalyticsError::NoHero(_))
    ));
}
