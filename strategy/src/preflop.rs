//! Matrice d'équité préflop 169 × 169 et outils sur les classes de mains.
//!
//! # Indexation des classes
//!
//! Les 169 classes sont indexées par leur position dans la grille 13 × 13 habituelle
//! ([`HandClass::grid_position`]) : `index = ligne × 13 + colonne`, As en haut à gauche,
//! paires sur la diagonale, assorties au-dessus, dépareillées en dessous. Ainsi `AA` = 0,
//! `AKs` = 1, `AKo` = 13, `22` = 168. Toutes les tables par classe de ce crate
//! ([`ClassTable`], matrice, tableaux push/fold) suivent cet ordre.
//!
//! # Matrice d'équité
//!
//! [`PreflopMatrix`] donne, pour chaque couple de classes `(a, b)`, l'équité de `a` contre `b`
//! à tapis préflop (board complet de cinq cartes), en moyenne **pondérée sur toutes les
//! combinaisons compatibles** : une combinaison de `a` et une de `b` ne partagent aucune carte
//! (effets de blocage). Les égalités comptent pour moitié dans l'équité, et leur probabilité
//! est conservée à part.
//!
//! La matrice embarquée ([`PreflopMatrix::embedded`]) est générée par
//! `cargo run --release -p stratyx-strategy --example gen_preflop_matrix`, qui s'appuie sur
//! [`stratyx_poker::equity`] en Monte-Carlo à graine fixe (200 000 tirages par couple, soit
//! une erreur-type ≤ 0,12 point ; ±0,3 point à trois écarts-types). Elle est stockée dans
//! `strategy/data/preflop_equity_169.bin` (format décrit dans [`PreflopMatrix::to_bytes`],
//! 114 Ko) et intégrée au binaire par `include_bytes!`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use stratyx_poker::equity::{EquityError, EquityOptions, equity};
use stratyx_poker::{CardSet, HandClass, HoleCards, Range, Rank};
use thiserror::Error;

/// Nombre de classes de mains de départ.
pub const NUM_CLASSES: usize = 169;

/// Nombre de combinaisons de deux cartes : C(52, 2).
pub const NUM_COMBOS: usize = stratyx_poker::range::NUM_COMBOS;

/// Index de la classe dans `0..169` (ordre de la grille 13 × 13, voir le module).
#[must_use]
pub const fn class_index(class: HandClass) -> usize {
    let (row, col) = class.grid_position();
    row * 13 + col
}

/// Classe correspondant à un index de `0..169`.
#[must_use]
pub const fn class_at(index: usize) -> Option<HandClass> {
    if index >= NUM_CLASSES {
        return None;
    }
    let (row, col) = (index / 13, index % 13);
    let row_rank = Rank::ALL[12 - row];
    let col_rank = Rank::ALL[12 - col];
    Some(if row == col {
        HandClass::Pair(row_rank)
    } else if row < col {
        HandClass::Suited(row_rank, col_rank)
    } else {
        HandClass::Offsuit(col_rank, row_rank)
    })
}

/// Les 169 classes, dans l'ordre des index.
pub fn classes() -> impl Iterator<Item = HandClass> {
    (0..NUM_CLASSES).filter_map(class_at)
}

/// Nombre de combinaisons de la classe : 6 (paire), 4 (assortie) ou 12 (dépareillée).
#[must_use]
pub const fn class_combo_count(class: HandClass) -> usize {
    match class {
        HandClass::Pair(_) => 6,
        HandClass::Suited(..) => 4,
        HandClass::Offsuit(..) => 12,
    }
}

/// Analyse le nom d'une classe : `AA`, `AKs`, `AKo` (casse indifférente pour les couleurs).
///
/// `AK` sans précision est ambigu et refusé, de même que `AAs`.
#[must_use]
pub fn parse_class(s: &str) -> Option<HandClass> {
    let chars: Vec<char> = s.trim().chars().collect();
    let (a, b, kind) = match chars.as_slice() {
        [a, b] => (*a, *b, None),
        [a, b, k] => (*a, *b, Some(k.to_ascii_lowercase())),
        _ => return None,
    };
    let ra = Rank::from_char(a)?;
    let rb = Rank::from_char(b)?;
    let (hi, lo) = if ra >= rb { (ra, rb) } else { (rb, ra) };
    match (hi == lo, kind) {
        (true, None) => Some(HandClass::Pair(hi)),
        (false, Some('s')) => Some(HandClass::Suited(hi, lo)),
        (false, Some('o')) => Some(HandClass::Offsuit(hi, lo)),
        _ => None,
    }
}

/// Combinaisons de chaque classe, dans l'ordre des index.
pub(crate) fn class_combos() -> &'static [Vec<HoleCards>] {
    static COMBOS: LazyLock<Vec<Vec<HoleCards>>> =
        LazyLock::new(|| classes().map(HandClass::combos).collect());
    &COMBOS
}

/// Nombre de couples de combinaisons sans carte commune, pour chaque couple de classes.
fn compat_table() -> &'static [u8] {
    static TABLE: LazyLock<Vec<u8>> = LazyLock::new(|| {
        let combos = class_combos();
        let mut out = vec![0u8; NUM_CLASSES * NUM_CLASSES];
        for (i, ci) in combos.iter().enumerate() {
            for (j, cj) in combos.iter().enumerate() {
                let n = ci
                    .iter()
                    .flat_map(|a| cj.iter().map(move |b| (a.set(), b.set())))
                    .filter(|(a, b)| !a.intersects(*b))
                    .count();
                out[i * NUM_CLASSES + j] = u8::try_from(n).expect("au plus 144 couples");
            }
        }
        out
    });
    &TABLE
}

/// Nombre de couples (combinaison de `a`, combinaison de `b`) sans carte commune.
///
/// C'est le poids, à l'échelle des classes, d'une confrontation `a` contre `b` : il intègre
/// les effets de blocage (`AA` contre `AKs` : 6 × 4 − 3 × 4 = 12 couples, pas 24).
#[must_use]
pub fn compatible_combos(a: HandClass, b: HandClass) -> u32 {
    u32::from(compat_table()[class_index(a) * NUM_CLASSES + class_index(b)])
}

/// Même chose que [`compatible_combos`], par index de classe.
pub(crate) fn compatible_combos_idx(i: usize, j: usize) -> u32 {
    u32::from(compat_table()[i * NUM_CLASSES + j])
}

/// Une valeur (fréquence, EV…) par classe de main, dans l'ordre des index.
///
/// Sérialisée sous forme de dictionnaire `{"AA": 1.0, "AKs": 0.5, …}`, dans l'ordre de la
/// grille ; les classes absentes valent 0 à la désérialisation.
#[derive(Clone, PartialEq)]
pub struct ClassTable {
    values: Vec<f32>,
}

impl ClassTable {
    /// Table dont toutes les valeurs valent `value`.
    #[must_use]
    pub fn filled(value: f32) -> ClassTable {
        ClassTable {
            values: vec![value; NUM_CLASSES],
        }
    }

    /// Table construite à partir d'une fonction de la classe.
    #[must_use]
    pub fn from_fn(mut f: impl FnMut(HandClass) -> f32) -> ClassTable {
        ClassTable {
            values: classes().map(&mut f).collect(),
        }
    }

    /// Table construite à partir de 169 valeurs dans l'ordre des index.
    ///
    /// Renvoie `None` si la longueur n'est pas 169.
    #[must_use]
    pub fn from_values(values: Vec<f32>) -> Option<ClassTable> {
        (values.len() == NUM_CLASSES).then_some(ClassTable { values })
    }

    #[must_use]
    pub fn get(&self, class: HandClass) -> f32 {
        self.values[class_index(class)]
    }

    pub fn set(&mut self, class: HandClass, value: f32) {
        self.values[class_index(class)] = value;
    }

    /// Les 169 valeurs dans l'ordre des index.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Couples (classe, valeur) dans l'ordre des index.
    pub fn iter(&self) -> impl Iterator<Item = (HandClass, f32)> + '_ {
        classes().zip(self.values.iter().copied())
    }

    /// Valeurs disposées en grille 13 × 13 (`grille[ligne][colonne]`, voir
    /// [`HandClass::grid_position`]).
    #[must_use]
    pub fn grid(&self) -> [[f32; 13]; 13] {
        let mut g = [[0.0; 13]; 13];
        for (i, v) in self.values.iter().enumerate() {
            g[i / 13][i % 13] = *v;
        }
        g
    }

    /// Moyenne pondérée par le nombre de combinaisons de chaque classe.
    #[must_use]
    pub fn combo_weighted_mean(&self) -> f64 {
        self.iter()
            .map(|(c, v)| class_combo_count(c) as f64 * f64::from(v))
            .sum::<f64>()
            / NUM_COMBOS as f64
    }
}

impl fmt::Debug for ClassTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.iter().map(|(c, v)| (c.to_string(), v)))
            .finish()
    }
}

impl Serialize for ClassTable {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter().map(|(c, v)| (c.to_string(), v)))
    }
}

impl<'de> Deserialize<'de> for ClassTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, f32>::deserialize(deserializer)?;
        let mut table = ClassTable::filled(0.0);
        for (name, value) in map {
            let class = parse_class(&name).ok_or_else(|| {
                serde::de::Error::custom(format!("classe de main inconnue : « {name} »"))
            })?;
            table.set(class, value);
        }
        Ok(table)
    }
}

/// Erreurs de calcul ou de lecture de la matrice préflop.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MatrixError {
    #[error("données de matrice préflop invalides : {0}")]
    Format(String),
    #[error("calcul d'équité impossible pour {a} contre {b} : {source}")]
    Equity {
        a: String,
        b: String,
        source: EquityError,
    },
}

/// Paramètres de génération de la matrice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixOptions {
    /// Tirages Monte-Carlo par couple de classes.
    pub samples: u64,
    /// Graine de base ; chaque couple utilise une graine dérivée de façon déterministe.
    pub seed: u64,
}

impl Default for MatrixOptions {
    fn default() -> Self {
        MatrixOptions {
            samples: 200_000,
            seed: 0x0053_5452_4154_5958, // « STRATYX »
        }
    }
}

/// Matrice d'équité préflop 169 × 169 (voir la documentation du module).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MatrixData")]
pub struct PreflopMatrix {
    samples: u64,
    seed: u64,
    /// `equity[i * 169 + j]` : équité de la classe `i` contre la classe `j`.
    equity: Vec<f32>,
    /// `tie[i * 169 + j]` : probabilité d'égalité (symétrique).
    tie: Vec<f32>,
}

/// Forme désérialisée de [`PreflopMatrix`], validée avant usage.
#[derive(Deserialize)]
struct MatrixData {
    samples: u64,
    seed: u64,
    equity: Vec<f32>,
    tie: Vec<f32>,
}

impl TryFrom<MatrixData> for PreflopMatrix {
    type Error = MatrixError;

    fn try_from(d: MatrixData) -> Result<Self, Self::Error> {
        let n = NUM_CLASSES * NUM_CLASSES;
        if d.equity.len() != n || d.tie.len() != n {
            return Err(MatrixError::Format(format!(
                "{n} équités et {n} probabilités d'égalité attendues"
            )));
        }
        if d.equity
            .iter()
            .chain(&d.tie)
            .any(|x| !(0.0..=1.0).contains(x))
        {
            return Err(MatrixError::Format(
                "probabilité hors de [0, 1]".to_string(),
            ));
        }
        Ok(PreflopMatrix {
            samples: d.samples,
            seed: d.seed,
            equity: d.equity,
            tie: d.tie,
        })
    }
}

const MAGIC: &[u8; 8] = b"STXPF169";
const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: usize = 8 + 2 + 8 + 8;
const SCALE: f64 = 65_535.0;

static EMBEDDED_BYTES: &[u8] = include_bytes!("../data/preflop_equity_169.bin");

static EMBEDDED: LazyLock<PreflopMatrix> = LazyLock::new(|| {
    PreflopMatrix::from_bytes(EMBEDDED_BYTES).expect(
        "matrice préflop embarquée invalide : relancer l'exemple gen_preflop_matrix \
         (cargo run --release -p stratyx-strategy --example gen_preflop_matrix)",
    )
});

impl fmt::Debug for PreflopMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreflopMatrix")
            .field("samples", &self.samples)
            .field("seed", &self.seed)
            .finish_non_exhaustive()
    }
}

impl PreflopMatrix {
    /// Matrice embarquée dans le binaire (générée par l'exemple `gen_preflop_matrix`).
    #[must_use]
    pub fn embedded() -> &'static PreflopMatrix {
        &EMBEDDED
    }

    /// Calcule la matrice complète par Monte-Carlo (plusieurs minutes en `--release`).
    ///
    /// Seuls les couples `i ≤ j` sont simulés ; `e(j, i) = 1 − e(i, j)` et `e(i, i) = 1/2`
    /// exactement (symétrie). `progress(fait, total)` est appelé après chaque couple.
    pub fn compute(
        options: &MatrixOptions,
        progress: &(dyn Fn(usize, usize) + Sync),
    ) -> Result<PreflopMatrix, MatrixError> {
        let n = NUM_CLASSES;
        let mut eq = vec![0.0f32; n * n];
        let mut tie = vec![0.0f32; n * n];
        let total = n * (n + 1) / 2;
        let mut done = 0;
        for i in 0..n {
            for j in i..n {
                let (a, b) = (class_at(i).expect("index"), class_at(j).expect("index"));
                let mut ra = Range::empty();
                ra.set_class(a, 1.0);
                let mut rb = Range::empty();
                rb.set_class(b, 1.0);
                let pair_index = (i * n + j) as u64;
                let opts = EquityOptions {
                    max_exact_evaluations: 0,
                    samples: options.samples,
                    seed: options.seed ^ pair_index.wrapping_mul(0x9E37_79B9_7F4A_7C15),
                };
                let res =
                    equity(&[ra, rb], CardSet::EMPTY, CardSet::EMPTY, &opts).map_err(|source| {
                        MatrixError::Equity {
                            a: a.to_string(),
                            b: b.to_string(),
                            source,
                        }
                    })?;
                let p = res.players[0];
                let e = if i == j { 0.5 } else { p.equity };
                eq[i * n + j] = e as f32;
                eq[j * n + i] = (1.0 - e) as f32;
                tie[i * n + j] = p.tie as f32;
                tie[j * n + i] = p.tie as f32;
                done += 1;
                progress(done, total);
            }
        }
        Ok(PreflopMatrix {
            samples: options.samples,
            seed: options.seed,
            equity: eq,
            tie,
        })
    }

    /// Tirages Monte-Carlo par couple utilisés pour la génération.
    #[must_use]
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Graine de base utilisée pour la génération.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Équité de `a` contre `b` (égalités comptées pour moitié), dans `[0, 1]`.
    #[must_use]
    pub fn equity(&self, a: HandClass, b: HandClass) -> f64 {
        self.equity_idx(class_index(a), class_index(b))
    }

    /// Probabilité d'égalité entre `a` et `b`.
    #[must_use]
    pub fn tie(&self, a: HandClass, b: HandClass) -> f64 {
        self.tie_idx(class_index(a), class_index(b))
    }

    /// Probabilité que `a` gagne seul contre `b` : `équité − égalité / 2`.
    #[must_use]
    pub fn win(&self, a: HandClass, b: HandClass) -> f64 {
        self.win_idx(class_index(a), class_index(b))
    }

    /// Équité par index de classe (voir [`class_index`]).
    #[must_use]
    pub fn equity_idx(&self, i: usize, j: usize) -> f64 {
        f64::from(self.equity[i * NUM_CLASSES + j])
    }

    /// Probabilité d'égalité par index de classe.
    #[must_use]
    pub fn tie_idx(&self, i: usize, j: usize) -> f64 {
        f64::from(self.tie[i * NUM_CLASSES + j])
    }

    /// Probabilité de victoire seule par index de classe.
    #[must_use]
    pub fn win_idx(&self, i: usize, j: usize) -> f64 {
        (self.equity_idx(i, j) - 0.5 * self.tie_idx(i, j)).max(0.0)
    }

    /// Équité de la classe `hero` contre une range, à tapis préflop.
    ///
    /// Chaque classe de la range est pondérée par la fréquence moyenne de ses combinaisons
    /// compatibles avec une main de `hero` (blocages inclus) ; l'approximation est exacte
    /// lorsque les fréquences sont uniformes au sein de chaque classe. Renvoie `None` si la
    /// range est vide (ou entièrement bloquée).
    #[must_use]
    pub fn equity_vs_range(&self, hero: HandClass, villain: &Range) -> Option<f64> {
        let hero_combos = hero.combos();
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for (j, combos) in class_combos().iter().enumerate() {
            let w: f64 = combos
                .iter()
                .map(|v| {
                    let wv = f64::from(villain.get(*v));
                    if wv == 0.0 {
                        return 0.0;
                    }
                    let compatible = hero_combos
                        .iter()
                        .filter(|h| !h.set().intersects(v.set()))
                        .count();
                    wv * compatible as f64
                })
                .sum();
            if w > 0.0 {
                num += w * self.equity_idx(class_index(hero), j);
                den += w;
            }
        }
        (den > 0.0).then(|| num / den)
    }

    /// Encodage binaire compact (petit-boutiste) :
    ///
    /// | Octets | Contenu                                                        |
    /// |--------|----------------------------------------------------------------|
    /// | 8      | signature `STXPF169`                                           |
    /// | 2      | version du format (`u16`, actuellement 1)                      |
    /// | 8      | tirages par couple (`u64`)                                     |
    /// | 8      | graine de base (`u64`)                                         |
    /// | 4 × 169² | pour chaque couple `(i, j)` en ordre ligne : équité puis égalité, `u16` sur 65 535 |
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + 4 * self.equity.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.samples.to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        let q = |x: f32| (f64::from(x).clamp(0.0, 1.0) * SCALE).round() as u16;
        for (e, t) in self.equity.iter().zip(&self.tie) {
            out.extend_from_slice(&q(*e).to_le_bytes());
            out.extend_from_slice(&q(*t).to_le_bytes());
        }
        out
    }

    /// Décode le format de [`PreflopMatrix::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<PreflopMatrix, MatrixError> {
        let err = |m: &str| MatrixError::Format(m.to_string());
        let expected = HEADER_LEN + 4 * NUM_CLASSES * NUM_CLASSES;
        if bytes.len() != expected {
            return Err(MatrixError::Format(format!(
                "taille {} octets, {expected} attendus",
                bytes.len()
            )));
        }
        if &bytes[..8] != MAGIC {
            return Err(err("signature absente"));
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != FORMAT_VERSION {
            return Err(MatrixError::Format(format!(
                "version {version} non prise en charge"
            )));
        }
        let u64_at = |o: usize| u64::from_le_bytes(bytes[o..o + 8].try_into().expect("8 octets"));
        let samples = u64_at(10);
        let seed = u64_at(18);
        let n = NUM_CLASSES * NUM_CLASSES;
        let mut equity = Vec::with_capacity(n);
        let mut tie = Vec::with_capacity(n);
        for chunk in bytes[HEADER_LEN..].chunks_exact(4) {
            let e = u16::from_le_bytes([chunk[0], chunk[1]]);
            let t = u16::from_le_bytes([chunk[2], chunk[3]]);
            equity.push((f64::from(e) / SCALE) as f32);
            tie.push((f64::from(t) / SCALE) as f32);
        }
        Ok(PreflopMatrix {
            samples,
            seed,
            equity,
            tie,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> HandClass {
        parse_class(s).unwrap()
    }

    #[test]
    fn class_indexing_is_a_bijection_following_the_grid() {
        for (i, class) in classes().enumerate() {
            assert_eq!(class_index(class), i);
            assert_eq!(class_at(i), Some(class));
            assert_eq!(parse_class(&class.to_string()), Some(class));
        }
        assert_eq!(class_index(c("AA")), 0);
        assert_eq!(class_index(c("AKs")), 1);
        assert_eq!(class_index(c("AKo")), 13);
        assert_eq!(class_index(c("22")), 168);
        assert_eq!(class_at(169), None);
        let total: usize = classes().map(class_combo_count).sum();
        assert_eq!(total, NUM_COMBOS);
        for bad in ["AK", "AAs", "A", "XYs", "AKx"] {
            assert_eq!(parse_class(bad), None, "{bad}");
        }
    }

    #[test]
    fn compatible_combos_include_card_removal() {
        assert_eq!(compatible_combos(c("AA"), c("KK")), 36);
        assert_eq!(compatible_combos(c("AA"), c("AA")), 6);
        assert_eq!(compatible_combos(c("AA"), c("AKs")), 12);
        assert_eq!(compatible_combos(c("AKs"), c("AKo")), 4 * 6);
        assert_eq!(compatible_combos(c("72o"), c("AA")), 72);
        for a in classes() {
            for b in classes() {
                assert_eq!(compatible_combos(a, b), compatible_combos(b, a));
            }
        }
    }

    #[test]
    fn class_table_grid_and_serde() {
        let t = ClassTable::from_fn(|c| class_index(c) as f32);
        assert_eq!(t.grid()[0][1], 1.0);
        assert_eq!(t.grid()[1][0], 13.0);
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.starts_with(r#"{"AA":0.0,"AKs":1.0"#), "{json}");
        let back: ClassTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(serde_json::from_str::<ClassTable>(r#"{"AX": 1.0}"#).is_err());
        assert!((ClassTable::filled(1.0).combo_weighted_mean() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn embedded_matrix_matches_reference_values() {
        let m = PreflopMatrix::embedded();
        let close = |a: &str, b: &str, expected: f64, tol: f64| {
            let e = m.equity(c(a), c(b));
            assert!((e - expected).abs() < tol, "{a} contre {b} : {e}");
        };
        close("AA", "KK", 0.82, 0.01);
        close("AKs", "QQ", 0.46, 0.01);
        close("AKo", "22", 0.47, 0.01);
        close("72o", "AA", 0.12, 0.01);
        // Quantification sur 16 bits : 1/2 est représenté à 1,5 × 10⁻⁵ près.
        close("AA", "AA", 0.5, 1e-4);
        // Les paires identiques partagent souvent le pot.
        assert!(m.tie(c("AA"), c("AA")) > 0.9);
        assert!(m.tie(c("AKo"), c("AKo")) > 0.4);
    }

    #[test]
    fn embedded_matrix_is_symmetric() {
        let m = PreflopMatrix::embedded();
        for i in 0..NUM_CLASSES {
            for j in 0..NUM_CLASSES {
                let s = m.equity_idx(i, j) + m.equity_idx(j, i);
                assert!((s - 1.0).abs() < 1e-4, "{i} {j} : {s}");
                assert_eq!(m.tie_idx(i, j), m.tie_idx(j, i));
                let total = m.win_idx(i, j) + m.win_idx(j, i) + m.tie_idx(i, j);
                assert!((total - 1.0).abs() < 1e-4);
            }
        }
        assert!(m.samples() >= 100_000);
    }

    #[test]
    fn equity_vs_range_is_consistent_with_the_matrix() {
        let m = PreflopMatrix::embedded();
        let kk = Range::parse("KK").unwrap();
        let e = m.equity_vs_range(c("AA"), &kk).unwrap();
        assert!((e - m.equity(c("AA"), c("KK"))).abs() < 1e-6);
        // Contre une range entière, AA dépasse 85 % et 72o reste sous 35 %.
        let all = Range::full();
        assert!(m.equity_vs_range(c("AA"), &all).unwrap() > 0.85);
        assert!(m.equity_vs_range(c("72o"), &all).unwrap() < 0.35);
        assert_eq!(m.equity_vs_range(c("AA"), &Range::empty()), None);
        // Face à AsAh, une seule main de AA reste possible (AdAc) : l'équité est définie.
        assert!(
            m.equity_vs_range(c("AA"), &Range::parse("AsAh").unwrap())
                .is_some()
        );
    }

    #[test]
    fn binary_roundtrip_and_errors() {
        let m = PreflopMatrix::embedded();
        let bytes = m.to_bytes();
        assert!(bytes.len() < 300 * 1024);
        assert_eq!(&PreflopMatrix::from_bytes(&bytes).unwrap(), m);
        assert!(PreflopMatrix::from_bytes(&bytes[1..]).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(PreflopMatrix::from_bytes(&bad).is_err());
        // Sérialisation serde (JSON) : aller-retour et validation des tailles.
        let json = serde_json::to_string(m).unwrap();
        assert_eq!(&serde_json::from_str::<PreflopMatrix>(&json).unwrap(), m);
        let short = r#"{"samples":1,"seed":0,"equity":[0.5],"tie":[0.0]}"#;
        assert!(serde_json::from_str::<PreflopMatrix>(short).is_err());
    }
}
