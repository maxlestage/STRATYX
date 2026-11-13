//! Mesure la convergence du solveur de river CFR+ (exploitabilité en % du pot).
//!
//! ```text
//! cargo run --release -p stratyx-strategy --example river_convergence
//! ```

use std::time::Instant;

use stratyx_poker::{CardSet, Range};
use stratyx_strategy::river::{BetSizes, RiverConfig, RiverSolver};

fn report(label: &str, config: RiverConfig) {
    let mut solver = RiverSolver::new(config).expect("paramètres valides");
    let sol = solver.solution();
    println!(
        "== {label} : {} nœuds, {} / {} mains",
        sol.nodes.len(),
        sol.hands[0].len(),
        sol.hands[1].len()
    );
    let start = Instant::now();
    let mut done = 0;
    for target in [10, 50, 100, 200, 500, 1000, 2000] {
        solver.iterate(target - done);
        done = target;
        let pot = sol.config.pot;
        println!(
            "  {target:>5} itérations : exploitabilité {:.4} % du pot ({:.2} s)",
            100.0 * solver.exploitability() / pot,
            start.elapsed().as_secs_f64()
        );
    }
}

fn main() {
    let sizes = |bets: &[f64], raises: &[f64]| BetSizes {
        bets: bets.to_vec(),
        raises: raises.to_vec(),
    };
    for (label, bet, stack) in [
        ("jouet polarisé (65 + air contre AK, mise pot)", 1.0, 10.0),
        ("jouet polarisé (65 + air contre AK, demi-pot)", 0.5, 100.0),
    ] {
        report(
            label,
            RiverConfig {
                board: CardSet::parse("Ks8d4c3h2s").expect("board"),
                oop_range: Range::parse("65,QJ,T9").expect("range"),
                ip_range: Range::parse("AK").expect("range"),
                pot: 10.0,
                effective_stack: stack,
                oop_sizes: sizes(&[bet], &[]),
                ip_sizes: sizes(&[], &[]),
                max_raises: 0,
                allow_all_in: false,
            },
        );
    }
    report(
        "river réaliste (2 tailles, relances, tapis)",
        RiverConfig {
            board: CardSet::parse("QsJh7d4c2s").expect("board"),
            oop_range: Range::parse("22+,A2s+,K9s+,Q9s+,J9s+,T9s,98s,87s,ATo+,KJo+,QJo")
                .expect("range"),
            ip_range: Range::parse(
                "22+,A2s+,K2s+,Q8s+,J8s+,T8s+,97s+,86s+,75s+,64s+,54s,A8o+,KTo+,QTo+,JTo",
            )
            .expect("range"),
            pot: 10.0,
            effective_stack: 30.0,
            oop_sizes: sizes(&[0.5, 1.0], &[1.0]),
            ip_sizes: sizes(&[0.33, 0.75], &[1.0]),
            max_raises: 2,
            allow_all_in: true,
        },
    );
    report(
        "river large (ranges entières, 3 tailles, 3 relances)",
        RiverConfig {
            board: CardSet::parse("Ah9c6d5s2h").expect("board"),
            oop_range: Range::full(),
            ip_range: Range::full(),
            pot: 20.0,
            effective_stack: 100.0,
            oop_sizes: sizes(&[0.33, 0.75, 1.5], &[0.75]),
            ip_sizes: sizes(&[0.33, 0.75, 1.5], &[0.75]),
            max_raises: 3,
            allow_all_in: true,
        },
    );
}
