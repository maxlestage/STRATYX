//! Une petite police bitmap des rangs et des couleurs, pour la reconnaissance générique
//! (avant toute calibration) : chaque glyphe est rendu dans la même grille normalisée que
//! les coins des cartes détectées.

/// Les rangs, du Deux à l'As (index de `stratyx_poker::Rank`), en 5 × 7.
pub(crate) const RANKS: [&[&str]; 13] = [
    // 2
    &[
        ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
    ],
    // 3
    &[
        "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
    ],
    // 4
    &[
        "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
    ],
    // 5
    &[
        "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
    ],
    // 6
    &[
        "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
    ],
    // 7
    &[
        "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
    ],
    // 8
    &[
        ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
    ],
    // 9
    &[
        ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
    ],
    // 10 (« 1 » et « 0 » côte à côte)
    &[
        "..#...###.",
        ".##..#...#",
        "..#..#..##",
        "..#..#.#.#",
        "..#..##..#",
        "..#..#...#",
        ".###..###.",
    ],
    // J
    &[
        "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
    ],
    // Q
    &[
        ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
    ],
    // K
    &[
        "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
    ],
    // A
    &[
        ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
    ],
];

/// Les couleurs (index de `stratyx_poker::Suit` : trèfle, carreau, cœur, pique), en 7 × 7.
pub(crate) const SUITS: [&[&str]; 4] = [
    // trèfle
    &[
        "..###..", "..###..", "##.#.##", "#######", "##.#.##", "...#...", "..###..",
    ],
    // carreau
    &[
        "...#...", "..###..", ".#####.", "#######", ".#####.", "..###..", "...#...",
    ],
    // cœur
    &[
        ".##.##.", "#######", "#######", ".#####.", "..###..", "...#...",
    ],
    // pique
    &[
        "...#...", "..###..", ".#####.", "#######", "#######", "...#...", "..###..",
    ],
];

/// Un glyphe : est-ce que `(x, y)` est encré ? (`bold` épaissit d'un pixel vers la droite.)
pub(crate) fn lit(glyph: &[&str], x: i32, y: i32, bold: bool) -> bool {
    let at = |x: i32, y: i32| -> bool {
        y >= 0
            && x >= 0
            && glyph
                .get(y as usize)
                .and_then(|row| row.as_bytes().get(x as usize))
                .is_some_and(|c| *c == b'#')
    };
    at(x, y) || (bold && at(x - 1, y))
}

/// Taille d'un glyphe (largeur, hauteur), épaississement compris.
pub(crate) fn size(glyph: &[&str], bold: bool) -> (u32, u32) {
    let w = glyph.iter().map(|r| r.len()).max().unwrap_or(0) as u32 + u32::from(bold);
    (w, glyph.len() as u32)
}

/// Nombre de morceaux séparés d'un rang (2 pour « 10 »).
pub(crate) fn parts(rank_index: usize) -> u8 {
    if rank_index == 8 { 2 } else { 1 }
}
