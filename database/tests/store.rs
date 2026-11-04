//! Base locale : persistance, espaces, déduplication, recherche, ingestion d'archives,
//! sessions, utilisateurs, stratégie, RGPD, export et revalidation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use stratyx_database::text::ImportOptions;
use stratyx_database::{
    ArchiveBatch, Database, DatabaseError, DatabaseExport, EXPORT_FORMAT, Erasure, Fingerprint,
    HandQuery, HeroResult, IngestPolicy, PlayerAlias, ProgressEntry, Provenance, PutOutcome,
    RejectReason, SCHEMA_VERSION, SortOrder, Space, StrategyBlob, UserProfile, Validation,
    ValidationKind,
};
use stratyx_poker::card::parse_cards;
use stratyx_poker::game::{BettingStructure, Blinds};
use stratyx_poker::{
    Action, GameFormat, GameType, HandEvent, HandMeta, HandRecord, SeatRecord, Session,
    SessionSource,
};
use stratyx_shared::{Chips, Confidence, Observed, Source};

const CASH: &str = include_str!("data/pokerstars_cash_nl6.txt");

/// Fichier temporaire unique, supprimé à la fin du test.
struct TempFile(PathBuf);

impl TempFile {
    fn new(tag: &str) -> TempFile {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        TempFile(
            std::env::temp_dir().join(format!("stratyx-{tag}-{}-{nanos}.redb", std::process::id())),
        )
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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

fn board(cards: &str) -> Observed<HandEvent> {
    ok(HandEvent::Board {
        cards: parse_cards(cards).unwrap(),
    })
}

/// Expresso : le héros (siège 1) pousse son tapis, Villain2 paie et montre deux Dames.
/// Le héros gagne avec le flop `Kd7c2s`, perd avec `2d7c2s`.
fn expresso(id: &str, hero_stack: u64, hero_wins: bool, played_at_ms: u64) -> HandRecord {
    HandRecord {
        id: id.into(),
        format: GameFormat::EXPRESSO,
        blinds: ok(Blinds::new(10, 20)),
        button_seat: 1,
        seats: vec![
            seat(1, "Hero", hero_stack, Some("AhKh")),
            seat(2, "Villain1", 500, None),
            seat(3, "Villain2", 500, None),
        ],
        events: vec![
            act(1, Action::Raise(Chips(hero_stack))),
            act(2, Action::Fold),
            act(3, Action::Call),
            board(if hero_wins { "Kd7c2s" } else { "2d7c2s" }),
            board("9h"),
            board("3c"),
            ok(HandEvent::Show {
                seat: 3,
                cards: "QsQd".parse().unwrap(),
            }),
        ],
        meta: HandMeta {
            hero_seat: Some(1),
            played_at_ms: Some(played_at_ms),
            site: Some("winamax".into()),
            ..HandMeta::default()
        },
    }
}

/// Tournoi 6-max : tout le monde se couche sur la relance du héros (pas d'abattage).
fn tournament(id: &str, hero_stack: u64) -> HandRecord {
    HandRecord {
        id: id.into(),
        format: GameFormat::TOURNAMENT_6MAX,
        blinds: ok(Blinds::new(25, 50)),
        button_seat: 1,
        seats: (1..=6)
            .map(|i| {
                let stack = if i == 4 { hero_stack } else { 1500 };
                seat(i, if i == 4 { "Hero" } else { "Reg" }, stack, None)
            })
            .enumerate()
            .map(|(i, mut s)| {
                if s.player == "Reg" {
                    s.player = format!("Reg{i}");
                }
                s
            })
            .collect(),
        events: vec![
            act(4, Action::Raise(Chips(125))),
            act(5, Action::Fold),
            act(6, Action::Fold),
            act(1, Action::Fold),
            act(2, Action::Fold),
            act(3, Action::Fold),
        ],
        meta: HandMeta {
            hero_seat: Some(4),
            ..HandMeta::default()
        },
    }
}

fn cash_hands() -> Vec<HandRecord> {
    stratyx_database::text::import_pokerstars(CASH, &ImportOptions::default())
        .into_iter()
        .map(|h| h.unwrap().record)
        .collect()
}

fn provenance() -> Provenance {
    Provenance::new(
        "mains personnelles (PokerStars)",
        "données de l'utilisateur",
    )
    .unwrap()
}

#[test]
fn database_is_shareable_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Database>();
    let db = Database::in_memory().unwrap();
    std::thread::scope(|s| {
        for i in 0..4u64 {
            let db = &db;
            s.spawn(move || {
                db.put_hand(&expresso(&format!("t{i}"), 100 + i * 10, true, i))
                    .unwrap();
            });
        }
    });
    assert_eq!(db.count_hands(Some(Space::Expresso)).unwrap(), 4);
}

#[test]
fn file_database_persists_and_migrates() {
    let file = TempFile::new("persist");
    let hand = expresso("e1", 500, true, 1_000);
    {
        let db = Database::open(&file.0).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let log = db.migration_log().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].version, 1);
        assert!(matches!(
            db.put_hand(&hand).unwrap(),
            PutOutcome::Inserted(_)
        ));
    }
    let db = Database::open(&file.0).unwrap();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(db.migration_log().unwrap().len(), 1);
    let stored = db.get_hand("e1").unwrap().unwrap();
    assert_eq!(stored.record, hand);
    assert_eq!(stored.format_version, stratyx_database::HAND_FORMAT_VERSION);
    assert_eq!(stored.space, Space::Expresso);
    assert_eq!(stored.status.validation, Validation::Valid);
    assert_eq!(stored.fingerprint, Fingerprint::of(&hand));
    assert_eq!(db.count_hands(None).unwrap(), 1);
}

#[test]
fn hands_are_stored_in_the_space_of_their_format() {
    let db = Database::in_memory().unwrap();
    db.put_hand(&expresso("e1", 500, true, 1)).unwrap();
    db.put_hand(&tournament("t1", 1500)).unwrap();
    for h in cash_hands() {
        db.put_hand(&h).unwrap();
    }
    assert_eq!(db.get_hand("e1").unwrap().unwrap().space, Space::Expresso);
    assert_eq!(
        db.get_hand("t1").unwrap().unwrap().space,
        Space::Tournaments
    );
    assert_eq!(
        db.get_hand("254372383936").unwrap().unwrap().space,
        Space::Holdem
    );
    assert_eq!(db.count_hands(Some(Space::Expresso)).unwrap(), 1);
    assert_eq!(db.count_hands(Some(Space::Tournaments)).unwrap(), 1);
    assert_eq!(db.count_hands(Some(Space::Holdem)).unwrap(), 2);
    assert_eq!(db.count_hands(Some(Space::Users)).unwrap(), 0);
    assert_eq!(db.count_hands(None).unwrap(), 4);

    // Changer le format d'une main la déplace d'espace.
    let mut moved = tournament("t1", 1500);
    moved.format = GameFormat::new(GameType::Tournament, 6, BettingStructure::PotLimit).unwrap();
    assert!(matches!(
        db.put_hand(&moved).unwrap(),
        PutOutcome::Updated(_)
    ));
    assert_eq!(db.get_hand("t1").unwrap().unwrap().space, Space::Holdem);
    assert_eq!(db.count_hands(Some(Space::Tournaments)).unwrap(), 0);
}

#[test]
fn duplicates_updates_and_deletion() {
    let db = Database::in_memory().unwrap();
    let h = expresso("e1", 500, true, 1);
    let PutOutcome::Inserted(summary) = db.put_hand(&h).unwrap() else {
        panic!("insertion attendue")
    };
    assert_eq!(summary.hero.as_deref(), Some("Hero"));
    assert_eq!(summary.hero_net, Some(510));
    assert!(summary.showdown);

    // Même contenu de jeu, autre identifiant et autres métadonnées : doublon.
    let mut copy = h.clone();
    copy.id = "copie".into();
    copy.meta.site = Some("pokerstars".into());
    copy.events[2] = Observed::vision(
        copy.events[2].value.clone(),
        Confidence::new(0.99).unwrap(),
        12_000,
    );
    match db.put_hand(&copy).unwrap() {
        PutOutcome::Duplicate {
            existing_id,
            fingerprint,
        } => {
            assert_eq!(existing_id, "e1");
            assert_eq!(fingerprint, Fingerprint::of(&h));
        }
        other => panic!("doublon attendu : {other:?}"),
    }
    assert!(db.get_hand("copie").unwrap().is_none());

    // Correction de la main : remplacée, empreinte mise à jour.
    let corrected = expresso("e1", 400, true, 1);
    assert!(matches!(
        db.put_hand(&corrected).unwrap(),
        PutOutcome::Updated(_)
    ));
    assert_eq!(db.find_by_fingerprint(&Fingerprint::of(&h)).unwrap(), None);
    assert_eq!(
        db.find_by_fingerprint(&Fingerprint::of(&corrected))
            .unwrap(),
        Some("e1".into())
    );
    // L'ancienne version peut maintenant être enregistrée sous un autre identifiant.
    assert!(matches!(
        db.put_hand(&copy).unwrap(),
        PutOutcome::Inserted(_)
    ));

    assert!(db.delete_hand("e1").unwrap());
    assert!(!db.delete_hand("e1").unwrap());
    assert!(db.get_hand("e1").unwrap().is_none());
    assert!(db.hand_summary("e1").unwrap().is_none());
    assert_eq!(
        db.find_by_fingerprint(&Fingerprint::of(&corrected))
            .unwrap(),
        None
    );
    assert_eq!(
        db.search(&HandQuery::new().player("Hero")).unwrap().total,
        1
    );

    let mut empty = h;
    empty.id = " ".into();
    assert!(matches!(
        db.put_hand(&empty),
        Err(DatabaseError::EmptyId(_))
    ));
}

#[test]
fn invalid_and_uncertain_hands_are_kept_for_review() {
    let db = Database::in_memory().unwrap();
    let mut invalid = expresso("bad", 500, true, 5);
    invalid.events[1] = act(3, Action::Fold); // hors tour
    db.put_hand(&invalid).unwrap();
    let s = db.hand_summary("bad").unwrap().unwrap();
    let Validation::Invalid { reason } = &s.status.validation else {
        panic!("main invalide attendue")
    };
    assert!(reason.contains("événement 1"), "{reason}");
    assert!(s.status.needs_review());

    let mut uncertain = expresso("vision", 450, true, 6);
    uncertain.events[2] = Observed::vision(
        uncertain.events[2].value.clone(),
        Confidence::new(0.7).unwrap(),
        3_000,
    );
    db.put_hand(&uncertain).unwrap();
    let s = db.hand_summary("vision").unwrap().unwrap();
    assert_eq!(s.status.validation, Validation::Valid);
    assert_eq!(s.status.review_items, 1);

    let mut unresolved = expresso("unres", 400, true, 7);
    unresolved.events.pop();
    db.put_hand(&unresolved).unwrap();
    let mut incomplete = expresso("incomplete", 350, true, 8);
    incomplete.events.truncate(4);
    db.put_hand(&incomplete).unwrap();
    db.put_hand(&expresso("fine", 300, true, 9)).unwrap();

    let ids = |q: HandQuery| -> Vec<String> {
        let mut v: Vec<String> = db
            .search(&q)
            .unwrap()
            .items
            .into_iter()
            .map(|s| s.id)
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        ids(HandQuery::new().needs_review(true)),
        ["bad", "incomplete", "unres", "vision"]
    );
    assert_eq!(ids(HandQuery::new().needs_review(false)), ["fine"]);
    assert_eq!(
        ids(HandQuery::new().validation(ValidationKind::Invalid)),
        ["bad"]
    );
    assert_eq!(
        ids(HandQuery::new().validation(ValidationKind::Unresolved)),
        ["unres"]
    );
    assert_eq!(
        ids(HandQuery::new().validation(ValidationKind::Incomplete)),
        ["incomplete"]
    );
}

#[test]
fn search_combines_filters_with_pagination() {
    let db = Database::in_memory().unwrap();
    let mut session_hands = Vec::new();
    for i in 0..6u64 {
        let mut h = expresso(&format!("e{i}"), 300 + i * 10, i % 2 == 0, 1_000 * (i + 1));
        if i < 3 {
            h.meta.session_id = Some("s1".into());
            session_hands.push(h.id.clone());
        }
        db.put_hand(&h).unwrap();
    }
    db.put_hand(&tournament("t1", 1500)).unwrap();
    for h in cash_hands() {
        db.put_hand(&h).unwrap();
    }

    let all = db.search(&HandQuery::new()).unwrap();
    assert_eq!(all.total, 9);
    // Plus récentes d'abord ; les mains sans date en dernier.
    let dated: Vec<Option<u64>> = all.items.iter().map(|s| s.played_at_ms).collect();
    assert_eq!(dated.last(), Some(&None));
    assert!(dated[0] > dated[1]);

    let page = db
        .search(&HandQuery::new().space(Space::Expresso).page(2, 2))
        .unwrap();
    assert_eq!(page.total, 6);
    assert_eq!(
        page.items.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["e3", "e2"]
    );
    let oldest = db
        .search(
            &HandQuery::new()
                .space(Space::Expresso)
                .order(SortOrder::OldestFirst)
                .page(0, 1),
        )
        .unwrap();
    assert_eq!(oldest.items[0].id, "e0");

    let q = HandQuery::new().session("s1");
    assert_eq!(db.search(&q).unwrap().total, 3);
    let won = HandQuery::new().session("s1").hero_result(HeroResult::Won);
    assert_eq!(db.search(&won).unwrap().total, 2); // e0 et e2
    let lost = HandQuery::new().hero_result(HeroResult::Lost);
    let lost_ids: Vec<String> = db
        .search(&lost)
        .unwrap()
        .items
        .into_iter()
        .map(|s| s.id)
        .collect();
    // e1, e3, e5 (Expresso perdues) et la seconde main de cash.
    assert_eq!(lost_ids.len(), 4);
    assert!(lost_ids.contains(&"254372401127".to_string()));

    // Période [2000, 4000[ : e1, e2.
    assert_eq!(
        db.search(&HandQuery::new().between(2_000, 4_000))
            .unwrap()
            .total,
        2
    );
    // Joueur + format.
    let q = HandQuery::new().player("Nordic Fox");
    assert_eq!(db.search(&q).unwrap().total, 2);
    let cash6 = GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap();
    assert_eq!(db.search(&HandQuery::new().format(cash6)).unwrap().total, 2);
    assert_eq!(
        db.search(&HandQuery::new().format(cash6).showdown(true))
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        db.search(&HandQuery::new().game_type(GameType::Tournament))
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        db.search(&HandQuery::new().site("winamax")).unwrap().total,
        6
    );
    assert_eq!(
        db.search(&HandQuery::new().player("Hero").showdown(false))
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        db.search(&HandQuery::new().player("Personne"))
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        db.search(&HandQuery::new().space(Space::Strategy))
            .unwrap()
            .total,
        0
    );
}

#[test]
fn archive_ingestion_validates_deduplicates_and_reports() {
    let db = Database::in_memory().unwrap();
    db.put_hand(&expresso("deja-la", 500, true, 1)).unwrap();

    let valid = expresso("a1", 450, true, 2);
    let mut duplicate = expresso("a2", 500, true, 3); // même jeu que « deja-la »
    duplicate.meta.site = None;
    let mut invalid = expresso("a3", 400, true, 4);
    invalid.events[0] = act(1, Action::Raise(Chips(25))); // relance trop petite
    let mut incomplete = expresso("a4", 350, true, 5);
    incomplete.events.truncate(4);
    let mut unresolved = expresso("a5", 300, true, 6);
    unresolved.events.pop();
    let conflict = expresso("deja-la", 250, true, 7); // identifiant pris, autre contenu
    let mut nameless = expresso("", 200, true, 8);
    nameless.id = String::new();
    let mut uncertain = expresso("a6", 150, true, 9);
    uncertain.blinds = Observed::vision(Blinds::new(10, 20), Confidence::new(0.5).unwrap(), 0);
    let repeated = expresso("a7", 450, true, 2); // doublon de a1 dans le même lot

    let batch = ArchiveBatch::new(
        provenance(),
        vec![
            valid, duplicate, invalid, incomplete, unresolved, conflict, nameless, uncertain,
            repeated,
        ],
    );
    let report = db.ingest(batch, IngestPolicy::default()).unwrap();
    assert_eq!(report.batch_id, "lot-000001");
    assert_eq!(report.total(), 9);
    let accepted: Vec<&str> = report.accepted.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(accepted, ["a1", "a5"]);
    assert_eq!(report.accepted[1].validation, ValidationKind::Unresolved);
    let dups: Vec<(&str, &str)> = report
        .duplicates
        .iter()
        .map(|d| (d.id.as_str(), d.existing_id.as_str()))
        .collect();
    assert_eq!(dups, [("a2", "deja-la"), ("a7", "a1")]);
    let rejected: Vec<(usize, &RejectReason)> = report
        .rejected
        .iter()
        .map(|r| (r.position, &r.reason))
        .collect();
    assert!(matches!(rejected[0], (2, RejectReason::Invalid(_))));
    assert!(matches!(rejected[1], (3, RejectReason::Incomplete)));
    assert!(matches!(rejected[2], (5, RejectReason::IdConflict(_))));
    assert!(matches!(rejected[3], (6, RejectReason::EmptyId)));
    assert!(matches!(rejected[4], (7, RejectReason::NeedsReview(1))));
    assert_eq!(report.rejected[2].id.as_deref(), Some("deja-la"));
    assert!(
        report.rejected[0]
            .reason
            .to_string()
            .contains("rejeu refusé")
    );

    // La provenance du lot est reportée sur les mains admises.
    let a1 = db.get_hand("a1").unwrap().unwrap();
    assert_eq!(a1.batch_id.as_deref(), Some("lot-000001"));
    assert_eq!(
        a1.record.meta.provenance.as_deref(),
        Some("mains personnelles (PokerStars) (licence : données de l'utilisateur)")
    );
    // La main déjà présente n'a pas été modifiée.
    assert_eq!(
        db.get_hand("deja-la").unwrap().unwrap().record,
        expresso("deja-la", 500, true, 1)
    );

    // Règles plus permissives : la main incomplète est admise dans un second lot.
    let mut incomplete = expresso("a4", 350, true, 5);
    incomplete.events.truncate(4);
    let policy = IngestPolicy {
        accept_incomplete: true,
        ..IngestPolicy::default()
    };
    let report = db
        .ingest(ArchiveBatch::new(provenance(), vec![incomplete]), policy)
        .unwrap();
    assert_eq!(report.batch_id, "lot-000002");
    assert_eq!(report.accepted.len(), 1);

    let reports = db.ingestion_reports().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].rejected.len(), 5);

    // Provenance ou licence manquante : lot refusé en bloc.
    let mut no_license = provenance();
    no_license.license = "  ".into();
    let err = db
        .ingest(
            ArchiveBatch::new(no_license, vec![expresso("x", 100, true, 1)]),
            IngestPolicy::default(),
        )
        .unwrap_err();
    assert!(matches!(err, DatabaseError::MissingProvenance(_)));
    assert!(err.to_string().contains("D5"));
    assert!(Provenance::new("", "x").is_err());
    assert!(db.get_hand("x").unwrap().is_none());
}

#[test]
fn pokerstars_files_are_ingested() {
    let db = Database::in_memory().unwrap();
    let options = ImportOptions {
        session_id: Some("ps-2024-12-01".into()),
        ..ImportOptions::default()
    };
    let report = db
        .ingest_pokerstars(provenance(), CASH, &options, IngestPolicy::default())
        .unwrap();
    assert_eq!(report.accepted.len(), 2, "{report:?}");
    assert!(report.accepted.iter().all(|a| a.space == Space::Holdem));
    assert_eq!(
        db.search(&HandQuery::new().session("ps-2024-12-01"))
            .unwrap()
            .total,
        2
    );

    // Second import du même fichier : doublons.
    let again = db
        .ingest_pokerstars(provenance(), CASH, &options, IngestPolicy::default())
        .unwrap();
    assert_eq!(again.duplicates.len(), 2);
    assert!(again.accepted.is_empty());

    // Main illisible : rejetée avec son identifiant, les autres sont examinées.
    let broken = CASH.replace("Nordic Fox: calls $0.70", "Nordic Fox: calls $0.80");
    let report = db
        .ingest_pokerstars(provenance(), &broken, &options, IngestPolicy::default())
        .unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.rejected[0].id.as_deref(), Some("254372383936"));
    assert!(matches!(
        report.rejected[0].reason,
        RejectReason::Unreadable(_)
    ));
    assert_eq!(report.duplicates.len(), 1);
}

#[test]
fn sessions_can_be_erased_with_their_hands() {
    let db = Database::in_memory().unwrap();
    let mut listed = expresso("listed", 500, true, 1);
    listed.meta.session_id = None;
    let mut tagged = expresso("tagged", 450, true, 2);
    tagged.meta.session_id = Some("s1".into());
    let other = expresso("other", 400, true, 3);
    for h in [&listed, &tagged, &other] {
        db.put_hand(h).unwrap();
    }
    let session = Session {
        id: "s1".into(),
        source: SessionSource::LiveCapture,
        format: GameFormat::EXPRESSO,
        started_at_ms: 10,
        ended_at_ms: Some(20),
        hand_ids: vec!["listed".into()],
        tournament: None,
    };
    db.put_session(&session).unwrap();
    let mut s2 = session.clone();
    s2.id = "s0".into();
    s2.started_at_ms = 5;
    s2.hand_ids.clear();
    db.put_session(&s2).unwrap();
    assert_eq!(db.get_session("s1").unwrap(), Some(session));
    let ids: Vec<String> = db
        .list_sessions()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, ["s0", "s1"]);

    let erased = db.delete_session("s1").unwrap();
    assert!(erased.session_found);
    assert_eq!(erased.hands_deleted, 2);
    assert!(db.get_session("s1").unwrap().is_none());
    assert!(db.get_hand("listed").unwrap().is_none());
    assert!(db.get_hand("tagged").unwrap().is_none());
    assert!(db.get_hand("other").unwrap().is_some());
    let none = db.delete_session("inconnue").unwrap();
    assert!(!none.session_found);
    assert_eq!(none.hands_deleted, 0);
}

#[test]
fn users_and_progress() {
    let db = Database::in_memory().unwrap();
    let mut user = UserProfile::new("u1", "Max", 1_000);
    user.aliases.push(PlayerAlias {
        site: Some("winamax".into()),
        name: "Hero".into(),
    });
    user.settings
        .insert("langue".into(), serde_json::json!("fr"));
    db.put_user(&user).unwrap();
    db.put_user(&UserProfile::new("u0", "Autre", 0)).unwrap();
    assert_eq!(db.get_user("u1").unwrap(), Some(user.clone()));
    assert_eq!(db.list_users().unwrap().len(), 2);

    let entry = |at_ms: u64, value: f64| ProgressEntry {
        at_ms,
        topic: "pushfold".into(),
        metric: "score".into(),
        value,
        details: None,
    };
    db.add_progress("u1", entry(20, 0.8)).unwrap();
    db.add_progress("u1", entry(10, 0.6)).unwrap();
    let progress = db.progress("u1").unwrap();
    assert_eq!(
        progress.iter().map(|e| e.at_ms).collect::<Vec<_>>(),
        [10, 20]
    );
    assert!(matches!(
        db.add_progress("inconnu", entry(1, 0.1)),
        Err(DatabaseError::UnknownUser(_))
    ));

    assert!(db.delete_user("u1").unwrap());
    assert!(db.get_user("u1").unwrap().is_none());
    assert!(db.progress("u1").unwrap().is_empty());
    assert!(!db.delete_user("u1").unwrap());
}

#[test]
fn strategy_blobs() {
    let db = Database::in_memory().unwrap();
    let chart = StrategyBlob {
        content_type: "text/x-range".into(),
        description: Some("push 15 bb au bouton".into()),
        data: b"22+,A2s+,K9s+".to_vec(),
    };
    let entry = db
        .put_strategy("charts/expresso/pushfold/15bb", &chart)
        .unwrap();
    assert_eq!(entry.size, 13);
    assert_eq!(entry.sha256.len(), 64);
    let solver = StrategyBlob {
        content_type: "application/octet-stream".into(),
        description: None,
        data: (0..=255u8).collect(),
    };
    db.put_strategy("solveur/srp/k72r", &solver).unwrap();
    db.put_strategy("charts/tournois/open/utg", &chart).unwrap();

    assert_eq!(
        db.get_strategy("charts/expresso/pushfold/15bb").unwrap(),
        Some(chart)
    );
    assert_eq!(db.get_strategy("solveur/srp/k72r").unwrap(), Some(solver));
    let keys: Vec<String> = db
        .strategy_entries("charts/")
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(
        keys,
        ["charts/expresso/pushfold/15bb", "charts/tournois/open/utg"]
    );
    assert_eq!(db.strategy_entries("").unwrap().len(), 3);
    assert!(db.delete_strategy("solveur/srp/k72r").unwrap());
    assert!(db.get_strategy("solveur/srp/k72r").unwrap().is_none());
    assert!(!db.delete_strategy("solveur/srp/k72r").unwrap());
    assert!(
        db.put_strategy(
            "",
            &StrategyBlob {
                content_type: String::new(),
                description: None,
                data: vec![],
            }
        )
        .is_err()
    );
}

#[test]
fn players_can_be_anonymized_or_erased() {
    let db = Database::in_memory().unwrap();
    for h in cash_hands() {
        db.put_hand(&h).unwrap();
    }
    db.put_hand(&expresso("e1", 500, true, 1)).unwrap();

    let report = db.erase_player("Nordic Fox", Erasure::Anonymize).unwrap();
    assert_eq!(report.hands_anonymized, 2);
    assert_eq!(
        db.search(&HandQuery::new().player("Nordic Fox"))
            .unwrap()
            .total,
        0
    );
    let h = db.get_hand("254372383936").unwrap().unwrap();
    assert_eq!(h.record.seats[1].player, "Anonyme (siège 2)");
    assert_eq!(h.status.validation, Validation::Valid);
    assert_eq!(
        db.search(&HandQuery::new().player("Anonyme (siège 2)"))
            .unwrap()
            .total,
        2
    );
    // Le reste de la main est intact.
    assert_eq!(h.record.seats[2].player, "HeroStx");

    let report = db.erase_player("HeroStx", Erasure::DeleteHands).unwrap();
    assert_eq!(report.hands_deleted, 2);
    assert_eq!(db.count_hands(None).unwrap(), 1);
    assert!(db.erase_player("", Erasure::DeleteHands).is_err());
}

#[test]
fn full_json_export() {
    let db = Database::in_memory().unwrap();
    for h in cash_hands() {
        db.put_hand(&h).unwrap();
    }
    db.ingest(
        ArchiveBatch::new(provenance(), vec![expresso("e1", 500, true, 1)]),
        IngestPolicy::default(),
    )
    .unwrap();
    db.put_user(&UserProfile::new("u1", "Max", 0)).unwrap();
    db.add_progress(
        "u1",
        ProgressEntry {
            at_ms: 1,
            topic: "icm".into(),
            metric: "score".into(),
            value: 0.5,
            details: Some(serde_json::json!({ "exercice": 3 })),
        },
    )
    .unwrap();
    db.put_strategy(
        "charts/a",
        &StrategyBlob {
            content_type: "application/octet-stream".into(),
            description: None,
            data: vec![0, 255],
        },
    )
    .unwrap();
    db.put_session(&Session {
        id: "s1".into(),
        source: SessionSource::TextImport,
        format: GameFormat::EXPRESSO,
        started_at_ms: 0,
        ended_at_ms: None,
        hand_ids: vec![],
        tournament: None,
    })
    .unwrap();

    let mut out = Vec::new();
    db.export_json(&mut out).unwrap();
    let export: DatabaseExport = serde_json::from_slice(&out).unwrap();
    assert_eq!(export.format, EXPORT_FORMAT);
    assert_eq!(export.schema_version, SCHEMA_VERSION);
    assert_eq!(export.hands.len(), 3);
    assert_eq!(export.sessions.len(), 1);
    assert_eq!(export.users.len(), 1);
    assert_eq!(export.users[0].progress.len(), 1);
    assert_eq!(export.strategy[0].data_hex, "00ff");
    assert_eq!(export.ingestions.len(), 1);
    assert_eq!(
        export,
        db.export()
            .map(|mut e| {
                e.exported_at_ms = export.exported_at_ms;
                e
            })
            .unwrap()
    );
}

#[test]
fn revalidation_applies_a_new_review_threshold() {
    let file = TempFile::new("revalidate");
    let mut h = expresso("v1", 500, true, 1);
    h.events[2] = Observed::vision(
        h.events[2].value.clone(),
        Confidence::new(0.9).unwrap(),
        4_000,
    );
    {
        let db = Database::open(&file.0).unwrap();
        db.put_hand(&h).unwrap();
        db.put_hand(&expresso("v2", 450, true, 2)).unwrap();
        assert_eq!(
            db.hand_summary("v1").unwrap().unwrap().status.review_items,
            1
        );
        assert_eq!(db.revalidate().unwrap(), 0);
    }
    let db = Database::open(&file.0)
        .unwrap()
        .with_review_threshold(Confidence::new(0.8).unwrap());
    assert_eq!(db.revalidate().unwrap(), 1);
    assert_eq!(
        db.hand_summary("v1").unwrap().unwrap().status.review_items,
        0
    );
    assert_eq!(
        db.search(&HandQuery::new().needs_review(true))
            .unwrap()
            .total,
        0
    );
    assert_eq!(db.count_hands(None).unwrap(), 2);
}
