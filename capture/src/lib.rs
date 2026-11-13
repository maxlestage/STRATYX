//! Capture d'écran multiplateforme via les API natives des systèmes (décision D1).
//!
//! | Système | Moteur | État |
//! |---|---|---|
//! | Linux (X11, XWayland) | protocole X11 en Rust pur (`x11rb`) | disponible, testé sur Xvfb |
//! | Windows | GDI (`PrintWindow`, `BitBlt`) via la crate `windows` | disponible, vérifié à la compilation |
//! | macOS | ScreenCaptureKit | à venir (nécessite une machine macOS pour être validé) |
//! | Wayland natif | portail XDG + PipeWire | à venir (exception D1 à confirmer) |
//!
//! La capture ne produit que des images : aucune analyse stratégique n'est disponible
//! pendant une session de capture ouverte (décision D6, verrou du crate `security`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(windows)]
mod windows;
#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
mod x11;

#[cfg(windows)]
pub use windows::GdiCapturer;
#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
pub use x11::X11Capturer;

/// Image capturée, en RGBA 8 bits par canal, lignes de haut en bas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` octets.
    pub rgba: Vec<u8>,
    /// Instant de la capture, en millisecondes depuis l'époque Unix.
    pub timestamp_ms: u64,
}

impl CapturedFrame {
    /// Pixel `(x, y)` au format `[r, g, b, a]`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }
}

/// Rectangle en pixels, relatif à la cible capturée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Ce que l'on peut capturer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CaptureTarget {
    /// Un écran entier (index dans la liste des écrans).
    Screen { index: usize },
    /// Une fenêtre (identifiant natif du système).
    Window { id: u64 },
}

/// Description d'une cible disponible, pour le choix de l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInfo {
    pub target: CaptureTarget,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("capture non prise en charge sur ce système : {0}")]
    Unsupported(String),
    #[error("connexion au système d'affichage impossible : {0}")]
    Connection(String),
    #[error("cible introuvable : {0:?}")]
    TargetNotFound(CaptureTarget),
    #[error("zone hors de la cible : {0:?}")]
    InvalidRegion(Rect),
    #[error("format de pixels non géré : {0}")]
    PixelFormat(String),
    #[error("erreur du système : {0}")]
    Backend(String),
}

/// Moteur de capture d'un système.
pub trait ScreenCapturer: Send {
    /// Écrans et fenêtres visibles.
    fn list_targets(&mut self) -> Result<Vec<TargetInfo>, CaptureError>;

    /// Capture une cible entière ou une zone de celle-ci.
    fn grab(
        &mut self,
        target: &CaptureTarget,
        region: Option<Rect>,
    ) -> Result<CapturedFrame, CaptureError>;
}

/// Moteur de capture du système courant.
pub fn default_capturer() -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    {
        Ok(Box::new(X11Capturer::connect()?))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(GdiCapturer::new()))
    }
    #[cfg(not(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    )))]
    {
        Err(CaptureError::Unsupported(
            "macOS (ScreenCaptureKit) et mobile : à venir".into(),
        ))
    }
}

/// Horodatage courant en millisecondes depuis l'époque Unix.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Convertit des pixels BGRX/BGRA 32 bits (ordre des octets des API X11 et Windows) en
/// RGBA opaque. `stride` est la longueur d'une ligne source en octets.
#[must_use]
pub fn bgrx_to_rgba(src: &[u8], width: u32, height: u32, stride: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        let row = &src[y * stride..y * stride + width as usize * 4];
        for px in row.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    out
}

/// Intersecte une zone demandée avec une cible de taille `width × height`.
pub(crate) fn clip_region(
    region: Option<Rect>,
    width: u32,
    height: u32,
) -> Result<Rect, CaptureError> {
    let full = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let Some(r) = region else { return Ok(full) };
    let inside = r.x >= 0
        && r.y >= 0
        && r.width > 0
        && r.height > 0
        && r.x as u64 + u64::from(r.width) <= u64::from(width)
        && r.y as u64 + u64::from(r.height) <= u64::from(height);
    if inside {
        Ok(r)
    } else {
        Err(CaptureError::InvalidRegion(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_conversion() {
        // 2×1 pixels, ligne source avec 4 octets de remplissage.
        let src = [10, 20, 30, 0, 40, 50, 60, 0, 9, 9, 9, 9];
        let rgba = bgrx_to_rgba(&src, 2, 1, 12);
        assert_eq!(rgba, vec![30, 20, 10, 255, 60, 50, 40, 255]);
        let f = CapturedFrame {
            width: 2,
            height: 1,
            rgba,
            timestamp_ms: 0,
        };
        assert_eq!(f.pixel(1, 0), Some([60, 50, 40, 255]));
        assert_eq!(f.pixel(2, 0), None);
    }

    #[test]
    fn region_clipping() {
        assert_eq!(
            clip_region(None, 10, 5).unwrap(),
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 5
            }
        );
        let r = Rect {
            x: 2,
            y: 1,
            width: 8,
            height: 4,
        };
        assert_eq!(clip_region(Some(r), 10, 5).unwrap(), r);
        assert!(
            clip_region(
                Some(Rect {
                    x: 3,
                    y: 0,
                    width: 8,
                    height: 1
                }),
                10,
                5
            )
            .is_err()
        );
        assert!(
            clip_region(
                Some(Rect {
                    x: -1,
                    y: 0,
                    width: 1,
                    height: 1
                }),
                10,
                5
            )
            .is_err()
        );
    }
}
