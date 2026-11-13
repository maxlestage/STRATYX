//! `stratyx-train` : entraîne les modèles des deux IA et écrit leurs fichiers JSON.
//!
//! ```text
//! cargo run --release -p stratyx-ai-training --bin stratyx-train -- [options]
//!
//!   --format 3max|6max|tout   IA à entraîner (tout par défaut)
//!   --mains N                 mains simulées par IA (40 000 par défaut)
//!   --archives DOSSIER        archives réelles : DOSSIER/expresso-3max, DOSSIER/tournois-6max
//!   --ecrire-archives DOSSIER écrit les mains simulées en JSON Lines
//!   --tirages N               tirages Monte-Carlo par équité de la référence (600)
//!   --graine N                graine (reproductibilité)
//!   --rapide                  réglages réduits, pour un essai
//!   --rapport FICHIER         rapport Markdown (docs/entrainement.md par défaut)
//! ```
//!
//! À lancer depuis la racine du dépôt.

use std::path::PathBuf;
use std::time::Instant;

use stratyx_ai_training::{Format, Options, train_format, write_bundle};

fn today() -> String {
    // Date civile UTC à partir de l'horloge système (algorithme de H. Hinnant).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() -> std::io::Result<()> {
    let mut formats = Format::ALL.to_vec();
    let mut options = Options {
        simulated_hands: 40_000,
        archives: None,
        write_archives: None,
        reference_samples: 600,
        quick: false,
        seed: 0x5354_5241_5459_5821,
        date: today(),
    };
    let mut report_path = PathBuf::from("docs/entrainement.md");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("valeur manquante après {a}");
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--format" => {
                formats = match value().as_str() {
                    "3max" | "3" | "expresso" => vec![Format::Expresso3Max],
                    "6max" | "6" | "tournois" => vec![Format::Tournament6Max],
                    _ => Format::ALL.to_vec(),
                }
            }
            "--mains" => options.simulated_hands = value().parse().unwrap_or(40_000),
            "--archives" => options.archives = Some(PathBuf::from(value())),
            "--ecrire-archives" => options.write_archives = Some(PathBuf::from(value())),
            "--tirages" => options.reference_samples = value().parse().unwrap_or(600),
            "--graine" => options.seed = value().parse().unwrap_or(options.seed),
            "--rapport" => report_path = PathBuf::from(value()),
            "--rapide" => {
                options.quick = true;
                options.simulated_hands = options.simulated_hands.min(2_000);
                options.reference_samples = options.reference_samples.min(200);
            }
            "-h" | "--help" => {
                println!(
                    "{}",
                    include_str!("main.rs")
                        .lines()
                        .skip(2)
                        .take(14)
                        .map(|l| l.trim_start_matches("//! "))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                return Ok(());
            }
            other => {
                eprintln!("option inconnue : {other} (voir --help)");
                std::process::exit(2);
            }
        }
    }

    let mut sections = Vec::new();
    for format in formats {
        let start = Instant::now();
        let outcome = train_format(format, &options, |l| println!("{l}"))?;
        let path = write_bundle(std::path::Path::new("."), format, &outcome.bundle)?;
        println!(
            "✓ {} : {} ({:.0} s)",
            format.title(),
            path.display(),
            start.elapsed().as_secs_f32()
        );
        sections.push(outcome.report);
    }
    let header = "# Entraînement des IA\n\n\
        Rapport écrit par `stratyx-train` (`ai/training`). Les modèles sont embarqués par \
        `ai/expresso_3max` et `ai/tournament_6max`, et utilisés par l'analyse des mains \
        (`analytics`), l'outil `stratyx analyse` et la page `/app` du site.\n\n\
        Relancer : `cargo run --release -p stratyx-ai-training --bin stratyx-train` \
        (ajoutez `--archives <dossier>` pour vos propres historiques).\n\n";
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report_path, format!("{header}{}", sections.join("\n")))?;
    println!("✓ rapport : {}", report_path.display());
    Ok(())
}
