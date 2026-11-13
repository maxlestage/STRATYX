//! Équité rapide par Monte-Carlo d'une main contre des ranges pondérées, avec une graine
//! explicite (décision T8) : utilisée par la référence EV, les simulations et l'analyse.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use stratyx_poker::{Card, CardSet, HoleCards, Range, evaluate};

use crate::classes::draw;

/// Une range prête à être tirée au sort : combinaisons et poids cumulés.
#[derive(Debug, Clone)]
pub struct Sampler {
    combos: Vec<HoleCards>,
    cumulative: Vec<f32>,
}

impl Sampler {
    /// Les combinaisons de `range` compatibles avec les cartes `dead`.
    #[must_use]
    pub fn new(range: &Range, dead: CardSet) -> Sampler {
        let mut combos = Vec::new();
        let mut cumulative = Vec::new();
        let mut acc = 0.0f32;
        for (h, w) in range.iter() {
            if w > 0.0 && !h.set().intersects(dead) {
                acc += w;
                combos.push(h);
                cumulative.push(acc);
            }
        }
        Sampler { combos, cumulative }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.combos.is_empty()
    }

    /// Une combinaison tirée selon les poids, qui ne touche pas `used` (`None` après
    /// plusieurs échecs).
    pub fn sample(&self, used: CardSet, rng: &mut impl Rng) -> Option<HoleCards> {
        let total = *self.cumulative.last()?;
        for _ in 0..32 {
            let x = rng.random::<f32>() * total;
            let i = self
                .cumulative
                .partition_point(|c| *c <= x)
                .min(self.combos.len() - 1);
            let h = self.combos[i];
            if !h.set().intersects(used) {
                return Some(h);
            }
        }
        None
    }
}

/// Part du pot de `hero` contre une main de chaque range, sur `board` complété au hasard.
/// Une range vide compte comme une main au hasard.
#[must_use]
pub fn equity(hero: HoleCards, ranges: &[Range], board: &[Card], samples: usize, seed: u64) -> f32 {
    let dead: CardSet = board.iter().collect::<CardSet>() | hero.set();
    let samplers: Vec<Sampler> = ranges.iter().map(|r| Sampler::new(r, dead)).collect();
    equity_with(hero, &samplers, board, samples, seed)
}

/// Comme [`equity`], avec des ranges déjà préparées.
#[must_use]
pub fn equity_with(
    hero: HoleCards,
    samplers: &[Sampler],
    board: &[Card],
    samples: usize,
    seed: u64,
) -> f32 {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let board_set: CardSet = board.iter().collect();
    let base = board_set | hero.set();
    let mut total = 0.0f64;
    let mut n = 0usize;
    'sample: for _ in 0..samples.max(1) {
        let mut used = base;
        let mut opp: [CardSet; 9] = [CardSet::EMPTY; 9];
        for (k, s) in samplers.iter().take(9).enumerate() {
            let h = if s.is_empty() {
                let a = draw(&mut used, &mut rng);
                let b = draw(&mut used, &mut rng);
                CardSet::from_card(a) | CardSet::from_card(b)
            } else {
                match s.sample(used, &mut rng) {
                    Some(h) => {
                        used |= h.set();
                        h.set()
                    }
                    None => continue 'sample,
                }
            };
            opp[k] = h;
        }
        let mut full = board_set;
        while full.len() < 5 {
            full.insert(draw(&mut used, &mut rng));
        }
        let mine = evaluate(full | hero.set());
        let mut ties = 1u32;
        let mut lost = false;
        for o in opp.iter().take(samplers.len().min(9)) {
            let theirs = evaluate(full | *o);
            if theirs > mine {
                lost = true;
                break;
            }
            if theirs == mine {
                ties += 1;
            }
        }
        if !lost {
            total += 1.0 / f64::from(ties);
        }
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        (total / n as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aa_vs_kk() {
        let e = equity(
            "AsAh".parse().unwrap(),
            &[Range::parse("KK").unwrap()],
            &[],
            20_000,
            1,
        );
        assert!((0.79..0.85).contains(&e), "{e}");
        let random = equity("AsAh".parse().unwrap(), &[Range::empty()], &[], 20_000, 2);
        assert!((0.83..0.87).contains(&random), "{random}");
    }
}
