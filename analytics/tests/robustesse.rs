//! Robustesse de l'analyse des mains sur des historiques abîmés : la page `/app` du site
//! l'exécute dans le navigateur, en WebAssembly compilé avec `panic = "abort"` ; la moindre
//! panique y ferait planter la page. Fuzzing déterministe du chemin complet du site : import,
//! analyse de chaque main (avec ou sans pseudo du héros), bilan de session.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_analytics::review::Options;
use stratyx_analytics::session::SessionBuilder;
use stratyx_poker::import::{SAMPLE_POKERSTARS, SAMPLE_WINAMAX, import_text};

/// Peu de tirages : on cherche les paniques, pas la précision.
const OPTIONS: Options = Options {
    samples: 40,
    seed: 7,
};

/// Le parcours de la page `/app` : chaque main analysée pour le héros donné (ou celui de
/// l'historique), puis le bilan de session.
fn survives(text: &str, hero: &str) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(|| {
        let report = import_text(text);
        let mut session = SessionBuilder::new();
        for (k, h) in report.hands.iter().enumerate() {
            let hero = if hero.is_empty() {
                h.hero.as_deref()
            } else {
                Some(hero)
            };
            match stratyx_analytics::analyse(&h.record, hero, &OPTIONS) {
                Ok(a) => session.add(k, &h.record, &a),
                Err(_) => session.skip(),
            }
        }
        let _ = session.finish();
    }))
    .map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    })
}

/// Position valide (frontière de caractère) au plus égale à `at`.
fn boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn assert_none(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} paniques, dont :\n{}",
        failures.len(),
        failures[0]
    );
}

/// Pseudos essayés : celui de l'historique, des joueurs de la table, un inconnu, du bruit.
const HEROES: &[&str] = &["", "Hero", "Villain One", "Bouton", "Inconnu", " ", "💥"];

#[test]
fn every_seat_as_hero_on_intact_histories() {
    // Garde-fou du test lui-même : sur les historiques intacts, l'analyse aboutit vraiment.
    for sample in [SAMPLE_POKERSTARS, SAMPLE_WINAMAX] {
        let report = import_text(sample);
        assert_eq!(report.hands.len(), 1);
        let h = &report.hands[0];
        let analysis = stratyx_analytics::analyse(&h.record, h.hero.as_deref(), &OPTIONS)
            .expect("analyse de l'exemple");
        assert!(!analysis.decisions.is_empty());
    }
    let mut failures = Vec::new();
    for sample in [SAMPLE_POKERSTARS, SAMPLE_WINAMAX] {
        for hero in HEROES {
            if let Err(msg) = survives(sample, hero) {
                failures.push(format!("héros « {hero} » : {msg}"));
            }
        }
    }
    assert_none(&failures);
}

#[test]
fn damaged_histories_never_panic() {
    const NOISE: &[&str] = &[
        "é",
        "€",
        "\u{feff}",
        "\r",
        ":",
        "[",
        "]",
        "(",
        "#",
        "-",
        " ",
        "0",
        "9",
        "99999999999999999999",
        "1.5",
        "Seat ",
        "*** ",
        "folds",
        "calls",
        "raises",
        "to",
        "and is all-in",
        "collected",
        "[Ah Kh]",
        "\n",
    ];
    let mut rng = ChaCha8Rng::seed_from_u64(0x414E_414C);
    let mut failures = Vec::new();
    for round in 0..400 {
        let sample = *[SAMPLE_POKERSTARS, SAMPLE_WINAMAX]
            .choose(&mut rng)
            .expect("échantillons");
        let mut text = sample.to_owned();
        for _ in 0..rng.random_range(1..4) {
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
        let hero = *HEROES.choose(&mut rng).expect("héros");
        if let Err(msg) = survives(&text, hero) {
            failures.push(format!(
                "corruption n° {round}, héros « {hero} » : {msg}\n--- texte ---\n{text}"
            ));
        }
    }
    assert_none(&failures);
}
