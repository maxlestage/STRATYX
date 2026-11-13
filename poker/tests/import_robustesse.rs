//! Robustesse de l'import d'historiques : des textes abîmés (tronqués, lignes manquantes,
//! caractères inattendus) doivent donner des erreurs, jamais une panique. Fuzzing
//! déterministe à partir des historiques d'exemple.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_poker::import::{SAMPLE_POKERSTARS, SAMPLE_WINAMAX, import_text};

/// Importe `text` et rejoue chaque main lue (comme le fait l'analyse) ; renvoie le message
/// de la panique éventuelle.
fn survives(text: &str) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(|| {
        let report = import_text(text);
        for h in &report.hands {
            let _ = h.record.replay();
        }
    }))
    .map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    })
}

fn check(text: &str, what: &str, failures: &mut Vec<String>) {
    if let Err(msg) = survives(text) {
        failures.push(format!(
            "{what} : {msg}\n--- texte ---\n{text}\n-------------"
        ));
    }
}

fn samples() -> [&'static str; 2] {
    [SAMPLE_POKERSTARS, SAMPLE_WINAMAX]
}

#[test]
fn truncated_histories_never_panic() {
    let mut failures = Vec::new();
    for sample in samples() {
        for (i, _) in sample.char_indices() {
            check(&sample[..i], &format!("tronqué à {i}"), &mut failures);
        }
    }
    assert!(
        failures.is_empty(),
        "{} paniques, dont :\n{}",
        failures.len(),
        failures[0]
    );
}

#[test]
fn missing_or_duplicated_lines_never_panic() {
    let mut failures = Vec::new();
    for sample in samples() {
        let lines: Vec<&str> = sample.lines().collect();
        for skip in 0..lines.len() {
            let mut without = lines.clone();
            without.remove(skip);
            check(
                &without.join("\n"),
                &format!("sans la ligne {skip}"),
                &mut failures,
            );
            let mut twice = lines.clone();
            twice.insert(skip, lines[skip]);
            check(
                &twice.join("\n"),
                &format!("ligne {skip} doublée"),
                &mut failures,
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} paniques, dont :\n{}",
        failures.len(),
        failures[0]
    );
}

/// Position valide (frontière de caractère) au plus égale à `at`.
fn boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[test]
fn random_corruptions_never_panic() {
    // Fragments plausibles dans un historique abîmé ou venu d'un autre site.
    const NOISE: &[&str] = &[
        "é",
        "€",
        "💥",
        "\u{feff}",
        "\r",
        "\t",
        ":",
        "[",
        "]",
        "(",
        ")",
        "#",
        "$",
        "-",
        " ",
        "0",
        "9",
        "99999999999999999999",
        "-1",
        "1.5",
        "Seat ",
        "*** ",
        "folds",
        "raises",
        "to",
        "and is all-in",
        "collected",
        "[Ah Kh]",
        "[Xx]",
        "\n",
    ];
    let mut rng = ChaCha8Rng::seed_from_u64(0x5354_5241);
    let mut failures = Vec::new();
    for round in 0..4_000 {
        let sample = *samples().choose(&mut rng).expect("échantillons");
        let mut text = sample.to_owned();
        for _ in 0..rng.random_range(1..6) {
            let at = boundary(&text, rng.random_range(0..=text.len()));
            let noise = *NOISE.choose(&mut rng).expect("bruit");
            match rng.random_range(0..3) {
                0 => text.insert_str(at, noise),
                1 => {
                    let end = boundary(&text, at + rng.random_range(1..12));
                    text.replace_range(at..end, "");
                }
                _ => {
                    let end = boundary(&text, at + rng.random_range(1..6));
                    text.replace_range(at..end, noise);
                }
            }
        }
        check(&text, &format!("corruption n° {round}"), &mut failures);
    }
    assert!(
        failures.is_empty(),
        "{} paniques, dont :\n{}",
        failures.len(),
        failures[0]
    );
}

#[test]
fn huge_stacks_are_rejected_not_overflowed() {
    // Trouvé par le fuzzing : des tapis proches de 2⁶⁴ faisaient déborder la répartition des
    // pots au rejeu. La main doit être refusée proprement.
    let text = SAMPLE_POKERSTARS
        .replace("(2350 in chips)", "(18446744073709551615 in chips)")
        .replace("(1800 in chips)", "(18446744073709551615 in chips)");
    assert_eq!(survives(&text), Ok(()));
    let report = import_text(&text);
    assert!(
        report.hands.iter().all(|h| h.record.replay().is_err()),
        "{report:?}"
    );
}
