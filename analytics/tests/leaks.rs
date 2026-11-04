//! Détection des fuites : bandes par format et position, seuil de fiabilité.

mod common;

use common::six_max;
use stratyx_analytics::leaks::{FormatFilter, LeakDirection, StructureKind};
use stratyx_analytics::{
    Analyzer, LeakConfig, ProfileBook, ReferenceBand, Reliability, StatKind, detect_leaks,
};
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::BettingStructure;
use stratyx_poker::position::Position;

/// `n` mains où le héros (UTG) ouvre et tout le monde se couche.
fn utg_opens(n: usize, format: GameFormat) -> Vec<stratyx_poker::HandRecord> {
    (0..n)
        .map(|i| {
            six_max(&format!("utg-{i}"), format, 10_000)
                .hero(4, "7c2d")
                .raise(4, 250)
                .fold(5)
                .fold(6)
                .fold(1)
                .fold(2)
                .fold(3)
                .build()
        })
        .collect()
}

fn cash6() -> GameFormat {
    GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap()
}

#[test]
fn opening_every_hand_utg_is_flagged() {
    let mut analyzer = Analyzer::new();
    assert!(analyzer.add_hands(&utg_opens(120, cash6())).is_empty());
    let leaks = analyzer.leaks("Utg", &LeakConfig::default());
    let found: Vec<(StatKind, Option<Position>)> =
        leaks.iter().map(|l| (l.stat, l.position)).collect();
    // Triées par écart décroissant (toutes significatives).
    assert_eq!(
        found,
        vec![
            (StatKind::Vpip, Some(Position::UnderTheGun)),
            (StatKind::Pfr, None),
            (StatKind::Vpip, None),
        ]
    );
    let vpip_utg = &leaks[0];
    assert_eq!(vpip_utg.direction, LeakDirection::TooHigh);
    assert!(vpip_utg.significant);
    assert_eq!(vpip_utg.reliability, Reliability::Reliable);
    assert_eq!(vpip_utg.opportunities, 120);
    assert!((vpip_utg.gap - 0.78).abs() < 1e-9);
    assert!(
        vpip_utg.message.contains("VPIP (position UTG"),
        "{}",
        vpip_utg.message
    );
    assert!(vpip_utg.message.contains("100,0 %"), "{}", vpip_utg.message);
    assert!(serde_json::to_string(&leaks).unwrap().contains("too_high"));

    // Les adversaires qui se couchent toujours : VPIP trop bas, mais aucune bande de fold.
    let low = analyzer.leaks("Hj", &LeakConfig::default());
    assert!(low.iter().all(|l| l.direction == LeakDirection::TooLow));
    assert!(
        low.iter()
            .any(|l| l.stat == StatKind::Vpip && l.position.is_none())
    );
}

#[test]
fn reliability_and_format_filters() {
    let mut book = ProfileBook::new();
    for h in utg_opens(50, cash6()) {
        book.add_hand(&h).unwrap();
    }
    let profile = book.profile("Utg").unwrap();
    // 50 opportunités : statistique « indicative », pas assez pour une fuite par défaut.
    assert!(detect_leaks(profile, &LeakConfig::default()).is_empty());
    let lenient = LeakConfig {
        min_reliability: Reliability::Indicative,
        ..LeakConfig::default()
    };
    assert_eq!(detect_leaks(profile, &lenient).len(), 3);

    // Les bandes 6-max ne s'appliquent pas à un autre format.
    let mut book = ProfileBook::new();
    let pot_limit = GameFormat::new(GameType::Tournament, 6, BettingStructure::PotLimit).unwrap();
    for h in utg_opens(120, pot_limit) {
        book.add_hand(&h).unwrap();
    }
    assert!(detect_leaks(book.profile("Utg").unwrap(), &LeakConfig::default()).is_empty());

    // Bande personnalisée, sérialisable, sur ce format : le CO ne 3-bet jamais l'ouverture.
    let band = ReferenceBand::new(
        StatKind::ThreeBet,
        FormatFilter {
            game_type: Some(GameType::Tournament),
            table_size: None,
            structure: Some(StructureKind::PotLimit),
        },
        Some(Position::Cutoff),
        0.03,
        0.15,
    );
    let config = LeakConfig::with_bands(vec![band]);
    let config: LeakConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    let leaks = detect_leaks(book.profile("Co").unwrap(), &config);
    assert_eq!(leaks.len(), 1);
    assert_eq!(leaks[0].direction, LeakDirection::TooLow);
    assert_eq!(leaks[0].rate, 0.0);
    // 0 sur 120 : la borne haute de Wilson (≈ 3,1 %) dépasse encore la borne de 3 % ;
    // l'écart est signalé mais n'est pas statistiquement établi.
    assert!(!leaks[0].significant);
    assert!(leaks[0].interval.high > 0.03 && leaks[0].interval.high < 0.032);
}
