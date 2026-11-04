//! Issues des confrontations à trois joueurs à tapis préflop, par triplet de classes.
//!
//! Pour un tapis à trois avec des tapis inégaux (pots secondaires) et pour l'ICM, l'équité
//! seule ne suffit pas : il faut la loi du **classement** des trois mains. Ce module estime,
//! pour chacun des C(171, 3) = 818 805 triplets de classes non ordonnés, la probabilité des
//! 13 classements possibles (6 stricts, 6 avec deux mains à égalité, 1 avec trois).
//!
//! # Méthode (Monte-Carlo à graine fixe, mis en cache)
//!
//! 1. `boards` boards de cinq cartes sont tirés une fois pour toutes (ChaCha8, graine fixe).
//!    Pour chacun, la force de chacune des 1 326 combinaisons est évaluée, puis réduite à
//!    un rang dense sur ce board (`u16`) : les comparaisons ne se font qu'entre mains d'un
//!    même board.
//! 2. Pour chaque triplet de classes, on énumère les triplets de combinaisons mutuellement
//!    compatibles (leur nombre est le poids exact du triplet, blocages compris). Pour chaque
//!    board, on tire un triplet de combinaisons uniformément ; le tirage est rejeté si le board
//!    touche l'une des six cartes. On obtient ainsi un échantillon exactement uniforme des
//!    distributions valides (≈ 53 % d'acceptation, soit ≈ 2 200 tirages utiles par triplet
//!    avec 4 096 boards).
//! 3. Les triplets contenant deux fois la même classe sont symétrisés (deux joueurs de même
//!    classe ont exactement la même équité).
//!
//! Le calcul complet prend environ 11 s en `--release` sur 4 cœurs virtuels et occupe
//! ≈ 23 Mo ; il est fait une fois par processus pour la table partagée
//! ([`ThreeWayEquities::shared`]) et réutilisé pour toutes les profondeurs de tapis (les
//! classements ne dépendent pas des tapis). Précision : erreur-type ≤ 1,1 point par
//! probabilité de classement ; les EV du solveur, qui moyennent des milliers de triplets,
//! sont bien plus précises.
//!
//! Comme dans les calculateurs push/fold usuels, les mains **couchées** ne retirent pas de
//! cartes : seules les mains qui vont à l'abattage se bloquent mutuellement.

use std::sync::OnceLock;

use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use stratyx_poker::{Card, CardSet, HandClass, evaluate};

use crate::preflop::{NUM_CLASSES, NUM_COMBOS, class_combos, class_index};

/// Nombre de classements possibles de trois mains (avec égalités).
pub const NUM_OUTCOMES: usize = 13;

/// Les 13 classements : pour chaque joueur, nombre d'adversaires ayant une main strictement
/// meilleure (0 = meilleure main).
pub const OUTCOMES: [[u8; 3]; NUM_OUTCOMES] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [2, 0, 1],
    [1, 2, 0],
    [2, 1, 0],
    [0, 0, 2],
    [0, 2, 0],
    [2, 0, 0],
    [0, 1, 1],
    [1, 0, 1],
    [1, 1, 0],
    [0, 0, 0],
];

/// Nombre de triplets de classes non ordonnés : C(171, 3).
pub const NUM_TRIPLES: usize = NUM_CLASSES * (NUM_CLASSES + 1) * (NUM_CLASSES + 2) / 6;

const SENTINEL: u16 = u16::MAX;
const PROB_SCALE: f32 = 65_535.0;

/// Index d'un classement à partir du code `r0 * 9 + r1 * 3 + r2` (255 si impossible).
const CODE_TO_OUTCOME: [u8; 27] = {
    let mut t = [255u8; 27];
    let mut o = 0;
    while o < NUM_OUTCOMES {
        let r = OUTCOMES[o];
        t[(r[0] * 9 + r[1] * 3 + r[2]) as usize] = o as u8;
        o += 1;
    }
    t
};

fn outcome_of(ranks: [u8; 3]) -> usize {
    CODE_TO_OUTCOME[usize::from(ranks[0] * 9 + ranks[1] * 3 + ranks[2])] as usize
}

/// Code de classement `r0 × 9 + r1 × 3 + r2` à partir des forces (plus grand = meilleur),
/// calculé sans branchement.
#[inline]
fn rank_code(s: [u16; 3]) -> usize {
    let b0 = u8::from(s[1] > s[0]) + u8::from(s[2] > s[0]);
    let b1 = u8::from(s[0] > s[1]) + u8::from(s[2] > s[1]);
    let b2 = u8::from(s[0] > s[2]) + u8::from(s[1] > s[2]);
    usize::from(b0 * 9 + b1 * 3 + b2)
}

/// Index d'un triplet trié `i ≤ j ≤ k` (système combinatoire des multi-ensembles).
const fn triple_index(i: usize, j: usize, k: usize) -> usize {
    k * (k + 1) * (k + 2) / 6 + j * (j + 1) / 2 + i
}

/// Paramètres de l'estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreeWayOptions {
    /// Nombre de boards tirés (et de tirages proposés par triplet).
    pub boards: u32,
    /// Graine du générateur (boards et tirages, flux distinct par triplet).
    pub seed: u64,
}

impl Default for ThreeWayOptions {
    fn default() -> Self {
        ThreeWayOptions {
            boards: 4096,
            seed: 0x0053_5452_4154_5958, // « STRATYX »
        }
    }
}

/// Table des classements à trois joueurs (voir la documentation du module).
///
/// Sérialisable pour être mise en cache sur disque ; la désérialisation vérifie la taille.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ThreeWayData")]
pub struct ThreeWayEquities {
    options: ThreeWayOptions,
    /// Probabilités des 13 classements par triplet trié, sur 65 535.
    probs: Vec<[u16; NUM_OUTCOMES]>,
    /// Nombre exact de triplets de combinaisons compatibles par triplet trié.
    counts: Vec<u16>,
}

/// Forme désérialisée de [`ThreeWayEquities`], validée avant usage.
#[derive(Deserialize)]
struct ThreeWayData {
    options: ThreeWayOptions,
    probs: Vec<[u16; NUM_OUTCOMES]>,
    counts: Vec<u16>,
}

impl TryFrom<ThreeWayData> for ThreeWayEquities {
    type Error = String;

    fn try_from(d: ThreeWayData) -> Result<Self, Self::Error> {
        if d.probs.len() != NUM_TRIPLES || d.counts.len() != NUM_TRIPLES {
            return Err(format!(
                "table des confrontations à trois invalide : {NUM_TRIPLES} triplets attendus"
            ));
        }
        Ok(ThreeWayEquities {
            options: d.options,
            probs: d.probs,
            counts: d.counts,
        })
    }
}

impl std::fmt::Debug for ThreeWayEquities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreeWayEquities")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// Rangs denses de chaque combinaison sur chaque board, regroupés par classe :
/// `slabs[classe][board * taille + combinaison]`.
struct BoardRanks {
    boards: usize,
    slabs: Vec<Vec<u16>>,
}

impl BoardRanks {
    fn compute(options: &ThreeWayOptions) -> BoardRanks {
        let boards = options.boards.max(1) as usize;
        let mut rng = ChaCha8Rng::seed_from_u64(options.seed);
        let board_sets: Vec<CardSet> = (0..boards)
            .map(|_| {
                let mut b = CardSet::EMPTY;
                while b.len() < 5 {
                    b.insert(Card::from_id(rng.random_range(0..52)).expect("carte valide"));
                }
                b
            })
            .collect();
        let combos = class_combos();
        // Rangs denses par board, dans l'ordre (classe, combinaison).
        let per_board: Vec<Vec<u16>> = board_sets
            .par_iter()
            .map(|&board| {
                let values: Vec<Option<u32>> = combos
                    .iter()
                    .flatten()
                    .map(|h| {
                        (!h.set().intersects(board)).then(|| evaluate(board | h.set()).value())
                    })
                    .collect();
                let mut distinct: Vec<u32> = values.iter().flatten().copied().collect();
                distinct.sort_unstable();
                distinct.dedup();
                values
                    .iter()
                    .map(|v| match v {
                        Some(v) => {
                            u16::try_from(distinct.binary_search(v).expect("valeur présente"))
                                .expect("au plus 1 326 rangs")
                        }
                        None => SENTINEL,
                    })
                    .collect()
            })
            .collect();
        let mut slabs = Vec::with_capacity(NUM_CLASSES);
        let mut offset = 0;
        for class in combos {
            let size = class.len();
            let mut slab = Vec::with_capacity(boards * size);
            for ranks in &per_board {
                slab.extend_from_slice(&ranks[offset..offset + size]);
            }
            slabs.push(slab);
            offset += size;
        }
        debug_assert_eq!(offset, NUM_COMBOS);
        BoardRanks { boards, slabs }
    }
}

impl ThreeWayEquities {
    /// Calcule la table complète (≈ 11 s en `--release` sur 4 cœurs avec 4 096 boards).
    #[must_use]
    pub fn compute(options: &ThreeWayOptions) -> ThreeWayEquities {
        let ranks = BoardRanks::compute(options);
        let combos = class_combos();
        let masks: Vec<Vec<u64>> = combos
            .iter()
            .map(|c| c.iter().map(|h| h.set().bits()).collect())
            .collect();
        let triples: Vec<(usize, usize, usize)> = (0..NUM_CLASSES)
            .flat_map(|k| (0..=k).flat_map(move |j| (0..=j).map(move |i| (i, j, k))))
            .collect();
        debug_assert_eq!(triples.len(), NUM_TRIPLES);
        let results: Vec<([u16; NUM_OUTCOMES], u16)> = triples
            .par_iter()
            .enumerate()
            .map_init(Vec::new, |list, (index, &(i, j, k))| {
                debug_assert_eq!(triple_index(i, j, k), index);
                estimate_triple(&ranks, &masks, [i, j, k], index, options.seed, list)
            })
            .collect();
        let (probs, counts) = results.into_iter().unzip();
        ThreeWayEquities {
            options: *options,
            probs,
            counts,
        }
    }

    /// Table partagée, calculée au premier appel avec les options par défaut.
    pub fn shared() -> &'static ThreeWayEquities {
        static SHARED: OnceLock<ThreeWayEquities> = OnceLock::new();
        SHARED.get_or_init(|| ThreeWayEquities::compute(&ThreeWayOptions::default()))
    }

    /// Options utilisées pour le calcul.
    #[must_use]
    pub fn options(&self) -> ThreeWayOptions {
        self.options
    }

    /// Nombre exact de triplets de combinaisons sans carte commune.
    #[must_use]
    pub fn compatible_combos(&self, a: HandClass, b: HandClass, c: HandClass) -> u32 {
        self.lookup_idx(class_index(a), class_index(b), class_index(c))
            .0
    }

    /// Probabilités des 13 classements ([`OUTCOMES`]) pour les joueurs tenant `a`, `b`, `c`.
    #[must_use]
    pub fn outcome_probabilities(
        &self,
        a: HandClass,
        b: HandClass,
        c: HandClass,
    ) -> [f64; NUM_OUTCOMES] {
        self.lookup_idx(class_index(a), class_index(b), class_index(c))
            .1
            .map(f64::from)
    }

    /// Équité (part du pot, mises égales) de chacun des trois joueurs.
    #[must_use]
    pub fn equities(&self, a: HandClass, b: HandClass, c: HandClass) -> [f64; 3] {
        let probs = self.outcome_probabilities(a, b, c);
        let mut eq = [0.0; 3];
        for (o, p) in OUTCOMES.iter().zip(probs) {
            let winners = o.iter().filter(|r| **r == 0).count() as f64;
            for (e, r) in eq.iter_mut().zip(o) {
                if *r == 0 {
                    *e += p / winners;
                }
            }
        }
        eq
    }

    /// Nombre de triplets compatibles et probabilités des classements, joueurs dans l'ordre
    /// des index donnés.
    pub(crate) fn lookup_idx(&self, a: usize, b: usize, c: usize) -> (u32, [f32; NUM_OUTCOMES]) {
        let mut seats = [(a, 0usize), (b, 1), (c, 2)];
        seats.sort_unstable();
        let index = triple_index(seats[0].0, seats[1].0, seats[2].0);
        let count = u32::from(self.counts[index]);
        // Position canonique de chaque siège.
        let mut pos = [0usize; 3];
        for (p, &(_, seat)) in seats.iter().enumerate() {
            pos[seat] = p;
        }
        let mut out = [0.0f32; NUM_OUTCOMES];
        for (o, &q) in self.probs[index].iter().enumerate() {
            if q == 0 {
                continue;
            }
            let r = OUTCOMES[o];
            let seat_ranks = [r[pos[0]], r[pos[1]], r[pos[2]]];
            out[outcome_of(seat_ranks)] += f32::from(q) / PROB_SCALE;
        }
        (count, out)
    }
}

/// Estime un triplet trié de classes ; renvoie les probabilités quantifiées et le nombre de
/// triplets de combinaisons compatibles.
fn estimate_triple(
    ranks: &BoardRanks,
    masks: &[Vec<u64>],
    classes: [usize; 3],
    index: usize,
    seed: u64,
    list: &mut Vec<[u8; 3]>,
) -> ([u16; NUM_OUTCOMES], u16) {
    let [i, j, k] = classes;
    list.clear();
    for (a, ma) in masks[i].iter().enumerate() {
        for (b, mb) in masks[j].iter().enumerate() {
            if ma & mb != 0 {
                continue;
            }
            for (c, mc) in masks[k].iter().enumerate() {
                if (ma | mb) & mc == 0 {
                    list.push([a as u8, b as u8, c as u8]);
                }
            }
        }
    }
    let count = u16::try_from(list.len()).expect("au plus 1 728 triplets");
    if list.is_empty() {
        return ([0; NUM_OUTCOMES], 0);
    }
    let sizes = [masks[i].len(), masks[j].len(), masks[k].len()];
    let slabs = [&ranks.slabs[i], &ranks.slabs[j], &ranks.slabs[k]];
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x3_3333);
    rng.set_stream(index as u64);
    // Comptes par code de classement `r0 × 9 + r1 × 3 + r2`. Le code 26 ([2, 2, 2],
    // impossible) recueille les tirages rejetés : la boucle reste sans branchement imprévisible.
    const REJECTED: usize = 26;
    let mut tally = [0u32; 27];
    let mut proposed = 0u32;
    let n = list.len() as u64;
    // Quelques passes supplémentaires si trop peu de tirages sont acceptés.
    for _pass in 0..8 {
        for board in 0..ranks.boards {
            let pick = list[((u64::from(rng.next_u32()) * n) >> 32) as usize];
            let r = [0, 1, 2].map(|p| slabs[p][board * sizes[p] + usize::from(pick[p])]);
            let ok = (r[0] != SENTINEL) & (r[1] != SENTINEL) & (r[2] != SENTINEL);
            tally[if ok { rank_code(r) } else { REJECTED }] += 1;
        }
        proposed += ranks.boards as u32;
        if proposed - tally[REJECTED] >= 256 {
            break;
        }
    }
    let accepted = proposed - tally[REJECTED];
    let mut probs = [0.0f64; NUM_OUTCOMES];
    if accepted > 0 {
        for (p, r) in probs.iter_mut().zip(OUTCOMES) {
            let code = usize::from(r[0] * 9 + r[1] * 3 + r[2]);
            *p = f64::from(tally[code]) / f64::from(accepted);
        }
    }
    symmetrize(&mut probs, classes);
    let quantized = probs.map(|p| (p * f64::from(PROB_SCALE)).round() as u16);
    (quantized, count)
}

/// Moyenne sur les permutations des joueurs qui tiennent la même classe.
fn symmetrize(probs: &mut [f64; NUM_OUTCOMES], classes: [usize; 3]) {
    const PERMS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [1, 0, 2],
        [0, 2, 1],
        [2, 1, 0],
        [1, 2, 0],
        [2, 0, 1],
    ];
    let valid: Vec<&[usize; 3]> = PERMS
        .iter()
        .filter(|p| (0..3).all(|s| classes[p[s]] == classes[s]))
        .collect();
    if valid.len() == 1 {
        return;
    }
    let mut out = [0.0; NUM_OUTCOMES];
    for perm in &valid {
        for (o, p) in probs.iter().enumerate() {
            let r = OUTCOMES[o];
            out[outcome_of([r[perm[0]], r[perm[1]], r[perm[2]]])] += p;
        }
    }
    for (dst, src) in probs.iter_mut().zip(out) {
        *dst = src / valid.len() as f64;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::preflop::parse_class;

    /// Table rapide pour les tests (256 boards).
    pub(crate) fn small_table() -> &'static ThreeWayEquities {
        static TABLE: OnceLock<ThreeWayEquities> = OnceLock::new();
        TABLE.get_or_init(|| {
            ThreeWayEquities::compute(&ThreeWayOptions {
                boards: 256,
                ..ThreeWayOptions::default()
            })
        })
    }

    fn c(s: &str) -> HandClass {
        parse_class(s).unwrap()
    }

    #[test]
    fn outcome_tables_are_consistent() {
        let mut seen = [false; NUM_OUTCOMES];
        for (code, o) in CODE_TO_OUTCOME.iter().enumerate() {
            if *o != 255 {
                assert!(!seen[*o as usize]);
                seen[*o as usize] = true;
                let r = OUTCOMES[*o as usize];
                assert_eq!(usize::from(r[0] * 9 + r[1] * 3 + r[2]), code);
            }
        }
        assert!(seen.iter().all(|s| *s));
        let outcome = |s: [u16; 3]| CODE_TO_OUTCOME[rank_code(s)];
        assert_eq!(outcome([5, 3, 1]), 0);
        assert_eq!(outcome([2, 2, 2]), 12);
        assert_eq!(outcome([1, 4, 4]), 8);
        assert_eq!(outcome([4, 1, 4]), 7);
        assert_eq!(outcome([3, 3, 1]), 6);
        assert_eq!(triple_index(0, 0, 0), 0);
        assert_eq!(triple_index(168, 168, 168), NUM_TRIPLES - 1);
    }

    #[test]
    fn three_way_table_is_plausible() {
        let t = small_table();
        // Comptes exacts : AA, KK, QQ sans blocage ; trois AA impossibles.
        assert_eq!(t.compatible_combos(c("AA"), c("KK"), c("QQ")), 216);
        assert_eq!(t.compatible_combos(c("AA"), c("AA"), c("AA")), 0);
        assert_eq!(t.compatible_combos(c("AA"), c("AA"), c("KK")), 6 * 6);
        // AA contre KK et QQ : ≈ 66 % ; somme des probabilités = 1.
        let eq = t.equities(c("AA"), c("KK"), c("QQ"));
        assert!((eq[0] - 0.665).abs() < 0.05, "{eq:?}");
        assert!((eq.iter().sum::<f64>() - 1.0).abs() < 1e-3);
        // Permuter les joueurs permute les équités.
        let eq2 = t.equities(c("QQ"), c("AA"), c("KK"));
        assert!((eq2[1] - eq[0]).abs() < 1e-6 && (eq2[0] - eq[2]).abs() < 1e-6);
        // Même classe : équités identiques par symétrisation.
        let eq3 = t.equities(c("AKs"), c("AKs"), c("72o"));
        assert!((eq3[0] - eq3[1]).abs() < 1e-4, "{eq3:?}");
        let p = t.outcome_probabilities(c("AKo"), c("AKo"), c("AKo"));
        assert!(p[12] > 0.2, "{p:?}");
        // Une table tronquée est refusée à la désérialisation.
        let bad = r#"{"options":{"boards":1,"seed":0},"probs":[],"counts":[]}"#;
        assert!(serde_json::from_str::<ThreeWayEquities>(bad).is_err());
    }
}
