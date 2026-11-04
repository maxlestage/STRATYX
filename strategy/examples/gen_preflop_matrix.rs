//! Génère la matrice d'équité préflop 169 × 169 embarquée par `stratyx-strategy`.
//!
//! ```text
//! cargo run --release -p stratyx-strategy --example gen_preflop_matrix [-- --samples N --seed S --output CHEMIN]
//! ```
//!
//! Par défaut : 200 000 tirages Monte-Carlo par couple de classes, graine fixe, écriture dans
//! `strategy/data/preflop_equity_169.bin`. À options égales, le fichier produit est identique
//! octet pour octet, quel que soit le nombre de cœurs (décision T8).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use stratyx_strategy::preflop::{MatrixOptions, PreflopMatrix};

fn main() -> ExitCode {
    let mut options = MatrixOptions::default();
    let mut output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/preflop_equity_169.bin");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next();
        let parsed = match (arg.as_str(), value) {
            ("--samples", Some(v)) => v.parse().map(|n| options.samples = n).is_ok(),
            ("--seed", Some(v)) => v.parse().map(|n| options.seed = n).is_ok(),
            ("--output", Some(v)) => {
                output = PathBuf::from(v);
                true
            }
            _ => false,
        };
        if !parsed {
            eprintln!("usage : gen_preflop_matrix [--samples N] [--seed S] [--output CHEMIN]");
            return ExitCode::FAILURE;
        }
    }

    eprintln!(
        "Matrice préflop 169 × 169 : {} tirages par couple, graine {:#x}",
        options.samples, options.seed
    );
    let start = Instant::now();
    let last = AtomicUsize::new(0);
    let progress = |done: usize, total: usize| {
        let pct = done * 100 / total;
        if last.swap(pct, Ordering::Relaxed) != pct {
            eprintln!(
                "  {pct:3} % ({done}/{total}) — {:.0} s",
                start.elapsed().as_secs_f64()
            );
        }
    };
    let matrix = match PreflopMatrix::compute(&options, &progress) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("échec : {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = matrix.to_bytes();
    if let Err(e) = std::fs::write(&output, &bytes) {
        eprintln!("écriture impossible dans {} : {e}", output.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "Écrit : {} ({} octets) en {:.1} s",
        output.display(),
        bytes.len(),
        start.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}
