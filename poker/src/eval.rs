//! Évaluation des mains de Texas Hold'em (5 à 7 cartes).
//!
//! L'évaluateur travaille directement sur les masques de rangs par couleur d'un [`CardSet`],
//! sans table précalculée : il est exact, sans allocation, et suffisamment rapide pour les
//! simulations de Monte-Carlo (quelques dizaines de nanosecondes par main).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::card::{Card, Rank, Suit};
use crate::cardset::CardSet;

/// Catégorie de main, de la plus faible à la plus forte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandCategory {
    HighCard = 0,
    OnePair,
    TwoPair,
    ThreeOfAKind,
    Straight,
    Flush,
    FullHouse,
    FourOfAKind,
    StraightFlush,
}

impl HandCategory {
    pub const ALL: [HandCategory; 9] = [
        HandCategory::HighCard,
        HandCategory::OnePair,
        HandCategory::TwoPair,
        HandCategory::ThreeOfAKind,
        HandCategory::Straight,
        HandCategory::Flush,
        HandCategory::FullHouse,
        HandCategory::FourOfAKind,
        HandCategory::StraightFlush,
    ];

    /// Nombre de rangs significatifs encodés pour départager deux mains de cette catégorie.
    const fn significant_ranks(self) -> usize {
        match self {
            HandCategory::HighCard | HandCategory::Flush => 5,
            HandCategory::OnePair => 4,
            HandCategory::TwoPair | HandCategory::ThreeOfAKind => 3,
            HandCategory::FullHouse | HandCategory::FourOfAKind => 2,
            HandCategory::Straight | HandCategory::StraightFlush => 1,
        }
    }

    /// Nom français de la catégorie.
    #[must_use]
    pub const fn name_fr(self) -> &'static str {
        match self {
            HandCategory::HighCard => "Hauteur",
            HandCategory::OnePair => "Paire",
            HandCategory::TwoPair => "Double paire",
            HandCategory::ThreeOfAKind => "Brelan",
            HandCategory::Straight => "Quinte",
            HandCategory::Flush => "Couleur",
            HandCategory::FullHouse => "Full",
            HandCategory::FourOfAKind => "Carré",
            HandCategory::StraightFlush => "Quinte flush",
        }
    }
}

impl fmt::Display for HandCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name_fr())
    }
}

/// Force d'une main : plus la valeur est grande, plus la main est forte.
///
/// Encodage : `catégorie << 20`, puis jusqu'à cinq rangs sur 4 bits chacun, par ordre
/// d'importance (rang principal, puis kickers). Deux mains de même force sont à égalité.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HandRank(u32);

impl HandRank {
    /// Catégorie et rangs déjà placés (voir [`at`] et [`top_at`]).
    #[inline(always)]
    const fn new(category: HandCategory, ranks: u32) -> HandRank {
        HandRank(((category as u32) << 20) | ranks)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn category(self) -> HandCategory {
        HandCategory::ALL[(self.0 >> 20) as usize]
    }

    /// Rangs significatifs (rang principal puis kickers).
    #[must_use]
    pub fn ranks(self) -> Vec<Rank> {
        (0..self.category().significant_ranks())
            .map(|i| Rank::ALL[((self.0 >> (16 - 4 * i)) & 0xF) as usize])
            .collect()
    }
}

impl fmt::Display for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ranks: Vec<String> = self.ranks().iter().map(ToString::to_string).collect();
        write!(f, "{} ({})", self.category(), ranks.join(" "))
    }
}

impl fmt::Debug for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HandRank({self})")
    }
}

#[inline(always)]
fn highest(mask: u32) -> u8 {
    debug_assert!(mask != 0);
    (31 - mask.leading_zeros()) as u8
}

/// Le rang `r` placé à la position `slot` de l'encodage (0 = rang principal, bits 16 à 19,
/// puis les kickers sur les 4 bits suivants vers le bas).
#[inline(always)]
const fn at(slot: u32, r: u8) -> u32 {
    (r as u32) << (16 - 4 * slot)
}

/// Les `n` rangs les plus hauts du masque, par ordre décroissant, placés aux positions
/// `slot`, `slot + 1`… : la valeur est construite directement, sans tableau intermédiaire
/// (chemin chaud de toutes les simulations).
#[inline(always)]
fn top_at(mut mask: u32, slot: u32, n: u32) -> u32 {
    let mut v = 0;
    for i in slot..slot + n {
        let r = highest(mask);
        v |= at(i, r);
        mask ^= 1 << r;
    }
    v
}

/// Rang de la carte haute de la meilleure quinte contenue dans le masque, s'il y en a une.
/// La quinte blanche (A-2-3-4-5) a pour carte haute le Cinq.
#[inline]
fn straight_high(mask: u32) -> Option<u8> {
    // Bit 0 = As joué en carte basse, bits 1..=13 = Deux..As.
    let e = (mask << 1) | ((mask >> 12) & 1);
    let s = e & (e >> 1) & (e >> 2) & (e >> 3) & (e >> 4);
    if s == 0 { None } else { Some(highest(s) + 3) }
}

/// Masque des rangs de la couleur qui compte au moins cinq cartes, s'il y en a une.
///
/// Les quatre couleurs sont comptées d'un coup (« SWAR », un compteur par tranche de
/// 16 bits du [`CardSet`]) : la cible x86-64 de base n'a pas l'instruction `popcnt`, et
/// quatre `count_ones` y coûtaient près de 70 instructions par main. Au plus 7 cartes : une
/// seule couleur peut en compter cinq.
#[inline]
fn flush_mask(bits: u64) -> Option<u32> {
    const M1: u64 = 0x5555_5555_5555_5555;
    const M2: u64 = 0x3333_3333_3333_3333;
    const M4: u64 = 0x0F0F_0F0F_0F0F_0F0F;
    const M8: u64 = 0x00FF_00FF_00FF_00FF;
    let x = bits - ((bits >> 1) & M1);
    let x = (x & M2) + ((x >> 2) & M2);
    let x = (x + (x >> 4)) & M4; // cartes par octet
    let x = (x + (x >> 8)) & M8; // cartes par couleur (au plus 13)
    // Compteur + 11 ≥ 16 si et seulement si compteur ≥ 5 (sans débordement : 13 + 11 < 2¹⁶).
    let five = (x + 0x000B_000B_000B_000B) & 0x0010_0010_0010_0010;
    if five == 0 {
        return None;
    }
    let suit = five.trailing_zeros() / 16;
    Some(((bits >> (16 * suit)) & 0x1FFF) as u32)
}

/// Évalue la meilleure main de 5 cartes contenue dans un ensemble de 5 à 7 cartes.
///
/// Au-delà de 7 cartes, le résultat n'est pas garanti (une couleur et un full pourraient
/// coexister) ; le Texas Hold'em n'en utilise jamais plus.
#[must_use]
pub fn evaluate(cards: CardSet) -> HandRank {
    debug_assert!(cards.len() <= 7, "l'évaluateur accepte au plus 7 cartes");

    // Avec 7 cartes au plus, une couleur exclut carré et full : on peut conclure tout de suite.
    if let Some(m) = flush_mask(cards.bits()) {
        if let Some(hi) = straight_high(m) {
            return HandRank::new(HandCategory::StraightFlush, at(0, hi));
        }
        return HandRank::new(HandCategory::Flush, top_at(m, 0, 5));
    }

    let c = u32::from(cards.suit_mask(Suit::Clubs));
    let d = u32::from(cards.suit_mask(Suit::Diamonds));
    let h = u32::from(cards.suit_mask(Suit::Hearts));
    let s = u32::from(cards.suit_mask(Suit::Spades));
    // Multiplicités par paires de couleurs : un rang est présent au moins deux fois s'il
    // l'est deux fois dans {♣, ♦}, deux fois dans {♥, ♠}, ou une fois dans chacune.
    let (cd_both, cd_any) = (c & d, c | d);
    let (hs_both, hs_any) = (h & s, h | s);
    let r1 = cd_any | hs_any; // rangs présents au moins une fois
    let r2 = cd_both | hs_both | (cd_any & hs_any); // au moins deux fois
    let r3 = (cd_both & hs_any) | (hs_both & cd_any); // au moins trois fois
    let r4 = cd_both & hs_both; // quatre fois

    if r4 != 0 {
        let q = highest(r4);
        let k = highest(r1 & !(1 << q));
        return HandRank::new(HandCategory::FourOfAKind, at(0, q) | at(1, k));
    }
    if r3 != 0 {
        let t = highest(r3);
        let pairs = r2 & !(1 << t);
        if pairs != 0 {
            return HandRank::new(HandCategory::FullHouse, at(0, t) | at(1, highest(pairs)));
        }
    }
    if let Some(hi) = straight_high(r1) {
        return HandRank::new(HandCategory::Straight, at(0, hi));
    }
    if r3 != 0 {
        let t = highest(r3);
        let kickers = top_at(r1 & !(1 << t), 1, 2);
        return HandRank::new(HandCategory::ThreeOfAKind, at(0, t) | kickers);
    }
    // Au moins deux paires : au moins deux bits (sans `count_ones`, coûteux sans `popcnt`).
    if r2 & r2.wrapping_sub(1) != 0 {
        let p1 = highest(r2);
        let p2 = highest(r2 ^ (1 << p1));
        let k = highest(r1 & !(1 << p1) & !(1 << p2));
        return HandRank::new(HandCategory::TwoPair, at(0, p1) | at(1, p2) | at(2, k));
    }
    if r2 != 0 {
        let p = highest(r2);
        let kickers = top_at(r1 & !(1 << p), 1, 3);
        return HandRank::new(HandCategory::OnePair, at(0, p) | kickers);
    }
    HandRank::new(HandCategory::HighCard, top_at(r1, 0, 5))
}

/// Les cinq cartes qui composent la meilleure main (pour l'affichage et l'export).
/// Renvoie `None` si l'ensemble ne contient pas entre 5 et 7 cartes.
#[must_use]
pub fn best_five(cards: CardSet) -> Option<([Card; 5], HandRank)> {
    let v = cards.to_vec();
    if !(5..=7).contains(&v.len()) {
        return None;
    }
    let mut best: Option<([Card; 5], HandRank)> = None;
    let n = v.len();
    for a in 0..n {
        for b in a + 1..n {
            for c in b + 1..n {
                for d in c + 1..n {
                    for e in d + 1..n {
                        let five = [v[a], v[b], v[c], v[d], v[e]];
                        let rank = evaluate(five.iter().collect());
                        if best.is_none_or(|(_, r)| rank > r) {
                            best = Some((five, rank));
                        }
                    }
                }
            }
        }
    }
    best.map(|(mut five, rank)| {
        five.sort_by(|x, y| y.cmp(x));
        (five, rank)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rand::SeedableRng;
    use rand::seq::SliceRandom;
    use rand_chacha::ChaCha8Rng;

    use super::*;

    fn rank_of(s: &str) -> HandRank {
        evaluate(CardSet::parse(s).unwrap())
    }

    #[test]
    fn flush_mask_matches_naive_count() {
        let naive = |cards: CardSet| {
            Suit::ALL
                .iter()
                .map(|s| u32::from(cards.suit_mask(*s)))
                .find(|m| m.count_ones() >= 5)
        };
        // Couleurs pleines et cas limites (4, 5 cartes), puis des milliers de mains au hasard.
        for s in [
            "",
            "AsKsQsJs",
            "AsKsQsJs2s",
            "AsKsQsJsTs9s8s",
            "2c3c4c5c6c7c8c",
        ] {
            let cards = CardSet::parse(s).unwrap();
            assert_eq!(flush_mask(cards.bits()), naive(cards), "{s}");
        }
        for s in Suit::ALL {
            let suit: CardSet = Rank::ALL.iter().map(|r| Card::new(*r, s)).collect();
            assert_eq!(flush_mask(suit.bits()), naive(suit));
        }
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut deck = CardSet::full_deck().to_vec();
        for _ in 0..200_000 {
            deck.shuffle(&mut rng);
            for n in 0..=7 {
                let cards: CardSet = deck[..n].iter().copied().collect();
                assert_eq!(flush_mask(cards.bits()), naive(cards), "{cards:?}");
            }
        }
    }

    #[test]
    fn categories_of_known_hands() {
        use HandCategory::*;
        let cases = [
            ("AsKsQsJsTs", StraightFlush),
            ("As2s3s4s5s9d9h", StraightFlush),
            ("9c9d9h9s2c", FourOfAKind),
            ("KcKdKh7s7c", FullHouse),
            ("KcKdKh7s7c7d2h", FullHouse),
            ("Ah9h7h4h2hKdKc", Flush),
            ("As2d3c4h5s", Straight),
            ("Ts9d8c7h6sAsAd", Straight),
            ("QcQdQh8s3c", ThreeOfAKind),
            ("JcJd4h4s9c", TwoPair),
            ("JcJd4h4s9c9d2h", TwoPair),
            ("AcAd8h5s3c", OnePair),
            ("AcJd8h5s3c", HighCard),
        ];
        for (cards, cat) in cases {
            assert_eq!(rank_of(cards).category(), cat, "{cards}");
        }
    }

    #[test]
    fn ordering_and_kickers() {
        // Quinte blanche < quinte hauteur 6.
        assert!(rank_of("As2d3c4h5s") < rank_of("2d3c4h5s6s"));
        // La quinte blanche est hauteur Cinq.
        assert_eq!(rank_of("As2d3c4h5s").ranks(), vec![Rank::Five]);
        // Kicker décisif.
        assert!(rank_of("AcAd8h5s3c") > rank_of("AhAs8d5c2c"));
        // Double paire : la troisième paire peut servir de kicker.
        assert_eq!(
            rank_of("JcJd4h4s2c2dQh").ranks(),
            vec![Rank::Jack, Rank::Four, Rank::Queen]
        );
        // Full : le second brelan sert de paire.
        assert_eq!(
            rank_of("KcKdKh7s7c7d2h").ranks(),
            vec![Rank::King, Rank::Seven]
        );
        // Égalité parfaite entre couleurs différentes.
        assert_eq!(rank_of("AcKd8h5s3c"), rank_of("AhKs8d5c3h"));
    }

    #[test]
    fn best_five_cards() {
        let (five, rank) = best_five(CardSet::parse("AhKh2c3d4h5h9h").unwrap()).unwrap();
        assert_eq!(rank.category(), HandCategory::Flush);
        assert!(five.iter().all(|c| c.suit() == Suit::Hearts));
        assert!(best_five(CardSet::parse("AhKh").unwrap()).is_none());
    }

    /// Dénombrement exhaustif des 2 598 960 mains de 5 cartes : fréquences de référence
    /// et 7 462 classes d'équivalence distinctes.
    #[test]
    fn exhaustive_five_card_frequencies() {
        let mut counts = [0u32; 9];
        let mut distinct = HashSet::new();
        let cards: Vec<Card> = Card::all().collect();
        for a in 0..52 {
            for b in a + 1..52 {
                for c in b + 1..52 {
                    for d in c + 1..52 {
                        for e in d + 1..52 {
                            let set: CardSet = [cards[a], cards[b], cards[c], cards[d], cards[e]]
                                .iter()
                                .collect();
                            let r = evaluate(set);
                            counts[r.category() as usize] += 1;
                            distinct.insert(r);
                        }
                    }
                }
            }
        }
        assert_eq!(
            counts,
            [
                1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40
            ]
        );
        assert_eq!(distinct.len(), 7_462);
    }

    /// Évaluateur naïf indépendant (meilleure des 21 combinaisons, tri par fréquences),
    /// utilisé comme référence croisée sur des mains de 7 cartes tirées au hasard.
    fn naive_five(cards: &[Card; 5]) -> (u8, Vec<u8>) {
        let mut counts = [0u8; 13];
        for c in cards {
            counts[c.rank().index() as usize] += 1;
        }
        let flush = cards.iter().all(|c| c.suit() == cards[0].suit());
        let mut ranks: Vec<u8> = cards.iter().map(|c| c.rank().index()).collect();
        ranks.sort_unstable_by(|a, b| b.cmp(a));
        ranks.dedup();
        let straight_hi = if ranks.len() == 5 && ranks[0] - ranks[4] == 4 {
            Some(ranks[0])
        } else if ranks == [12, 3, 2, 1, 0] {
            Some(3)
        } else {
            None
        };
        // Groupes triés par (multiplicité, rang) décroissants.
        let mut groups: Vec<(u8, u8)> = (0..13u8)
            .filter(|&r| counts[r as usize] > 0)
            .map(|r| (counts[r as usize], r))
            .collect();
        groups.sort_unstable_by(|a, b| b.cmp(a));
        let order: Vec<u8> = groups.iter().map(|g| g.1).collect();
        let shape: Vec<u8> = groups.iter().map(|g| g.0).collect();
        match (straight_hi, flush, shape.as_slice()) {
            (Some(h), true, _) => (8, vec![h]),
            (_, _, [4, 1]) => (7, order),
            (_, _, [3, 2]) => (6, order),
            (_, true, _) => (5, order),
            (Some(h), false, _) => (4, vec![h]),
            (_, _, [3, 1, 1]) => (3, order),
            (_, _, [2, 2, 1]) => (2, order),
            (_, _, [2, 1, 1, 1]) => (1, order),
            _ => (0, order),
        }
    }

    fn naive_seven(cards: &[Card]) -> (u8, Vec<u8>) {
        let mut best = (0, vec![]);
        for skip_a in 0..7 {
            for skip_b in skip_a + 1..7 {
                let five: Vec<Card> = (0..7)
                    .filter(|&i| i != skip_a && i != skip_b)
                    .map(|i| cards[i])
                    .collect();
                let r = naive_five(&five.try_into().unwrap());
                if r > best {
                    best = r;
                }
            }
        }
        best
    }

    #[test]
    fn cross_check_against_naive_evaluator() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut deck: Vec<Card> = Card::all().collect();
        let mut prev: Option<(HandRank, (u8, Vec<u8>))> = None;
        for _ in 0..50_000 {
            deck.shuffle(&mut rng);
            let hand = &deck[..7];
            let fast = evaluate(hand.iter().collect());
            let naive = naive_seven(hand);
            assert_eq!(fast.category() as u8, naive.0, "{hand:?}");
            // Valeur encodée exacte (elle est sérialisée telle quelle) : catégorie puis rangs
            // significatifs sur 4 bits chacun.
            let encoded = naive
                .1
                .iter()
                .enumerate()
                .fold(u32::from(naive.0) << 20, |v, (i, r)| {
                    v | u32::from(*r) << (16 - 4 * i)
                });
            assert_eq!(fast.value(), encoded, "{hand:?}");
            if let Some((pf, pn)) = &prev {
                assert_eq!(fast.cmp(pf), naive.cmp(pn), "{hand:?}");
            }
            prev = Some((fast, naive));
        }
    }

    /// Dénombrement exhaustif des 133 784 560 mains de 7 cartes (long : lancer avec
    /// `cargo test --release -- --ignored`).
    #[test]
    #[ignore = "long : ~133 millions d'évaluations"]
    fn exhaustive_seven_card_frequencies() {
        use rayon::prelude::*;
        let cards: Vec<Card> = Card::all().collect();
        let counts = (0..52usize)
            .into_par_iter()
            .map(|a| {
                let mut counts = [0u64; 9];
                let sa = CardSet::from_card(cards[a]);
                for b in a + 1..52 {
                    let sb = sa | CardSet::from_card(cards[b]);
                    for c in b + 1..52 {
                        let sc = sb | CardSet::from_card(cards[c]);
                        for d in c + 1..52 {
                            let sd = sc | CardSet::from_card(cards[d]);
                            for e in d + 1..52 {
                                let se = sd | CardSet::from_card(cards[e]);
                                for f in e + 1..52 {
                                    let sf = se | CardSet::from_card(cards[f]);
                                    for g in &cards[f + 1..] {
                                        let r = evaluate(sf | CardSet::from_card(*g));
                                        counts[r.category() as usize] += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                counts
            })
            .reduce(
                || [0u64; 9],
                |mut x, y| {
                    for (a, b) in x.iter_mut().zip(y) {
                        *a += b;
                    }
                    x
                },
            );
        assert_eq!(
            counts,
            [
                23_294_460, 58_627_800, 31_433_400, 6_461_620, 6_180_020, 4_047_644, 3_473_184,
                224_848, 41_584
            ]
        );
    }
}
