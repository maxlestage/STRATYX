//! Tests sur des tables synthétiques : tapis avec dégradé et bruit, cartes arrondies
//! dessinées avec une police bitmap, jeux à 2 et 4 couleurs, cartes du joueur qui se
//! chevauchent, board de 0 à 5 cartes, cartes montrées, jetons et dos de carte pour
//! brouiller.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_poker::{Card, Rank, Suit};

use crate::font;
use crate::{Frame, Keyframer, Rect, Templates, arrange, detect_cards, recognize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Hero,
    Board,
    Other,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // `role` sert aux messages de diagnostic
struct Truth {
    rect: Rect,
    card: Card,
    role: Role,
}

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32, rng: &mut ChaCha8Rng, noise: i32) -> Canvas {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 / w as f32 - 0.5;
                let dy = y as f32 / h as f32 - 0.45;
                let v = 1.0 - (dx * dx + dy * dy).sqrt() * 0.9;
                let n = rng.random_range(-noise..=noise);
                let i = ((y * w + x) * 4) as usize;
                px[i] = (18.0 * v) as u8;
                px[i + 1] = (40.0 + 70.0 * v + n as f32).clamp(0.0, 255.0) as u8;
                px[i + 2] = (30.0 + 30.0 * v) as u8;
                px[i + 3] = 255;
            }
        }
        Canvas { w, h, px }
    }

    fn set(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        self.px[i..i + 3].copy_from_slice(&c);
    }

    fn rounded(&mut self, r: Rect, radius: f32, c: [u8; 3]) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let fx = (x - r.x) as f32 + 0.5;
                let fy = (y - r.y) as f32 + 0.5;
                let cx = fx.clamp(radius, r.w as f32 - radius);
                let cy = fy.clamp(radius, r.h as f32 - radius);
                if (fx - cx).powi(2) + (fy - cy).powi(2) <= radius * radius {
                    self.set(x as i32, y as i32, c);
                }
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3]) {
        for y in (cy - r) as i32..=(cy + r) as i32 {
            for x in (cx - r) as i32..=(cx + r) as i32 {
                if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) <= r * r {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn glyph(&mut self, glyph: &[&str], x: f32, y: f32, h: f32, c: [u8; 3]) {
        let (gw, gh) = font::size(glyph, false);
        let w = h * gw as f32 / gh as f32;
        for py in y as i32..(y + h) as i32 {
            for px in x as i32..(x + w) as i32 {
                let sx = ((px as f32 + 0.5 - x) / w * gw as f32).floor() as i32;
                let sy = ((py as f32 + 0.5 - y) / h * gh as f32).floor() as i32;
                if font::lit(glyph, sx, sy, false) {
                    self.set(px, py, c);
                }
            }
        }
    }

    fn frame(self) -> Frame {
        Frame::new(self.w, self.h, self.px).expect("taille")
    }
}

fn ink(suit: Suit, four: bool) -> [u8; 3] {
    match (suit, four) {
        (Suit::Hearts, _) | (Suit::Diamonds, false) => [205, 30, 45],
        (Suit::Diamonds, true) => [30, 70, 215],
        (Suit::Clubs, true) => [20, 145, 55],
        _ => [22, 22, 26],
    }
}

fn draw_card(cv: &mut Canvas, x: f32, y: f32, h: f32, card: Card, four: bool) -> Rect {
    let w = h * 0.72;
    let r = Rect {
        x: x as u32,
        y: y as u32,
        w: w as u32,
        h: h as u32,
    };
    cv.rounded(r, w * 0.09, [246, 244, 238]);
    let c = ink(card.suit(), four);
    let rank = font::RANKS[card.rank().index() as usize];
    let suit = font::SUITS[card.suit().index() as usize];
    cv.glyph(rank, x + w * 0.08, y + h * 0.06, h * 0.2, c);
    cv.glyph(suit, x + w * 0.08, y + h * 0.31, h * 0.13, c);
    cv.glyph(suit, x + w * 0.33, y + h * 0.36, h * 0.3, c);
    r
}

fn card_back(cv: &mut Canvas, x: f32, y: f32, h: f32) {
    let w = h * 0.72;
    let r = Rect {
        x: x as u32,
        y: y as u32,
        w: w as u32,
        h: h as u32,
    };
    cv.rounded(r, w * 0.09, [240, 240, 240]);
    let b = (h * 0.04).max(2.0);
    let inner = Rect {
        x: (x + b) as u32,
        y: (y + b) as u32,
        w: (w - 2.0 * b) as u32,
        h: (h - 2.0 * b) as u32,
    };
    cv.rounded(inner, w * 0.06, [30, 50, 150]);
    for k in 0..6 {
        let yy = inner.y + inner.h * k / 6;
        for xx in inner.x..inner.x + inner.w {
            cv.set(xx as i32, yy as i32, [70, 100, 200]);
        }
    }
}

/// Une table : cartes du joueur, board, cartes montrées, dos de carte et jetons.
fn table(
    w: u32,
    hero: [Card; 2],
    board: &[Card],
    shown: &[Card],
    four: bool,
    seed: u64,
) -> (Frame, Vec<Truth>) {
    let h = w * 9 / 16;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut cv = Canvas::new(w, h, &mut rng, 6);
    let (fw, fh) = (w as f32, h as f32);
    let mut truth = Vec::new();
    // Jetons (avec des points blancs) et un dos de carte.
    for _ in 0..6 {
        let cx = rng.random_range(0.1..0.9) * fw;
        let cy = rng.random_range(0.6..0.68) * fh;
        let r = fh * 0.025;
        cv.circle(cx, cy, r, [190, 40, 40]);
        for k in 0..4 {
            let a = k as f32 * std::f32::consts::FRAC_PI_2;
            cv.circle(
                cx + a.cos() * r * 0.75,
                cy + a.sin() * r * 0.75,
                r * 0.15,
                [250, 250, 250],
            );
        }
    }
    card_back(&mut cv, fw * 0.82, fh * 0.12, fh * 0.11);
    // Board.
    let bh = fh * 0.12;
    let bw = bh * 0.72;
    let gap = bw * 0.12;
    let total = 5.0 * bw + 4.0 * gap;
    let x0 = (fw - total) / 2.0;
    for (i, c) in board.iter().enumerate() {
        let r = draw_card(&mut cv, x0 + i as f32 * (bw + gap), fh * 0.38, bh, *c, four);
        truth.push(Truth {
            rect: r,
            card: *c,
            role: Role::Board,
        });
    }
    // Cartes du joueur : la droite chevauche la gauche, un peu plus bas.
    let hh = fh * 0.15;
    let hw = hh * 0.72;
    let hx = fw / 2.0 - hw * 0.8;
    let r1 = draw_card(&mut cv, hx, fh * 0.74, hh, hero[0], four);
    let r2 = draw_card(&mut cv, hx + hw * 0.6, fh * 0.75, hh, hero[1], four);
    truth.push(Truth {
        rect: r1,
        card: hero[0],
        role: Role::Hero,
    });
    truth.push(Truth {
        rect: r2,
        card: hero[1],
        role: Role::Hero,
    });
    // Cartes montrées par un adversaire, en haut à gauche, côte à côte.
    let sh = fh * 0.1;
    for (i, c) in shown.iter().enumerate() {
        let r = draw_card(
            &mut cv,
            fw * 0.12 + i as f32 * sh * 0.8,
            fh * 0.1,
            sh,
            *c,
            four,
        );
        truth.push(Truth {
            rect: r,
            card: *c,
            role: Role::Other,
        });
    }
    (cv.frame(), truth)
}

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

/// La vérité terrain qui correspond à une détection (centre contenu).
fn matching(truth: &[Truth], r: Rect) -> Option<Truth> {
    let (cx, cy) = r.center();
    truth
        .iter()
        .filter(|t| t.rect.contains(cx, cy))
        .max_by_key(|t| t.rect.intersection(&r))
        .copied()
}

fn deck(seed: u64) -> Vec<Card> {
    let mut d: Vec<Card> = Card::all().collect();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    for i in (1..d.len()).rev() {
        let j = rng.random_range(0..=i);
        d.swap(i, j);
    }
    d
}

#[test]
fn detects_and_arranges_tables() {
    let hero = [card("Ah"), card("Kc")];
    let board = [card("Qd"), card("Ts"), card("7h"), card("2c"), card("9d")];
    let shown = [card("Js"), card("Jd")];
    for &w in &[640u32, 1024, 1280, 1920] {
        for n in [0usize, 3, 4, 5] {
            for four in [false, true] {
                let (f, truth) = table(w, hero, &board[..n], &shown, four, w as u64 + n as u64);
                let cards = detect_cards(&f);
                assert_eq!(
                    cards.len(),
                    truth.len(),
                    "{w} px, board {n}, 4 couleurs {four} : {:?}",
                    cards.iter().map(|c| c.rect).collect::<Vec<_>>()
                );
                let read = arrange(&f, &cards);
                assert_eq!(read.board.len(), n, "{w} {n} {four}");
                assert_eq!(read.hero.len(), 2, "{w} {n} {four}");
                let hero_truth: Vec<Card> = read
                    .hero
                    .iter()
                    .map(|i| matching(&truth, cards[*i].rect).expect("héros").card)
                    .collect();
                assert_eq!(hero_truth, hero.to_vec(), "{w} {n} {four}");
                let board_truth: Vec<Card> = read
                    .board
                    .iter()
                    .map(|i| matching(&truth, cards[*i].rect).expect("board").card)
                    .collect();
                assert_eq!(board_truth, board[..n].to_vec(), "{w} {n} {four}");
                assert_eq!(read.others.len(), shown.len());
            }
        }
    }
}

/// Apprend sur des tables à 1280 px, puis lit des tables à d'autres tailles.
fn learned(four: bool) -> Templates {
    let mut t = Templates::default();
    let d = deck(11);
    for (k, chunk) in d.chunks(9).enumerate() {
        if chunk.len() < 9 {
            continue;
        }
        let (f, truth) = table(
            1280,
            [chunk[0], chunk[1]],
            &chunk[2..7],
            &chunk[7..9],
            four,
            100 + k as u64,
        );
        for c in detect_cards(&f) {
            if let Some(tr) = matching(&truth, c.rect) {
                t.learn(&c.patch, tr.card);
            }
        }
    }
    // Les cartes restantes du paquet.
    let rest = &d[d.len() - d.len() % 9..];
    if !rest.is_empty() {
        let mut v = rest.to_vec();
        while v.len() < 9 {
            v.push(d[v.len()]);
        }
        let (f, truth) = table(1280, [v[0], v[1]], &v[2..7], &v[7..9], four, 99);
        for c in detect_cards(&f) {
            if let Some(tr) = matching(&truth, c.rect) {
                t.learn(&c.patch, tr.card);
            }
        }
    }
    t
}

#[test]
fn learned_templates_read_other_tables() {
    for four in [false, true] {
        let t = learned(four);
        assert_eq!(t.ranks_known(), 13);
        assert_eq!(t.suits_known(), 4);
        let mut ok = 0;
        let mut total = 0;
        let mut low = 0;
        let mut wrong_sure = 0;
        for (k, &w) in [1000u32, 1600, 1920, 1152].iter().enumerate() {
            let d = deck(500 + k as u64);
            for (j, chunk) in d.chunks(9).take(5).enumerate() {
                let (f, truth) = table(
                    w,
                    [chunk[0], chunk[1]],
                    &chunk[2..7],
                    &chunk[7..9],
                    four,
                    1000 + (k * 10 + j) as u64,
                );
                for c in detect_cards(&f) {
                    let Some(tr) = matching(&truth, c.rect) else {
                        continue;
                    };
                    total += 1;
                    if let Some(r) = t.classify(&c.patch) {
                        if r.card == tr.card {
                            ok += 1;
                        } else if r.confidence >= 0.8 {
                            wrong_sure += 1;
                        }
                        if r.confidence < 0.5 {
                            low += 1;
                        }
                    }
                }
            }
        }
        let accuracy = ok as f32 / total as f32;
        eprintln!(
            "modèles appris (4 couleurs : {four}) : {ok}/{total} = {:.1} %, {low} lectures sous 0,5, {wrong_sure} erreurs à confiance ≥ 0,8",
            accuracy * 100.0
        );
        assert!(accuracy >= 0.95, "{accuracy}");
    }
}

#[test]
fn generic_guess_is_capped() {
    let empty = Templates::default();
    let d = deck(7);
    let mut ok = 0;
    let mut total = 0;
    for (j, chunk) in d.chunks(9).take(5).enumerate() {
        let (f, truth) = table(
            1280,
            [chunk[0], chunk[1]],
            &chunk[2..7],
            &chunk[7..9],
            false,
            70 + j as u64,
        );
        for c in detect_cards(&f) {
            let Some(tr) = matching(&truth, c.rect) else {
                continue;
            };
            total += 1;
            let r = recognize(&empty, &c.patch).expect("une hypothèse");
            assert!(r.confidence <= 0.5);
            ok += usize::from(r.card == tr.card);
        }
    }
    eprintln!(
        "police générique (même dessin que la police intégrée) : {ok}/{total} = {:.1} %",
        ok as f32 / total as f32 * 100.0
    );
    assert!(ok * 2 > total, "{ok}/{total}");
}

#[test]
fn learned_templates_win_over_the_generic_guess() {
    let t = learned(false);
    let (f, truth) = table(
        1600,
        [card("8s"), card("8h")],
        &[card("Ac"), card("5d"), card("Kh")],
        &[],
        false,
        3,
    );
    for c in detect_cards(&f) {
        let tr = matching(&truth, c.rect).unwrap();
        let r = recognize(&t, &c.patch).unwrap();
        assert_eq!(r.card, tr.card);
        assert!(r.confidence > 0.5, "{r:?}");
        assert!(r.confidence().get() > 0.5);
    }
}

#[test]
fn keyframer_keeps_only_changes() {
    let hero = [card("Ah"), card("Kc")];
    let flop = [card("Qd"), card("Ts"), card("7h")];
    let turn = [card("Qd"), card("Ts"), card("7h"), card("2c")];
    let mut k = Keyframer::new();
    let (a, _) = table(1280, hero, &flop, &[], false, 1);
    // Même table, avec un bruit de capture supplémentaire.
    let mut b = a.clone();
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    for v in b.rgba.as_chunks_mut::<4>().0 {
        for c in &mut v[..3] {
            *c = (i32::from(*c) + rng.random_range(-8..=8)).clamp(0, 255) as u8;
        }
    }
    let (c, _) = table(1280, hero, &turn, &[], false, 1);
    assert!(k.push(&a));
    assert!(!k.push(&b), "le bruit seul ne compte pas");
    assert!(k.push(&c), "la turn est une nouvelle image");
    assert!(!k.push(&c));
}

#[test]
fn templates_roundtrip_and_frame_tools() {
    let t = learned(true);
    let json = serde_json::to_string(&t).unwrap();
    let back: Templates = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
    assert!(!t.is_empty() && t.len() >= 17);
    assert!(Frame::new(2, 2, vec![0; 15]).is_none());
    let (f, _) = table(1280, [card("Ah"), card("Kc")], &[], &[], false, 1);
    let small = f.downscale(320);
    assert_eq!((small.width, small.height), (320, 180));
    assert_eq!(f.downscale(4000), f);
    let c = f.crop(Rect {
        x: 1270,
        y: 710,
        w: 50,
        h: 50,
    });
    assert_eq!((c.width, c.height), (10, 10));
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "mesure de temps : en mode release seulement"
)]
fn fast_enough() {
    let t = learned(false);
    let d = deck(3);
    let (f, _) = table(1280, [d[0], d[1]], &d[2..7], &d[7..9], false, 5);
    let start = std::time::Instant::now();
    let runs = 20;
    for _ in 0..runs {
        let cards = detect_cards(&f);
        let _ = arrange(&f, &cards);
        for c in &cards {
            let _ = recognize(&t, &c.patch);
        }
    }
    let per = start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
    eprintln!("1280 × 720 : {per:.2} ms par image (détection, rangement, reconnaissance)");
    assert!(per < 50.0, "{per} ms");
    let _ = (Rank::Ace, Suit::Spades);
}

#[test]
#[ignore = "diagnostic"]
fn diagnose() {
    let t = learned(false);
    let d = deck(500);
    for (j, chunk) in d.chunks(9).take(3).enumerate() {
        let (f, truth) = table(
            1600,
            [chunk[0], chunk[1]],
            &chunk[2..7],
            &chunk[7..9],
            false,
            1000 + j as u64,
        );
        for c in detect_cards(&f) {
            let Some(tr) = matching(&truth, c.rect) else {
                continue;
            };
            let (r, s) = crate::recognize::debug_distances(&t, &c.patch);
            eprintln!(
                "{} {:?} rang {:?} couleur {:?} ink {:?} h={}",
                tr.card, tr.role, r, s, c.ink, c.rect.h
            );
        }
    }
}
