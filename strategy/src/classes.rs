//! Les 169 classes de mains de départ : index, combinaisons, force et ranges « top X % ».
//!
//! L'index d'une classe est sa case dans la grille 13 × 13 habituelle (ligne × 13 +
//! colonne, As en haut à gauche). La force d'une classe est son équité préflop contre une,
//! deux ou cinq mains au hasard ([`crate::tables`], calculées par Monte-Carlo avec le
//! moteur et une graine fixe : `cargo run -p stratyx-strategy --example tables`).

use std::sync::OnceLock;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_poker::{Card, CardSet, HandClass, HoleCards, Range, Rank, evaluate};

use crate::tables;

/// Nombre de classes de mains de départ.
pub const CLASSES: usize = 169;

/// Index de la classe dans la grille.
#[must_use]
pub const fn class_index(c: HandClass) -> usize {
    let (r, col) = c.grid_position();
    r * 13 + col
}

/// La classe à un index de la grille.
#[must_use]
pub fn class_at(i: usize) -> HandClass {
    let (r, c) = (i / 13, i % 13);
    let rank = |x: usize| Rank::ALL[12 - x];
    match r.cmp(&c) {
        std::cmp::Ordering::Equal => HandClass::Pair(rank(r)),
        std::cmp::Ordering::Less => HandClass::Suited(rank(r), rank(c)),
        std::cmp::Ordering::Greater => HandClass::Offsuit(rank(c), rank(r)),
    }
}

/// Index de la classe d'une main.
#[must_use]
pub fn hole_class(h: HoleCards) -> usize {
    // Calcul direct depuis les identifiants `rang * 4 + couleur` (la carte haute a le plus
    // grand identifiant, donc le rang le plus haut), sans construire de `HandClass` : même
    // résultat que `class_index(h.class())`, appelé à chaque main simulée.
    let [high, low] = h.cards();
    let high_row = 12 - usize::from(high.id() / 4); // ligne de la grille : As en haut
    let low_row = 12 - usize::from(low.id() / 4);
    if high_row == low_row || high.id() % 4 == low.id() % 4 {
        high_row * 13 + low_row // paire (diagonale) ou assortie (au-dessus)
    } else {
        low_row * 13 + high_row // dépareillée (en dessous)
    }
}

/// Les combinaisons de chaque classe.
pub fn combos_of(i: usize) -> &'static [HoleCards] {
    static TABLE: OnceLock<Vec<Vec<HoleCards>>> = OnceLock::new();
    &TABLE.get_or_init(|| (0..CLASSES).map(|i| class_at(i).combos()).collect())[i]
}

/// Équité d'une classe contre `opponents` mains au hasard (1, 2 ou 5 ; interpolée entre).
#[must_use]
pub fn strength(i: usize, opponents: usize) -> f32 {
    match opponents {
        0 | 1 => tables::VS1[i],
        2 => tables::VS2[i],
        3 => tables::VS2[i] + (tables::VS5[i] - tables::VS2[i]) / 3.0,
        4 => tables::VS2[i] + 2.0 * (tables::VS5[i] - tables::VS2[i]) / 3.0,
        _ => tables::VS5[i],
    }
}

/// La note qui ordonne les classes (moyenne des trois équités) : la plus forte d'abord.
#[must_use]
pub fn score(i: usize) -> f32 {
    (tables::VS1[i] + tables::VS2[i] + tables::VS5[i]) / 3.0
}

/// Les classes de la plus forte à la plus faible.
pub fn order() -> &'static [usize] {
    static ORDER: OnceLock<Vec<usize>> = OnceLock::new();
    ORDER.get_or_init(|| {
        let mut v: Vec<usize> = (0..CLASSES).collect();
        v.sort_by(|a, b| score(*b).total_cmp(&score(*a)).then(a.cmp(b)));
        v
    })
}

/// Part des 1326 combinaisons au moins aussi fortes que la classe (0 : AA, 1 : 32o).
#[must_use]
pub fn percentile(i: usize) -> f32 {
    static PCT: OnceLock<Vec<f32>> = OnceLock::new();
    PCT.get_or_init(|| {
        let mut out = vec![0.0f32; CLASSES];
        let mut acc = 0usize;
        for &c in order() {
            acc += combos_of(c).len();
            out[c] = acc as f32 / 1326.0;
        }
        out
    })[i]
}

/// Les meilleures `fraction` des mains (la dernière classe est prise en partie).
#[must_use]
pub fn top_range(fraction: f32) -> Range {
    band_range(0.0, fraction)
}

/// Les mains entre les percentiles `from` et `to` (ex. 0,05 → 0,25 : une range de call
/// qui exclut les mains de relance).
#[must_use]
pub fn band_range(from: f32, to: f32) -> Range {
    let mut r = Range::empty();
    let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
    let mut lo = 0.0f32;
    for &c in order() {
        let hi = percentile(c);
        let overlap = (hi.min(to) - lo.max(from)).max(0.0);
        let width = (hi - lo).max(1e-9);
        if overlap > 0.0 {
            r.set_class(class_at(c), (overlap / width).min(1.0));
        }
        lo = hi;
    }
    r
}

/// Calcule les trois tables de force (contre 1, 2 et 5 mains au hasard) par Monte-Carlo.
#[must_use]
pub fn compute_tables(samples: usize, seed: u64) -> [[f32; CLASSES]; 3] {
    let mut out = [[0.0f32; CLASSES]; 3];
    for (t, opponents) in [1usize, 2, 5].into_iter().enumerate() {
        for (i, slot) in out[t].iter_mut().enumerate() {
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ ((t as u64) << 32) ^ i as u64);
            let combos = combos_of(i);
            let mut total = 0.0f64;
            for s in 0..samples {
                let hero = combos[s % combos.len()];
                total += f64::from(random_showdown(hero, opponents, &mut rng));
            }
            *slot = (total / samples as f64) as f32;
        }
    }
    out
}

/// Une carte au hasard qui n'est pas dans `used`.
pub fn draw(used: &mut CardSet, rng: &mut impl Rng) -> Card {
    loop {
        let id = rng.random_range(0..52u8);
        if let Some(c) = Card::from_id(id)
            && used.insert(c)
        {
            return c;
        }
    }
}

/// Part du pot gagnée par `hero` contre `opponents` mains au hasard, sur un board au hasard.
fn random_showdown(hero: HoleCards, opponents: usize, rng: &mut impl Rng) -> f32 {
    let mut used = hero.set();
    let mut opp = Vec::with_capacity(opponents);
    for _ in 0..opponents {
        let a = draw(&mut used, rng);
        let b = draw(&mut used, rng);
        opp.push(CardSet::from_card(a) | CardSet::from_card(b));
    }
    let mut board = CardSet::EMPTY;
    for _ in 0..5 {
        board.insert(draw(&mut used, rng));
    }
    let mine = evaluate(board | hero.set());
    let mut ties = 1u32;
    for o in opp {
        let theirs = evaluate(board | o);
        if theirs > mine {
            return 0.0;
        }
        if theirs == mine {
            ties += 1;
        }
    }
    1.0 / ties as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hole_class_matches_hand_class() {
        for h in HoleCards::all() {
            assert_eq!(hole_class(h), class_index(h.class()), "{h}");
        }
    }

    #[test]
    fn index_roundtrip() {
        for i in 0..CLASSES {
            assert_eq!(class_index(class_at(i)), i);
        }
        assert_eq!(class_at(0).to_string(), "AA");
        assert_eq!(class_at(1).to_string(), "AKs");
        assert_eq!(class_at(13).to_string(), "AKo");
        assert_eq!(class_at(168).to_string(), "22");
        let total: usize = (0..CLASSES).map(|i| combos_of(i).len()).sum();
        assert_eq!(total, 1326);
    }

    #[test]
    fn order_and_ranges() {
        assert_eq!(order()[0], 0, "AA est la meilleure main");
        let aa = class_index(HandClass::Pair(Rank::Ace));
        let sept_deux = class_index(HandClass::Offsuit(Rank::Seven, Rank::Two));
        assert!(strength(aa, 1) > 0.84 && strength(aa, 1) < 0.86);
        assert!(percentile(aa) < 0.01);
        assert!(percentile(sept_deux) > 0.9);
        let r = top_range(0.1);
        assert!((r.fraction() - 0.1).abs() < 0.01, "{}", r.fraction());
        let b = band_range(0.1, 0.3);
        assert!((b.fraction() - 0.2).abs() < 0.01);
    }
}
