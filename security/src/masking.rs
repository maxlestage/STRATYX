//! Masquage de zones (droit à l'image) : webcam, chat et autres zones hors table sont
//! recouverts d'une couleur unie **avant** l'enregistrement ou l'analyse de l'image.
//!
//! Le remplissage uni est irréversible, contrairement au flou ou à la pixellisation qui
//! laissent parfois reconstituer un texte. Les zones peuvent être données en pixels
//! ([`MaskRect`]) ou en fractions de l'image ([`RelativeRect`]), ce qui permet d'enregistrer un
//! gabarit par salle indépendant de la résolution ([`MaskTemplate`]).
//!
//! Les images sont des tampons RGBA 8 bits bruts (4 octets par pixel), avec un pas de ligne
//! éventuellement plus grand que la largeur (alignement des API de capture).

use serde::{Deserialize, Serialize};

/// Octets par pixel RGBA.
const BYTES_PER_PIXEL: usize = 4;

/// Erreur de masquage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MaskError {
    /// Taille du tampon incohérente avec les dimensions annoncées.
    #[error(
        "tampon d'image de {actual} octets, {expected} attendus \
         ({width}×{height} pixels RGBA, pas de {stride} octets)"
    )]
    BufferSize {
        /// Largeur en pixels.
        width: u32,
        /// Hauteur en pixels.
        height: u32,
        /// Pas de ligne en octets.
        stride: usize,
        /// Taille minimale attendue.
        expected: usize,
        /// Taille reçue.
        actual: usize,
    },
    /// Pas de ligne inférieur à `largeur × 4`.
    #[error("pas de ligne de {stride} octets inférieur à la largeur ({width} pixels RGBA)")]
    StrideTooSmall {
        /// Largeur en pixels.
        width: u32,
        /// Pas de ligne en octets.
        stride: usize,
    },
    /// Dimensions trop grandes pour la mémoire adressable.
    #[error("dimensions d'image trop grandes")]
    TooLarge,
    /// Rectangle relatif hors de `[0, 1]` ou non fini.
    #[error("zone relative invalide : coordonnées hors de [0, 1]")]
    InvalidRelativeRect,
}

/// Couleur RGBA de remplissage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// Noir opaque, remplissage par défaut.
    pub const BLACK: Self = Self([0, 0, 0, 255]);
}

impl Default for Rgba {
    fn default() -> Self {
        Self::BLACK
    }
}

/// Rectangle en pixels ; la partie hors de l'image est ignorée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaskRect {
    /// Abscisse du coin supérieur gauche.
    pub x: u32,
    /// Ordonnée du coin supérieur gauche.
    pub y: u32,
    /// Largeur.
    pub width: u32,
    /// Hauteur.
    pub height: u32,
}

impl MaskRect {
    /// Intersection avec une image `width × height`, en bornes demi-ouvertes `(x0, y0, x1, y1)`.
    fn clip(&self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let x1 = self.x.saturating_add(self.width).min(width);
        let y1 = self.y.saturating_add(self.height).min(height);
        (self.x < x1 && self.y < y1).then_some((self.x, self.y, x1, y1))
    }
}

/// Rectangle en fractions de l'image (`0.0` à `1.0`), arrondi **vers l'extérieur** lors de
/// la conversion en pixels pour couvrir entièrement la zone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RelativeRect {
    /// Abscisse du coin supérieur gauche.
    pub x: f32,
    /// Ordonnée du coin supérieur gauche.
    pub y: f32,
    /// Largeur.
    pub width: f32,
    /// Hauteur.
    pub height: f32,
}

impl RelativeRect {
    /// Rectangle en pixels pour une image `width × height`.
    ///
    /// # Errors
    /// [`MaskError::InvalidRelativeRect`] si une coordonnée n'est pas finie, est négative, ou
    /// si la zone déborde de `[0, 1]`.
    pub fn to_pixels(&self, width: u32, height: u32) -> Result<MaskRect, MaskError> {
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if !(unit(self.x)
            && unit(self.y)
            && unit(self.width)
            && unit(self.height)
            && unit(self.x + self.width)
            && unit(self.y + self.height))
        {
            return Err(MaskError::InvalidRelativeRect);
        }
        let (w, h) = (f64::from(width), f64::from(height));
        let x0 = (f64::from(self.x) * w).floor();
        let y0 = (f64::from(self.y) * h).floor();
        let x1 = (f64::from(self.x + self.width) * w).ceil().min(w);
        let y1 = (f64::from(self.y + self.height) * h).ceil().min(h);
        // Valeurs entières dans [0, dimension] : conversions exactes.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = |v: f64| v as u32;
        Ok(MaskRect {
            x: px(x0),
            y: px(y0),
            width: px(x1 - x0),
            height: px(y1 - y0),
        })
    }
}

/// Nature d'une zone masquée (pour l'interface et l'audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    /// Image de webcam d'un joueur.
    Webcam,
    /// Fenêtre de discussion.
    Chat,
    /// Autre zone hors table (notifications, fenêtres tierces…).
    Other,
}

/// Position d'une zone, absolue ou relative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneArea {
    /// En pixels.
    Pixels(MaskRect),
    /// En fractions de l'image.
    Relative(RelativeRect),
}

/// Zone à masquer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaskZone {
    /// Nature de la zone.
    pub kind: ZoneKind,
    /// Position.
    pub area: ZoneArea,
}

/// Gabarit de masquage (par salle ou par disposition de table), sérialisable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MaskTemplate {
    /// Zones à masquer.
    pub zones: Vec<MaskZone>,
    /// Couleur de remplissage.
    #[serde(default)]
    pub fill: Rgba,
}

impl MaskTemplate {
    /// Rectangles en pixels pour une image `width × height`.
    ///
    /// # Errors
    /// [`MaskError::InvalidRelativeRect`].
    pub fn resolve(&self, width: u32, height: u32) -> Result<Vec<MaskRect>, MaskError> {
        self.zones
            .iter()
            .map(|zone| match zone.area {
                ZoneArea::Pixels(rect) => Ok(rect),
                ZoneArea::Relative(rel) => rel.to_pixels(width, height),
            })
            .collect()
    }

    /// Applique le gabarit à une image ; renvoie le nombre de pixels recouverts.
    ///
    /// # Errors
    /// Voir [`RgbaFrame::new`] et [`MaskTemplate::resolve`].
    pub fn apply(&self, frame: &mut RgbaFrame<'_>) -> Result<u64, MaskError> {
        let rects = self.resolve(frame.width, frame.height)?;
        Ok(frame.fill_rects(&rects, self.fill))
    }
}

/// Image RGBA mutable, dimensions vérifiées.
#[derive(Debug)]
pub struct RgbaFrame<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
}

impl<'a> RgbaFrame<'a> {
    /// Image compacte : pas de ligne de `width × 4` octets.
    ///
    /// # Errors
    /// [`MaskError::BufferSize`] si `data` n'a pas exactement la taille attendue.
    pub fn new(data: &'a mut [u8], width: u32, height: u32) -> Result<Self, MaskError> {
        let stride = row_bytes(width)?;
        let expected = stride
            .checked_mul(usize::try_from(height).map_err(|_| MaskError::TooLarge)?)
            .ok_or(MaskError::TooLarge)?;
        if data.len() != expected {
            return Err(MaskError::BufferSize {
                width,
                height,
                stride,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Image avec un pas de ligne de `stride` octets (≥ `width × 4`) ; la dernière ligne peut
    /// ne pas avoir de remplissage final.
    ///
    /// # Errors
    /// [`MaskError::StrideTooSmall`], [`MaskError::BufferSize`].
    pub fn with_stride(
        data: &'a mut [u8],
        width: u32,
        height: u32,
        stride: usize,
    ) -> Result<Self, MaskError> {
        let row = row_bytes(width)?;
        if stride < row {
            return Err(MaskError::StrideTooSmall { width, stride });
        }
        let rows = usize::try_from(height).map_err(|_| MaskError::TooLarge)?;
        let expected = match rows {
            0 => 0,
            n => stride
                .checked_mul(n - 1)
                .and_then(|v| v.checked_add(row))
                .ok_or(MaskError::TooLarge)?,
        };
        if data.len() < expected {
            return Err(MaskError::BufferSize {
                width,
                height,
                stride,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Largeur en pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Hauteur en pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Recouvre les rectangles (rognés à l'image) de `fill` ; renvoie le nombre de pixels
    /// écrits (un pixel couvert par deux rectangles compte deux fois).
    pub fn fill_rects(&mut self, rects: &[MaskRect], fill: Rgba) -> u64 {
        let mut written = 0u64;
        for rect in rects {
            let Some((x0, y0, x1, y1)) = rect.clip(self.width, self.height) else {
                continue;
            };
            written += u64::from(x1 - x0) * u64::from(y1 - y0);
            let (start, end) = (x0 as usize * BYTES_PER_PIXEL, x1 as usize * BYTES_PER_PIXEL);
            for y in y0..y1 {
                let row = y as usize * self.stride;
                self.data[row + start..row + end]
                    .as_chunks_mut::<BYTES_PER_PIXEL>()
                    .0
                    .iter_mut()
                    .for_each(|px| px.copy_from_slice(&fill.0));
            }
        }
        written
    }
}

fn row_bytes(width: u32) -> Result<usize, MaskError> {
    usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(BYTES_PER_PIXEL))
        .ok_or(MaskError::TooLarge)
}

/// Recouvre `rects` de `fill` dans une image RGBA compacte `width × height` ; renvoie le
/// nombre de pixels recouverts.
///
/// # Errors
/// [`MaskError::BufferSize`] si la taille du tampon ne correspond pas aux dimensions.
pub fn apply_masks(
    data: &mut [u8],
    width: u32,
    height: u32,
    rects: &[MaskRect],
    fill: Rgba,
) -> Result<u64, MaskError> {
    Ok(RgbaFrame::new(data, width, height)?.fill_rects(rects, fill))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = Rgba([255, 0, 0, 255]);

    fn image(width: u32, height: u32) -> Vec<u8> {
        (0..width * height * 4)
            .map(|i| (i % 200) as u8 + 1)
            .collect()
    }

    fn pixel(data: &[u8], stride: usize, x: usize, y: usize) -> [u8; 4] {
        let at = y * stride + x * 4;
        data[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn fills_exactly_the_rectangle() {
        let (w, h) = (8, 6);
        let original = image(w, h);
        let mut data = original.clone();
        let rect = MaskRect {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        assert_eq!(apply_masks(&mut data, w, h, &[rect], RED).unwrap(), 6);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let inside = (2..5).contains(&x) && (1..3).contains(&y);
                let expected = if inside {
                    RED.0
                } else {
                    pixel(&original, w as usize * 4, x, y)
                };
                assert_eq!(pixel(&data, w as usize * 4, x, y), expected, "({x}, {y})");
            }
        }
    }

    #[test]
    fn clips_overlaps_and_ignores_outside() {
        let (w, h) = (4, 4);
        let mut data = image(w, h);
        let rects = [
            MaskRect {
                x: 2,
                y: 2,
                width: 100,
                height: 100,
            }, // rogné : 2×2
            MaskRect {
                x: 3,
                y: 3,
                width: 1,
                height: 1,
            }, // chevauchement : recompté
            MaskRect {
                x: 10,
                y: 0,
                width: 5,
                height: 5,
            }, // hors image
            MaskRect {
                x: 0,
                y: 0,
                width: 0,
                height: 4,
            }, // vide
            MaskRect {
                x: u32::MAX,
                y: u32::MAX,
                width: u32::MAX,
                height: 1,
            },
        ];
        assert_eq!(
            apply_masks(&mut data, w, h, &rects, Rgba::BLACK).unwrap(),
            4 + 1
        );
        assert_eq!(pixel(&data, 16, 3, 3), Rgba::BLACK.0);
        assert_ne!(pixel(&data, 16, 1, 1), Rgba::BLACK.0);
    }

    #[test]
    fn respects_stride_padding() {
        let (w, h, stride) = (3u32, 2u32, 16usize); // 12 octets utiles + 4 de remplissage
        let mut data = vec![7u8; stride + 12]; // dernière ligne sans remplissage
        let mut frame = RgbaFrame::with_stride(&mut data, w, h, stride).unwrap();
        let full = MaskRect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        assert_eq!(frame.fill_rects(&[full], RED), 6);
        assert_eq!(&data[12..16], &[7, 7, 7, 7], "remplissage de ligne intact");
        assert_eq!(pixel(&data, stride, 2, 1), RED.0);

        assert!(matches!(
            RgbaFrame::with_stride(&mut [0u8; 64], 4, 2, 15),
            Err(MaskError::StrideTooSmall { .. })
        ));
        assert!(matches!(
            RgbaFrame::with_stride(&mut [0u8; 27], 3, 2, 16),
            Err(MaskError::BufferSize { .. })
        ));
    }

    #[test]
    fn rejects_inconsistent_buffers() {
        let mut data = vec![0u8; 15];
        let err = apply_masks(&mut data, 2, 2, &[], RED).unwrap_err();
        assert!(matches!(
            err,
            MaskError::BufferSize {
                expected: 16,
                actual: 15,
                ..
            }
        ));
        assert!(err.to_string().contains("16"));
        assert!(apply_masks(&mut [], 0, 0, &[], RED).is_ok());
    }

    #[test]
    fn relative_template_rounds_outward() {
        // Webcam en haut à droite (dernier quart), chat en bas sur toute la largeur.
        let template = MaskTemplate {
            zones: vec![
                MaskZone {
                    kind: ZoneKind::Webcam,
                    area: ZoneArea::Relative(RelativeRect {
                        x: 0.75,
                        y: 0.0,
                        width: 0.25,
                        height: 0.25,
                    }),
                },
                MaskZone {
                    kind: ZoneKind::Chat,
                    area: ZoneArea::Pixels(MaskRect {
                        x: 0,
                        y: 9,
                        width: 10,
                        height: 1,
                    }),
                },
            ],
            fill: RED,
        };
        let rects = template.resolve(10, 10).unwrap();
        assert_eq!(
            rects[0],
            MaskRect {
                x: 7,
                y: 0,
                width: 3,
                height: 3
            }
        );

        let mut data = image(10, 10);
        let mut frame = RgbaFrame::new(&mut data, 10, 10).unwrap();
        assert_eq!(template.apply(&mut frame).unwrap(), 9 + 10);
        assert_eq!(pixel(&data, 40, 9, 0), RED.0);
        assert_eq!(pixel(&data, 40, 0, 9), RED.0);

        let json = serde_json::to_string(&template).unwrap();
        assert_eq!(
            serde_json::from_str::<MaskTemplate>(&json).unwrap(),
            template
        );

        for bad in [
            RelativeRect {
                x: -0.1,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            RelativeRect {
                x: 0.8,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            RelativeRect {
                x: f32::NAN,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
        ] {
            assert_eq!(bad.to_pixels(10, 10), Err(MaskError::InvalidRelativeRect));
        }
    }
}
