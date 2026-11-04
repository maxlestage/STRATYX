//! Calcule et affiche des équilibres push/fold, avec temps de calcul et exploitabilité.
//!
//! ```text
//! cargo run --release -p stratyx-strategy --example pushfold_charts [-- hu|3max|all]
//! ```

use std::time::Instant;

use stratyx_poker::position::Position;
use stratyx_strategy::pushfold::{
    HeadsUpConfig, PayoffModel, SolverOptions, ThreeMaxConfig, ThreeWayEquities, solve_heads_up,
    solve_three_max,
};

fn heads_up() {
    println!("== Tête-à-tête, chipEV, sans ante ==");
    for stack in [1.0, 2.0, 3.0, 5.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0] {
        let start = Instant::now();
        let sol = solve_heads_up(
            &HeadsUpConfig {
                stack_bb: stack,
                ante_bb: 0.0,
            },
            &SolverOptions::HEADS_UP,
        )
        .expect("paramètres valides");
        let push = sol.push_chart(Position::SmallBlind).expect("nœud SB");
        let call = sol
            .call_chart(Position::BigBlind, &[Position::SmallBlind])
            .expect("nœud BB");
        println!(
            "{stack:>5} BB : push {:5.1} %, call {:5.1} %, EV SB {:+.4} BB, exploitabilité {:.5} / {:.5} BB, {} itérations, {:.2} s",
            100.0 * push.fraction(),
            100.0 * call.fraction(),
            sol.ev[0],
            sol.exploitability[0],
            sol.exploitability[1],
            sol.iterations,
            start.elapsed().as_secs_f64()
        );
        if stack == 10.0 {
            println!("SB push 10 BB :\n{push}\nBB call 10 BB :\n{call}");
        }
    }
}

fn three_max() {
    let start = Instant::now();
    let table = ThreeWayEquities::shared();
    println!(
        "Table des confrontations à trois : {:.1} s",
        start.elapsed().as_secs_f64()
    );
    for (label, payoff) in [
        ("chipEV", PayoffModel::ChipEv),
        (
            "ICM 50/30/20",
            PayoffModel::Icm {
                payouts: vec![0.5, 0.3, 0.2],
            },
        ),
    ] {
        for stacks in [[10.0, 10.0, 10.0], [5.0, 5.0, 5.0], [15.0, 8.0, 12.0]] {
            let start = Instant::now();
            let sol = solve_three_max(
                &ThreeMaxConfig {
                    stacks_bb: stacks,
                    ante_bb: 0.0,
                    payoff: payoff.clone(),
                },
                table,
                &SolverOptions::THREE_MAX,
            )
            .expect("paramètres valides");
            println!(
                "== 3-max {label} {stacks:?} : {} itérations, {:.1} s, exploitabilité {:?}, EV {:?}",
                sol.iterations,
                start.elapsed().as_secs_f64(),
                sol.exploitability,
                sol.ev
            );
            for spot in &sol.spots {
                println!(
                    "   {:?} face à {:?} : {:5.1} %",
                    spot.position,
                    spot.facing,
                    100.0 * spot.chart.fraction()
                );
            }
        }
    }
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if which == "hu" || which == "all" {
        heads_up();
    }
    if which == "3max" || which == "all" {
        three_max();
    }
}
