//! Capture Windows via GDI : `PrintWindow` (rendu complet, y compris les fenêtres
//! accélérées) pour une fenêtre, `BitBlt` pour l'écran. Appels système encapsulés ici,
//! seul endroit du projet où `unsafe` est autorisé (décision T1).
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetSystemMetrics, GetWindowTextW, IsWindowVisible, SM_CXSCREEN,
    SM_CYSCREEN,
};
use windows::core::BOOL;

use crate::{
    CaptureError, CaptureTarget, CapturedFrame, Rect, ScreenCapturer, TargetInfo, bgrx_to_rgba,
    clip_region, now_ms,
};

/// `PW_RENDERFULLCONTENT` : capture aussi le contenu rendu par DirectX/DWM.
const PW_RENDERFULLCONTENT: u32 = 2;

/// Moteur de capture GDI.
#[derive(Debug, Default)]
pub struct GdiCapturer;

impl GdiCapturer {
    #[must_use]
    pub fn new() -> GdiCapturer {
        GdiCapturer
    }
}

fn backend(what: &str) -> CaptureError {
    CaptureError::Backend(format!("échec de {what}"))
}

/// Contexte mémoire et bitmap libérés automatiquement.
struct MemSurface {
    screen_dc: HDC,
    mem_dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}

impl MemSurface {
    fn new(width: i32, height: i32) -> Result<MemSurface, CaptureError> {
        // SAFETY : appels GDI documentés ; chaque ressource est libérée dans `Drop`.
        unsafe {
            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                return Err(backend("GetDC"));
            }
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
            if mem_dc.is_invalid() || bitmap.is_invalid() {
                ReleaseDC(None, screen_dc);
                return Err(backend("CreateCompatibleBitmap"));
            }
            let previous = SelectObject(mem_dc, bitmap.into());
            Ok(MemSurface {
                screen_dc,
                mem_dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }

    /// Lit les pixels en BGRA 32 bits, lignes de haut en bas.
    fn read(&self) -> Result<Vec<u8>, CaptureError> {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.width,
                biHeight: -self.height, // négatif : lignes de haut en bas
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (self.width * self.height * 4) as usize];
        // SAFETY : `buf` a exactement la taille décrite par `info`.
        let lines = unsafe {
            GetDIBits(
                self.mem_dc,
                self.bitmap,
                0,
                self.height as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        if lines != self.height {
            return Err(backend("GetDIBits"));
        }
        Ok(buf)
    }
}

impl Drop for MemSurface {
    fn drop(&mut self) {
        // SAFETY : ressources créées dans `new`, libérées une seule fois.
        unsafe {
            SelectObject(self.mem_dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.mem_dc);
            ReleaseDC(None, self.screen_dc);
        }
    }
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    // SAFETY : tampon local de taille connue.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn client_size(hwnd: HWND) -> Result<(u32, u32), CaptureError> {
    let mut rect = RECT::default();
    // SAFETY : `rect` est une sortie locale.
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(|_| backend("GetClientRect"))?;
    Ok((
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    ))
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY : `lparam` pointe vers le `Vec<HWND>` passé par `list_targets`.
    unsafe {
        if IsWindowVisible(hwnd).as_bool() {
            let list = &mut *(lparam.0 as *mut Vec<HWND>);
            list.push(hwnd);
        }
    }
    BOOL(1)
}

impl ScreenCapturer for GdiCapturer {
    fn list_targets(&mut self) -> Result<Vec<TargetInfo>, CaptureError> {
        // SAFETY : métriques système sans effet de bord.
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let mut out = vec![TargetInfo {
            target: CaptureTarget::Screen { index: 0 },
            title: "Écran principal".into(),
            width: w.max(0) as u32,
            height: h.max(0) as u32,
        }];
        let mut windows: Vec<HWND> = Vec::new();
        // SAFETY : le rappel ne vit que le temps de l'appel et reçoit un pointeur valide.
        unsafe {
            EnumWindows(
                Some(collect_window),
                LPARAM(&mut windows as *mut _ as isize),
            )
        }
        .map_err(|_| backend("EnumWindows"))?;
        for hwnd in windows {
            let title = window_title(hwnd);
            let (width, height) = client_size(hwnd)?;
            if title.is_empty() || width == 0 || height == 0 {
                continue;
            }
            out.push(TargetInfo {
                target: CaptureTarget::Window {
                    id: hwnd.0 as usize as u64,
                },
                title,
                width,
                height,
            });
        }
        Ok(out)
    }

    fn grab(
        &mut self,
        target: &CaptureTarget,
        region: Option<Rect>,
    ) -> Result<CapturedFrame, CaptureError> {
        let timestamp_ms = now_ms();
        let (surface, full_w, full_h) = match *target {
            CaptureTarget::Screen { index: 0 } => {
                // SAFETY : métriques système sans effet de bord.
                let (w, h) =
                    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
                let s = MemSurface::new(w, h)?;
                // SAFETY : copie de l'écran vers la surface mémoire de même taille.
                unsafe { BitBlt(s.mem_dc, 0, 0, w, h, Some(s.screen_dc), 0, 0, SRCCOPY) }
                    .map_err(|_| backend("BitBlt"))?;
                (s, w as u32, h as u32)
            }
            CaptureTarget::Screen { .. } => {
                return Err(CaptureError::TargetNotFound(target.clone()));
            }
            CaptureTarget::Window { id } => {
                let hwnd = HWND(id as usize as *mut core::ffi::c_void);
                let (w, h) = client_size(hwnd)?;
                if w == 0 || h == 0 {
                    return Err(CaptureError::TargetNotFound(target.clone()));
                }
                let s = MemSurface::new(w as i32, h as i32)?;
                // SAFETY : rendu de la fenêtre dans la surface mémoire.
                let ok = unsafe {
                    PrintWindow(hwnd, s.mem_dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT | 1))
                };
                if !ok.as_bool() {
                    return Err(backend("PrintWindow"));
                }
                (s, w, h)
            }
        };
        let r = clip_region(region, full_w, full_h)?;
        let bgra = surface.read()?;
        let stride = full_w as usize * 4;
        let offset = r.y as usize * stride + r.x as usize * 4;
        let rgba = bgrx_to_rgba(&bgra[offset..], r.width, r.height, stride);
        Ok(CapturedFrame {
            width: r.width,
            height: r.height,
            rgba,
            timestamp_ms,
        })
    }
}
