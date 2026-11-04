//! Ensemble de cartes représenté par un masque de bits.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::card::{Card, ParseCardError, Rank, Suit, parse_cards};

/// Ensemble de cartes sur 64 bits : bit `couleur * 16 + rang`.
///
/// Cette disposition permet d'extraire directement le masque des rangs d'une couleur
/// (13 bits), ce qu'exploite l'évaluateur de mains.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CardSet(u64);

const FULL_SUIT: u64 = 0x1FFF;
const FULL_DECK: u64 = FULL_SUIT | (FULL_SUIT << 16) | (FULL_SUIT << 32) | (FULL_SUIT << 48);

impl CardSet {
    pub const EMPTY: CardSet = CardSet(0);

    /// Le jeu complet de 52 cartes.
    #[must_use]
    pub const fn full_deck() -> CardSet {
        CardSet(FULL_DECK)
    }

    #[inline]
    const fn bit(card: Card) -> u64 {
        1u64 << (card.suit() as u64 * 16 + card.rank() as u64)
    }

    #[must_use]
    pub const fn from_card(card: Card) -> CardSet {
        CardSet(Self::bit(card))
    }

    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn insert(&mut self, card: Card) -> bool {
        let had = self.contains(card);
        self.0 |= Self::bit(card);
        !had
    }

    #[inline]
    pub fn remove(&mut self, card: Card) -> bool {
        let had = self.contains(card);
        self.0 &= !Self::bit(card);
        had
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, card: Card) -> bool {
        self.0 & Self::bit(card) != 0
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    #[must_use]
    pub const fn intersects(self, other: CardSet) -> bool {
        self.0 & other.0 != 0
    }

    /// Masque des rangs (13 bits, bit 0 = Deux) présents dans la couleur donnée.
    #[inline]
    #[must_use]
    pub const fn suit_mask(self, suit: Suit) -> u16 {
        ((self.0 >> (suit as u64 * 16)) & FULL_SUIT) as u16
    }

    #[must_use]
    pub fn iter(self) -> CardSetIter {
        CardSetIter(self.0)
    }

    pub fn to_vec(self) -> Vec<Card> {
        self.iter().collect()
    }

    /// Analyse une liste de cartes (`"AsKd7h"`), en refusant les doublons.
    pub fn parse(s: &str) -> Result<CardSet, ParseCardError> {
        Ok(parse_cards(s)?.into_iter().collect())
    }
}

impl BitOr for CardSet {
    type Output = CardSet;
    fn bitor(self, rhs: CardSet) -> CardSet {
        CardSet(self.0 | rhs.0)
    }
}

impl BitOrAssign for CardSet {
    fn bitor_assign(&mut self, rhs: CardSet) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for CardSet {
    type Output = CardSet;
    fn bitand(self, rhs: CardSet) -> CardSet {
        CardSet(self.0 & rhs.0)
    }
}

impl Not for CardSet {
    type Output = CardSet;
    /// Complément dans le jeu de 52 cartes.
    fn not(self) -> CardSet {
        CardSet(!self.0 & FULL_DECK)
    }
}

impl FromIterator<Card> for CardSet {
    fn from_iter<I: IntoIterator<Item = Card>>(iter: I) -> Self {
        let mut s = CardSet::EMPTY;
        for c in iter {
            s.insert(c);
        }
        s
    }
}

impl<'a> FromIterator<&'a Card> for CardSet {
    fn from_iter<I: IntoIterator<Item = &'a Card>>(iter: I) -> Self {
        iter.into_iter().copied().collect()
    }
}

impl IntoIterator for CardSet {
    type Item = Card;
    type IntoIter = CardSetIter;
    fn into_iter(self) -> CardSetIter {
        self.iter()
    }
}

/// Itérateur sur les cartes d'un [`CardSet`].
#[derive(Debug, Clone)]
pub struct CardSetIter(u64);

impl Iterator for CardSetIter {
    type Item = Card;

    #[inline]
    fn next(&mut self) -> Option<Card> {
        if self.0 == 0 {
            return None;
        }
        let bit = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        let suit = Suit::ALL[(bit / 16) as usize];
        let rank = Rank::ALL[(bit % 16) as usize];
        Some(Card::new(rank, suit))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.0.count_ones() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for CardSetIter {}

impl fmt::Display for CardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.iter() {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CardSet({self})")
    }
}

impl Serialize for CardSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CardSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        CardSet::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_deck_has_52_distinct_cards() {
        let deck = CardSet::full_deck();
        assert_eq!(deck.len(), 52);
        let v = deck.to_vec();
        let back: CardSet = v.iter().collect();
        assert_eq!(back, deck);
        assert_eq!(Card::all().collect::<CardSet>(), deck);
        assert!((!deck).is_empty());
    }

    #[test]
    fn insert_remove_contains() {
        let mut s = CardSet::EMPTY;
        let a: Card = "As".parse().unwrap();
        assert!(s.insert(a));
        assert!(!s.insert(a));
        assert!(s.contains(a));
        assert_eq!(s.suit_mask(Suit::Spades), 1 << 12);
        assert!(s.remove(a));
        assert!(s.is_empty());
    }

    #[test]
    fn complement_and_parse() {
        let s = CardSet::parse("AsKsQs").unwrap();
        assert_eq!((!s).len(), 49);
        assert!(!s.intersects(!s));
        assert_eq!(s.to_string().len(), 6);
        assert!(CardSet::parse("AsAs").is_err());
    }
}
