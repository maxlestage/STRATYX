//! Images RGBA et rectangles.

use serde::{Deserialize, Serialize};

/// Un rectangle en pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Centre du rectangle.
    #[must_use]
    pub fn center(&self) -> (f32, f32) {
        (
            self.x as f32 + self.w as f32 / 2.0,
            self.y as f32 + self.h as f32 / 2.0,
        )
    }

    /// Surface d'intersection avec `o`.
    #[must_use]
    pub fn intersection(&self, o: &Rect) -> u64 {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        if x1 <= x0 || y1 <= y0 {
            0
        } else {
            u64::from(x1 - x0) * u64::from(y1 - y0)
        }
    }

    /// Le point `(px, py)` est-il dans le rectangle ?
    #[must_use]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x as f32
            && py >= self.y as f32
            && px < (self.x + self.w) as f32
            && py < (self.y + self.h) as f32
    }
}

/// Une image RGBA (8 bits par canal), ligne par ligne.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// `None` si la taille du tampon ne correspond pas aux dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Frame> {
        (width > 0 && height > 0 && rgba.len() == width as usize * height as usize * 4).then_some(
            Frame {
                width,
                height,
                rgba,
            },
        )
    }

    /// Le pixel `(x, y)` : `[r, g, b, a]`.
    #[inline]
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Réduit l'image à `max_width` pixels de large au plus (moyenne des pixels couverts),
    /// en gardant ses proportions.
    #[must_use]
    pub fn downscale(&self, max_width: u32) -> Frame {
        if self.width <= max_width || max_width == 0 {
            return self.clone();
        }
        let nw = max_width;
        let nh = ((self.height as u64 * nw as u64) / self.width as u64).max(1) as u32;
        let mut out = vec![0u8; nw as usize * nh as usize * 4];
        let sx = self.width as f32 / nw as f32;
        let sy = self.height as f32 / nh as f32;
        // Les boîtes de pixels couvertes par chaque colonne de sortie, calculées une fois
        // (mêmes bornes qu'un calcul pixel par pixel, donc même résultat au bit près) : la
        // colonne de sortie de chaque colonne source, et la largeur de chaque boîte.
        let mut column_of = vec![u32::MAX; self.width as usize];
        let mut widths = vec![0u32; nw as usize];
        for x in 0..nw {
            let x0 = (x as f32 * sx) as u32;
            let x1 = (((x + 1) as f32 * sx) as u32).clamp(x0 + 1, self.width);
            column_of[x0 as usize..x1 as usize].fill(x);
            widths[x as usize] = x1 - x0;
        }
        // Les boîtes se suivent sans se chevaucher (réduction : `sx > 1`) ; sinon, retour au
        // calcul pixel par pixel.
        let partition = (0..nw).all(|x| {
            let x0 = (x as f32 * sx) as u32 as usize;
            column_of[x0] == x
        });
        if !partition {
            return self.downscale_per_pixel(nw, nh, sx, sy);
        }
        let stride = self.width as usize * 4;
        // Sommes d'une ligne de sortie, canal par canal : une seule passe linéaire par ligne
        // source (la réduction pesait 80 % de `detect_cards` en 4K).
        let mut acc = vec![0u32; nw as usize * 4];
        for (y, out_row) in out.chunks_exact_mut(nw as usize * 4).enumerate() {
            let y0 = (y as f32 * sy) as u32;
            let y1 = (((y + 1) as f32 * sy) as u32).clamp(y0 + 1, self.height);
            acc.fill(0);
            for yy in y0..y1 {
                let row = &self.rgba[yy as usize * stride..(yy as usize + 1) * stride];
                for (px, &col) in row.chunks_exact(4).zip(&column_of) {
                    if col == u32::MAX {
                        continue;
                    }
                    let sum = &mut acc[col as usize * 4..col as usize * 4 + 4];
                    sum[0] += u32::from(px[0]);
                    sum[1] += u32::from(px[1]);
                    sum[2] += u32::from(px[2]);
                    sum[3] += u32::from(px[3]);
                }
            }
            let rows = y1 - y0;
            for ((o, sum), w) in out_row
                .chunks_exact_mut(4)
                .zip(acc.chunks_exact(4))
                .zip(&widths)
            {
                let n = rows * w;
                for c in 0..4 {
                    o[c] = (sum[c] / n) as u8;
                }
            }
        }
        Frame {
            width: nw,
            height: nh,
            rgba: out,
        }
    }

    /// La réduction pixel par pixel, pour le cas (théorique) de boîtes qui se chevauchent.
    pub(crate) fn downscale_per_pixel(&self, nw: u32, nh: u32, sx: f32, sy: f32) -> Frame {
        let mut out = vec![0u8; nw as usize * nh as usize * 4];
        for y in 0..nh {
            let y0 = (y as f32 * sy) as u32;
            let y1 = (((y + 1) as f32 * sy) as u32).clamp(y0 + 1, self.height);
            for x in 0..nw {
                let x0 = (x as f32 * sx) as u32;
                let x1 = (((x + 1) as f32 * sx) as u32).clamp(x0 + 1, self.width);
                let mut acc = [0u32; 4];
                for yy in y0..y1 {
                    let row = yy as usize * self.width as usize;
                    for xx in x0..x1 {
                        let i = (row + xx as usize) * 4;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += u32::from(self.rgba[i + c]);
                        }
                    }
                }
                let n = (y1 - y0) * (x1 - x0);
                let o = (y as usize * nw as usize + x as usize) * 4;
                for c in 0..4 {
                    out[o + c] = (acc[c] / n) as u8;
                }
            }
        }
        Frame {
            width: nw,
            height: nh,
            rgba: out,
        }
    }

    /// Une partie de l'image (bornée à l'image).
    #[must_use]
    pub fn crop(&self, r: Rect) -> Frame {
        let x0 = r.x.min(self.width - 1);
        let y0 = r.y.min(self.height - 1);
        let w = r.w.min(self.width - x0).max(1);
        let h = r.h.min(self.height - y0).max(1);
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for y in y0..y0 + h {
            let i = (y as usize * self.width as usize + x0 as usize) * 4;
            out.extend_from_slice(&self.rgba[i..i + w as usize * 4]);
        }
        Frame {
            width: w,
            height: h,
            rgba: out,
        }
    }
}

/// Luminance (0 à 255).
#[inline]
pub(crate) fn luma(p: [u8; 4]) -> u32 {
    (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000
}

/// Saturation grossière : écart entre le canal le plus fort et le plus faible.
#[inline]
pub(crate) fn spread(p: [u8; 4]) -> u32 {
    let mx = p[0].max(p[1]).max(p[2]);
    let mn = p[0].min(p[1]).min(p[2]);
    u32::from(mx - mn)
}

/// Un pixel de la face d'une carte : clair et peu coloré (blanc, crème, gris très clair).
#[inline]
pub(crate) fn is_face(p: [u8; 4]) -> bool {
    luma(p) >= 178 && spread(p) <= 60
}

/// Un pixel d'encre sur une carte : sombre, ou nettement coloré.
#[inline]
pub(crate) fn is_ink(p: [u8; 4]) -> bool {
    let l = luma(p);
    l < 140 || (spread(p) > 75 && l < 215)
}
