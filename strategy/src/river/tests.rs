//! Tests du solveur de river.

use stratyx_poker::equity::{EquityOptions, equity};
use stratyx_poker::range::HandClass;
use stratyx_poker::{CardSet, Range};

use super::*;
use crate::preflop::parse_class;

fn range(s: &str) -> Range {
    Range::parse(s).unwrap()
}

fn sizes(bets: &[f64], raises: &[f64]) -> BetSizes {
    BetSizes {
        bets: bets.to_vec(),
        raises: raises.to_vec(),
    }
}

/// Jeu jouet : OOP a une range polarisée (16 quintes 65 = nuts, 32 mains d'air QJ et T9)
/// contre des bluff-catchers (12 AK, paire de Rois) ; une seule mise possible, de fraction
/// `bet` du pot. Avec `bet = 1` et pot = tapis, la mise pot est un tapis.
fn toy_config(bet: f64, stack: f64) -> RiverConfig {
    RiverConfig {
        board: CardSet::parse("Ks8d4c3h2s").unwrap(),
        oop_range: range("65,QJ,T9"),
        ip_range: range("AK"),
        pot: 10.0,
        effective_stack: stack,
        oop_sizes: sizes(&[bet], &[]),
        ip_sizes: sizes(&[], &[]),
        max_raises: 0,
        allow_all_in: false,
    }
}

fn class_of(sol: &RiverSolution, p: Player, i: usize) -> HandClass {
    sol.hands[p.index()][i].class()
}

/// Théorie du jeu polarisé contre bluff-catchers, mise `f × pot` :
/// part de bluffs dans la range de mise `f / (1 + 2f)` (ratio valeur:bluff `(1 + f) / f`),
/// défense `1 / (1 + f)` (MDF). Mise pot : 2:1 et 50 % ; demi-pot : 3:1 et 2/3.
#[test]
fn polarized_toy_game_matches_theory() {
    let options = RiverOptions {
        max_iterations: 5_000,
        target_exploitability: 1e-4,
        check_every: 100,
    };
    for (bet, stack) in [(1.0, 10.0), (0.5, 100.0)] {
        let sol = RiverSolver::solve(toy_config(bet, stack), &options).unwrap();
        assert!(
            sol.exploitability_fraction() < 1e-3,
            "mise {bet} : {}",
            sol.exploitability
        );
        let root = &sol.nodes[0];
        let expected_action = if bet == 1.0 {
            RiverAction::AllIn(10.0)
        } else {
            RiverAction::Bet(5.0)
        };
        assert_eq!(root.actions, vec![RiverAction::Check, expected_action]);
        // Fréquences de mise par catégorie de main de OOP.
        let freq = &sol.strategies[0][1];
        let (mut value, mut bluff, mut air) = (0.0f64, 0.0f64, 0.0f64);
        for (i, f) in freq.iter().enumerate() {
            if class_of(&sol, Player::Oop, i).to_string().starts_with("65") {
                value += f64::from(*f);
            } else {
                bluff += f64::from(*f);
                air += 1.0;
            }
        }
        assert_eq!(air, 32.0);
        // Toutes les nuts misent ; le ratio valeur:bluff suit la taille de mise.
        assert!((value - 16.0).abs() < 0.2, "mise {bet} : valeur {value}");
        let ratio = value / bluff;
        let expected_ratio = (1.0 + bet) / bet;
        assert!(
            (ratio - expected_ratio).abs() < 0.05 * expected_ratio,
            "mise {bet} : ratio valeur:bluff {ratio}, attendu {expected_ratio}"
        );
        // IP défend selon la MDF avec ses bluff-catchers.
        let facing = root.children[1];
        let freqs = sol.action_frequencies(facing).unwrap();
        assert_eq!(freqs[1].0, RiverAction::Call);
        let mdf = 1.0 / (1.0 + bet);
        assert!(
            (freqs[1].1 - mdf).abs() < 0.02,
            "mise {bet} : défense {:?}, attendu {mdf}",
            freqs
        );
        assert!((sol.value[0] + sol.value[1] - 10.0).abs() < 1e-3);
        // Après parole, IP ne peut que checker (aucune taille).
        let after_check = &sol.nodes[root.children[0]];
        assert_eq!(after_check.actions, vec![RiverAction::Check]);
    }
}

#[test]
fn showdown_values_match_exact_equities_with_blockers() {
    // Sans mise possible, la valeur de chaque main est son équité × pot.
    let config = RiverConfig {
        board: CardSet::parse("Qs9h7d4c2s").unwrap(),
        oop_range: range("QQ,99,AQ,KQs,T8,A5s,22:0.5,AsKs"),
        ip_range: range("AA,KK,Q9s,QJ,JT,86s,97,AhKh:0.3"),
        pot: 7.0,
        effective_stack: 20.0,
        oop_sizes: BetSizes::default(),
        ip_sizes: BetSizes::default(),
        max_raises: 0,
        allow_all_in: false,
    };
    let sol = RiverSolver::solve(
        config.clone(),
        &RiverOptions {
            max_iterations: 1,
            ..RiverOptions::default()
        },
    )
    .unwrap();
    // Aucune décision réelle : la stratégie est trivialement optimale.
    assert!(sol.exploitability < 1e-5, "{}", sol.exploitability);
    let options = EquityOptions::default();
    for (i, hand) in sol.hands[0].iter().enumerate() {
        let res = equity(
            &[Range::single(*hand), config.ip_range.clone()],
            config.board,
            CardSet::EMPTY,
            &options,
        )
        .unwrap();
        let expected = res.players[0].equity * config.pot;
        let got = f64::from(sol.ev[0][i]);
        assert!(
            (got - expected).abs() < 1e-4,
            "{hand} : {got} au lieu de {expected}"
        );
    }
    let total = equity(
        &[config.oop_range.clone(), config.ip_range.clone()],
        config.board,
        CardSet::EMPTY,
        &options,
    )
    .unwrap();
    assert!((sol.value[0] - total.players[0].equity * config.pot).abs() < 1e-4);
    assert!((sol.value[0] + sol.value[1] - config.pot).abs() < 1e-4);
}

fn realistic_config() -> RiverConfig {
    RiverConfig {
        board: CardSet::parse("Qs Jh 7d 4c 2s").unwrap(),
        oop_range: range("22+,A2s+,K9s+,Q9s+,J9s+,T9s,98s,87s,ATo+,KJo+,QJo"),
        ip_range: range("22+,A2s+,K2s+,Q8s+,J8s+,T8s+,97s+,86s+,75s+,64s+,54s,A8o+,KTo+,QTo+,JTo"),
        pot: 10.0,
        effective_stack: 30.0,
        oop_sizes: sizes(&[0.5, 1.0], &[1.0]),
        ip_sizes: sizes(&[0.33, 0.75], &[1.0]),
        max_raises: 2,
        allow_all_in: true,
    }
}

/// Convergence documentée sur cette river (212 et 324 mains, 45 nœuds : 2 tailles de mise,
/// relances, tapis). Mesuré : 9 % du pot après 10 itérations, 0,43 % après 100, 0,20 % après
/// 200, 0,05 % après 500. Exigence testée : < 0,5 % du pot après 200 itérations, et
/// décroissance de l'exploitabilité.
#[test]
fn realistic_river_converges() {
    let mut solver = RiverSolver::new(realistic_config()).unwrap();
    let mut previous = solver.exploitability();
    for _ in 0..4 {
        solver.iterate(50);
        let now = solver.exploitability();
        assert!(now < previous, "{previous} → {now}");
        previous = now;
    }
    assert_eq!(solver.iterations(), 200);
    let end = solver.exploitability();
    assert!(
        end / 10.0 < 0.005,
        "exploitabilité {:.3} % du pot après {} itérations",
        100.0 * end / 10.0,
        solver.iterations()
    );
    let sol = solver.solution();
    assert!((sol.value[0] + sol.value[1] - 10.0).abs() < 1e-3);
    // Les nuts (brelan de Dames ou mieux) ne se couchent jamais face à une mise.
    let root = &sol.nodes[0];
    let check = root.children[0];
    let ip_bet = sol.nodes[check].children[1];
    let qq = Range::parse("QhQd").unwrap().iter().next().unwrap().0;
    let strat = sol.strategy(ip_bet, qq).unwrap();
    assert_eq!(strat[0].0, RiverAction::Fold);
    assert!(strat[0].1 < 0.01, "{strat:?}");
    // Les fréquences d'actions somment à 1.
    for (i, node) in sol.nodes.iter().enumerate() {
        if let Some(f) = sol.action_frequencies(i) {
            let s: f64 = f.iter().map(|(_, x)| x).sum();
            assert!((s - 1.0).abs() < 1e-3 || node.actions.is_empty());
        }
    }
}

/// Ranges complètes (1 081 mains chacune), 3 tailles, 3 relances, tapis : 93 nœuds.
/// Mesuré : 0,37 % du pot après 200 itérations (0,5 s en release), 0,04 % après 1 000.
#[test]
fn full_range_river_converges() {
    let config = RiverConfig {
        board: CardSet::parse("Ah9c6d5s2h").unwrap(),
        oop_range: Range::full(),
        ip_range: Range::full(),
        pot: 20.0,
        effective_stack: 100.0,
        oop_sizes: sizes(&[0.33, 0.75, 1.5], &[0.75]),
        ip_sizes: sizes(&[0.33, 0.75, 1.5], &[0.75]),
        max_raises: 3,
        allow_all_in: true,
    };
    let sol = RiverSolver::solve(
        config,
        &RiverOptions {
            max_iterations: 300,
            target_exploitability: 0.005,
            check_every: 100,
        },
    )
    .unwrap();
    assert_eq!(sol.hands[0].len(), 1081);
    assert!(
        sol.exploitability_fraction() < 0.005,
        "{}",
        sol.exploitability
    );
    // Symétrie des ranges : IP, qui a la position, gagne plus de la moitié du pot.
    assert!(sol.value[1] > sol.value[0], "{:?}", sol.value);
}

#[test]
fn solution_serde_roundtrip_and_helpers() {
    let sol = RiverSolver::solve(
        toy_config(1.0, 10.0),
        &RiverOptions {
            max_iterations: 50,
            ..RiverOptions::default()
        },
    )
    .unwrap();
    let json = serde_json::to_string(&sol).unwrap();
    assert!(json.contains(r#""kind":"all_in""#), "{}", &json[..200]);
    let back: RiverSolution = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sol);
    let hand = sol.hands[0][0];
    assert!(sol.hand_ev(Player::Oop, hand).is_some());
    assert!(sol.hand_ev(Player::Ip, hand).is_none());
    assert!(sol.strategy(0, hand).is_some());
    assert!(sol.strategy(0, sol.hands[1][0]).is_none());
    assert_eq!(RiverAction::Raise(25.0).to_string(), "relancer à 25.00");
    assert_eq!(Player::Oop.opponent(), Player::Ip);
    let _ = parse_class("AA");
}

#[test]
fn invalid_configurations_are_rejected() {
    let ok = toy_config(1.0, 10.0);
    let check = |f: &dyn Fn(&mut RiverConfig), expected: RiverError| {
        let mut c = ok.clone();
        f(&mut c);
        assert_eq!(RiverSolver::new(c).unwrap_err(), expected);
    };
    check(
        &|c| c.board = CardSet::parse("Ks8d4c3h").unwrap(),
        RiverError::Board(4),
    );
    check(&|c| c.pot = 0.0, RiverError::Pot(0.0));
    check(&|c| c.effective_stack = -1.0, RiverError::Stack(-1.0));
    check(
        &|c| c.oop_sizes.bets = vec![-0.5],
        RiverError::BetSize(-0.5),
    );
    check(
        &|c| c.ip_range = range("KsKh"),
        RiverError::EmptyRange(Player::Ip),
    );
    check(
        &|c| {
            c.oop_range = range("AhAd");
            c.ip_range = range("AhAd");
        },
        RiverError::NoMatchup,
    );
}
