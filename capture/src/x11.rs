//! Capture X11 en Rust pur : protocole X parlé directement sur la socket du serveur
//! (`x11rb`), sans Xlib ni bibliothèque C. Fonctionne aussi sous XWayland.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, ImageOrder, Window};
use x11rb::rust_connection::RustConnection;

use crate::{
    CaptureError, CaptureTarget, CapturedFrame, Rect, ScreenCapturer, TargetInfo, bgrx_to_rgba,
    clip_region, now_ms,
};

/// Moteur de capture X11.
#[derive(Debug)]
pub struct X11Capturer {
    conn: RustConnection,
}

fn backend<E: std::fmt::Display>(e: E) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

impl X11Capturer {
    /// Se connecte au serveur désigné par la variable `DISPLAY`.
    pub fn connect() -> Result<X11Capturer, CaptureError> {
        let (conn, _) =
            x11rb::connect(None).map_err(|e| CaptureError::Connection(e.to_string()))?;
        Ok(X11Capturer { conn })
    }

    fn root(&self, index: usize) -> Result<Window, CaptureError> {
        self.conn
            .setup()
            .roots
            .get(index)
            .map(|s| s.root)
            .ok_or(CaptureError::TargetNotFound(CaptureTarget::Screen {
                index,
            }))
    }

    fn atom(&self, name: &str) -> Result<u32, CaptureError> {
        Ok(self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(backend)?
            .reply()
            .map_err(backend)?
            .atom)
    }

    fn window_title(&self, w: Window) -> Result<String, CaptureError> {
        let net_name = self.atom("_NET_WM_NAME")?;
        let utf8 = self.atom("UTF8_STRING")?;
        let reply = self
            .conn
            .get_property(false, w, net_name, utf8, 0, 1024)
            .map_err(backend)?
            .reply();
        if let Ok(r) = reply
            && !r.value.is_empty()
        {
            return Ok(String::from_utf8_lossy(&r.value).into_owned());
        }
        let r = self
            .conn
            .get_property(false, w, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        Ok(String::from_utf8_lossy(&r.value).into_owned())
    }

    /// Fenêtres gérées par le gestionnaire de fenêtres (`_NET_CLIENT_LIST`), à défaut les
    /// enfants directs de la racine.
    fn client_windows(&self, root: Window) -> Result<Vec<Window>, CaptureError> {
        let list = self.atom("_NET_CLIENT_LIST")?;
        let reply = self
            .conn
            .get_property(false, root, list, AtomEnum::WINDOW, 0, 4096)
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        if let Some(values) = reply.value32() {
            let v: Vec<Window> = values.collect();
            if !v.is_empty() {
                return Ok(v);
            }
        }
        Ok(self
            .conn
            .query_tree(root)
            .map_err(backend)?
            .reply()
            .map_err(backend)?
            .children)
    }

    /// Lit une zone de la racine (coordonnées absolues) et la convertit en RGBA.
    fn read_root(&self, root: Window, r: Rect) -> Result<Vec<u8>, CaptureError> {
        let setup = self.conn.setup();
        if setup.image_byte_order != ImageOrder::LSB_FIRST {
            return Err(CaptureError::PixelFormat("ordre des octets MSB".into()));
        }
        let to_i16 = |v: i32| i16::try_from(v).map_err(|_| CaptureError::InvalidRegion(r));
        let to_u16 = |v: u32| u16::try_from(v).map_err(|_| CaptureError::InvalidRegion(r));
        let reply = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                root,
                to_i16(r.x)?,
                to_i16(r.y)?,
                to_u16(r.width)?,
                to_u16(r.height)?,
                !0,
            )
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == reply.depth)
            .map(|f| f.bits_per_pixel)
            .unwrap_or(0);
        if bpp != 32 || reply.depth < 24 {
            return Err(CaptureError::PixelFormat(format!(
                "profondeur {} sur {bpp} bits (attendu : 24 ou 32 sur 32 bits)",
                reply.depth
            )));
        }
        let stride = reply.data.len() / r.height as usize;
        bgrx_to_rgba(&reply.data, r.width, r.height, stride)
    }
}

impl ScreenCapturer for X11Capturer {
    fn list_targets(&mut self) -> Result<Vec<TargetInfo>, CaptureError> {
        let mut out = Vec::new();
        let roots: Vec<(Window, u16, u16)> = self
            .conn
            .setup()
            .roots
            .iter()
            .map(|s| (s.root, s.width_in_pixels, s.height_in_pixels))
            .collect();
        for (index, (root, w, h)) in roots.iter().enumerate() {
            out.push(TargetInfo {
                target: CaptureTarget::Screen { index },
                title: format!("Écran {}", index + 1),
                width: u32::from(*w),
                height: u32::from(*h),
            });
            for win in self.client_windows(*root)? {
                let attrs = self
                    .conn
                    .get_window_attributes(win)
                    .map_err(backend)?
                    .reply();
                if !matches!(attrs, Ok(a) if a.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
                {
                    continue;
                }
                let geo = self
                    .conn
                    .get_geometry(win)
                    .map_err(backend)?
                    .reply()
                    .map_err(backend)?;
                out.push(TargetInfo {
                    target: CaptureTarget::Window { id: u64::from(win) },
                    title: self.window_title(win)?,
                    width: u32::from(geo.width),
                    height: u32::from(geo.height),
                });
            }
        }
        Ok(out)
    }

    fn grab(
        &mut self,
        target: &CaptureTarget,
        region: Option<Rect>,
    ) -> Result<CapturedFrame, CaptureError> {
        let timestamp_ms = now_ms();
        match *target {
            CaptureTarget::Screen { index } => {
                let root = self.root(index)?;
                let screen = &self.conn.setup().roots[index];
                let r = clip_region(
                    region,
                    u32::from(screen.width_in_pixels),
                    u32::from(screen.height_in_pixels),
                )?;
                let rgba = self.read_root(root, r)?;
                Ok(CapturedFrame {
                    width: r.width,
                    height: r.height,
                    rgba,
                    timestamp_ms,
                })
            }
            CaptureTarget::Window { id } => {
                let win = Window::try_from(id)
                    .map_err(|_| CaptureError::TargetNotFound(target.clone()))?;
                let geo = self
                    .conn
                    .get_geometry(win)
                    .map_err(backend)?
                    .reply()
                    .map_err(|_| CaptureError::TargetNotFound(target.clone()))?;
                let r = clip_region(region, u32::from(geo.width), u32::from(geo.height))?;
                // Lecture depuis la racine aux coordonnées absolues : on capture ce qui est
                // réellement affiché, comme le voit le joueur.
                let pos = self
                    .conn
                    .translate_coordinates(win, geo.root, 0, 0)
                    .map_err(backend)?
                    .reply()
                    .map_err(backend)?;
                let abs = Rect {
                    x: i32::from(pos.dst_x) + r.x,
                    y: i32::from(pos.dst_y) + r.y,
                    width: r.width,
                    height: r.height,
                };
                let rgba = self.read_root(geo.root, abs)?;
                Ok(CapturedFrame {
                    width: r.width,
                    height: r.height,
                    rgba,
                    timestamp_ms,
                })
            }
        }
    }
}
