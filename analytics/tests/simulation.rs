//! Bout en bout : mains simulées par des joueurs aléatoires pilotant le moteur de règles,
//! puis invariants vérifiés indépendamment de l'implémentation.

mod common;

use common::ok;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_analytics::{
    AnalyticsError, Analyzer, DecisionOptions, StatKind, StatLine, analyze_hero_decisions,
};
use stratyx_poker::card::Card;
use stratyx_poker::equity::EquityOptions;
use stratyx_poker::format::{GameFormat, GameType};
use stratyx_poker::game::{
    Action, Ante, BettingStructure, Blinds, HandSetup, HandState, LegalActions, Phase, PlayerStatus,
};
use stratyx_poker::history::{HandEvent, HandMeta, HandRecord, Outcome, SeatRecord};
use stratyx_poker::range::HoleCards;
use stratyx_shared::Chips;

const NAMES: [&str; 8] = [
    "Hero", "Alice", "Bruno", "Chloé", "David", "Emma", "Farid", "Gaëlle",
];

fn sized(rng: &mut ChaCha8Rng, (min, max): (Chips, Chips), bet: bool) -> Action {
    if rng.random_bool(0.15) {
        return Action::AllIn;
    }
    let to = Chips(rng.random_range(min.get()..=max.get()));
    if bet {
        Action::Bet(to)
    } else {
        Action::Raise(to)
    }
}

/// Politique aléatoire : checks et mises quand rien n'est à suivre ; fold, call ou relance
/// face à une mise.
fn choose(rng: &mut ChaCha8Rng, legal: &LegalActions) -> Action {
    let aggression = legal
        .bet
        .map(|b| (b, true))
        .or(legal.raise.map(|r| (r, false)));
    if legal.check {
        return match aggression {
            Some((bounds, bet)) if rng.random_bool(0.3) => sized(rng, bounds, bet),
            _ => Action::Check,
        };
    }
    let r: f64 = rng.random();
    match aggression {
        _ if r < 0.35 => Action::Fold,
        Some((bounds, bet)) if r > 0.85 => sized(rng, bounds, bet),
        _ => Action::Call,
    }
}

fn simulate(rng: &mut ChaCha8Rng, id: usize) -> HandRecord {
    let cash6 = GameFormat::new(GameType::CashGame, 6, BettingStructure::NoLimit).unwrap();
    let (format, blinds) = match rng.random_range(0..3) {
        0 => (cash6, Blinds::new(50, 100)),
        1 => (
            GameFormat::TOURNAMENT_6MAX,
            Blinds::new(50, 100).with_ante(Ante::BigBlind(Chips(100))),
        ),
        _ => (GameFormat::EXPRESSO, Blinds::new(50, 100)),
    };
    let max = format.table_size;
    let n = rng.random_range(2..=usize::from(max));
    let mut seats: Vec<u8> = (1..=max).collect();
    seats.shuffle(rng);
    seats.truncate(n);
    seats.sort_unstable();
    let mut others: Vec<&str> = NAMES[1..].to_vec();
    others.shuffle(rng);
    let hero = rng.random_range(0..n);
    let stacks: Vec<Chips> = (0..n)
        .map(|_| Chips(rng.random_range(300..=8_000)))
        .collect();
    let button = rng.random_range(0..n);

    let mut deck: Vec<Card> = Card::all().collect();
    deck.shuffle(rng);
    let holes: Vec<HoleCards> = (0..n)
        .map(|_| HoleCards::new(deck.pop().unwrap(), deck.pop().unwrap()).unwrap())
        .collect();

    let setup = HandSetup::new(stacks.clone(), button, blinds).with_structure(format.structure);
    let mut state = HandState::new(&setup).unwrap();
    let mut events = Vec::new();
    loop {
        match state.phase() {
            Phase::Betting => {
                let legal = state.legal_actions().unwrap();
                let action = choose(rng, &legal);
                state.apply(legal.player, action).unwrap();
                events.push(ok(HandEvent::Action {
                    seat: seats[legal.player],
                    action,
                }));
            }
            Phase::AwaitingBoard(street) => {
                let cards: Vec<Card> = (0..street.cards_dealt())
                    .map(|_| deck.pop().unwrap())
                    .collect();
                state.deal(&cards).unwrap();
                events.push(ok(HandEvent::Board { cards }));
            }
            Phase::Showdown => {
                for p in (0..n).filter(|p| *p != hero) {
                    // Certaines mains adverses ne sont pas vues : main non résolue.
                    if state.players()[p].status != PlayerStatus::Folded && rng.random_bool(0.9) {
                        events.push(ok(HandEvent::Show {
                            seat: seats[p],
                            cards: holes[p],
                        }));
                    }
                }
                break;
            }
            Phase::Finished => break,
        }
    }
    // Quelques vidéos coupées en cours de main.
    if events.len() > 1 && rng.random_bool(0.03) {
        let cut = rng.random_range(1..events.len());
        events.truncate(cut);
    }

    let record = HandRecord {
        id: format!("sim-{id}"),
        format,
        blinds: ok(blinds),
        button_seat: seats[button],
        seats: (0..n)
            .map(|i| SeatRecord {
                seat: seats[i],
                player: if i == hero { NAMES[0] } else { others[i] }.to_string(),
                stack: ok(stacks[i]),
                hole_cards: (i == hero).then(|| ok(holes[i])),
            })
            .collect(),
        events,
        meta: HandMeta {
            hero_seat: Some(seats[hero]),
            played_at_ms: rng
                .random_bool(0.9)
                .then(|| rng.random_range(0..10_000_000)),
            ..HandMeta::default()
        },
    };
    record.replay().expect("main simulée valide");
    record
}

/// Résultat du héros recalculé directement à partir du rejeu.
fn hero_net(record: &HandRecord) -> Option<i64> {
    let replay = record.replay().unwrap();
    let h = replay
        .seats
        .iter()
        .position(|s| Some(*s) == record.meta.hero_seat)
        .unwrap();
    match &replay.outcome {
        Outcome::Settled(s) => Some(s.net[h]),
        _ => {
            let pl = &replay.state.players()[h];
            (pl.status == PlayerStatus::Folded).then(|| -(pl.invested().get() as i64))
        }
    }
}

fn check_line(line: &StatLine) {
    for kind in StatKind::ALL {
        let s = line.get(kind);
        assert!(s.occurrences <= s.opportunities, "{kind:?} : {s:?}");
        if !kind.is_derived() {
            assert!(
                s.opportunities <= line.hands,
                "{kind:?} : {s:?} pour {} mains",
                line.hands
            );
        }
    }
    assert_eq!(line.vpip.opportunities, line.pfr.opportunities);
    assert!(line.vpip.occurrences >= line.pfr.occurrences);
    assert!(line.won_at_showdown.opportunities <= line.went_to_showdown.occurrences);
    assert!(line.cbet_turn.opportunities <= line.cbet_flop.occurrences);
}

fn sum<'a>(lines: impl Iterator<Item = &'a StatLine>) -> StatLine {
    let mut total = StatLine::default();
    for l in lines {
        total.merge(l);
    }
    total
}

#[test]
fn simulated_hands_end_to_end() {
    // Graine modifiable pour explorer d'autres parties : STRATYX_SIM_SEED=7 cargo test …
    let seed = std::env::var("STRATYX_SIM_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2026);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let records: Vec<HandRecord> = (0..400).map(|i| simulate(&mut rng, i)).collect();

    // Équité allégée : les valeurs exactes sont vérifiées dans `results.rs`.
    let light = EquityOptions {
        max_exact_evaluations: 50_000,
        samples: 2_000,
        ..EquityOptions::default()
    };
    let mut analyzer = Analyzer::with_equity_options(light);
    let errors = analyzer.add_hands(&records);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(matches!(
        analyzer.add_hand(&records[0]),
        Err(AnalyticsError::DuplicateHand(_))
    ));
    let book = analyzer.profiles();
    assert_eq!(book.hand_count(), records.len());

    // Profils : cohérence interne et recomptage indépendant du VPIP.
    for profile in book.profiles() {
        let name = profile.player.as_str();
        let (mut hands, mut vpip_opp, mut vpip) = (0u64, 0u64, 0u64);
        for r in &records {
            let Some(seat) = r.seats.iter().find(|s| s.player == name).map(|s| s.seat) else {
                continue;
            };
            hands += 1;
            let preflop: Vec<Action> = r
                .events
                .iter()
                .map(|e| &e.value)
                .take_while(|e| !matches!(e, HandEvent::Board { .. }))
                .filter_map(|e| match e {
                    HandEvent::Action { seat: s, action } if *s == seat => Some(*action),
                    _ => None,
                })
                .collect();
            vpip_opp += u64::from(!preflop.is_empty());
            vpip += u64::from(preflop.iter().any(|a| {
                matches!(
                    a,
                    Action::Call | Action::Raise(_) | Action::Bet(_) | Action::AllIn
                )
            }));
        }
        let overall = &profile.stats.overall;
        assert_eq!(overall.hands, hands, "{name}");
        assert_eq!(overall.vpip.opportunities, vpip_opp, "{name}");
        assert_eq!(overall.vpip.occurrences, vpip, "{name}");

        check_line(overall);
        profile.stats.by_position.values().for_each(check_line);
        assert_eq!(&sum(profile.stats.by_position.values()), overall, "{name}");
        assert_eq!(
            &sum(profile.by_format.iter().map(|f| &f.stats.overall)),
            overall
        );
        for f in &profile.by_format {
            assert_eq!(sum(f.stats.by_position.values()), f.stats.overall);
        }
    }
    let hero = book.profile("Hero").unwrap();
    assert_eq!(hero.hands(), records.len() as u64);
    assert_eq!(hero.by_format.len(), 3);
    let total = &hero.stats.overall;
    for kind in [
        StatKind::Vpip,
        StatKind::Steal,
        StatKind::ThreeBet,
        StatKind::CbetFlop,
    ] {
        assert!(total.get(kind).occurrences > 0, "{kind:?} jamais observé");
    }

    // Résultats : recalcul indépendant du résultat net.
    let summary = analyzer.results().summary();
    let nets: Vec<Option<i64>> = records.iter().map(hero_net).collect();
    let known: Vec<i64> = nets.iter().flatten().copied().collect();
    assert_eq!(summary.overall.hands, known.len() as u64);
    assert_eq!(
        summary.unresolved_hands,
        nets.iter().filter(|n| n.is_none()).count() as u64
    );
    assert!(
        summary.unresolved_hands > 0,
        "le simulateur produit des mains non résolues"
    );
    assert_eq!(summary.overall.net_chips, known.iter().sum::<i64>());
    let by_pos: i64 = summary.by_position.values().map(|l| l.net_chips).sum();
    let by_fmt: i64 = summary.by_format.iter().map(|f| f.overall.net_chips).sum();
    assert_eq!(
        (by_pos, by_fmt),
        (summary.overall.net_chips, summary.overall.net_chips)
    );
    assert_eq!(
        summary.curve.last().unwrap().cumulative_chips,
        summary.overall.net_chips
    );
    let times: Vec<Option<u64>> = summary.curve.iter().map(|p| p.played_at_ms).collect();
    let dated = times.iter().take_while(|t| t.is_some()).count();
    assert!(times[dated..].iter().all(Option::is_none));
    assert!(times[..dated].windows(2).all(|w| w[0] <= w[1]));

    // Gains ajustés à l'all-in : bornés par les tapis en jeu.
    assert!(summary.overall.all_in_adjusted_hands > 0);
    for r in analyzer.results().hands() {
        if let Some(ev) = &r.all_in {
            let record = records.iter().find(|x| x.id == r.hand_id).unwrap();
            let chips: u64 = record.seats.iter().map(|s| s.stack.value.get()).sum();
            assert!(ev.expected_net.abs() <= chips as f64);
            assert!(ev.pots.iter().all(|p| (0.0..=1.0).contains(&p.equity)));
        } else {
            assert_eq!(r.adjusted_net(), r.net_chips as f64);
        }
    }

    // Confrontations : la somme des transferts redonne le résultat du héros sur les mains
    // résolues, à l'arrondi au jeton près.
    let (mut settled_net, mut pairs) = (0i64, 0u64);
    for r in &records {
        let replay = r.replay().unwrap();
        if let Outcome::Settled(_) = replay.outcome {
            settled_net += hero_net(r).unwrap();
            pairs += r.seats.len() as u64 - 1;
        }
    }
    let h2h: i64 = book.head_to_heads().map(|h| h.net_chips).sum();
    // Chaque paire d'une main résolue est attribuée : les pots reconstitués coïncident
    // avec ceux du moteur (antes de grosse blinde comprises).
    let attributed: u64 = book.head_to_heads().map(|h| h.settled_hands).sum();
    assert_eq!(attributed, pairs);
    assert!(
        (h2h - settled_net).unsigned_abs() <= pairs / 2,
        "{h2h} contre {settled_net}"
    );

    // Calcul incrémental : deux moitiés, avec sérialisation entre les deux.
    let (a, b) = records.split_at(records.len() / 2);
    let mut first = Analyzer::with_equity_options(light);
    assert!(first.add_hands(a).is_empty());
    let mut resumed: Analyzer =
        serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    assert!(resumed.add_hands(b).is_empty());
    for p in book.profiles() {
        assert_eq!(resumed.profiles().profile(&p.player), Some(p));
    }
    for (x, y) in book.head_to_heads().zip(resumed.profiles().head_to_heads()) {
        assert_eq!(
            (x.hands_together, x.net_chips),
            (y.hands_together, y.net_chips)
        );
    }
    let again = resumed.results().summary();
    assert_eq!(again.overall.net_chips, summary.overall.net_chips);
    assert_eq!(again.curve.len(), summary.curve.len());

    // Analyse de décision sur des mains du héros : invariants des formules.
    let options = DecisionOptions {
        equity: light,
        ..DecisionOptions::default()
    };
    let mut analyzed = 0;
    for r in records.iter().take(40) {
        for d in analyze_hero_decisions(r, &options).unwrap() {
            analyzed += 1;
            assert!(d.spr >= 0.0);
            assert!(d.to_call.is_zero() || d.effective_stack.get() > 0, "{d:?}");
            if let (Some(req), Some(eq), Some(ev)) = (d.required_equity, d.equity, d.call_ev) {
                assert!((0.0..=1.0).contains(&req));
                // EV du call = (pot gagnable + call) × (équité − équité requise).
                if (eq.equity - req).abs() > 1e-6 {
                    assert_eq!(ev > 0.0, eq.equity > req, "{d:?}");
                }
            }
            assert!(d.fold_equity.is_none() || d.action.kind.is_aggressive());
        }
    }
    assert!(analyzed > 40);
}
