//! Tests des solveurs push/fold.

use stratyx_poker::position::Position::{BigBlind, Button, SmallBlind};

use super::three_way::tests::small_table;
use super::*;
use crate::preflop::parse_class;

fn c(s: &str) -> HandClass {
    parse_class(s).unwrap()
}

fn hu(stack_bb: f64, ante_bb: f64) -> PushFoldSolution {
    solve_heads_up(
        &HeadsUpConfig { stack_bb, ante_bb },
        &SolverOptions::HEADS_UP,
    )
    .unwrap()
}

fn sb_push(sol: &PushFoldSolution) -> &PushFoldChart {
    sol.push_chart(SmallBlind).unwrap()
}

fn bb_call(sol: &PushFoldSolution) -> &PushFoldChart {
    sol.call_chart(BigBlind, &[SmallBlind]).unwrap()
}

/// Exigence du cahier des charges : la meilleure réponse de chaque joueur gagne moins de
/// 0,01 BB par main contre la solution, de 1 à 25 BB, avec ou sans ante.
#[test]
fn heads_up_exploitability_below_one_hundredth_of_a_big_blind() {
    for ante in [0.0, 0.125] {
        // Avec ante, le BB doit pouvoir poster 1 BB + ante : on commence à 1,125 BB.
        let stacks: Vec<f64> = (1..=25).map(|s| f64::from(s).max(1.0 + ante)).collect();
        let table = solve_heads_up_table(&stacks, ante, &SolverOptions::HEADS_UP).unwrap();
        for sol in &table {
            assert_eq!(sol.exploitability.len(), 2);
            for (p, gain) in sol.exploitability.iter().enumerate() {
                assert!(
                    *gain < 0.01,
                    "{} BB, ante {ante} : joueur {p} exploitable de {gain} BB",
                    sol.stacks_bb[0]
                );
            }
            // Jeu à somme nulle en chipEV.
            assert!((sol.ev[0] + sol.ev[1]).abs() < 1e-6);
        }
    }
}

#[test]
fn heads_up_ranges_shrink_with_depth() {
    let stacks: Vec<f64> = (1..=25).map(f64::from).collect();
    let table = solve_heads_up_table(&stacks, 0.0, &SolverOptions::HEADS_UP).unwrap();
    for pair in table.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        // Tolérance : quelques mains mixtes peuvent osciller d'une profondeur à l'autre.
        assert!(
            sb_push(b).fraction() <= sb_push(a).fraction() + 0.005,
            "push {} BB : {} > {}",
            b.stacks_bb[0],
            sb_push(b).fraction(),
            sb_push(a).fraction()
        );
        assert!(bb_call(b).fraction() <= bb_call(a).fraction() + 0.005);
    }
    // ≈ 100 % à 1 BB ; à 2 BB, seules les pires mains (moins de 37,5 % d'équité contre une
    // main quelconque) se couchent.
    assert!(sb_push(&table[0]).fraction() > 0.995);
    assert!(bb_call(&table[0]).fraction() > 0.995);
    assert!(sb_push(&table[1]).fraction() > 0.88);
    assert!(bb_call(&table[1]).fraction() > 0.99);
    // Ordres de grandeur connus de l'équilibre à 10 BB (≈ 58 % / 37 %).
    let ten = &table[9];
    assert!((sb_push(ten).fraction() - 0.58).abs() < 0.03);
    assert!((bb_call(ten).fraction() - 0.37).abs() < 0.03);
    for sol in &table {
        for strong in ["AA", "KK", "AKs"] {
            assert!(sb_push(sol).frequency(c(strong)) > 0.99);
            assert!(bb_call(sol).frequency(c(strong)) > 0.99);
        }
    }
    // À 25 BB, les pires mains ne sont jamais jouées.
    assert!(sb_push(&table[24]).frequency(c("72o")) < 0.01);
    assert!(bb_call(&table[24]).frequency(c("72o")) < 0.01);
}

#[test]
fn antes_widen_ranges() {
    let without = hu(10.0, 0.0);
    let with = hu(10.0, 0.125);
    assert!(sb_push(&with).fraction() > sb_push(&without).fraction() + 0.01);
    assert!(bb_call(&with).fraction() > bb_call(&without).fraction());
}

#[test]
fn heads_up_action_evs_are_consistent() {
    let sol = hu(10.0, 0.0);
    let spot = sol.spot(SmallBlind, &[]).unwrap();
    // Se coucher au SB coûte exactement la petite blinde.
    for (_, v) in spot.ev_fold.iter() {
        assert!((f64::from(v) + 0.5).abs() < 1e-5);
    }
    // AA : pousser rapporte, 72o : pousser perd plus que la blinde.
    assert!(spot.ev_aggressive.get(c("AA")) > 1.0);
    assert!(spot.ev_aggressive.get(c("72o")) < -0.5);
    // L'EV moyenne est cohérente avec la stratégie.
    let ev: f64 = classes()
        .map(|cl| {
            let f = spot.chart.frequency(cl);
            let w = crate::preflop::class_combo_count(cl) as f64 / 1326.0;
            w * (f * f64::from(spot.ev_aggressive.get(cl))
                + (1.0 - f) * f64::from(spot.ev_fold.get(cl)))
        })
        .sum();
    assert!((ev - sol.ev[0]).abs() < 1e-4, "{ev} {}", sol.ev[0]);
}

#[test]
fn chart_helpers_and_serde() {
    let sol = hu(8.0, 0.0);
    let push = sb_push(&sol);
    let range = push.to_range();
    assert!((range.fraction() - push.fraction()).abs() < 1e-6);
    let shown = push.to_string();
    assert_eq!(shown.lines().count(), 14);
    assert!(shown.starts_with("      A    K"));
    assert!(push.classes_at_least(0.5).contains(&c("AA")));
    assert_eq!(push.grid()[0][0], push.frequencies().get(c("AA")));
    let json = serde_json::to_string(&sol).unwrap();
    assert!(json.contains(r#""position":"SB""#));
    assert!(json.contains(r#""AKs":"#));
    let back: PushFoldSolution = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sol);
}

#[test]
fn invalid_parameters_are_rejected() {
    let o = SolverOptions::HEADS_UP;
    let err = |stack_bb: f64, ante_bb: f64| {
        solve_heads_up(&HeadsUpConfig { stack_bb, ante_bb }, &o).unwrap_err()
    };
    assert!(matches!(err(0.9, 0.0), PushFoldError::Stack { .. }));
    assert!(matches!(err(1.0, 0.1), PushFoldError::Stack { .. }));
    assert!(matches!(err(f64::NAN, 0.0), PushFoldError::Stack { .. }));
    assert_eq!(err(10.0, -0.1), PushFoldError::Ante(-0.1));
    let bad_icm = ThreeMaxConfig {
        stacks_bb: [10.0; 3],
        ante_bb: 0.0,
        payoff: PayoffModel::Icm { payouts: vec![] },
    };
    assert!(matches!(
        solve_three_max(&bad_icm, small_table(), &SolverOptions::THREE_MAX),
        Err(PushFoldError::Payouts(_))
    ));
    let short_bb = ThreeMaxConfig {
        stacks_bb: [10.0, 10.0, 0.5],
        ante_bb: 0.0,
        payoff: PayoffModel::ChipEv,
    };
    assert!(
        solve_three_max(&short_bb, small_table(), &SolverOptions::THREE_MAX)
            .unwrap_err()
            .to_string()
            .contains("BB")
    );
}

fn three_max(stacks_bb: [f64; 3], payoff: PayoffModel) -> PushFoldSolution {
    let options = SolverOptions {
        max_iterations: 1_000,
        target_exploitability: 1e-3,
        check_every: 50,
    };
    solve_three_max(
        &ThreeMaxConfig {
            stacks_bb,
            ante_bb: 0.0,
            payoff,
        },
        small_table(),
        &options,
    )
    .unwrap()
}

/// Tolérance retenue à trois joueurs : 0,05 BB par joueur (mesuré : < 0,001 BB en une
/// centaine d'itérations).
#[test]
fn three_max_chip_ev_equilibrium() {
    for stacks in [[10.0, 10.0, 10.0], [15.0, 6.0, 9.0]] {
        let sol = three_max(stacks, PayoffModel::ChipEv);
        assert_eq!(sol.spots.len(), 6);
        assert!(sol.max_exploitability() < 0.05, "{:?}", sol.exploitability);
        assert!(sol.ev.iter().sum::<f64>().abs() < 1e-6);
        for spot in &sol.spots {
            assert!(spot.chart.frequency(c("AA")) > 0.99, "{spot:?}");
        }
        // Le sur-call exige une main plus forte que le call simple.
        let call = sol.call_chart(BigBlind, &[Button]).unwrap();
        let overcall = sol.call_chart(BigBlind, &[Button, SmallBlind]).unwrap();
        assert!(overcall.fraction() < call.fraction());
    }
}

#[test]
fn three_max_blind_battle_matches_heads_up() {
    // BTN couché : SB contre BB est exactement le tête-à-tête au même tapis (sans ante).
    let sol = three_max([10.0, 10.0, 10.0], PayoffModel::ChipEv);
    let heads_up = hu(10.0, 0.0);
    let sb = sol.push_chart(SmallBlind).unwrap();
    let bb = sol.call_chart(BigBlind, &[SmallBlind]).unwrap();
    assert!((sb.fraction() - sb_push(&heads_up).fraction()).abs() < 0.01);
    assert!((bb.fraction() - bb_call(&heads_up).fraction()).abs() < 0.01);
}

#[test]
fn icm_winner_take_all_equals_chip_ev_and_payouts_tighten_calls() {
    let stacks = [10.0, 10.0, 10.0];
    let chip = three_max(stacks, PayoffModel::ChipEv);
    let wta = three_max(stacks, PayoffModel::Icm { payouts: vec![1.0] });
    for (a, b) in chip.spots.iter().zip(&wta.spots) {
        assert!((a.chart.fraction() - b.chart.fraction()).abs() < 0.01);
    }
    for (a, b) in chip.ev.iter().zip(&wta.ev) {
        assert!((a - b).abs() < 0.01, "{:?} {:?}", chip.ev, wta.ev);
    }
    let icm = three_max(
        stacks,
        PayoffModel::Icm {
            payouts: vec![0.5, 0.3, 0.2],
        },
    );
    assert!(icm.max_exploitability() < 0.05, "{:?}", icm.exploitability);
    for facing in [&[Button][..], &[SmallBlind], &[Button, SmallBlind]] {
        let tight = icm.call_chart(BigBlind, facing).unwrap().fraction();
        let loose = chip.call_chart(BigBlind, facing).unwrap().fraction();
        assert!(tight < loose, "{facing:?} : ICM {tight} ≥ chipEV {loose}");
    }
}

#[test]
#[ignore = "calcule la table complète des confrontations à trois (≈ 12 s en release, plus en profil test)"]
fn three_max_with_full_table() {
    let start = std::time::Instant::now();
    let sol = solve_three_max(
        &ThreeMaxConfig {
            stacks_bb: [10.0, 10.0, 10.0],
            ante_bb: 0.0,
            payoff: PayoffModel::Icm { payouts: vec![1.0] },
        },
        ThreeWayEquities::shared(),
        &SolverOptions::THREE_MAX,
    )
    .unwrap();
    assert!(sol.max_exploitability() < 1e-3, "{:?}", sol.exploitability);
    assert!(start.elapsed().as_secs() < 60);
}
