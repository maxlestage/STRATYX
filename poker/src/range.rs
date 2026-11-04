//! Mains privées, classes de mains (169) et ranges pondérées (1 326 combinaisons).
//!
//! Syntaxe de range acceptée (séparateur `,`, espaces ignorés) :
//!
//! | Exemple        | Signification                                   |
//! |----------------|-------------------------------------------------|
//! | `AA`, `AKs`, `AKo`, `AK` | une classe (paire, assortie, dépareillée, les deux) |
//! | `TT+`          | TT, JJ, QQ, KK, AA                              |
//! | `ATs+`         | ATs, AJs, AQs, AKs (le kicker monte)            |
//! | `22-55`, `A2s-A5s` | intervalle inclusif                         |
//! | `AsKd`         | une combinaison précise                         |
//! | `AKs:0.5`      | pondération (fréquence entre 0 et 1)            |
//! | `random`, `*`  | toutes les mains                                |

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::card::{Card, ParseCardError, Rank, Suit};
use crate::cardset::CardSet;

/// Nombre de combinaisons de deux cartes distinctes : C(52, 2).
pub const NUM_COMBOS: usize = 1326;

/// Deux cartes privées, la plus forte (par identifiant) en premier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HoleCards {
    high: Card,
    low: Card,
}

impl HoleCards {
    /// Renvoie `None` si les deux cartes sont identiques.
    #[must_use]
    pub fn new(a: Card, b: Card) -> Option<HoleCards> {
        match a.id().cmp(&b.id()) {
            std::cmp::Ordering::Greater => Some(HoleCards { high: a, low: b }),
            std::cmp::Ordering::Less => Some(HoleCards { high: b, low: a }),
            std::cmp::Ordering::Equal => None,
        }
    }

    #[must_use]
    pub const fn cards(self) -> [Card; 2] {
        [self.high, self.low]
    }

    #[must_use]
    pub fn set(self) -> CardSet {
        CardSet::from_card(self.high) | CardSet::from_card(self.low)
    }

    /// Index compact dans `0..1326`.
    #[must_use]
    pub const fn index(self) -> usize {
        let h = self.high.id() as usize;
        let l = self.low.id() as usize;
        h * (h - 1) / 2 + l
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<HoleCards> {
        if index >= NUM_COMBOS {
            return None;
        }
        // Plus grand h tel que h(h-1)/2 <= index.
        let mut h = 1usize;
        while (h + 1) * h / 2 <= index {
            h += 1;
        }
        let l = index - h * (h - 1) / 2;
        Some(HoleCards {
            high: Card::from_id(h as u8)?,
            low: Card::from_id(l as u8)?,
        })
    }

    /// Toutes les combinaisons, dans l'ordre des index.
    pub fn all() -> impl Iterator<Item = HoleCards> {
        (0..NUM_COMBOS).filter_map(HoleCards::from_index)
    }

    #[must_use]
    pub fn class(self) -> HandClass {
        let (a, b) = (self.high.rank(), self.low.rank());
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        if hi == lo {
            HandClass::Pair(hi)
        } else if self.high.suit() == self.low.suit() {
            HandClass::Suited(hi, lo)
        } else {
            HandClass::Offsuit(hi, lo)
        }
    }
}

impl fmt::Display for HoleCards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.high, self.low)
    }
}

impl fmt::Debug for HoleCards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for HoleCards {
    type Err = ParseCardError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let cards = crate::card::parse_cards(s)?;
        match cards.as_slice() {
            [a, b] => Ok(HoleCards::new(*a, *b).expect("doublons déjà refusés")),
            _ => Err(ParseCardError::Invalid(s.to_string())),
        }
    }
}

impl Serialize for HoleCards {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HoleCards {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Une des 169 classes de mains de départ (rang haut en premier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandClass {
    Pair(Rank),
    Suited(Rank, Rank),
    Offsuit(Rank, Rank),
}

impl HandClass {
    /// Les 169 classes.
    pub fn all() -> impl Iterator<Item = HandClass> {
        Rank::ALL.into_iter().rev().flat_map(|hi| {
            Rank::ALL
                .into_iter()
                .rev()
                .filter(move |lo| *lo <= hi)
                .flat_map(move |lo| {
                    if hi == lo {
                        vec![HandClass::Pair(hi)]
                    } else {
                        vec![HandClass::Suited(hi, lo), HandClass::Offsuit(hi, lo)]
                    }
                })
        })
    }

    /// Combinaisons concrètes de la classe (6, 4 ou 12).
    #[must_use]
    pub fn combos(self) -> Vec<HoleCards> {
        let mut out = Vec::with_capacity(12);
        match self {
            HandClass::Pair(r) => {
                for (i, s1) in Suit::ALL.iter().enumerate() {
                    for s2 in &Suit::ALL[i + 1..] {
                        out.extend(HoleCards::new(Card::new(r, *s1), Card::new(r, *s2)));
                    }
                }
            }
            HandClass::Suited(h, l) => {
                for s in Suit::ALL {
                    out.extend(HoleCards::new(Card::new(h, s), Card::new(l, s)));
                }
            }
            HandClass::Offsuit(h, l) => {
                for s1 in Suit::ALL {
                    for s2 in Suit::ALL {
                        if s1 != s2 {
                            out.extend(HoleCards::new(Card::new(h, s1), Card::new(l, s2)));
                        }
                    }
                }
            }
        }
        out
    }

    /// Position dans la grille 13×13 habituelle (ligne, colonne), As en haut à gauche :
    /// paires sur la diagonale, assorties au-dessus, dépareillées en dessous.
    #[must_use]
    pub const fn grid_position(self) -> (usize, usize) {
        match self {
            HandClass::Pair(r) => (12 - r as usize, 12 - r as usize),
            HandClass::Suited(h, l) => (12 - h as usize, 12 - l as usize),
            HandClass::Offsuit(h, l) => (12 - l as usize, 12 - h as usize),
        }
    }
}

impl fmt::Display for HandClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandClass::Pair(r) => write!(f, "{r}{r}"),
            HandClass::Suited(h, l) => write!(f, "{h}{l}s"),
            HandClass::Offsuit(h, l) => write!(f, "{h}{l}o"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
#[error("range invalide, élément « {token} » : {reason}")]
pub struct ParseRangeError {
    pub token: String,
    pub reason: String,
}

/// Range pondérée : une fréquence dans `[0, 1]` pour chacune des 1 326 combinaisons.
#[derive(Clone, PartialEq)]
pub struct Range {
    weights: Box<[f32; NUM_COMBOS]>,
}

impl Default for Range {
    fn default() -> Self {
        Range::empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Suitedness {
    Suited,
    Offsuit,
    Any,
}

impl Range {
    #[must_use]
    pub fn empty() -> Range {
        Range {
            weights: Box::new([0.0; NUM_COMBOS]),
        }
    }

    #[must_use]
    pub fn full() -> Range {
        Range {
            weights: Box::new([1.0; NUM_COMBOS]),
        }
    }

    #[must_use]
    pub fn single(hole: HoleCards) -> Range {
        let mut r = Range::empty();
        r.set(hole, 1.0);
        r
    }

    /// Fixe la fréquence d'une combinaison (bornée à `[0, 1]`).
    pub fn set(&mut self, hole: HoleCards, weight: f32) {
        self.weights[hole.index()] = weight.clamp(0.0, 1.0);
    }

    #[must_use]
    pub fn get(&self, hole: HoleCards) -> f32 {
        self.weights[hole.index()]
    }

    pub fn set_class(&mut self, class: HandClass, weight: f32) {
        for h in class.combos() {
            self.set(h, weight);
        }
    }

    /// Combinaisons de fréquence non nulle.
    pub fn iter(&self) -> impl Iterator<Item = (HoleCards, f32)> + '_ {
        self.weights
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > 0.0)
            .map(|(i, w)| (HoleCards::from_index(i).expect("index valide"), *w))
    }

    /// Nombre de combinaisons pondéré.
    #[must_use]
    pub fn combo_count(&self) -> f64 {
        self.weights.iter().map(|w| f64::from(*w)).sum()
    }

    /// Part des 1 326 combinaisons couverte par la range, dans `[0, 1]`.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        self.combo_count() / NUM_COMBOS as f64
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.weights.iter().all(|w| *w <= 0.0)
    }

    /// Analyse une range textuelle ; les éléments suivants écrasent les précédents.
    pub fn parse(s: &str) -> Result<Range, ParseRangeError> {
        let mut range = Range::empty();
        for raw in s.split(',') {
            let token: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
            if token.is_empty() {
                continue;
            }
            let err = |reason: &str| ParseRangeError {
                token: token.clone(),
                reason: reason.into(),
            };
            let (body, weight) = match token.split_once(':') {
                Some((b, w)) => {
                    let w: f32 = w.parse().map_err(|_| err("pondération illisible"))?;
                    if !(0.0..=1.0).contains(&w) {
                        return Err(err("la pondération doit être comprise entre 0 et 1"));
                    }
                    (b, w)
                }
                None => (token.as_str(), 1.0),
            };
            if matches!(body.to_ascii_lowercase().as_str(), "random" | "*" | "100%") {
                for w in range.weights.iter_mut() {
                    *w = weight;
                }
                continue;
            }
            for class_or_combo in expand_token(body).map_err(|r| err(&r))? {
                match class_or_combo {
                    Item::Class(c) => range.set_class(c, weight),
                    Item::Combo(h) => range.set(h, weight),
                }
            }
        }
        Ok(range)
    }
}

enum Item {
    Class(HandClass),
    Combo(HoleCards),
}

/// Forme « gabarit » d'un élément : deux rangs et une contrainte de couleur.
fn parse_pattern(s: &str) -> Result<(Rank, Rank, Suitedness), String> {
    let chars: Vec<char> = s.chars().collect();
    let (a, b, suit) = match chars.as_slice() {
        [a, b] => (*a, *b, Suitedness::Any),
        [a, b, 's' | 'S'] => (*a, *b, Suitedness::Suited),
        [a, b, 'o' | 'O'] => (*a, *b, Suitedness::Offsuit),
        _ => return Err("forme attendue : « AA », « AKs », « AKo » ou « AK »".into()),
    };
    let ra = Rank::from_char(a).ok_or("rang inconnu")?;
    let rb = Rank::from_char(b).ok_or("rang inconnu")?;
    let (hi, lo) = if ra >= rb { (ra, rb) } else { (rb, ra) };
    if hi == lo && suit != Suitedness::Any {
        return Err("une paire ne peut pas être assortie ou dépareillée".into());
    }
    Ok((hi, lo, suit))
}

fn classes_for(hi: Rank, lo: Rank, suit: Suitedness) -> Vec<HandClass> {
    if hi == lo {
        return vec![HandClass::Pair(hi)];
    }
    match suit {
        Suitedness::Suited => vec![HandClass::Suited(hi, lo)],
        Suitedness::Offsuit => vec![HandClass::Offsuit(hi, lo)],
        Suitedness::Any => vec![HandClass::Suited(hi, lo), HandClass::Offsuit(hi, lo)],
    }
}

fn expand_token(body: &str) -> Result<Vec<Item>, String> {
    // Combinaison précise : « AsKd ».
    if body.len() == 4 {
        if let Ok(h) = body.parse::<HoleCards>() {
            return Ok(vec![Item::Combo(h)]);
        }
    }
    let classes = if let Some(base) = body.strip_suffix('+') {
        let (hi, lo, suit) = parse_pattern(base)?;
        if hi == lo {
            Rank::ALL
                .iter()
                .filter(|r| **r >= hi)
                .map(|r| HandClass::Pair(*r))
                .collect()
        } else {
            Rank::ALL
                .iter()
                .filter(|r| **r >= lo && **r < hi)
                .flat_map(|k| classes_for(hi, *k, suit))
                .collect()
        }
    } else if let Some((from, to)) = body.split_once('-') {
        let (h1, l1, s1) = parse_pattern(from)?;
        let (h2, l2, s2) = parse_pattern(to)?;
        if s1 != s2 {
            return Err("les deux bornes doivent avoir la même contrainte de couleur".into());
        }
        if h1 == l1 && h2 == l2 {
            let (a, b) = if h1 <= h2 { (h1, h2) } else { (h2, h1) };
            Rank::ALL
                .iter()
                .filter(|r| **r >= a && **r <= b)
                .map(|r| HandClass::Pair(*r))
                .collect()
        } else if h1 == h2 && h1 != l1 && h2 != l2 {
            let (a, b) = if l1 <= l2 { (l1, l2) } else { (l2, l1) };
            Rank::ALL
                .iter()
                .filter(|r| **r >= a && **r <= b)
                .flat_map(|k| classes_for(h1, *k, s1))
                .collect()
        } else {
            return Err("intervalle attendu entre deux paires ou avec la même carte haute".into());
        }
    } else {
        let (hi, lo, suit) = parse_pattern(body)?;
        classes_for(hi, lo, suit)
    };
    Ok(classes.into_iter().map(Item::Class).collect())
}

impl FromStr for Range {
    type Err = ParseRangeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Range::parse(s)
    }
}

impl fmt::Display for Range {
    /// Écriture canonique relisible par [`Range::parse`] : classes complètes quand toutes
    /// leurs combinaisons ont la même fréquence, combinaisons individuelles sinon.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        for class in HandClass::all() {
            let combos = class.combos();
            let w0 = self.get(combos[0]);
            let uniform = combos.iter().all(|h| self.get(*h) == w0);
            let fmt_w = |w: f32| {
                if w >= 1.0 {
                    String::new()
                } else {
                    format!(":{w}")
                }
            };
            if uniform {
                if w0 > 0.0 {
                    parts.push(format!("{class}{}", fmt_w(w0)));
                }
            } else {
                for h in combos {
                    let w = self.get(h);
                    if w > 0.0 {
                        parts.push(format!("{h}{}", fmt_w(w)));
                    }
                }
            }
        }
        f.write_str(&parts.join(","))
    }
}

impl fmt::Debug for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Range({self})")
    }
}

impl Serialize for Range {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Range {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Range::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_index_is_a_bijection() {
        let mut seen = vec![false; NUM_COMBOS];
        for (i, slot) in seen.iter_mut().enumerate() {
            let h = HoleCards::from_index(i).unwrap();
            assert_eq!(h.index(), i);
            assert!(!*slot);
            *slot = true;
        }
        assert!(HoleCards::from_index(NUM_COMBOS).is_none());
        assert_eq!(HoleCards::all().count(), NUM_COMBOS);
    }

    #[test]
    fn classes_cover_all_combos() {
        let classes: Vec<HandClass> = HandClass::all().collect();
        assert_eq!(classes.len(), 169);
        let total: usize = classes.iter().map(|c| c.combos().len()).sum();
        assert_eq!(total, NUM_COMBOS);
        for h in HoleCards::all() {
            assert!(h.class().combos().contains(&h));
        }
        let mut grid = [[false; 13]; 13];
        for c in classes {
            let (r, col) = c.grid_position();
            assert!(!grid[r][col]);
            grid[r][col] = true;
        }
    }

    #[test]
    fn parse_common_ranges() {
        let count = |s: &str| Range::parse(s).unwrap().combo_count();
        assert_eq!(count("AA"), 6.0);
        assert_eq!(count("AKs"), 4.0);
        assert_eq!(count("AKo"), 12.0);
        assert_eq!(count("AK"), 16.0);
        assert_eq!(count("TT+"), 30.0);
        assert_eq!(count("22-55"), 24.0);
        assert_eq!(count("55-22"), 24.0);
        assert_eq!(count("ATs+"), 16.0);
        assert_eq!(count("A2s-A5s"), 16.0);
        assert_eq!(count("KTo+"), 36.0);
        assert_eq!(count("AsKd"), 1.0);
        assert_eq!(count("AKs:0.5"), 2.0);
        assert_eq!(count("random"), 1326.0);
        assert_eq!(count("QQ+, AKs, AKo"), 34.0);
        assert_eq!(count("KA"), 16.0);
    }

    #[test]
    fn parse_errors() {
        for bad in ["AAs", "AX", "AKs:2", "AKs-KQs", "22-AKs", "XYZW"] {
            assert!(Range::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn display_roundtrip() {
        for s in ["QQ+,AKs,AKo:0.5", "AsKd,22-44,T9s:0.25", "random", ""] {
            let r = Range::parse(s).unwrap();
            let again = Range::parse(&r.to_string()).unwrap();
            assert_eq!(r, again, "{s} -> {r}");
        }
        assert_eq!(Range::parse("AKs:0.5").unwrap().to_string(), "AKs:0.5");
    }
}
