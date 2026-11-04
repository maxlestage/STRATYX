//! Résultats personnels : gains, BB/100, positions, formats, courbe et EV à l'all-in.

mod common;

use common::{HandBuilder, six_max};
use stratyx_analytics::{AnalyticsError, HeroResults};
use stratyx_poker::HandRecord;
use stratyx_poker::cardset::CardSet;
use stratyx_poker::equity::{EquityOptions, equity};
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::{BettingStructure, Blinds, Street};
use stratyx_poker::position::Position;
use stratyx_poker::range::Range;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn cash(players: u8) -> GameFormat {
    GameFormat::new(GameType::CashGame, players, BettingStructure::NoLimit).unwrap()
}

/// Équité exacte des mains données, sur un board donné, pour le premier joueur.
fn exact_equity(hands: &[&str], board: &str) -> f64 {
    let ranges: Vec<Range> = hands
        .iter()
        .map(|h| Range::single(h.parse().unwrap()))
        .collect();
    let board = if board.is_empty() {
        CardSet::EMPTY
    } else {
        CardSet::parse(board).unwrap()
    };
    equity(&ranges, board, CardSet::EMPTY, &EquityOptions::default())
        .unwrap()
        .players[0]
        .equity
}

fn btn_open(at: Option<u64>) -> HandRecord {
    let mut h = six_max("btn-open", cash(6), 10_000)
        .hero(1, "AhQh")
        .fold(4)
        .fold(5)
        .fold(6)
        .raise(1, 250)
        .fold(2)
        .call(3)
        .board("Kd7c2s")
        .check(3)
        .bet(1, 300)
        .fold(3)
        .build();
    h.meta.played_at_ms = at;
    h
}

#[test]
fn net_results_by_position_format_and_time() {
    // 1. +300 au bouton (cash 6-max, BB 100), joué en dernier.
    let a = btn_open(Some(3_000));
    // 2. +80 au bouton en Expresso (BB 20), joué en premier.
    let b = HandBuilder::new("limp", GameFormat::EXPRESSO, Blinds::new(10, 20), 1)
        .seat(1, "Hero", 500)
        .seat(2, "Sb", 500)
        .seat(3, "Bb", 500)
        .hero(1, "KhTh")
        .played_at(1_000)
        .call(1)
        .call(2)
        .check(3)
        .board("Ks9d4c")
        .check(2)
        .bet(3, 40)
        .call(1)
        .fold(2)
        .board("2h")
        .check(3)
        .check(1)
        .board("7s")
        .check(3)
        .check(1)
        .show(3, "QhJh")
        .build();
    // 3. Le héros (SB) se couche ; l'abattage qui suit est non résolu, mais sa perte est
    //    connue : la petite blinde. Pas de date : en fin de courbe.
    let c = six_max("sb-fold", cash(6), 10_000)
        .hero(2, "8c4d")
        .fold(4)
        .fold(5)
        .raise(6, 300)
        .fold(1)
        .fold(2)
        .call(3)
        .board("Kd7c2s")
        .check(3)
        .check(6)
        .board("9h")
        .check(3)
        .check(6)
        .board("3c")
        .check(3)
        .check(6)
        .build();
    // 4. Le héros va à l'abattage sans que l'on voie les cartes adverses : non résolue.
    let d = six_max("unresolved", cash(6), 10_000)
        .hero(6, "QdJd")
        .played_at(2_000)
        .fold(4)
        .fold(5)
        .raise(6, 300)
        .fold(1)
        .fold(2)
        .call(3)
        .board("Kd7c2s")
        .check(3)
        .check(6)
        .board("9h")
        .check(3)
        .check(6)
        .board("3c")
        .check(3)
        .check(6)
        .build();

    let mut results = HeroResults::default();
    for h in [&a, &b, &c, &d] {
        results.add_hand(h).unwrap();
    }
    assert!(matches!(
        results.add_hand(&a),
        Err(AnalyticsError::DuplicateHand(_))
    ));
    let mut anonymous = a.clone();
    anonymous.id = "anonymous".into();
    anonymous.meta.hero_seat = None;
    assert_eq!(
        results.add_hand(&anonymous),
        Err(AnalyticsError::NoHero("anonymous".into()))
    );

    let s = results.summary();
    assert_eq!(s.unresolved_hands, 1);
    assert_eq!(s.all_in_unknown_cards, 0);
    assert_eq!(s.overall.hands, 3);
    assert_eq!(s.overall.net_chips, 300 + 80 - 50);
    assert!(close(s.overall.net_bb, 3.0 + 4.0 - 0.5));
    assert!(close(s.overall.bb_per_100().unwrap(), 650.0 / 3.0));
    assert!(close(s.overall.adjusted_net_bb, s.overall.net_bb));

    let btn = &s.by_position[&Position::Button];
    assert_eq!((btn.hands, btn.net_chips), (2, 380));
    assert!(close(btn.net_bb, 7.0));
    let sb = &s.by_position[&Position::SmallBlind];
    assert_eq!((sb.hands, sb.net_chips), (1, -50));
    assert!(close(sb.bb_per_100().unwrap(), -50.0));

    assert_eq!(s.by_format.len(), 2);
    let cash6 = s.by_format.iter().find(|f| f.format == cash(6)).unwrap();
    assert_eq!((cash6.overall.hands, cash6.overall.net_chips), (2, 250));
    assert_eq!(cash6.by_position.len(), 2);
    let expresso = s
        .by_format
        .iter()
        .find(|f| f.format == GameFormat::EXPRESSO)
        .unwrap();
    assert!(close(expresso.overall.net_bb, 4.0));

    // Courbe : ordre chronologique, mains sans date à la fin.
    let order: Vec<&str> = s.curve.iter().map(|p| p.hand_id.as_str()).collect();
    assert_eq!(order, vec!["limp", "btn-open", "sb-fold"]);
    let cumul: Vec<i64> = s.curve.iter().map(|p| p.cumulative_chips).collect();
    assert_eq!(cumul, vec![80, 380, 330]);
    assert!(close(s.curve[2].cumulative_bb, 6.5));

    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"curve\""));
}

#[test]
fn all_in_adjusted_heads_up_preflop() {
    // As contre Rois, tapis préflop : les Rois touchent un brelan.
    let hand = HandBuilder::new("aa-kk", cash(2), Blinds::new(50, 100), 1)
        .seat(1, "Hero", 10_000)
        .seat(2, "Villain", 10_000)
        .hero(1, "AhAs")
        .raise(1, 300)
        .all_in(2)
        .call(1)
        .board("Kc7d2s")
        .board("3h")
        .board("9c")
        .show(2, "KsKd")
        .build();
    let mut results = HeroResults::default();
    results.add_hand(&hand).unwrap();
    let r = &results.hands()[0];
    assert_eq!(r.net_chips, -10_000);
    let ev = r.all_in.as_ref().expect("EV à l'all-in calculée");
    assert_eq!(ev.street, Street::Preflop);
    assert_eq!(ev.pots.len(), 1);
    let eq = exact_equity(&["AhAs", "KsKd"], "");
    assert!((0.80..0.84).contains(&eq), "{eq}");
    assert!(close(ev.pots[0].equity, eq));
    assert!(close(ev.expected_net, eq * 20_000.0 - 10_000.0));

    let s = results.summary();
    assert_eq!(s.overall.all_in_adjusted_hands, 1);
    assert!(close(s.overall.net_bb, -100.0));
    assert!(close(
        s.overall.adjusted_net_bb,
        (eq * 20_000.0 - 10_000.0) / 100.0
    ));
    assert!(s.overall.adjusted_bb_per_100().unwrap() > 0.0);
}

#[test]
fn all_in_adjusted_with_side_pot() {
    // Héros court au bouton (300) : il n'est éligible qu'au pot principal (900).
    let hand = HandBuilder::new("side-pot", GameFormat::EXPRESSO, Blinds::new(50, 100), 1)
        .seat(1, "Hero", 300)
        .seat(2, "Sb", 1_000)
        .seat(3, "Bb", 600)
        .hero(1, "7c7h")
        .all_in(1)
        .all_in(2)
        .call(3)
        .board("2c7d9h")
        .board("Js")
        .board("3d")
        .show(2, "AsAh")
        .show(3, "KsKh")
        .build();
    let mut results = HeroResults::default();
    results.add_hand(&hand).unwrap();
    let r = &results.hands()[0];
    assert_eq!(r.net_chips, 600);
    let ev = r.all_in.as_ref().unwrap();
    assert_eq!(ev.pots.len(), 1);
    assert_eq!((ev.pots[0].amount.get(), ev.pots[0].players), (900, 3));
    let eq = exact_equity(&["7c7h", "AsAh", "KsKh"], "");
    assert!(close(ev.pots[0].equity, eq));
    assert!(close(ev.expected_net, eq * 900.0 - 300.0));
}

#[test]
fn all_in_on_the_flop_and_unknown_cards() {
    let hu = |id: &str| {
        HandBuilder::new(id, cash(2), Blinds::new(50, 100), 1)
            .seat(1, "Hero", 5_000)
            .seat(2, "Villain", 5_000)
            .hero(1, "7h7d")
    };
    // Tapis au flop, cartes adverses montrées : équité sur le board du flop.
    let flop = hu("flop-all-in")
        .call(1)
        .check(2)
        .board("Ah7c2d")
        .bet(2, 200)
        .all_in(1)
        .call(2)
        .board("Ad")
        .board("Kh")
        .show(2, "AcKc")
        .build();
    // Tapis préflop, l'adversaire jette ses cartes sans les montrer : le héros gagne, mais
    // l'espérance n'est pas calculable sans inventer de cartes.
    let mucked = hu("mucked")
        .all_in(1)
        .call(2)
        .board("Ah7c2d")
        .board("5d")
        .board("Kh")
        .muck(2)
        .build();

    let mut results = HeroResults::default();
    results.add_hand(&flop).unwrap();
    results.add_hand(&mucked).unwrap();

    let r = &results.hands()[0];
    assert_eq!(r.net_chips, -5_000, "full house adverse à la river");
    let ev = r.all_in.as_ref().unwrap();
    assert_eq!(ev.street, Street::Flop);
    let eq = exact_equity(&["7h7d", "AcKc"], "Ah7c2d");
    assert!(eq > 0.9, "{eq}");
    assert!(close(ev.expected_net, eq * 10_000.0 - 5_000.0));

    let m = &results.hands()[1];
    assert_eq!(m.net_chips, 5_000);
    assert!(m.all_in.is_none());
    assert!(close(m.adjusted_net(), 5_000.0));
    let s = results.summary();
    assert_eq!(s.all_in_unknown_cards, 1);
    assert_eq!(s.overall.all_in_adjusted_hands, 1);
    assert!(close(
        s.overall.adjusted_net_chips,
        eq * 10_000.0 - 5_000.0 + 5_000.0
    ));
}
