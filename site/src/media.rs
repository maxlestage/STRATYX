//! Les médias du navigateur pour la page `/app` : capture de l'écran, caméra, vidéo ou
//! images choisies sur l'appareil, et lecture des pixels d'une image (pour la vision).
//!
//! Tout reste sur l'appareil : aucune image n'est envoyée au serveur.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, HtmlVideoElement, MediaStream,
};

/// D'où viennent les images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ecran,
    Camera,
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Démarre la capture de l'écran ou de la caméra dans `video` ; `fin` reçoit le résultat.
pub fn demarrer(
    source: Source,
    video: &HtmlVideoElement,
    fin: impl FnOnce(Result<(), String>) + 'static,
) {
    let Some(devices) = web_sys::window().and_then(|w| w.navigator().media_devices().ok()) else {
        fin(Err("ce navigateur ne permet pas la capture vidéo".into()));
        return;
    };
    let promesse = match source {
        Source::Ecran => {
            let c = web_sys::DisplayMediaStreamConstraints::new();
            c.set_video(&wasm_bindgen::JsValue::TRUE);
            c.set_audio(&wasm_bindgen::JsValue::FALSE);
            devices.get_display_media_with_constraints(&c)
        }
        Source::Camera => {
            let c = web_sys::MediaStreamConstraints::new();
            let video_c = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&video_c, &"facingMode".into(), &"environment".into());
            let _ = js_sys::Reflect::set(&video_c, &"width".into(), &1920.into());
            c.set_video(&video_c);
            c.set_audio(&wasm_bindgen::JsValue::FALSE);
            devices.get_user_media_with_constraints(&c)
        }
    };
    let promesse = match promesse {
        Ok(p) => p,
        Err(_) => {
            fin(Err("capture refusée par le navigateur".into()));
            return;
        }
    };
    let fin = Rc::new(RefCell::new(Some(fin)));
    let v = video.clone();
    let f1 = fin.clone();
    let ok = Closure::once(move |flux: wasm_bindgen::JsValue| {
        if let Ok(flux) = flux.dyn_into::<MediaStream>() {
            v.set_src_object(Some(&flux));
            v.set_muted(true);
            let _ = v.play();
            if let Some(f) = f1.borrow_mut().take() {
                f(Ok(()));
            }
        }
    });
    let ko = Closure::once(move |_e: wasm_bindgen::JsValue| {
        if let Some(f) = fin.borrow_mut().take() {
            f(Err("capture annulée ou refusée".into()));
        }
    });
    let _ = promesse.then2(&ok, &ko);
    ok.forget();
    ko.forget();
}

/// Arrête la capture (libère l'écran ou la caméra) et la lecture.
pub fn arreter(video: &HtmlVideoElement) {
    if let Some(flux) = video
        .src_object()
        .and_then(|s| s.dyn_into::<MediaStream>().ok())
    {
        for piste in flux.get_tracks().iter() {
            if let Ok(p) = piste.dyn_into::<web_sys::MediaStreamTrack>() {
                p.stop();
            }
        }
    }
    video.set_src_object(None);
    let _ = video.pause();
    let src = video.src();
    if src.starts_with("blob:") {
        let _ = web_sys::Url::revoke_object_url(&src);
        video.set_src("");
    }
}

/// Lit une vidéo de l'appareil en accéléré (sans son) pour en extraire les images.
pub fn lire_video(
    fichier: &web_sys::File,
    video: &HtmlVideoElement,
    vitesse: f64,
) -> Result<(), String> {
    let url = web_sys::Url::create_object_url_with_blob(fichier)
        .map_err(|_| "vidéo illisible".to_owned())?;
    video.set_src_object(None);
    video.set_src(&url);
    video.set_muted(true);
    video.set_playback_rate(vitesse);
    video.set_default_playback_rate(vitesse);
    let _ = video.play();
    Ok(())
}

/// La vidéo est-elle finie ?
pub fn finie(video: &HtmlVideoElement) -> bool {
    video.ended()
}

/// Position de lecture, en millisecondes.
pub fn temps_ms(video: &HtmlVideoElement) -> u64 {
    (video.current_time() * 1000.0).max(0.0) as u64
}

/// Une image lue : taille, pixels RGBA et vignette JPEG (adresse `data:`).
pub struct Capture {
    pub largeur: u32,
    pub hauteur: u32,
    pub rgba: Vec<u8>,
}

thread_local! {
    static TOILE: RefCell<Option<(HtmlCanvasElement, CanvasRenderingContext2d)>> = const { RefCell::new(None) };
}

fn toile(largeur: u32, hauteur: u32) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
    TOILE.with(|t| {
        if t.borrow().is_none() {
            let c = document()?
                .create_element("canvas")
                .ok()?
                .dyn_into::<HtmlCanvasElement>()
                .ok()?;
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &opts,
                &"willReadFrequently".into(),
                &wasm_bindgen::JsValue::TRUE,
            );
            let ctx = c
                .get_context_with_context_options("2d", &opts)
                .ok()??
                .dyn_into::<CanvasRenderingContext2d>()
                .ok()?;
            *t.borrow_mut() = Some((c, ctx));
        }
        let pair = t.borrow().clone()?;
        if pair.0.width() != largeur || pair.0.height() != hauteur {
            pair.0.set_width(largeur);
            pair.0.set_height(hauteur);
        }
        Some(pair)
    })
}

fn lire(source: &wasm_bindgen::JsValue, w: u32, h: u32, max: u32) -> Option<Capture> {
    if w == 0 || h == 0 {
        return None;
    }
    let echelle = (f64::from(max) / f64::from(w)).min(1.0);
    let (lw, lh) = (
        ((f64::from(w) * echelle).round() as u32).max(1),
        ((f64::from(h) * echelle).round() as u32).max(1),
    );
    let (_, ctx) = toile(lw, lh)?;
    if let Some(v) = source.dyn_ref::<HtmlVideoElement>() {
        ctx.draw_image_with_html_video_element_and_dw_and_dh(
            v,
            0.0,
            0.0,
            f64::from(lw),
            f64::from(lh),
        )
        .ok()?;
    } else {
        let i = source.dyn_ref::<HtmlImageElement>()?;
        ctx.draw_image_with_html_image_element_and_dw_and_dh(
            i,
            0.0,
            0.0,
            f64::from(lw),
            f64::from(lh),
        )
        .ok()?;
    }
    let data = ctx
        .get_image_data(0.0, 0.0, f64::from(lw), f64::from(lh))
        .ok()?;
    Some(Capture {
        largeur: lw,
        hauteur: lh,
        rgba: data.data().0,
    })
}

/// L'image courante d'une vidéo, réduite à `max` pixels de large.
pub fn image_video(video: &HtmlVideoElement, max: u32) -> Option<Capture> {
    lire(
        video.as_ref(),
        video.video_width(),
        video.video_height(),
        max,
    )
}

/// Une vignette JPEG de la dernière image lue (adresse `data:`), `largeur` pixels de large.
pub fn vignette(capture: &Capture, largeur: u32) -> Option<String> {
    let (c, ctx) = toile(capture.largeur, capture.hauteur)?;
    let donnees = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        wasm_bindgen::Clamped(&capture.rgba),
        capture.largeur,
        capture.hauteur,
    )
    .ok()?;
    ctx.put_image_data(&donnees, 0.0, 0.0).ok()?;
    let petite = document()?
        .create_element("canvas")
        .ok()?
        .dyn_into::<HtmlCanvasElement>()
        .ok()?;
    let h = (f64::from(capture.hauteur) * f64::from(largeur) / f64::from(capture.largeur)).round()
        as u32;
    petite.set_width(largeur);
    petite.set_height(h.max(1));
    let pctx = petite
        .get_context("2d")
        .ok()??
        .dyn_into::<CanvasRenderingContext2d>()
        .ok()?;
    pctx.draw_image_with_html_canvas_element_and_dw_and_dh(
        &c,
        0.0,
        0.0,
        f64::from(largeur),
        f64::from(h),
    )
    .ok()?;
    petite
        .to_data_url_with_type_and_encoder_options("image/jpeg", &0.72.into())
        .ok()
}

/// Lit des images de l'appareil une à une ; `chacune` reçoit (rang, capture) pour chaque
/// image décodée, `fin` est appelé après la dernière.
pub fn lire_images(
    fichiers: Vec<web_sys::File>,
    max: u32,
    chacune: impl Fn(usize, Capture) + 'static,
    fin: impl Fn() + 'static,
) {
    let chacune = Rc::new(chacune);
    let fin = Rc::new(fin);
    suivante(Rc::new(fichiers), 0, max, chacune, fin);
}

fn suivante(
    fichiers: Rc<Vec<web_sys::File>>,
    i: usize,
    max: u32,
    chacune: Rc<dyn Fn(usize, Capture)>,
    fin: Rc<dyn Fn()>,
) {
    let Some(fichier) = fichiers.get(i).cloned() else {
        fin();
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&fichier) else {
        suivante(fichiers, i + 1, max, chacune, fin);
        return;
    };
    let Some(img) = document()
        .and_then(|d| d.create_element("img").ok())
        .and_then(|e| e.dyn_into::<HtmlImageElement>().ok())
    else {
        fin();
        return;
    };
    let i2 = img.clone();
    let u2 = url.clone();
    let src = url.clone();
    let (f2, c2, fin2) = (fichiers.clone(), chacune.clone(), fin.clone());
    let charge = Closure::once_into_js(move || {
        if let Some(c) = lire(i2.as_ref(), i2.natural_width(), i2.natural_height(), max) {
            c2(i, c);
        }
        let _ = web_sys::Url::revoke_object_url(&u2);
        suivante(f2, i + 1, max, c2, fin2);
    });
    let erreur = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
        suivante(fichiers, i + 1, max, chacune, fin);
    });
    img.set_onload(Some(charge.unchecked_ref()));
    img.set_onerror(Some(erreur.unchecked_ref()));
    img.set_src(&src);
}

/// Les fichiers choisis dans un `<input type="file">`.
pub fn fichiers(e: &paquets::active::Event) -> Vec<web_sys::File> {
    let Some(input) = e
        .raw()
        .target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return Vec::new();
    };
    let Some(liste) = input.files() else {
        return Vec::new();
    };
    (0..liste.length()).filter_map(|i| liste.get(i)).collect()
}

/// Un minuteur qui appelle `f` toutes les `ms` millisecondes ; le lâcher l'arrête.
pub struct Minuteur {
    id: i32,
    _rappel: Closure<dyn FnMut()>,
}

impl Minuteur {
    pub fn new(ms: i32, f: impl FnMut() + 'static) -> Option<Minuteur> {
        let rappel = Closure::wrap(Box::new(f) as Box<dyn FnMut()>);
        let id = web_sys::window()?
            .set_interval_with_callback_and_timeout_and_arguments_0(
                rappel.as_ref().unchecked_ref(),
                ms,
            )
            .ok()?;
        Some(Minuteur {
            id,
            _rappel: rappel,
        })
    }
}

impl Drop for Minuteur {
    fn drop(&mut self) {
        if let Some(w) = web_sys::window() {
            w.clear_interval_with_handle(self.id);
        }
    }
}

/// Garde un objet dans le stockage local du navigateur (préférences de reconnaissance).
pub fn garder(cle: &str, valeur: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(cle, valeur);
    }
}

/// Relit un objet du stockage local.
pub fn relire(cle: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(cle)
        .ok()
        .flatten()
}
