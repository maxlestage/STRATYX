//! Robustesse de la vision face à des images quelconques : la page `/app` lit dans le
//! navigateur des captures d'écran et des vidéos de n'importe quelle taille et de n'importe
//! quel contenu, en WebAssembly compilé avec `panic = "abort"` — une panique y ferait planter
//! la page. Images dégénérées (1 pixel de large, très allongées), bruit, aplats, rectangles
//! clairs qui ressemblent à des cartes (y compris coupés par le bord), rectangles quelconques
//! pour `crop` et `read_corner`, et images de tailles différentes d'une trame à l'autre.

use std::panic::{AssertUnwindSafe, catch_unwind};

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_vision::{
    Frame, Keyframer, Rect, Templates, arrange, detect_cards, read_corner, recognize,
};

/// Une image au hasard : taille parfois dégénérée, fond uni ou bruité, rectangles clairs.
fn random_frame(rng: &mut ChaCha8Rng) -> Frame {
    let (w, h) = *[
        (1, 1),
        (1, 300),
        (300, 1),
        (2, 2),
        (3, 1000),
        (1500, 2),
        (17, 9),
        (64, 64),
    ]
    .choose(rng)
    .filter(|_| rng.random_bool(0.3))
    .unwrap_or(&(rng.random_range(1..=640), rng.random_range(1..=480)));
    let base: [u8; 3] = [rng.random(), rng.random(), rng.random()];
    let noise = rng.random_range(0..=255u8);
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for _ in 0..w * h {
        for c in base {
            let n = if noise == 0 {
                0
            } else {
                rng.random_range(0..=noise)
            };
            rgba.push(c.saturating_add(n));
        }
        rgba.push(255);
    }
    // Des rectangles clairs (cartes possibles), parfois coupés par le bord de l'image.
    for _ in 0..rng.random_range(0..8) {
        let rw = rng.random_range(1..=w.max(2));
        let rh = rng.random_range(1..=h.max(2));
        let x0 = rng.random_range(0..w);
        let y0 = rng.random_range(0..h);
        let shade = rng.random_range(180..=255u8);
        for y in y0..(y0 + rh).min(h) {
            for x in x0..(x0 + rw).min(w) {
                let i = (y as usize * w as usize + x as usize) * 4;
                rgba[i..i + 3].copy_from_slice(&[shade; 3]);
            }
        }
    }
    Frame::new(w, h, rgba).expect("tampon à la bonne taille")
}

fn random_rect(rng: &mut ChaCha8Rng, frame: &Frame) -> Rect {
    let pick = |rng: &mut ChaCha8Rng, size: u32| -> u32 {
        *[
            0,
            1,
            size.saturating_sub(1),
            size,
            size + 1,
            u32::MAX / 2,
            u32::MAX,
        ]
        .choose(rng)
        .filter(|_| rng.random_bool(0.4))
        .unwrap_or(&rng.random_range(0..=size.saturating_mul(2)))
    };
    Rect {
        x: pick(rng, frame.width),
        y: pick(rng, frame.height),
        w: pick(rng, frame.width),
        h: pick(rng, frame.height),
    }
}

#[test]
fn arbitrary_images_never_panic() {
    let templates = Templates::default();
    let mut rng = ChaCha8Rng::seed_from_u64(0x5649_5349);
    let mut keyframer = Keyframer::new();
    let mut failures = Vec::new();
    let mut cards_seen = 0usize;
    for round in 0..300 {
        let frame = random_frame(&mut rng);
        let rects: Vec<Rect> = (0..4).map(|_| random_rect(&mut rng, &frame)).collect();
        let size = (frame.width, frame.height);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            // Le parcours de `/app` : images clés, détection, disposition, reconnaissance.
            keyframer.push(&frame);
            let cards = detect_cards(&frame);
            let _ = arrange(&frame, &cards);
            for c in &cards {
                let _ = recognize(&templates, &c.patch);
            }
            for r in &rects {
                let _ = frame.crop(*r);
                let _ = read_corner(&frame, *r);
            }
            let _ = frame.downscale(rng.random_range(0..=frame.width + 1));
            cards.len()
        }));
        match outcome {
            Ok(n) => cards_seen += n,
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_default();
                failures.push(format!("image n° {round} ({size:?}), {rects:?} : {msg}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} paniques, dont :\n{}",
        failures.len(),
        failures[0]
    );
    // Garde-fou du test : les rectangles clairs sont bien pris pour des cartes, parfois.
    assert!(cards_seen > 0, "aucune carte détectée");
}

/// Les modèles appris sont relus depuis le stockage du navigateur (JSON) : écrits par une
/// ancienne version ou abîmés, ils peuvent être incohérents (grilles de mauvaise taille,
/// rang 200, couleur 9, aspects démesurés). Lecture refusée ou carte valide, jamais de
/// panique.
#[test]
fn inconsistent_stored_templates_never_panic() {
    use stratyx_vision::{InkColor, Patch};

    let stored = [
        r#"{"ranks":[],"suits":[]}"#,
        r#"{"ranks":[{"rank":200,"features":[0.5],"aspect":0.7,"parts":1}],"suits":[{"suit":9,"features":[],"aspect":1.0,"ink":"Red"}]}"#,
        r#"{"ranks":[{"rank":12,"features":[],"aspect":1e30,"parts":255}],"suits":[{"suit":2,"features":[1.0,1.0],"aspect":-3.0,"ink":"Unknown"}]}"#,
        r#"{"ranks":[{"rank":0,"features":[0.0],"aspect":0.0,"parts":0},{"rank":0,"features":[0.0],"aspect":0.0,"parts":0}],"suits":[{"suit":0,"features":[0.0],"aspect":0.0,"ink":"Black"}]}"#,
    ];
    let full = |n: usize, v: f32| vec![v; n];
    let patches = [
        Patch {
            rank: full(140, 0.3),
            rank_aspect: 0.7,
            rank_parts: 1,
            suit: full(100, 0.4),
            suit_aspect: 1.0,
            ink: InkColor::Red,
        },
        Patch {
            rank: Vec::new(),
            rank_aspect: f32::NAN,
            rank_parts: 9,
            suit: full(3, 1.0),
            suit_aspect: f32::INFINITY,
            ink: InkColor::Unknown,
        },
        Patch {
            rank: full(140, f32::NAN),
            rank_aspect: 1e30,
            rank_parts: 2,
            suit: Vec::new(),
            suit_aspect: 0.0,
            ink: InkColor::Black,
        },
    ];
    let mut failures = Vec::new();
    for json in stored {
        let templates: Templates = serde_json::from_str(json).expect(json);
        for (k, patch) in patches.iter().enumerate() {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let read = recognize(&templates, patch);
                if let Some(r) = read {
                    let c = r.confidence().get();
                    assert!((0.0..=1.0).contains(&c), "{r:?}");
                }
                let mut learner = templates.clone();
                learner.learn(patch, stratyx_poker::Card::all().next().expect("une carte"));
                let _ = recognize(&learner, patch);
            }));
            if outcome.is_err() {
                failures.push(format!("modèles {json}, coin n° {k}"));
            }
        }
    }
    assert!(failures.is_empty(), "paniques : {failures:#?}");
}

#[test]
fn an_unreadable_confidence_is_never_certain() {
    use stratyx_vision::Recognized;

    let card = stratyx_poker::Card::all().next().expect("une carte");
    let read = |confidence: f32| Recognized { card, confidence }.confidence().get();
    assert_eq!(read(f32::NAN), 0.0);
    assert_eq!(read(-0.5), 0.0);
    assert_eq!(read(2.0), 1.0);
    assert_eq!(read(0.4), 0.4);
}
