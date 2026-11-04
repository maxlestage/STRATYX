//! Cartes, rangs et couleurs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Rang d'une carte, du Deux à l'As.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Rank {
    Two = 0,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    /// Tous les rangs, du plus faible au plus fort.
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];

    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_index(index: u8) -> Option<Rank> {
        if index < 13 {
            Some(Rank::ALL[index as usize])
        } else {
            None
        }
    }

    /// Caractère standard des historiques de mains : `2`…`9`, `T`, `J`, `Q`, `K`, `A`.
    #[must_use]
    pub const fn to_char(self) -> char {
        b"23456789TJQKA"[self as usize] as char
    }

    #[must_use]
    pub fn from_char(c: char) -> Option<Rank> {
        let idx = "23456789TJQKA".find(c.to_ascii_uppercase())?;
        Rank::from_index(idx as u8)
    }
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

/// Couleur d'une carte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Suit {
    Clubs = 0,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_index(index: u8) -> Option<Suit> {
        if index < 4 {
            Some(Suit::ALL[index as usize])
        } else {
            None
        }
    }

    /// Caractère standard : `c` (trèfle), `d` (carreau), `h` (cœur), `s` (pique).
    #[must_use]
    pub const fn to_char(self) -> char {
        b"cdhs"[self as usize] as char
    }

    #[must_use]
    pub fn from_char(c: char) -> Option<Suit> {
        let idx = "cdhs".find(c.to_ascii_lowercase())?;
        Suit::from_index(idx as u8)
    }
}

impl fmt::Display for Suit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

/// Une carte du jeu de 52 cartes, encodée sur un octet (`rang * 4 + couleur`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Card(u8);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseCardError {
    #[error("carte invalide « {0} » : format attendu rang + couleur, ex. « As », « Td », « 9h »")]
    Invalid(String),
    #[error("carte en double : {0}")]
    Duplicate(Card),
}

impl Card {
    #[must_use]
    pub const fn new(rank: Rank, suit: Suit) -> Card {
        Card(rank as u8 * 4 + suit as u8)
    }

    /// Identifiant compact dans `0..52`.
    #[must_use]
    pub const fn id(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn from_id(id: u8) -> Option<Card> {
        if id < 52 { Some(Card(id)) } else { None }
    }

    #[must_use]
    pub const fn rank(self) -> Rank {
        Rank::ALL[(self.0 / 4) as usize]
    }

    #[must_use]
    pub const fn suit(self) -> Suit {
        Suit::ALL[(self.0 % 4) as usize]
    }

    /// Les 52 cartes, dans l'ordre des identifiants.
    pub fn all() -> impl Iterator<Item = Card> {
        (0..52).map(Card)
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.rank(), self.suit())
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Card {
    type Err = ParseCardError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let err = || ParseCardError::Invalid(s.to_string());
        let mut chars = t.chars();
        let (rank, suit) = match (chars.next(), chars.next(), chars.next(), chars.next()) {
            (Some(r), Some(su), None, None) => (Rank::from_char(r), Suit::from_char(su)),
            // Tolère « 10h » en plus de « Th ».
            (Some('1'), Some('0'), Some(su), None) => (Some(Rank::Ten), Suit::from_char(su)),
            _ => return Err(err()),
        };
        match (rank, suit) {
            (Some(r), Some(su)) => Ok(Card::new(r, su)),
            _ => Err(err()),
        }
    }
}

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Analyse une suite de cartes collées ou séparées : `"AsKd"`, `"As Kd"`, `"As,Kd,10h"`.
/// Refuse les doublons.
pub fn parse_cards(s: &str) -> Result<Vec<Card>, ParseCardError> {
    let compact: Vec<char> = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '-')
        .collect();
    let mut cards = Vec::with_capacity(compact.len() / 2);
    let mut i = 0;
    while i < compact.len() {
        let len = if compact[i] == '1' && compact.get(i + 1) == Some(&'0') {
            3
        } else {
            2
        };
        if i + len > compact.len() {
            return Err(ParseCardError::Invalid(s.to_string()));
        }
        let token: String = compact[i..i + len].iter().collect();
        let card: Card = token.parse()?;
        if cards.contains(&card) {
            return Err(ParseCardError::Duplicate(card));
        }
        cards.push(card);
        i += len;
    }
    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_cards() {
        for c in Card::all() {
            let s = c.to_string();
            assert_eq!(s.parse::<Card>().unwrap(), c);
            assert_eq!(Card::new(c.rank(), c.suit()), c);
        }
        assert_eq!(Card::all().count(), 52);
    }

    #[test]
    fn parse_variants() {
        assert_eq!(
            "As".parse::<Card>().unwrap(),
            Card::new(Rank::Ace, Suit::Spades)
        );
        assert_eq!(
            "10h".parse::<Card>().unwrap(),
            Card::new(Rank::Ten, Suit::Hearts)
        );
        assert_eq!(
            "tD".parse::<Card>().unwrap(),
            Card::new(Rank::Ten, Suit::Diamonds)
        );
        assert!("Ax".parse::<Card>().is_err());
        assert!("A".parse::<Card>().is_err());
        assert!("Asd".parse::<Card>().is_err());
    }

    #[test]
    fn parse_lists() {
        let v = parse_cards("As Kd,10h2c").unwrap();
        assert_eq!(v.len(), 4);
        assert_eq!(v[2], "Th".parse().unwrap());
        assert!(matches!(
            parse_cards("AsAs"),
            Err(ParseCardError::Duplicate(_))
        ));
        assert!(parse_cards("AsK").is_err());
    }

    #[test]
    fn serde_as_string() {
        let c: Card = "Qh".parse().unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"Qh\"");
        assert_eq!(serde_json::from_str::<Card>("\"Qh\"").unwrap(), c);
    }
}
