//! Import et export des historiques texte PokerStars : exemples écrits à la main (validés par
//! rejeu), allers-retours export → import, et erreurs localisées.

use stratyx_database::Fingerprint;
use stratyx_database::text::{
    BuyIn, ExportOptions, ImportErrorKind, ImportOptions, ImportedHand, TournamentInfo,
    export_pokerstars, export_pokerstars_many, import_pokerstars, import_pokerstars_hand,
};
use stratyx_poker::card::parse_cards;
use stratyx_poker::game::{Ante, BettingStructure, Blinds};
use stratyx_poker::history::Outcome;
use stratyx_poker::{Action, GameFormat, GameType, HandEvent, HandMeta, HandRecord, SeatRecord};
use stratyx_shared::{Chips, Confidence, Observed, Source};

const CASH: &str = include_str!("data/pokerstars_cash_nl6.txt");
const TOURNAMENT: &str = include_str!("data/pokerstars_tournament_antes.txt");
const FIXED_LIMIT: &str = include_str!("data/pokerstars_fixed_limit.txt");

fn import_all(text: &str) -> Vec<ImportedHand> {
    import_pokerstars(text, &ImportOptions::default())
        .into_iter()
        .map(|r| r.unwrap_or_else(|e| panic!("{e}")))
        .collect()
}

/// Résultat net par siège, d'après le moteur.
fn net(record: &HandRecord) -> Vec<(u8, i64)> {
    let r = record.replay().expect("rejeu");
    let Outcome::Settled(s) = r.outcome else {
        panic!("main non résolue : {:?}", r.outcome)
    };
    r.seats.iter().copied().zip(s.net).collect()
}

fn ok<T>(v: T) -> Observed<T> {
    Observed::certain(v, Source::Import)
}

fn seat(n: u8, name: &str, stack: u64, hole: Option<&str>) -> SeatRecord {
    SeatRecord {
        seat: n,
        player: name.into(),
        stack: ok(Chips(stack)),
        hole_cards: hole.map(|h| ok(h.parse().unwrap())),
    }
}

fn act(seat: u8, action: Action) -> Observed<HandEvent> {
    ok(HandEvent::Action { seat, action })
}

fn raise(seat: u8, to: u64) -> Observed<HandEvent> {
    act(seat, Action::Raise(Chips(to)))
}

fn bet(seat: u8, to: u64) -> Observed<HandEvent> {
    act(seat, Action::Bet(Chips(to)))
}

fn board(cards: &str) -> Observed<HandEvent> {
    ok(HandEvent::Board {
        cards: parse_cards(cards).unwrap(),
    })
}

fn show(seat: u8, cards: &str) -> Observed<HandEvent> {
    ok(HandEvent::Show {
        seat,
        cards: cards.parse().unwrap(),
    })
}

fn meta(hero: Option<u8>, played_at_ms: Option<u64>) -> HandMeta {
    HandMeta {
        hero_seat: hero,
        played_at_ms,
        session_id: None,
        site: Some("pokerstars".into()),
        table: Some("Stratyx 1".into()),
        provenance: None,
    }
}

/// Export puis import : l'historique relu est identique.
fn roundtrip(record: &HandRecord, export: &ExportOptions, import: &ImportOptions) -> ImportedHand {
    let text = export_pokerstars(record, export).unwrap();
    let back = import_pokerstars_hand(&text, import).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(back.record, *record, "\n{text}");
    assert!(back.warnings.is_empty(), "{:?}\n{text}", back.warnings);
    back
}

// --- Historiques écrits à la main --------------------------------------------------------

#[test]
fn cash_game_nl_6max() {
    let hands = import_all(CASH);
    assert_eq!(hands.len(), 2);
    for h in &hands {
        assert!(h.warnings.is_empty(), "{:?}", h.warnings);
        assert_eq!(h.currency.as_deref(), Some("USD"));
        assert!(h.tournament.is_none());
    }

    let a = &hands[0];
    let r = &a.record;
    assert_eq!(r.id, "254372383936");
    assert_eq!(
        r.format,
        GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap()
    );
    assert_eq!(r.blinds.value, Blinds::new(5, 10));
    assert_eq!(r.blinds.source, Source::Import);
    assert_eq!(r.blinds.confidence, Confidence::CERTAIN);
    assert_eq!(r.button_seat, 4);
    assert_eq!(r.seats.len(), 6);
    assert_eq!(r.seats[0].stack.value, Chips(1037)); // centimes
    assert_eq!(r.seats[1].player, "Nordic Fox");
    assert_eq!(r.meta.hero_seat, Some(3));
    assert_eq!(r.meta.site.as_deref(), Some("pokerstars"));
    assert_eq!(r.meta.table.as_deref(), Some("Aase III"));
    // 2024-12-01 21:15:03 CET = 20:15:03 UTC.
    assert_eq!(r.meta.played_at_ms, Some(1_733_084_103_000));
    assert_eq!(
        r.seats[2].hole_cards.as_ref().map(|h| h.value),
        Some("AhKd".parse().unwrap())
    );
    assert_eq!(a.rake, Some(Chips(10)));
    // Le héros gagne la relance de Nordic Fox (1 $) et les blindes.
    assert!(net(r).contains(&(3, 115)));
    assert!(net(r).contains(&(2, -100)));

    let b = &hands[1].record;
    assert_eq!(b.button_seat, 5);
    let n = net(b);
    // Le héros jette à l'abattage : il perd tout ce qu'il a misé, maxi_m gagne le pot
    // (le moteur ne prélève pas de râteau).
    assert!(n.contains(&(3, -407)));
    assert!(n.contains(&(5, 849 - 407)));
    assert!(matches!(
        b.events.last().map(|e| &e.value),
        Some(HandEvent::Muck { seat: 3 })
    ));
}

#[test]
fn tournament_with_antes_and_side_pots() {
    let hands = import_all(TOURNAMENT);
    assert_eq!(hands.len(), 1);
    let h = &hands[0];
    assert!(h.warnings.is_empty(), "{:?}", h.warnings);
    assert_eq!(
        h.tournament,
        Some(TournamentInfo {
            id: "3812345678".into(),
            buy_in: Some(BuyIn {
                total: 150,
                fee: 10,
                currency: Some("USD".into()),
            }),
            level: Some("V".into()),
        })
    );
    assert_eq!(h.currency, None);
    let r = &h.record;
    assert_eq!(
        r.format,
        GameFormat::new(GameType::Tournament, 9, BettingStructure::NoLimit).unwrap()
    );
    assert_eq!(
        r.blinds.value,
        Blinds::new(50, 100).with_ante(Ante::Classic(Chips(10)))
    );
    // Le joueur absent a payé son ante et s'est couché : il est servi.
    assert_eq!(r.seats.len(), 7);
    assert!(r.seats.iter().any(|s| s.player == "rivertrash"));
    assert_eq!(r.meta.table.as_deref(), Some("3812345678 12"));

    let replay = r.replay().unwrap();
    let Outcome::Settled(s) = &replay.outcome else {
        panic!("main non résolue")
    };
    // Trois pots : principal (4 joueurs), secondaire 1 (3), secondaire 2 (2).
    let amounts: Vec<Chips> = s.pots.pots.iter().map(|p| p.amount).collect();
    assert_eq!(amounts, vec![Chips(2200), Chips(2460), Chips(2700)]);
    let n = net(r);
    assert!(n.contains(&(1, 2200 - 480)));
    assert!(n.contains(&(4, 2460 - 1300)));
    assert!(n.contains(&(3, 2700 - 2650)));
    assert!(n.contains(&(2, -2650)));
    assert_eq!(n.iter().map(|(_, v)| v).sum::<i64>(), 0);
}

#[test]
fn fixed_limit_full_ring() {
    let hands = import_all(FIXED_LIMIT);
    let h = &hands[0];
    assert!(h.warnings.is_empty(), "{:?}", h.warnings);
    let r = &h.record;
    assert_eq!(
        r.format,
        GameFormat::new(GameType::CashGame, 9, BettingStructure::FIXED_LIMIT_CASH).unwrap()
    );
    // En-tête : petite et grosse mise (0,10 / 0,20 $) ; petite blinde lue sur sa ligne.
    assert_eq!(r.blinds.value, Blinds::new(5, 10));
    assert_eq!(r.seats.len(), 7);
    assert_eq!(h.rake, Some(Chips(15)));
    assert!(net(r).contains(&(3, 345 - 150)));
}

#[test]
fn imported_hands_survive_export_and_reimport() {
    for (text, export) in [
        (
            CASH,
            ExportOptions {
                currency: Some("USD".into()),
                tournament: None,
            },
        ),
        (TOURNAMENT, ExportOptions::default()),
        (
            FIXED_LIMIT,
            ExportOptions {
                currency: Some("USD".into()),
                tournament: None,
            },
        ),
    ] {
        for h in import_all(text) {
            let export = ExportOptions {
                tournament: h.tournament.clone(),
                ..export.clone()
            };
            let back = roundtrip(&h.record, &export, &ImportOptions::default());
            assert_eq!(back.tournament, h.tournament);
            assert_eq!(back.currency, h.currency);
        }
    }
}

#[test]
fn several_hands_in_one_file() {
    let records: Vec<HandRecord> = import_all(CASH).into_iter().map(|h| h.record).collect();
    let options = ExportOptions {
        currency: Some("USD".into()),
        tournament: None,
    };
    let text = export_pokerstars_many(&records, &options).unwrap();
    let back: Vec<HandRecord> = import_all(&text).into_iter().map(|h| h.record).collect();
    assert_eq!(back, records);
}

// --- Allers-retours sur des mains construites ---------------------------------------------

#[test]
fn roundtrip_expresso_push_and_showdown() {
    let record = HandRecord {
        id: "exp-0001".into(),
        format: GameFormat::EXPRESSO,
        blinds: ok(Blinds::new(10, 20)),
        button_seat: 1,
        seats: vec![
            seat(1, "Hero", 500, Some("AhKh")),
            seat(2, "Villain1", 500, None),
            seat(3, "Villain2", 500, None),
        ],
        events: vec![
            raise(1, 500),
            act(2, Action::Fold),
            act(3, Action::Call),
            board("Kd7c2s"),
            board("9h"),
            board("3c"),
            show(3, "QsQd"),
        ],
        meta: meta(Some(1), Some(1_733_084_103_000)),
    };
    let expresso = ImportOptions {
        tournament_type: GameType::Expresso,
        ..ImportOptions::default()
    };
    let text = export_pokerstars(&record, &ExportOptions::default()).unwrap();
    assert!(
        text.contains("Hero: raises 480 to 500 and is all-in"),
        "{text}"
    );
    assert!(text.contains("Villain2: calls 480 and is all-in"), "{text}");
    assert!(text.contains("*** SHOW DOWN ***"), "{text}");
    assert!(
        text.contains("Villain2: shows [Qs Qd] (a pair of Queens)"),
        "{text}"
    );
    assert!(text.contains("Hero collected 1010 from pot"), "{text}");
    roundtrip(&record, &ExportOptions::default(), &expresso);

    // Sans l'option, la main est relue comme un tournoi 3-max.
    let plain = import_pokerstars_hand(&text, &ImportOptions::default()).unwrap();
    assert_eq!(plain.record.format.game_type, GameType::Tournament);

    // Un tapis noté `AllIn` (vision) devient une relance explicite, de même empreinte.
    let mut vision = record.clone();
    vision.events[0] = act(1, Action::AllIn);
    let text = export_pokerstars(&vision, &ExportOptions::default()).unwrap();
    let back = import_pokerstars_hand(&text, &expresso).unwrap().record;
    assert_ne!(back, vision);
    assert_eq!(Fingerprint::of(&back), Fingerprint::of(&vision));
}

#[test]
fn roundtrip_pot_limit_cash_in_euros_with_uncalled_bet() {
    let record = HandRecord {
        id: "pl-42".into(),
        format: GameFormat::new(GameType::CashGame, 6, BettingStructure::PotLimit).unwrap(),
        blinds: ok(Blinds::new(5, 10)),
        button_seat: 6,
        seats: vec![
            seat(1, "Sb", 1000, None),
            seat(2, "Bb", 1000, None),
            seat(3, "Hero", 1000, Some("JcJd")),
            seat(4, "Mp", 1000, None),
            seat(5, "Co", 1000, None),
            seat(6, "Btn", 2500, None),
        ],
        events: vec![
            raise(3, 35), // relance au pot
            act(4, Action::Fold),
            act(5, Action::Fold),
            act(6, Action::Call),
            act(1, Action::Fold),
            act(2, Action::Fold),
            board("Td6s2c"),
            bet(3, 50),
            raise(6, 185),
            act(3, Action::Fold),
        ],
        meta: meta(Some(3), Some(1_719_835_200_000)),
    };
    let options = ExportOptions {
        currency: Some("EUR".into()),
        tournament: None,
    };
    let text = export_pokerstars(&record, &options).unwrap();
    assert!(
        text.contains("Hold'em Pot Limit (€0.05/€0.10 EUR)"),
        "{text}"
    );
    assert!(
        text.contains("Uncalled bet (€1.35) returned to Btn"),
        "{text}"
    );
    assert!(text.contains("[2024/07/01 08:00:00 ET]"), "{text}");
    let back = roundtrip(&record, &options, &ImportOptions::default());
    assert_eq!(back.currency.as_deref(), Some("EUR"));
}

#[test]
fn roundtrip_big_blind_ante_tournament_with_header() {
    let record = HandRecord {
        id: "77001".into(),
        format: GameFormat::TOURNAMENT_6MAX,
        blinds: ok(Blinds::new(50, 100).with_ante(Ante::BigBlind(Chips(100)))),
        button_seat: 2,
        seats: vec![
            seat(1, "A", 3000, None),
            seat(2, "B", 3000, None),
            seat(3, "C", 3000, None),
            seat(4, "D", 3000, None),
            seat(5, "Hero", 3000, Some("7c2d")),
            seat(6, "F", 3000, None),
        ],
        events: vec![
            act(5, Action::Fold),
            raise(6, 250),
            act(1, Action::Fold),
            act(2, Action::Fold),
            act(3, Action::Fold),
            act(4, Action::Fold),
        ],
        meta: meta(Some(5), None),
    };
    let options = ExportOptions {
        currency: None,
        tournament: Some(TournamentInfo {
            id: "3999".into(),
            buy_in: Some(BuyIn {
                total: 1100,
                fee: 100,
                currency: Some("EUR".into()),
            }),
            level: Some("XII".into()),
        }),
    };
    let text = export_pokerstars(&record, &options).unwrap();
    assert!(
        text.starts_with(
            "PokerStars Hand #77001: Tournament #3999, €10+€1 EUR Hold'em No Limit - Level XII (50/100)"
        ),
        "{text}"
    );
    assert!(text.contains("D: posts the ante 100"), "{text}");
    assert!(!text.contains("A: posts the ante"), "{text}");
    let back = roundtrip(&record, &options, &ImportOptions::default());
    assert_eq!(back.tournament, options.tournament);
}

#[test]
fn roundtrip_heads_up_play_money_split_pot() {
    let record = HandRecord {
        id: "hu-1".into(),
        format: GameFormat::new(GameType::CashGame, 2, BettingStructure::NoLimit).unwrap(),
        blinds: ok(Blinds::new(1, 2)),
        button_seat: 2,
        seats: vec![
            seat(1, "Hero", 200, Some("2c3d")),
            seat(2, "Villain", 300, None),
        ],
        events: vec![
            raise(2, 6),
            act(1, Action::Call),
            board("AsKsQs"),
            act(1, Action::Check),
            act(2, Action::Check),
            board("Js"),
            bet(1, 6),
            act(2, Action::Call),
            board("Ts"),
            act(1, Action::Check),
            act(2, Action::Check),
            show(1, "2c3d"),
            show(2, "4h4d"),
        ],
        meta: meta(Some(1), None),
    };
    let text = export_pokerstars(&record, &ExportOptions::default()).unwrap();
    assert!(text.contains("Hold'em No Limit (1/2)"), "{text}");
    assert!(text.contains("Hero collected 12 from pot"), "{text}");
    assert!(text.contains("Villain collected 12 from pot"), "{text}");
    assert!(text.contains("(a Royal Flush)"), "{text}");
    let back = roundtrip(
        &record,
        &ExportOptions::default(),
        &ImportOptions::default(),
    );
    assert_eq!(back.currency, None);
    assert!(net(&record).iter().all(|(_, v)| *v == 0));
}

#[test]
fn roundtrip_ten_max_fixed_limit_tournament() {
    let format = GameFormat::new(
        GameType::Tournament,
        10,
        BettingStructure::FIXED_LIMIT_TOURNAMENT,
    )
    .unwrap();
    let mut seats: Vec<SeatRecord> = (1..=10)
        .map(|i| seat(i, &format!("Joueur {i}"), 1500, None))
        .collect();
    seats[3].hole_cards = Some(ok("AsAc".parse().unwrap()));
    // Bouton au siège 1 : petite blinde 2, grosse blinde 3, premier à parler 4 (héros).
    let mut events = vec![raise(4, 40)];
    events.extend((5..=10).chain(1..=2).map(|s| act(s, Action::Fold)));
    events.push(act(3, Action::Call));
    events.push(board("Kd7c2s"));
    events.push(act(3, Action::Check));
    events.push(bet(4, 20));
    events.push(act(3, Action::Fold));
    let record = HandRecord {
        id: "fl10-1".into(),
        format,
        blinds: ok(Blinds::new(10, 20).with_ante(Ante::Classic(Chips(5)))),
        button_seat: 1,
        seats,
        events,
        meta: meta(Some(4), Some(1_700_000_000_000)),
    };
    let text = export_pokerstars(&record, &ExportOptions::default()).unwrap();
    assert!(text.contains("Hold'em Limit - Level I (20/40)"), "{text}");
    assert!(text.contains("10-max"), "{text}");
    roundtrip(
        &record,
        &ExportOptions::default(),
        &ImportOptions::default(),
    );
}

#[test]
fn unknown_cards_are_never_invented() {
    // Abattage sans les cartes adverses : résultat non résolu, aucune ligne de gain.
    let record = HandRecord {
        id: "unres-1".into(),
        format: GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap(),
        blinds: ok(Blinds::new(5, 10)),
        button_seat: 1,
        seats: vec![
            seat(1, "Hero", 1000, Some("AhKh")),
            seat(2, "Sb", 1000, None),
            seat(3, "Bb", 1000, None),
        ],
        events: vec![
            raise(1, 30),
            act(2, Action::Fold),
            act(3, Action::Call),
            board("2c7d9s"),
            act(3, Action::Check),
            act(1, Action::Check),
            board("Jh"),
            act(3, Action::Check),
            act(1, Action::Check),
            board("3s"),
            act(3, Action::Check),
            act(1, Action::Check),
        ],
        meta: meta(Some(1), None),
    };
    assert!(matches!(
        record.replay().unwrap().outcome,
        Outcome::Unresolved { .. }
    ));
    let text = export_pokerstars(&record, &ExportOptions::default()).unwrap();
    assert!(text.contains("*** SHOW DOWN ***"), "{text}");
    assert!(!text.contains("collected"), "{text}");
    assert!(!text.contains("Bb: shows"), "{text}");
    assert_eq!(text.matches("Dealt to").count(), 1, "{text}");
    roundtrip(
        &record,
        &ExportOptions::default(),
        &ImportOptions::default(),
    );

    // Main coupée au flop (vidéo interrompue) et héros sans cartes connues.
    let mut cut = record.clone();
    cut.events.truncate(5);
    cut.seats[0].hole_cards = None;
    let text = export_pokerstars(&cut, &ExportOptions::default()).unwrap();
    assert!(text.contains("Dealt to Hero\n"), "{text}");
    assert!(!text.contains("SHOW DOWN"), "{text}");
    roundtrip(&cut, &ExportOptions::default(), &ImportOptions::default());
}

#[test]
fn import_options_fill_metadata() {
    let options = ImportOptions {
        tournament_type: GameType::SitAndGo,
        session_id: Some("session-1".into()),
        provenance: Some("mains personnelles".into()),
    };
    let h = import_pokerstars_hand(TOURNAMENT, &options).unwrap();
    assert_eq!(h.record.format.game_type, GameType::SitAndGo);
    assert_eq!(h.record.meta.session_id.as_deref(), Some("session-1"));
    assert_eq!(
        h.record.meta.provenance.as_deref(),
        Some("mains personnelles")
    );
}

// --- Erreurs ------------------------------------------------------------------------------

fn first_error(text: &str) -> stratyx_database::text::ImportError {
    import_pokerstars(text, &ImportOptions::default())
        .into_iter()
        .find_map(Result::err)
        .expect("une erreur")
}

#[test]
fn errors_are_located() {
    // Montant de suivi faux (ligne 19) : refusé, la main suivante reste lisible.
    let text = CASH.replace("Nordic Fox: calls $0.70", "Nordic Fox: calls $0.80");
    let results = import_pokerstars(&text, &ImportOptions::default());
    assert_eq!(results.len(), 2);
    assert!(results[1].is_ok());
    let e = results[0].as_ref().unwrap_err();
    assert_eq!(e.line, 19);
    assert_eq!(e.hand_id(), Some("254372383936"));
    assert!(matches!(
        e.kind,
        ImportErrorKind::AmountMismatch {
            expected: Chips(70),
            found: Chips(80),
            ..
        }
    ));
    assert!(e.to_string().contains("ligne 19"), "{e}");

    // Relance trop petite : refusée par le moteur de règles, ligne 15.
    let e = first_error(&CASH.replace("raises $0.70 to $1", "raises $0.05 to $0.35"));
    assert_eq!(e.line, 15);
    assert!(matches!(e.kind, ImportErrorKind::Replay(_)), "{e}");

    // Mise non suivie incohérente.
    let e = first_error(&CASH.replace("Uncalled bet ($1.10)", "Uncalled bet ($1)"));
    assert!(matches!(e.kind, ImportErrorKind::Inconsistent(_)), "{e}");

    // Straddle : hors moteur.
    let e = first_error(&CASH.replacen(
        "*** HOLE CARDS ***",
        "Lucky_Lu: posts straddle $0.20\n*** HOLE CARDS ***",
        1,
    ));
    assert!(matches!(e.kind, ImportErrorKind::Unsupported(_)), "{e}");

    // Joueur inconnu.
    let e = first_error(&CASH.replace("Dealt to HeroStx", "Dealt to Ghost"));
    assert!(matches!(e.kind, ImportErrorKind::UnknownPlayer(_)), "{e}");

    // Autre variante.
    let e = first_error(&CASH.replace("Hold'em No Limit", "Omaha Pot Limit"));
    assert!(matches!(e.kind, ImportErrorKind::UnsupportedGame(_)), "{e}");

    // Bouton mort.
    let e = first_error(&CASH.replacen("Seat #4 is the button", "Seat #7 is the button", 1));
    assert!(matches!(e.kind, ImportErrorKind::Unsupported(_)), "{e}");

    // Ligne d'action inconnue.
    let e = first_error(&CASH.replace("Lucky_Lu: folds", "Lucky_Lu: dances"));
    assert!(matches!(e.kind, ImportErrorKind::Syntax(_)), "{e}");

    let e = import_pokerstars_hand("bonjour", &ImportOptions::default()).unwrap_err();
    assert_eq!(e.kind, ImportErrorKind::NoHand);
    let e = import_pokerstars_hand(CASH, &ImportOptions::default()).unwrap_err();
    assert_eq!(e.kind, ImportErrorKind::SeveralHands(2));
}

#[test]
fn announced_winnings_are_cross_checked() {
    let text = TOURNAMENT.replace("HeroStx collected 2700", "HeroStx collected 2600");
    let h = import_pokerstars_hand(&text, &ImportOptions::default()).unwrap();
    assert_eq!(h.warnings.len(), 1, "{:?}", h.warnings);
    assert!(h.warnings[0].contains("HeroStx"));
}

#[test]
fn export_requires_a_valid_hand() {
    let mut h = import_all(CASH).remove(0).record;
    h.events.swap(0, 1); // hors tour
    assert!(matches!(
        export_pokerstars(&h, &ExportOptions::default()),
        Err(stratyx_database::text::ExportError::Replay(_))
    ));
    let mut h = import_all(CASH).remove(0).record;
    h.id = "a b".into();
    assert!(export_pokerstars(&h, &ExportOptions::default()).is_err());
    let mut h = import_all(CASH).remove(0).record;
    h.seats[1].player = h.seats[0].player.clone();
    assert!(export_pokerstars(&h, &ExportOptions::default()).is_err());
}
