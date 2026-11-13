//! Détection des cartes face visible : les zones claires et peu colorées en forme de
//! carte (masque, composantes connexes, filtres de taille, de proportions et de
//! remplissage), puis lecture du coin de chaque carte.

use crate::frame::{Frame, Rect, is_face, is_ink};
use crate::recognize::{InkColor, Patch, RANK_H, RANK_W, SUIT_H, SUIT_W, coverage};

/// Largeur de travail de la détection (les grandes images sont réduites).
const WORK_WIDTH: u32 = 640;
/// Largeur / hauteur d'une carte.
const CARD_ASPECT: f32 = 0.71;

/// Une carte détectée : son rectangle (dans l'image d'origine), son coin et son encre.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedCard {
    pub rect: Rect,
    pub patch: Patch,
    pub ink: InkColor,
}

/// Une composante connexe : boîte et nombre de pixels.
#[derive(Clone, Copy, Debug)]
struct Blob {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    count: u32,
    touches_edge: bool,
}

impl Blob {
    fn w(&self) -> u32 {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> u32 {
        self.y1 - self.y0 + 1
    }
}

/// Composantes connexes d'un masque (`eight` : 8-connexité).
fn blobs(mask: &[bool], w: u32, h: u32, eight: bool) -> Vec<Blob> {
    let (wu, hu) = (w as usize, h as usize);
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut b = Blob {
            x0: u32::MAX,
            y0: u32::MAX,
            x1: 0,
            y1: 0,
            count: 0,
            touches_edge: false,
        };
        while let Some(i) = stack.pop() {
            let (x, y) = (i % wu, i / wu);
            b.x0 = b.x0.min(x as u32);
            b.x1 = b.x1.max(x as u32);
            b.y0 = b.y0.min(y as u32);
            b.y1 = b.y1.max(y as u32);
            b.count += 1;
            if x == 0 || y == 0 || x == wu - 1 || y == hu - 1 {
                b.touches_edge = true;
            }
            let mut visit = |nx: usize, ny: usize| {
                let j = ny * wu + nx;
                if mask[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(x - 1, y);
            }
            if x + 1 < wu {
                visit(x + 1, y);
            }
            if y > 0 {
                visit(x, y - 1);
            }
            if y + 1 < hu {
                visit(x, y + 1);
            }
            if eight {
                if x > 0 && y > 0 {
                    visit(x - 1, y - 1);
                }
                if x + 1 < wu && y > 0 {
                    visit(x + 1, y - 1);
                }
                if x > 0 && y + 1 < hu {
                    visit(x - 1, y + 1);
                }
                if x + 1 < wu && y + 1 < hu {
                    visit(x + 1, y + 1);
                }
            }
        }
        out.push(b);
    }
    out
}

/// La plus longue suite verticale de pixels de face dans la colonne `x`, entre `y0` et
/// `y1`, en tolérant des trous (les glyphes) de `gap` pixels.
fn vertical_run(mask: &[bool], w: u32, x: u32, y0: u32, y1: u32, gap: u32) -> Option<(u32, u32)> {
    let mut best: Option<(u32, u32)> = None;
    let mut cur: Option<(u32, u32)> = None;
    let mut holes = 0;
    for y in y0..=y1 {
        if mask[(y * w + x) as usize] {
            cur = Some(cur.map_or((y, y), |(s, _)| (s, y)));
            holes = 0;
        } else if let Some(c) = cur {
            holes += 1;
            if holes > gap {
                if best.is_none_or(|b| c.1 - c.0 > b.1 - b.0) {
                    best = Some(c);
                }
                cur = None;
                holes = 0;
            }
        }
    }
    if let Some(c) = cur
        && best.is_none_or(|b| c.1 - c.0 > b.1 - b.0)
    {
        best = Some(c);
    }
    best
}

/// Les cartes face visible de l'image (les dos de cartes sont ignorés).
#[must_use]
pub fn detect_cards(frame: &Frame) -> Vec<DetectedCard> {
    let work = frame.downscale(WORK_WIDTH);
    let (w, h) = (work.width, work.height);
    let mask: Vec<bool> = work
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| is_face(*p))
        .collect();
    let scale_x = frame.width as f32 / w as f32;
    let scale_y = frame.height as f32 / h as f32;
    let min_h = 12.max(h / 30);
    let mut rects: Vec<Rect> = Vec::new();
    for b in blobs(&mask, w, h, false) {
        let (bw, bh) = (b.w(), b.h());
        if bh < min_h || bh > h * 3 / 4 || bw < 6 {
            continue;
        }
        let fill = b.count as f32 / (bw * bh) as f32;
        if fill < 0.4 {
            continue;
        }
        // Le centre d'une face est clair ; celui d'un dos (ou d'un cadre blanc) ne l'est pas.
        let (ix0, ix1) = (b.x0 + bw / 4, b.x1 - bw / 4);
        let (iy0, iy1) = (b.y0 + bh / 4, b.y1 - bh / 4);
        let mut inner = 0u32;
        let mut total = 0u32;
        for y in iy0..=iy1 {
            for x in ix0..=ix1 {
                total += 1;
                inner += u32::from(mask[(y * w + x) as usize]);
            }
        }
        if (inner as f32) < 0.15 * total.max(1) as f32 {
            continue;
        }
        let aspect = bw as f32 / bh as f32;
        if aspect < 0.5 {
            continue;
        }
        let cw = CARD_ASPECT * bh as f32;
        let n = if aspect <= 0.92 {
            1
        } else {
            ((bw as f32 / cw - 0.2).ceil() as u32).clamp(2, 6)
        };
        if n == 1 {
            rects.push(Rect {
                x: b.x0,
                y: b.y0,
                w: bw,
                h: bh,
            });
            continue;
        }
        // Cartes collées ou qui se chevauchent : réparties dans la boîte, chacune recadrée
        // verticalement sur sa propre colonne (la carte du dessous peut être décalée).
        let cw = (cw.min(bw as f32)) as u32;
        let step = (bw - cw) as f32 / (n - 1) as f32;
        for i in 0..n {
            let x = b.x0 + (step * i as f32).round() as u32;
            let probe = (x + cw / 6).min(b.x1);
            let (y0, y1) =
                vertical_run(&mask, w, probe, b.y0, b.y1, bh / 4).unwrap_or((b.y0, b.y1));
            if y1 - y0 + 1 < min_h {
                continue;
            }
            rects.push(Rect {
                x,
                y: y0,
                w: cw,
                h: y1 - y0 + 1,
            });
        }
    }
    rects
        .into_iter()
        .map(|r| {
            let full = Rect {
                x: (r.x as f32 * scale_x) as u32,
                y: (r.y as f32 * scale_y) as u32,
                w: ((r.w as f32 * scale_x) as u32).max(1),
                h: ((r.h as f32 * scale_y) as u32).max(1),
            };
            let patch = read_corner(frame, full);
            DetectedCard {
                rect: full,
                ink: patch.ink,
                patch,
            }
        })
        .collect()
}

/// Lit le coin haut gauche d'une carte : glyphe du rang, puis glyphe de la couleur juste
/// en dessous, chacun recadré sur son encre.
#[must_use]
pub fn read_corner(frame: &Frame, card: Rect) -> Patch {
    let cx0 = card.x.min(frame.width - 1);
    let cy0 = card.y.min(frame.height - 1);
    let cw = ((card.w as f32 * 0.5) as u32).clamp(1, frame.width - cx0);
    let ch = ((card.h as f32 * 0.55) as u32).clamp(1, frame.height - cy0);
    let ink: Vec<bool> = (0..ch)
        .flat_map(|y| (0..cw).map(move |x| (x, y)))
        .map(|(x, y)| is_ink(frame.pixel(cx0 + x, cy0 + y)))
        .collect();
    let min_count = 4.max(cw * ch / 500);
    let mut parts: Vec<Blob> = blobs(&ink, cw, ch, true)
        .into_iter()
        .filter(|b| !b.touches_edge && b.count >= min_count)
        .collect();
    parts.sort_by_key(|b| (b.y0, b.x0));
    let empty = Patch {
        rank: Vec::new(),
        rank_aspect: 0.0,
        rank_parts: 0,
        suit: Vec::new(),
        suit_aspect: 0.0,
        ink: InkColor::Unknown,
    };
    let Some(first) = parts.first().copied() else {
        return empty;
    };
    // Le rang : le premier glyphe et ceux qui sont sur la même ligne, juste à côté (« 10 »).
    let mut rank = first;
    let mut rank_parts = 1u8;
    let mut used = vec![false; parts.len()];
    used[0] = true;
    for (i, p) in parts.iter().enumerate().skip(1) {
        let overlap = (rank.y1.min(p.y1) as i64 - rank.y0.max(p.y0) as i64 + 1).max(0) as f32;
        let min_h = rank.h().min(p.h()) as f32;
        let gap = p.x0 as i64 - rank.x1 as i64;
        if overlap >= 0.6 * min_h
            && gap < (rank.h() as f32 * 0.5) as i64
            && gap > -(rank.w() as i64)
        {
            rank.x0 = rank.x0.min(p.x0);
            rank.x1 = rank.x1.max(p.x1);
            rank.y0 = rank.y0.min(p.y0);
            rank.y1 = rank.y1.max(p.y1);
            rank.count += p.count;
            rank_parts = rank_parts.saturating_add(1);
            used[i] = true;
        }
    }
    // La couleur : le glyphe suivant, sous le rang et à peu près aligné.
    let suit = parts.iter().enumerate().find(|(i, p)| {
        let pcx = (p.x0 + p.x1) as f32 / 2.0;
        !used[*i]
            && p.y0 + 1 >= rank.y1
            && pcx >= rank.x0 as f32 - rank.w() as f32
            && pcx <= rank.x1 as f32 + rank.w() as f32
            && p.y0 <= rank.y1 + rank.h() * 2
    });
    // Les morceaux du symbole de couleur (la tige fine d'un trèfle ou d'un pique se détache
    // parfois après l'anticrénelage) : on les réunit pour que la forme reste la même d'une
    // taille d'image à l'autre.
    let suit: Option<Blob> = suit.map(|(_, s)| {
        let mut m = *s;
        let pieces: Vec<Blob> = blobs(&ink, cw, ch, true)
            .into_iter()
            .filter(|b| !b.touches_edge)
            .collect();
        for _ in 0..2 {
            for p in &pieces {
                let pcx = (p.x0 + p.x1) as f32 / 2.0;
                let in_rank = p.y1 <= rank.y1 && p.x0 <= rank.x1 && p.x1 >= rank.x0;
                let near_x = pcx >= m.x0 as f32 - m.w() as f32 / 3.0
                    && pcx <= m.x1 as f32 + m.w() as f32 / 3.0;
                let near_y = p.y0 <= m.y1 + m.h() / 2 + 1 && p.y1 + 1 >= m.y0;
                let small = p.count <= m.count;
                if !in_rank && near_x && near_y && small {
                    m.x0 = m.x0.min(p.x0);
                    m.x1 = m.x1.max(p.x1);
                    m.y0 = m.y0.min(p.y0);
                    m.y1 = m.y1.max(p.y1);
                }
            }
        }
        m
    });
    let at = |x: i32, y: i32| -> bool {
        x >= 0
            && y >= 0
            && (x as u32) < cw
            && (y as u32) < ch
            && ink[(y as u32 * cw + x as u32) as usize]
    };
    let boxed = |b: &Blob| (b.x0 as f32, b.y0 as f32, b.w() as f32, b.h() as f32);
    let rank_features = coverage(at, boxed(&rank), RANK_W, RANK_H);
    let (suit_features, suit_aspect) = match &suit {
        Some(s) => (
            coverage(at, boxed(s), SUIT_W, SUIT_H),
            s.w() as f32 / s.h() as f32,
        ),
        None => (Vec::new(), 0.0),
    };
    // Couleur de l'encre : moyenne des pixels d'encre nets du rang et de la couleur.
    let mut acc = [0.0f64; 3];
    let mut n = 0u32;
    let mut add = |b: &Blob| {
        for y in b.y0..=b.y1 {
            for x in b.x0..=b.x1 {
                if ink[(y * cw + x) as usize] {
                    let p = frame.pixel(cx0 + x, cy0 + y);
                    acc[0] += f64::from(p[0]);
                    acc[1] += f64::from(p[1]);
                    acc[2] += f64::from(p[2]);
                    n += 1;
                }
            }
        }
    };
    add(&rank);
    if let Some(s) = &suit {
        add(s);
    }
    let n = f64::from(n.max(1));
    let color = InkColor::classify(
        (acc[0] / n) as f32,
        (acc[1] / n) as f32,
        (acc[2] / n) as f32,
    );
    Patch {
        rank: rank_features,
        rank_aspect: rank.w() as f32 / rank.h() as f32,
        rank_parts,
        suit: suit_features,
        suit_aspect,
        ink: color,
    }
}
