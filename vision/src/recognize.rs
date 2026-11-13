//! Reconnaissance du rang et de la couleur d'une carte à partir de son coin.
//!
//! Le coin est réduit à un [`Patch`] : le glyphe du rang et celui de la couleur, chacun
//! recadré sur son encre et ramené à une grille fixe de taux d'encre, avec leurs
//! proportions et la couleur de l'encre. Les [`Templates`] appris sur les cartes étiquetées
//! par l'utilisateur (une salle dessine toujours ses cartes de la même façon) sont comparés
//! par plus proche voisin ; sans eux, une police générique donne une première hypothèse, à
//! faible confiance.

use serde::{Deserialize, Serialize};
use stratyx_poker::{Card, Rank, Suit};
use stratyx_shared::Confidence;

use crate::font;

/// Grille du glyphe du rang.
pub const RANK_W: usize = 10;
pub const RANK_H: usize = 14;
/// Grille du glyphe de la couleur.
pub const SUIT_W: usize = 10;
pub const SUIT_H: usize = 10;

/// Nombre maximal d'exemples gardés par rang et par couleur.
const PER_LABEL: usize = 8;

/// Couleur de l'encre (jeux à 2 ou 4 couleurs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InkColor {
    Red,
    Black,
    Green,
    Blue,
    Unknown,
}

impl InkColor {
    /// Classe une couleur moyenne d'encre.
    #[must_use]
    pub fn classify(r: f32, g: f32, b: f32) -> InkColor {
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        if r > 90.0 && r > 1.4 * g && r > 1.4 * b {
            InkColor::Red
        } else if g > 70.0 && g > 1.25 * r && g > 1.05 * b {
            InkColor::Green
        } else if b > 80.0 && b > 1.25 * r && b >= g {
            InkColor::Blue
        } else if l < 115.0 {
            InkColor::Black
        } else {
            InkColor::Unknown
        }
    }

    /// Les couleurs de cartes compatibles avec cette encre.
    #[must_use]
    pub fn suits(self) -> &'static [Suit] {
        match self {
            InkColor::Red => &[Suit::Hearts, Suit::Diamonds],
            InkColor::Black => &[Suit::Spades, Suit::Clubs],
            InkColor::Green => &[Suit::Clubs],
            InkColor::Blue => &[Suit::Diamonds],
            InkColor::Unknown => &Suit::ALL,
        }
    }
}

/// Les caractéristiques normalisées du coin d'une carte.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    /// Taux d'encre du glyphe du rang (`RANK_W × RANK_H`), vide s'il n'a pas été trouvé.
    pub rank: Vec<f32>,
    /// Largeur / hauteur du glyphe du rang.
    pub rank_aspect: f32,
    /// Morceaux du glyphe du rang (2 pour « 10 »).
    pub rank_parts: u8,
    /// Taux d'encre du glyphe de la couleur (`SUIT_W × SUIT_H`), vide s'il manque.
    pub suit: Vec<f32>,
    pub suit_aspect: f32,
    pub ink: InkColor,
}

impl Patch {
    /// Le coin a-t-il un glyphe de rang lisible ?
    #[must_use]
    pub fn has_rank(&self) -> bool {
        self.rank.len() == RANK_W * RANK_H
    }

    /// Le coin a-t-il un glyphe de couleur lisible ?
    #[must_use]
    pub fn has_suit(&self) -> bool {
        self.suit.len() == SUIT_W * SUIT_H
    }
}

/// Une carte reconnue et la confiance de la lecture (0 à 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recognized {
    pub card: Card,
    pub confidence: f32,
}

impl Recognized {
    /// La confiance au format de la plateforme (décision T7). Une confiance brute qui n'est
    /// pas un nombre (`NaN`, seul cas hors de `[0, 1]` après le bornage) vaut 0 : une lecture
    /// douteuse doit passer sous le seuil de vérification, jamais être déclarée certaine.
    #[must_use]
    pub fn confidence(&self) -> Confidence {
        Confidence::new(self.confidence.clamp(0.0, 1.0))
            .or_else(|_| Confidence::new(0.0))
            .expect("0 est une confiance valide")
    }
}

/// Taux d'encre d'une forme, échantillonné dans une grille `gw × gh` (4 × 4 points par
/// case) sur la boîte `(x0, y0, w, h)`.
pub(crate) fn coverage(
    ink: impl Fn(i32, i32) -> bool,
    (x0, y0, w, h): (f32, f32, f32, f32),
    gw: usize,
    gh: usize,
) -> Vec<f32> {
    const S: usize = 4;
    let mut out = vec![0.0f32; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let mut n = 0u32;
            for sy in 0..S {
                for sx in 0..S {
                    let px = x0 + w * (gx as f32 + (sx as f32 + 0.5) / S as f32) / gw as f32;
                    let py = y0 + h * (gy as f32 + (sy as f32 + 0.5) / S as f32) / gh as f32;
                    if ink(px.floor() as i32, py.floor() as i32) {
                        n += 1;
                    }
                }
            }
            out[gy * gw + gx] = n as f32 / (S * S) as f32;
        }
    }
    out
}

fn mse(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / a.len() as f32
}

fn rank_distance(p: &Patch, features: &[f32], aspect: f32, parts: u8) -> f32 {
    let da = (p.rank_aspect.min(3.0) - aspect.min(3.0)) / 1.5;
    mse(&p.rank, features) + 0.08 * da * da + if p.rank_parts == parts { 0.0 } else { 0.03 }
}

fn suit_distance(p: &Patch, features: &[f32], aspect: f32, ink: InkColor) -> f32 {
    let da = (p.suit_aspect.min(3.0) - aspect.min(3.0)) / 1.5;
    let color = match (p.ink, ink) {
        (InkColor::Unknown, _) | (_, InkColor::Unknown) => 0.0,
        (a, b) if a == b => 0.0,
        _ => 0.12,
    };
    mse(&p.suit, features) + 0.08 * da * da + color
}

/// Confiance d'un plus proche voisin : meilleure distance `d1` (bonne étiquette), meilleure
/// distance d'une autre étiquette `d2`.
fn confidence(d1: f32, d2: f32) -> f32 {
    // Loin de tout modèle : lecture douteuse quelle que soit la marge.
    let absolute = (1.0 - (d1 - 0.1) / 0.15).clamp(0.0, 1.0);
    let margin = if d2.is_finite() {
        let r = d1 / d2.max(1e-6);
        1.0 / (1.0 + (r / 0.6).powi(4))
    } else {
        // Une seule étiquette apprise : aucune comparaison possible.
        0.6
    };
    absolute * margin
}

/// Plus proche étiquette : (étiquette, confiance).
fn nearest<L: Copy + PartialEq>(items: impl Iterator<Item = (L, f32)>) -> Option<(L, f32)> {
    let mut best: Vec<(L, f32)> = Vec::new();
    for (label, d) in items {
        match best.iter_mut().find(|(l, _)| *l == label) {
            Some(e) => e.1 = e.1.min(d),
            None => best.push((label, d)),
        }
    }
    best.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (label, d1) = *best.first()?;
    let d2 = best.get(1).map_or(f32::INFINITY, |b| b.1);
    Some((label, confidence(d1, d2)))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct RankTemplate {
    /// Index de `Rank` (0 : Deux … 12 : As).
    rank: u8,
    features: Vec<f32>,
    aspect: f32,
    parts: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SuitTemplate {
    /// Index de `Suit` (0 : trèfle, 1 : carreau, 2 : cœur, 3 : pique).
    suit: u8,
    features: Vec<f32>,
    aspect: f32,
    ink: InkColor,
}

/// Les modèles appris pour le jeu de cartes d'une salle.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Templates {
    ranks: Vec<RankTemplate>,
    suits: Vec<SuitTemplate>,
}

impl Templates {
    /// Une carte étiquetée apprend son rang et sa couleur.
    pub fn learn(&mut self, patch: &Patch, card: Card) {
        if patch.has_rank() {
            let rank = card.rank().index();
            if self.ranks.iter().filter(|t| t.rank == rank).count() >= PER_LABEL
                && let Some(i) = self.ranks.iter().position(|t| t.rank == rank)
            {
                self.ranks.remove(i);
            }
            self.ranks.push(RankTemplate {
                rank,
                features: patch.rank.clone(),
                aspect: patch.rank_aspect,
                parts: patch.rank_parts,
            });
        }
        if patch.has_suit() {
            let suit = card.suit().index();
            if self.suits.iter().filter(|t| t.suit == suit).count() >= PER_LABEL
                && let Some(i) = self.suits.iter().position(|t| t.suit == suit)
            {
                self.suits.remove(i);
            }
            self.suits.push(SuitTemplate {
                suit,
                features: patch.suit.clone(),
                aspect: patch.suit_aspect,
                ink: patch.ink,
            });
        }
    }

    fn rank_of(&self, patch: &Patch) -> Option<(Rank, f32)> {
        if !patch.has_rank() || self.ranks.is_empty() {
            return None;
        }
        let (r, c) = nearest(
            self.ranks
                .iter()
                .map(|t| (t.rank, rank_distance(patch, &t.features, t.aspect, t.parts))),
        )?;
        Some((Rank::from_index(r)?, c))
    }

    fn suit_of(&self, patch: &Patch) -> Option<(Suit, f32)> {
        if self.suits.is_empty() {
            return None;
        }
        if !patch.has_suit() {
            // Sans glyphe de couleur, l'encre suffit quand elle est sans ambiguïté
            // (jeu à 4 couleurs, ou une seule couleur apprise pour cette encre).
            let known: Vec<u8> = self
                .suits
                .iter()
                .filter(|t| t.ink == patch.ink && patch.ink != InkColor::Unknown)
                .map(|t| t.suit)
                .collect();
            let first = *known.first()?;
            return known
                .iter()
                .all(|s| *s == first)
                .then(|| (Suit::from_index(first), 0.5))
                .and_then(|(s, c)| s.map(|s| (s, c)));
        }
        let (s, c) = nearest(
            self.suits
                .iter()
                .map(|t| (t.suit, suit_distance(patch, &t.features, t.aspect, t.ink))),
        )?;
        Some((Suit::from_index(s)?, c))
    }

    /// La carte reconnue par les modèles appris (`None` si aucun rang ou aucune couleur
    /// n'est encore appris, ou si le coin est illisible).
    #[must_use]
    pub fn classify(&self, patch: &Patch) -> Option<Recognized> {
        let (rank, cr) = self.rank_of(patch)?;
        let (suit, cs) = self.suit_of(patch)?;
        Some(Recognized {
            card: Card::new(rank, suit),
            confidence: cr * cs,
        })
    }

    /// Rangs différents appris.
    #[must_use]
    pub fn ranks_known(&self) -> usize {
        let mut v: Vec<u8> = self.ranks.iter().map(|t| t.rank).collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    /// Couleurs différentes apprises.
    #[must_use]
    pub fn suits_known(&self) -> usize {
        let mut v: Vec<u8> = self.suits.iter().map(|t| t.suit).collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    /// Nombre d'exemples gardés (rangs et couleurs).
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranks.len() + self.suits.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranks.is_empty() && self.suits.is_empty()
    }
}

/// Les glyphes de la police générique, rendus dans les grilles normalisées.
struct Generic {
    ranks: Vec<(u8, Vec<f32>, f32, u8)>,
    suits: Vec<(u8, Vec<f32>, f32)>,
}

fn generic() -> &'static Generic {
    static G: std::sync::OnceLock<Generic> = std::sync::OnceLock::new();
    G.get_or_init(|| {
        let render = |glyph: &[&str], bold: bool, gw: usize, gh: usize| {
            let (w, h) = font::size(glyph, bold);
            let f = coverage(
                |x, y| font::lit(glyph, x, y, bold),
                (0.0, 0.0, w as f32, h as f32),
                gw,
                gh,
            );
            (f, w as f32 / h as f32)
        };
        let mut ranks = Vec::new();
        for (i, g) in font::RANKS.iter().enumerate() {
            for bold in [false, true] {
                let (f, a) = render(g, bold, RANK_W, RANK_H);
                ranks.push((i as u8, f, a, font::parts(i)));
            }
        }
        let mut suits = Vec::new();
        for (i, g) in font::SUITS.iter().enumerate() {
            for bold in [false, true] {
                let (f, a) = render(g, bold, SUIT_W, SUIT_H);
                suits.push((i as u8, f, a));
            }
        }
        Generic { ranks, suits }
    })
}

/// La meilleure hypothèse sans calibration : police générique et couleur de l'encre.
/// Une hypothèse générique : rang et couleur, chacun avec sa confiance.
type Guess = (Option<(Rank, f32)>, Option<(Suit, f32)>);

fn generic_guess(patch: &Patch) -> Guess {
    let g = generic();
    let rank = if patch.has_rank() {
        nearest(
            g.ranks
                .iter()
                .map(|(r, f, a, p)| (*r, rank_distance(patch, f, *a, *p))),
        )
        .and_then(|(r, c)| Rank::from_index(r).map(|r| (r, c)))
    } else {
        None
    };
    let allowed = patch.ink.suits();
    let suit = if allowed.len() == 1 {
        Some((allowed[0], 0.8))
    } else if patch.has_suit() {
        nearest(
            g.suits
                .iter()
                .filter(|(s, _, _)| allowed.iter().any(|a| a.index() == *s))
                .map(|(s, f, a)| (*s, suit_distance(patch, f, *a, InkColor::Unknown))),
        )
        .and_then(|(s, c)| Suit::from_index(s).map(|s| (s, c)))
    } else {
        None
    };
    (rank, suit)
}

/// Reconnaît une carte : les modèles appris d'abord (rang et couleur séparément), sinon
/// la police générique, dont la confiance est plafonnée à 0,5.
#[must_use]
pub fn recognize(templates: &Templates, patch: &Patch) -> Option<Recognized> {
    if let Some(r) = templates.classify(patch) {
        return Some(r);
    }
    let (gr, gs) = generic_guess(patch);
    let (rank, cr, learned_rank) = match templates.rank_of(patch) {
        Some((r, c)) => (r, c, true),
        None => {
            let (r, c) = gr?;
            (r, c, false)
        }
    };
    let (suit, cs, learned_suit) = match templates.suit_of(patch) {
        Some((s, c)) => (s, c, true),
        None => {
            let (s, c) = gs?;
            (s, c, false)
        }
    };
    let mut confidence = cr * cs;
    if !(learned_rank && learned_suit) {
        confidence = confidence.min(0.5);
    }
    Some(Recognized {
        card: Card::new(rank, suit),
        confidence,
    })
}

#[cfg(test)]
pub(crate) fn debug_distances(t: &Templates, p: &Patch) -> ((u8, f32, f32), (u8, f32, f32)) {
    fn two<L: Copy + PartialEq>(items: impl Iterator<Item = (L, f32)>) -> (L, f32, f32) {
        let mut best: Vec<(L, f32)> = Vec::new();
        for (l, d) in items {
            match best.iter_mut().find(|(x, _)| *x == l) {
                Some(e) => e.1 = e.1.min(d),
                None => best.push((l, d)),
            }
        }
        best.sort_by(|a, b| a.1.total_cmp(&b.1));
        (best[0].0, best[0].1, best.get(1).map_or(9.0, |b| b.1))
    }
    (
        two(t
            .ranks
            .iter()
            .map(|x| (x.rank, rank_distance(p, &x.features, x.aspect, x.parts)))),
        two(t
            .suits
            .iter()
            .map(|x| (x.suit, suit_distance(p, &x.features, x.aspect, x.ink)))),
    )
}
