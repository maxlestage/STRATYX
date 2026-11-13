//! Robustesse de l'import PokerStars de la base locale : des fichiers abîmés (tronqués,
//! lignes manquantes ou doublées, caractères inattendus) doivent donner des erreurs
//! localisées, jamais une panique ; et toute main acceptée malgré tout doit être cohérente :
//! exportée puis relue, elle redonne exactement la même main. Fuzzing déterministe à partir
//! des historiques réels de `tests/data/`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_database::text::{
    ExportOptions, ImportOptions, export_pokerstars, import_pokerstars, import_pokerstars_hand,
};

const SAMPLES: [&str; 3] = [
    include_str!("data/pokerstars_cash_nl6.txt"),
    include_str!("data/pokerstars_tournament_antes.txt"),
    include_str!("data/pokerstars_fixed_limit.txt"),
];

/// Importe `text`, rejoue chaque main lue et vérifie l'aller-retour export → import des
/// mains valides ; renvoie le message de la panique ou de l'incohérence éventuelle.
fn survives(text: &str) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(|| {
        for hand in import_pokerstars(text, &ImportOptions::default())
            .into_iter()
            .flatten()
        {
            if hand.record.replay().is_err() {
                continue;
            }
            let options = ExportOptions {
                currency: hand.currency.clone(),
                tournament: hand.tournament.clone(),
            };
            let Ok(exported) = export_pokerstars(&hand.record, &options) else {
                continue;
            };
            match import_pokerstars_hand(&exported, &ImportOptions::default()) {
                Ok(back) => assert!(
                    back.record == hand.record,
                    "aller-retour différent :\n{exported}"
                ),
                Err(e) => panic!("export illisible ({e}) :\n{exported}"),
            }
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

fn assert_none(failures: &[String]) {
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
fn seats_outside_the_table_or_out_of_order_are_rejected() {
    // Trouvé par le fuzzing : « Seat 91 » à une table 6-max était accepté, et l'ordre des
    // sièges changeait à l'aller-retour.
    let cash = SAMPLES[0];
    let first_seat = cash
        .lines()
        .find(|l| l.starts_with("Seat "))
        .expect("un siège");
    let number = first_seat
        .trim_start_matches("Seat ")
        .split(':')
        .next()
        .expect("numéro");
    for bad in ["91", "0", "7"] {
        let text = cash.replacen(first_seat, &first_seat.replacen(number, bad, 1), 1);
        let first = import_pokerstars(&text, &ImportOptions::default())
            .into_iter()
            .next()
            .expect("une main");
        assert!(first.is_err(), "siège {bad} accepté");
    }
    let mut lines: Vec<&str> = cash.lines().collect();
    let i = lines
        .iter()
        .position(|l| l.starts_with("Seat "))
        .expect("un siège");
    lines.swap(i, i + 1);
    let first = import_pokerstars(&lines.join("\n"), &ImportOptions::default())
        .into_iter()
        .next()
        .expect("une main");
    assert!(first.is_err(), "sièges dans le désordre acceptés");
}

#[test]
fn truncated_files_never_panic() {
    let mut failures = Vec::new();
    for sample in SAMPLES {
        for (i, _) in sample.char_indices() {
            check(&sample[..i], &format!("tronqué à {i}"), &mut failures);
        }
    }
    assert_none(&failures);
}

#[test]
fn missing_or_duplicated_lines_never_panic() {
    let mut failures = Vec::new();
    for sample in SAMPLES {
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
    assert_none(&failures);
}

#[test]
fn random_corruptions_never_panic() {
    // Fragments plausibles dans un fichier abîmé, ré-encodé ou venu d'une autre version.
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
        ".",
        ",",
        "/",
        "99999999999999999999",
        "18446744073709551615",
        "-1",
        "1.5",
        "0.001",
        "Seat ",
        "*** ",
        "folds",
        "calls",
        "raises",
        "to",
        "and is all-in",
        "collected",
        "posts",
        "[Ah Kh]",
        "[Xx]",
        "\n",
        "\n\n\n",
    ];
    let mut rng = ChaCha8Rng::seed_from_u64(0x4241_5345);
    let mut failures = Vec::new();
    for round in 0..3_000 {
        let sample = *SAMPLES.choose(&mut rng).expect("échantillons");
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
    assert_none(&failures);
}
