//! Profils : chaque opportunité et chaque occurrence vérifiées sur des mains construites.

mod common;

use common::{HandBuilder, six_max};
use stratyx_analytics::profile::{PositionalStats, StatKind};
use stratyx_analytics::{
    AnalyticsError, Analyzer, ProfileBook, ReliabilityThresholds, Stat, StatLine,
};
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::{BettingStructure, Blinds};
use stratyx_poker::position::Position;

fn cash6() -> GameFormat {
    GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap()
}

/// Vérifie les compteurs `(occurrences, opportunités)` attendus ; toutes les autres
/// statistiques doivent être vides.
#[track_caller]
fn expect(line: &StatLine, expected: &[(StatKind, u64, u64)]) {
    for kind in StatKind::ALL {
        let want = expected
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map_or(Stat::default(), |(_, occ, opp)| Stat::new(*occ, *opp));
        assert_eq!(line.get(kind), want, "{kind:?}");
    }
}

fn overall<'a>(book: &'a ProfileBook, player: &str) -> &'a StatLine {
    &book.profile(player).unwrap().stats.overall
}

/// Le héros ouvre au bouton, la petite blinde se couche, la grosse blinde défend ;
/// c-bet au flop, la grosse blinde abandonne.
fn btn_open_bb_defends() -> stratyx_poker::HandRecord {
    six_max("btn-open", cash6(), 10_000)
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
        .build()
}

#[test]
fn button_open_and_big_blind_defense() {
    use StatKind::*;
    let mut book = ProfileBook::new();
    book.add_hand(&btn_open_bb_defends()).unwrap();

    for early in ["Utg", "Hj"] {
        expect(
            overall(&book, early),
            &[(Vpip, 0, 1), (Pfr, 0, 1), (Limp, 0, 1)],
        );
    }
    expect(
        overall(&book, "Co"),
        &[(Vpip, 0, 1), (Pfr, 0, 1), (Limp, 0, 1), (Steal, 0, 1)],
    );
    let btn = overall(&book, "Btn");
    expect(
        btn,
        &[
            (Vpip, 1, 1),
            (Pfr, 1, 1),
            (Limp, 0, 1),
            (Steal, 1, 1),
            (CbetFlop, 1, 1),
            (AggressionFrequency, 1, 1),
            (WentToShowdown, 0, 1),
        ],
    );
    assert_eq!(btn.postflop.bets, 1);
    assert_eq!(btn.aggression_factor(), None, "aucun call : AF indéfini");
    expect(
        overall(&book, "Sb"),
        &[
            (Vpip, 0, 1),
            (Pfr, 0, 1),
            (ThreeBet, 0, 1),
            (FoldToSteal, 1, 1),
        ],
    );
    let bb = overall(&book, "Bb");
    expect(
        bb,
        &[
            (Vpip, 1, 1),
            (Pfr, 0, 1),
            (ThreeBet, 0, 1),
            (FoldToSteal, 0, 1),
            (BigBlindDefense, 1, 1),
            (FoldToCbetFlop, 1, 1),
            (AggressionFrequency, 0, 1),
            (WentToShowdown, 0, 1),
        ],
    );
    assert_eq!((bb.postflop.checks, bb.postflop.folds), (1, 1));

    // Détail par position et par format.
    let hero = book.profile("Btn").unwrap();
    assert_eq!(hero.hands(), 1);
    assert_eq!(
        hero.stats.by_position.keys().copied().collect::<Vec<_>>(),
        vec![Position::Button]
    );
    assert_eq!(hero.by_format.len(), 1);
    assert_eq!(hero.format(&cash6()).unwrap(), &hero.stats);

    // Confrontations : le héros gagne la défense de la BB (250) et la SB (50).
    let vs_bb = book.head_to_head("Bb").unwrap();
    assert_eq!(
        (
            vs_bb.hands_together,
            vs_bb.flops_together,
            vs_bb.showdowns_together
        ),
        (1, 1, 0)
    );
    assert_eq!((vs_bb.settled_hands, vs_bb.net_chips), (1, 250));
    assert!((vs_bb.net_bb - 2.5).abs() < 1e-12);
    assert_eq!(book.head_to_head("Sb").unwrap().net_chips, 50);
    assert_eq!(book.head_to_head("Utg").unwrap().net_chips, 0);
    assert!(
        book.head_to_head("Btn").is_none(),
        "pas de confrontation avec soi-même"
    );
    assert_eq!(book.head_to_heads().count(), 5);

    // Même main deux fois : refusée.
    assert_eq!(
        book.add_hand(&btn_open_bb_defends()),
        Err(AnalyticsError::DuplicateHand("btn-open".into()))
    );
}

#[test]
fn three_bet_four_bet_and_cold_call() {
    use StatKind::*;
    // UTG ouvre, HJ suit (cold call), CO 3-bet, tout le monde se couche sauf HJ qui paie.
    // Flop checké ; HJ mise le turn, CO relance, HJ paie ; river : CO mise, HJ se couche.
    let hand = six_max("3bet", cash6(), 10_000)
        .hero(5, "9s9d")
        .raise(4, 300)
        .call(5)
        .raise(6, 1_000)
        .fold(1)
        .fold(2)
        .fold(3)
        .fold(4)
        .call(5)
        .board("8h7d2c")
        .check(5)
        .check(6)
        .board("Jc")
        .bet(5, 1_500)
        .raise(6, 4_000)
        .call(5)
        .board("3s")
        .check(5)
        .bet(6, 3_000)
        .fold(5)
        .build();
    let mut book = ProfileBook::new();
    book.add_hand(&hand).unwrap();

    expect(
        overall(&book, "Utg"),
        &[
            (Vpip, 1, 1),
            (Pfr, 1, 1),
            (Limp, 0, 1),
            (FoldToThreeBet, 1, 1),
            (FourBet, 0, 1),
        ],
    );
    let hj = overall(&book, "Hj");
    expect(
        hj,
        &[
            (Vpip, 1, 1),
            (Pfr, 0, 1),
            (ColdCall, 1, 1),
            (ThreeBet, 0, 1),
            (FourBet, 0, 1),
            (AggressionFrequency, 1, 3),
            (WentToShowdown, 0, 1),
        ],
    );
    assert_eq!(hj.aggression_factor(), Some(1.0));
    let co = overall(&book, "Co");
    expect(
        co,
        &[
            (Vpip, 1, 1),
            (Pfr, 1, 1),
            (ColdCall, 0, 1),
            (ThreeBet, 1, 1),
            (CbetFlop, 0, 1),
            (AggressionFrequency, 2, 2),
            (WentToShowdown, 0, 1),
        ],
    );
    assert_eq!(
        (co.postflop.bets, co.postflop.raises, co.postflop.checks),
        (1, 1, 1)
    );
    expect(
        overall(&book, "Btn"),
        &[(Vpip, 0, 1), (Pfr, 0, 1), (ColdCall, 0, 1), (FourBet, 0, 1)],
    );
    for blind in ["Sb", "Bb"] {
        expect(
            overall(&book, blind),
            &[(Vpip, 0, 1), (Pfr, 0, 1), (FourBet, 0, 1)],
        );
    }
    // HJ (héros) perd 5 000 jetons, tous au profit du CO.
    assert_eq!(book.head_to_head("Co").unwrap().net_chips, -5_000);
    assert_eq!(book.head_to_head("Utg").unwrap().net_chips, 0);
}

#[test]
fn turn_cbet_and_fold_to_turn_cbet() {
    use StatKind::*;
    let hand = six_max("turn-cbet", cash6(), 10_000)
        .fold(4)
        .fold(5)
        .fold(6)
        .raise(1, 250)
        .fold(2)
        .call(3)
        .board("Kd7c2s")
        .check(3)
        .bet(1, 300)
        .call(3)
        .board("5h")
        .check(3)
        .bet(1, 800)
        .fold(3)
        .build();
    let mut book = ProfileBook::new();
    book.add_hand(&hand).unwrap();
    let btn = overall(&book, "Btn");
    assert_eq!(btn.cbet_flop, Stat::new(1, 1));
    assert_eq!(btn.cbet_turn, Stat::new(1, 1));
    let bb = overall(&book, "Bb");
    assert_eq!(bb.fold_to_cbet_flop, Stat::new(0, 1));
    assert_eq!(bb.fold_to_cbet_turn, Stat::new(1, 1));
    assert_eq!(bb.aggression_factor(), Some(0.0));
    assert_eq!(bb.get(AggressionFrequency), Stat::new(0, 2));

    // Un c-bet flop relancé ne donne pas d'opportunité de c-bet au turn.
    let raised = six_max("cbet-raised", cash6(), 10_000)
        .fold(4)
        .fold(5)
        .fold(6)
        .raise(1, 250)
        .fold(2)
        .call(3)
        .board("Kd7c2s")
        .check(3)
        .bet(1, 300)
        .raise(3, 900)
        .call(1)
        .board("5h")
        .check(3)
        .check(1)
        .build();
    let mut book = ProfileBook::new();
    book.add_hand(&raised).unwrap();
    assert_eq!(overall(&book, "Btn").cbet_turn, Stat::default());
    assert_eq!(overall(&book, "Bb").fold_to_cbet_flop, Stat::new(0, 1));
}

#[test]
fn limped_pot_to_showdown() {
    use StatKind::*;
    // Expresso : le bouton (héros) limpe, la SB complète, la BB checke.
    let hand = HandBuilder::new("limp", GameFormat::EXPRESSO, Blinds::new(10, 20), 1)
        .seat(1, "Hero", 500)
        .seat(2, "Sb", 500)
        .seat(3, "Bb", 500)
        .hero(1, "KhTh")
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
    let mut book = ProfileBook::new();
    book.add_hand(&hand).unwrap();
    let hero = overall(&book, "Hero");
    expect(
        hero,
        &[
            (Vpip, 1, 1),
            (Pfr, 0, 1),
            (Limp, 1, 1),
            (Steal, 0, 1),
            (AggressionFrequency, 0, 1),
            (WentToShowdown, 1, 1),
            (WonAtShowdown, 1, 1),
        ],
    );
    assert_eq!(hero.aggression_factor(), Some(0.0));
    expect(
        overall(&book, "Sb"),
        &[
            (Vpip, 1, 1),
            (Pfr, 0, 1),
            (Limp, 1, 1),
            (AggressionFrequency, 0, 1),
            (WentToShowdown, 0, 1),
        ],
    );
    expect(
        overall(&book, "Bb"),
        &[
            (Vpip, 0, 1),
            (Pfr, 0, 1),
            (AggressionFrequency, 1, 1),
            (WentToShowdown, 1, 1),
            (WonAtShowdown, 0, 1),
        ],
    );
    let vs_bb = book.head_to_head("Bb").unwrap();
    assert_eq!((vs_bb.showdowns_together, vs_bb.net_chips), (1, 60));
    assert_eq!(book.head_to_head("Sb").unwrap().net_chips, 20);
}

#[test]
fn heads_up_steal_walk_and_unknown_cards() {
    use StatKind::*;
    let hu = GameFormat::new(GameType::CashGame, 2, BettingStructure::NoLimit).unwrap();
    let base = |id: &str| {
        HandBuilder::new(id, hu, Blinds::new(50, 100), 1)
            .seat(1, "Hero", 10_000)
            .seat(2, "Villain", 10_000)
            .hero(1, "As5s")
    };
    // 1. Vol réussi : la BB se couche.
    let steal = base("hu-steal").raise(1, 250).fold(2).build();
    // 2. Le bouton se couche : la BB n'a aucune décision (« walk »).
    let walk = base("hu-walk").fold(1).build();
    // 3. Abattage sans les cartes adverses : main non résolue.
    let unresolved = base("hu-unresolved")
        .call(1)
        .check(2)
        .board("Kd7c2s")
        .check(2)
        .check(1)
        .board("9h")
        .check(2)
        .check(1)
        .board("3c")
        .check(2)
        .check(1)
        .build();
    // 4. Vidéo coupée au flop : main incomplète.
    let incomplete = base("hu-incomplete")
        .call(1)
        .check(2)
        .board("Kd7c2s")
        .check(2)
        .build();

    let mut book = ProfileBook::new();
    for h in [&steal, &walk, &unresolved, &incomplete] {
        book.add_hand(h).unwrap();
    }
    assert_eq!(book.hand_count(), 4);
    expect(
        overall(&book, "Hero"),
        &[
            // Vol, limp, limp ; le fold du walk compte comme opportunité.
            (Vpip, 3, 4),
            (Pfr, 1, 4),
            (Limp, 2, 4),
            (Steal, 1, 4),
            (AggressionFrequency, 0, 0),
            // Seule la main menée à son terme compte ; non résolue : pas de W$SD.
            (WentToShowdown, 1, 1),
        ],
    );
    let villain = overall(&book, "Villain");
    expect(
        villain,
        &[
            // Le walk n'est pas une opportunité de VPIP.
            (Vpip, 0, 3),
            (Pfr, 0, 3),
            (ThreeBet, 0, 1),
            (FoldToSteal, 1, 1),
            (BigBlindDefense, 0, 1),
            (WentToShowdown, 1, 1),
        ],
    );
    assert_eq!(villain.hands, 4);
    assert_eq!(villain.postflop.checks, 4);

    let h2h = book.head_to_head("Villain").unwrap();
    assert_eq!(h2h.hands_together, 4);
    assert_eq!(h2h.flops_together, 2);
    assert_eq!(h2h.showdowns_together, 1);
    assert_eq!(h2h.settled_hands, 2, "steal et walk seulement");
    assert_eq!(h2h.net_chips, 100 - 50);
}

#[test]
fn incremental_equals_batch_and_survives_serialization() {
    let hands = vec![
        btn_open_bb_defends(),
        six_max("h2", cash6(), 10_000)
            .hero(1, "2c2d")
            .raise(4, 300)
            .fold(5)
            .fold(6)
            .call(1)
            .fold(2)
            .fold(3)
            .board("Kd7c2s")
            .bet(4, 400)
            .call(1)
            .board("2h")
            .check(4)
            .bet(1, 1_000)
            .fold(4)
            .build(),
    ];
    let mut batch = Analyzer::new();
    assert!(batch.add_hands(&hands).is_empty());

    let mut first = Analyzer::new();
    first.add_hand(&hands[0]).unwrap();
    let json = serde_json::to_string(&first).unwrap();
    let mut resumed: Analyzer = serde_json::from_str(&json).unwrap();
    resumed.add_hand(&hands[1]).unwrap();
    assert!(matches!(
        resumed.add_hand(&hands[0]),
        Err(AnalyticsError::DuplicateHand(_))
    ));

    for p in batch.profiles().profiles() {
        assert_eq!(resumed.profiles().profile(&p.player), Some(p));
    }
    assert_eq!(
        resumed.results().summary().overall.net_chips,
        batch.results().summary().overall.net_chips
    );

    // Le cumul par position redonne le total.
    let hero = batch.profiles().profile("Btn").unwrap();
    let mut sum = StatLine::default();
    for line in hero.stats.by_position.values() {
        sum.merge(line);
    }
    assert_eq!(sum, hero.stats.overall);
    assert_eq!(hero.stats.overall.cold_call, Stat::new(1, 1));

    // Rapport sérialisable : les clés de position et de statistique sont lisibles.
    let report = hero.report(&ReliabilityThresholds::default());
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"BTN\""), "{json}");
    assert!(json.contains("\"vpip\""));
    let back: stratyx_analytics::profile::ProfileReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back.hands, 2);
    let stats: PositionalStats =
        serde_json::from_str(&serde_json::to_string(&hero.stats).unwrap()).unwrap();
    assert_eq!(stats, hero.stats);
}

#[test]
fn invalid_records_are_reported() {
    let mut bad = btn_open_bb_defends();
    bad.events.swap(0, 1); // HJ parle avant UTG
    let mut book = ProfileBook::new();
    let err = book.add_hand(&bad).unwrap_err();
    assert!(matches!(err, AnalyticsError::InvalidRecord { .. }));
    assert!(err.to_string().contains("historique invalide"), "{err}");
    assert_eq!(book.hand_count(), 0);

    let mut no_hero_seat = btn_open_bb_defends();
    no_hero_seat.meta.hero_seat = Some(9);
    assert!(matches!(
        book.add_hand(&no_hero_seat),
        Err(AnalyticsError::SeatNotFound { seat: 9, .. })
    ));
}
