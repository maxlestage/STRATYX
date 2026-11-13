//! Le tri des images d'une vidéo : ne garder que celles où la table a changé.

use crate::frame::{Frame, luma};

const THUMB_W: usize = 48;
const THUMB_H: usize = 27;

/// Garde les images d'un enregistrement quand quelque chose a changé (une carte, une
/// mise…), d'après une vignette en niveaux de gris.
#[derive(Clone, Debug, Default)]
pub struct Keyframer {
    last: Option<Vec<f32>>,
    /// Écart (niveaux de gris, 0 à 255) d'une case de la vignette pour compter comme un
    /// changement.
    pub threshold: f32,
}

fn thumbnail(frame: &Frame) -> Vec<f32> {
    let mut out = vec![0.0f32; THUMB_W * THUMB_H];
    let (w, h) = (frame.width as usize, frame.height as usize);
    for ty in 0..THUMB_H {
        let y0 = ty * h / THUMB_H;
        let y1 = ((ty + 1) * h / THUMB_H).max(y0 + 1);
        for tx in 0..THUMB_W {
            let x0 = tx * w / THUMB_W;
            let x1 = ((tx + 1) * w / THUMB_W).max(x0 + 1);
            let mut acc = 0u64;
            let mut n = 0u64;
            // Un pixel sur deux suffit pour une moyenne.
            for y in (y0..y1).step_by(2) {
                for x in (x0..x1).step_by(2) {
                    acc += u64::from(luma(frame.pixel(x as u32, y as u32)));
                    n += 1;
                }
            }
            out[ty * THUMB_W + tx] = acc as f32 / n.max(1) as f32;
        }
    }
    out
}

impl Keyframer {
    #[must_use]
    pub fn new() -> Self {
        Keyframer {
            last: None,
            threshold: 12.0,
        }
    }

    /// Vrai si l'image diffère assez de la dernière gardée (elle devient alors la
    /// référence).
    pub fn push(&mut self, frame: &Frame) -> bool {
        let thumb = thumbnail(frame);
        let changed = match &self.last {
            None => true,
            Some(prev) => prev
                .iter()
                .zip(&thumb)
                .any(|(a, b)| (a - b).abs() > self.threshold),
        };
        if changed {
            self.last = Some(thumb);
        }
        changed
    }
}
