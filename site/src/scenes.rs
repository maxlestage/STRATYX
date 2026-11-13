//! Les scènes 2D, dans le navigateur : tout est dessiné à plat avec Canvas 2D (web-sys),
//! sans WebGL ni projection 3D.
//!
//! Chaque scène tourne dans une boucle d'animation qui s'arrête quand l'îlot disparaît, et
//! ne dessine rien tant que son canvas est hors de l'écran. La taille du canvas suit celle
//! de son élément et la densité de l'écran.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use paquets::prelude::State;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d as Ctx, EventTarget, HtmlCanvasElement, PointerEvent};

use crate::anim::{Boucle, boucle};
use crate::{FORMATS, Liens, Scene};

/// Une scène qui tourne : la lâcher l'arrête.
pub struct Running {
    _boucle: Option<Boucle>,
    _ecouteurs: Vec<Ecouteur>,
}

/// Un écouteur d'événement, retiré quand on le lâche.
struct Ecouteur {
    cible: EventTarget,
    evenement: &'static str,
    closure: Closure<dyn FnMut(PointerEvent)>,
}

impl Drop for Ecouteur {
    fn drop(&mut self) {
        let _ = self.cible.remove_event_listener_with_callback(
            self.evenement,
            self.closure.as_ref().unchecked_ref(),
        );
    }
}

fn ecouter(
    cible: &EventTarget,
    evenement: &'static str,
    f: impl FnMut(PointerEvent) + 'static,
) -> Ecouteur {
    let closure = Closure::<dyn FnMut(PointerEvent)>::new(f);
    let _ = cible.add_event_listener_with_callback(evenement, closure.as_ref().unchecked_ref());
    Ecouteur {
        cible: cible.clone(),
        evenement,
        closure,
    }
}

/// Lance `scene` dans `canvas` ; `info` reçoit les images par seconde (ou le nombre de
/// jetons pour la pluie).
pub fn start(
    canvas: &paquets::active::Dom,
    scene: Scene,
    info: State<u32>,
    liens: Liens,
) -> Running {
    let rien = || Running {
        _boucle: None,
        _ecouteurs: Vec::new(),
    };
    let Some(canvas) = canvas.dyn_ref::<HtmlCanvasElement>().cloned() else {
        return rien();
    };
    let Some(ctx) = canvas
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|c| c.dyn_into::<Ctx>().ok())
    else {
        // Pas de Canvas 2D : la page garde son fond animé en CSS.
        let _ = canvas.class_list().add_1("no-canvas");
        return rien();
    };
    let _ = canvas.class_list().add_1("live");
    match scene {
        Scene::Hero => hero(canvas, ctx, info),
        Scene::Table => table(canvas, ctx, info, liens),
        Scene::Jetons => jetons(canvas, ctx, info, liens),
    }
}

// ————————————————————————————————————————————————————————————————————————————
// Outils
// ————————————————————————————————————————————————————————————————————————————

/// Moins de mouvement quand le visiteur le demande.
fn reduced_motion() -> bool {
    web_sys::window()
        .and_then(|w| {
            w.match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|m| m.matches())
}

fn hauteur_fenetre() -> f64 {
    web_sys::window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|h| h.as_f64())
        .unwrap_or(1000.0)
}

/// Si le canvas est au moins en partie à l'écran.
fn visible(canvas: &HtmlCanvasElement) -> bool {
    let rect = canvas.get_bounding_client_rect();
    rect.bottom() > 0.0 && rect.top() < hauteur_fenetre() && rect.width() > 0.0
}

/// Ajuste la taille du canvas à son élément et à la densité de l'écran ; rend la taille en
/// pixels CSS. Le contexte dessine ensuite en pixels CSS.
fn ajuster(canvas: &HtmlCanvasElement, ctx: &Ctx) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let (w, h) = (rect.width().max(1.0), rect.height().max(1.0));
    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0)
        .clamp(1.0, 2.0);
    let (pw, ph) = ((w * dpr).round() as u32, (h * dpr).round() as u32);
    if canvas.width() != pw || canvas.height() != ph {
        canvas.set_width(pw);
        canvas.set_height(ph);
    }
    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    (w, h)
}

/// Le pointeur dans le repère du canvas (pixels CSS).
fn dans_canvas(canvas: &HtmlCanvasElement, e: &PointerEvent) -> (f64, f64) {
    let r = canvas.get_bounding_client_rect();
    (
        e.client_x() as f64 - r.left(),
        e.client_y() as f64 - r.top(),
    )
}

/// Compte les images et donne le résultat à `info` chaque seconde.
struct Fps {
    frames: u32,
    since: f64,
}

impl Fps {
    fn new() -> Fps {
        Fps {
            frames: 0,
            since: 0.0,
        }
    }

    fn tick(&mut self, time: f64, info: State<u32>) {
        self.frames += 1;
        if self.since == 0.0 {
            self.since = time;
        } else if time - self.since >= 1000.0 {
            info.set((self.frames as f64 * 1000.0 / (time - self.since)).round() as u32);
            self.frames = 0;
            self.since = time;
        }
    }
}

/// Le pas de temps entre deux images, en secondes, borné.
struct Horloge(f64);

impl Horloge {
    fn dt(&mut self, time: f64, max: f64) -> f64 {
        let dt = if self.0 == 0.0 {
            0.016
        } else {
            (time - self.0) / 1000.0
        };
        self.0 = time;
        dt.clamp(0.0, max)
    }
}

/// Des nombres au hasard pour placer et colorer les objets (xorshift).
struct Hasard(u64);

impl Hasard {
    fn new() -> Hasard {
        Hasard(((js_sys::Math::random() * u32::MAX as f64) as u64) << 16 | 1)
    }

    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next() * items.len() as f64) as usize).min(items.len() - 1)]
    }
}

fn douce(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

// ————————————————————————————————————————————————————————————————————————————
// Dessins : cartes et jetons, à plat
// ————————————————————————————————————————————————————————————————————————————

/// Les couleurs des jetons.
const JETONS: [&str; 6] = [
    "#e63946", "#2b6fff", "#16c784", "#1d1f24", "#f1efe8", "#f2c14e",
];
const EMERAUDE: &str = "#16c784";
const OR: &str = "#f2c14e";
const ROUGE: &str = "#e63946";
const ENCRE: &str = "#1d1f24";
const PAPIER: &str = "#f7f5ee";

fn chemin_arrondi(ctx: &Ctx, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    ctx.begin_path();
    ctx.move_to(x + r, y);
    let _ = ctx.arc_to(x + w, y, x + w, y + h, r);
    let _ = ctx.arc_to(x + w, y + h, x, y + h, r);
    let _ = ctx.arc_to(x, y + h, x, y, r);
    let _ = ctx.arc_to(x, y, x + w, y, r);
    ctx.close_path();
}

fn cercle(ctx: &Ctx, x: f64, y: f64, r: f64) {
    ctx.begin_path();
    let _ = ctx.arc(x, y, r.max(0.0), 0.0, TAU);
}

/// Une face de carte : rang et couleur.
#[derive(Clone, Copy)]
struct Face {
    rang: &'static str,
    symbole: &'static str,
    rouge: bool,
}

const FACES: [Face; 8] = [
    Face {
        rang: "A",
        symbole: "♠",
        rouge: false,
    },
    Face {
        rang: "K",
        symbole: "♥",
        rouge: true,
    },
    Face {
        rang: "Q",
        symbole: "♦",
        rouge: true,
    },
    Face {
        rang: "J",
        symbole: "♣",
        rouge: false,
    },
    Face {
        rang: "10",
        symbole: "♠",
        rouge: false,
    },
    Face {
        rang: "A",
        symbole: "♥",
        rouge: true,
    },
    Face {
        rang: "K",
        symbole: "♣",
        rouge: false,
    },
    Face {
        rang: "Q",
        symbole: "♠",
        rouge: false,
    },
];

/// La face d'une carte du moteur (identifiant 0 à 51).
fn face_de(id: u8) -> Option<Face> {
    let c = stratyx_poker::Card::from_id(id)?;
    let texte = c.to_string();
    let mut it = texte.chars();
    let rang = match it.next()? {
        'A' => "A",
        'K' => "K",
        'Q' => "Q",
        'J' => "J",
        'T' => "10",
        '9' => "9",
        '8' => "8",
        '7' => "7",
        '6' => "6",
        '5' => "5",
        '4' => "4",
        '3' => "3",
        _ => "2",
    };
    let (symbole, rouge) = match it.next()? {
        'h' => ("♥", true),
        'd' => ("♦", true),
        'c' => ("♣", false),
        _ => ("♠", false),
    };
    Some(Face {
        rang,
        symbole,
        rouge,
    })
}

/// Une carte de largeur `w`, centrée en (x, y), tournée de `rot` dans le plan. `largeur`
/// (de 0 à 1) la rétrécit horizontalement : c'est ainsi qu'elle se retourne, à plat.
#[allow(clippy::too_many_arguments)]
fn carte(
    ctx: &Ctx,
    x: f64,
    y: f64,
    w: f64,
    rot: f64,
    largeur: f64,
    face: Option<Face>,
    dos: &str,
    alpha: f64,
) {
    let h = w * 1.4;
    ctx.save();
    ctx.set_global_alpha(alpha.clamp(0.0, 1.0));
    let _ = ctx.translate(x, y);
    let _ = ctx.rotate(rot);
    let _ = ctx.scale(largeur.max(0.02), 1.0);
    // Ombre portée, plate.
    ctx.set_fill_style_str("rgba(0, 0, 0, 0.35)");
    chemin_arrondi(ctx, -w / 2.0 + w * 0.05, -h / 2.0 + w * 0.08, w, h, w * 0.1);
    ctx.fill();
    match face {
        Some(f) => {
            ctx.set_fill_style_str(PAPIER);
            chemin_arrondi(ctx, -w / 2.0, -h / 2.0, w, h, w * 0.1);
            ctx.fill();
            let encre = if f.rouge { ROUGE } else { ENCRE };
            ctx.set_fill_style_str(encre);
            ctx.set_text_align("left");
            ctx.set_text_baseline("top");
            ctx.set_font(&format!(
                "700 {:.1}px 'Space Grotesk', system-ui, sans-serif",
                w * 0.3
            ));
            let _ = ctx.fill_text(f.rang, -w / 2.0 + w * 0.1, -h / 2.0 + w * 0.08);
            ctx.set_font(&format!("{:.1}px system-ui, sans-serif", w * 0.24));
            let _ = ctx.fill_text(f.symbole, -w / 2.0 + w * 0.1, -h / 2.0 + w * 0.4);
            ctx.set_text_align("center");
            ctx.set_text_baseline("middle");
            ctx.set_font(&format!("{:.1}px system-ui, sans-serif", w * 0.62));
            let _ = ctx.fill_text(f.symbole, w * 0.08, h * 0.12);
        }
        None => {
            ctx.set_fill_style_str(PAPIER);
            chemin_arrondi(ctx, -w / 2.0, -h / 2.0, w, h, w * 0.1);
            ctx.fill();
            ctx.set_fill_style_str(dos);
            let m = w * 0.08;
            chemin_arrondi(
                ctx,
                -w / 2.0 + m,
                -h / 2.0 + m,
                w - 2.0 * m,
                h - 2.0 * m,
                w * 0.06,
            );
            ctx.fill();
            // Un losange au centre et des rayures.
            ctx.set_stroke_style_str("rgba(255, 255, 255, 0.22)");
            ctx.set_line_width((w * 0.03).max(0.6));
            ctx.save();
            chemin_arrondi(
                ctx,
                -w / 2.0 + m,
                -h / 2.0 + m,
                w - 2.0 * m,
                h - 2.0 * m,
                w * 0.06,
            );
            ctx.clip();
            let mut k = -h;
            while k < h {
                ctx.begin_path();
                ctx.move_to(-w / 2.0, k);
                ctx.line_to(w / 2.0, k + w);
                ctx.stroke();
                k += w * 0.18;
            }
            ctx.restore();
            ctx.set_fill_style_str("rgba(255, 255, 255, 0.85)");
            ctx.begin_path();
            ctx.move_to(0.0, -w * 0.22);
            ctx.line_to(w * 0.16, 0.0);
            ctx.line_to(0.0, w * 0.22);
            ctx.line_to(-w * 0.16, 0.0);
            ctx.close_path();
            ctx.fill();
        }
    }
    ctx.restore();
}

/// Un jeton vu de dessus : un disque coloré, des encoches claires sur le bord et un
/// médaillon au centre.
fn jeton(ctx: &Ctx, x: f64, y: f64, r: f64, rot: f64, couleur: &str, alpha: f64) {
    ctx.save();
    ctx.set_global_alpha(alpha.clamp(0.0, 1.0));
    let _ = ctx.translate(x, y);
    let _ = ctx.rotate(rot);
    ctx.set_fill_style_str("rgba(0, 0, 0, 0.3)");
    cercle(ctx, r * 0.08, r * 0.12, r);
    ctx.fill();
    ctx.set_fill_style_str(couleur);
    cercle(ctx, 0.0, 0.0, r);
    ctx.fill();
    let clair = if couleur == "#f1efe8" {
        "#2b6fff"
    } else {
        "#f1efe8"
    };
    ctx.set_stroke_style_str(clair);
    ctx.set_line_width(r * 0.22);
    for i in 0..6 {
        let a = i as f64 / 6.0 * TAU;
        ctx.begin_path();
        let _ = ctx.arc(0.0, 0.0, r * 0.86, a, a + 0.38);
        ctx.stroke();
    }
    ctx.set_stroke_style_str("rgba(255, 255, 255, 0.35)");
    ctx.set_line_width((r * 0.06).max(0.5));
    cercle(ctx, 0.0, 0.0, r * 0.58);
    ctx.stroke();
    ctx.set_fill_style_str("rgba(255, 255, 255, 0.12)");
    cercle(ctx, 0.0, 0.0, r * 0.5);
    ctx.fill();
    ctx.restore();
}

/// Une pile de jetons vue de côté : des tranches empilées.
fn pile(ctx: &Ctx, x: f64, y: f64, w: f64, n: usize, depart: usize) {
    let e = w * 0.16;
    for k in 0..n {
        let c = JETONS[(depart + k / 3) % JETONS.len()];
        let yy = y - k as f64 * e;
        ctx.set_fill_style_str(c);
        chemin_arrondi(ctx, x - w / 2.0, yy - e, w, e, e * 0.5);
        ctx.fill();
        ctx.set_fill_style_str("rgba(255, 255, 255, 0.55)");
        for j in 0..3 {
            let xx = x - w / 2.0 + w * (0.18 + j as f64 * 0.3);
            ctx.fill_rect(xx, yy - e * 0.85, w * 0.1, e * 0.7);
        }
        ctx.set_fill_style_str("rgba(0, 0, 0, 0.25)");
        ctx.fill_rect(x - w / 2.0, yy - e * 0.18, w, e * 0.18);
    }
}

// ————————————————————————————————————————————————————————————————————————————
// Le héros
// ————————————————————————————————————————————————————————————————————————————

/// Un objet qui flotte : jeton ou carte, sur un plan plus ou moins proche.
struct Flotteur {
    x: f64,
    y: f64,
    /// 0,3 (loin) à 1 (près) : taille, vitesse et parallaxe.
    plan: f64,
    vx: f64,
    vy: f64,
    rot: f64,
    spin: f64,
    phase: f64,
    genre: Genre,
}

#[derive(Clone, Copy)]
enum Genre {
    Jeton(&'static str),
    Carte(Option<Face>, &'static str),
}

struct Etoile {
    x: f64,
    y: f64,
    r: f64,
    phase: f64,
}

fn hero(canvas: HtmlCanvasElement, ctx: Ctx, info: State<u32>) -> Running {
    let calme = if reduced_motion() { 0.15 } else { 1.0 };
    let mut hasard = Hasard::new();

    // Des objets répartis sur une surface unité (0..1), mise à l'échelle au dessin.
    let mut flotteurs: Vec<Flotteur> = (0..30)
        .map(|i| {
            let plan = hasard.range(0.35, 1.0);
            let genre = if i % 3 == 0 {
                Genre::Carte(
                    if hasard.next() > 0.4 {
                        Some(hasard.pick(&FACES))
                    } else {
                        None
                    },
                    hasard.pick(&[EMERAUDE, ROUGE, "#2b6fff"]),
                )
            } else {
                Genre::Jeton(hasard.pick(&JETONS))
            };
            Flotteur {
                x: hasard.next(),
                y: hasard.next(),
                plan,
                vx: hasard.range(-0.012, 0.012),
                vy: hasard.range(-0.02, -0.006),
                rot: hasard.range(0.0, TAU),
                spin: hasard.range(-0.5, 0.5),
                phase: hasard.range(0.0, TAU),
                genre,
            }
        })
        .collect();
    flotteurs.sort_by(|a, b| a.plan.total_cmp(&b.plan));
    let etoiles: Vec<Etoile> = (0..140)
        .map(|_| Etoile {
            x: hasard.next(),
            y: hasard.next(),
            r: hasard.range(0.4, 1.4),
            phase: hasard.range(0.0, TAU),
        })
        .collect();

    // Le pointeur, de -1 à 1 sur chaque axe.
    let pointeur = Rc::new(Cell::new((0.0f64, 0.0f64)));
    let mut ecouteurs = Vec::new();
    if let Some(fenetre) = web_sys::window() {
        let pointeur = pointeur.clone();
        let cible: EventTarget = fenetre.into();
        ecouteurs.push(ecouter(&cible, "pointermove", move |e| {
            if let Some(w) = web_sys::window() {
                let lw = w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1.0);
                let lh = w
                    .inner_height()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                pointeur.set((
                    e.client_x() as f64 / lw * 2.0 - 1.0,
                    e.client_y() as f64 / lh * 2.0 - 1.0,
                ));
            }
        }));
    }

    let mut fps = Fps::new();
    let mut horloge = Horloge(0.0);
    let mut lisse = (0.0f64, 0.0f64);
    let mut defilement_lisse = 0.0f64;
    let boucle = boucle(move |time| {
        let dt = horloge.dt(time, 0.1);
        if !visible(&canvas) {
            return;
        }
        fps.tick(time, info);
        let (w, h) = ajuster(&canvas, &ctx);
        let t = time / 1000.0 * calme;

        let (px, py) = pointeur.get();
        lisse.0 += (px - lisse.0) * (dt * 3.0).min(1.0);
        lisse.1 += (py - lisse.1) * (dt * 3.0).min(1.0);
        let defilement = web_sys::window()
            .and_then(|w| w.scroll_y().ok())
            .unwrap_or(0.0);
        defilement_lisse += (defilement - defilement_lisse) * (dt * 6.0).min(1.0);

        ctx.clear_rect(0.0, 0.0, w, h);
        let unite = w.min(h).max(320.0);
        // Sur un écran large, la composition se range à droite du titre.
        let large = w > 900.0;
        let (cx, cy) = if large {
            (w * 0.72, h * 0.48)
        } else {
            (w * 0.5, h * 0.62)
        };

        // Les étoiles, très loin.
        for e in &etoiles {
            let a = 0.25 + 0.35 * ((t * 1.2 + e.phase).sin() * 0.5 + 0.5);
            ctx.set_fill_style_str(&format!("rgba(216, 245, 232, {a:.2})"));
            let x = e.x * w - lisse.0 * 6.0;
            let y = (e.y * h - defilement_lisse * 0.05).rem_euclid(h);
            ctx.fill_rect(x, y, e.r, e.r);
        }

        // Deux anneaux, à plat, qui tournent sur eux-mêmes.
        let decale = (lisse.0 * 14.0, lisse.1 * 10.0 - defilement_lisse * 0.25);
        ctx.save();
        ctx.set_line_width(1.5);
        let dash = js_sys::Array::of2(&JsValue::from_f64(14.0), &JsValue::from_f64(10.0));
        let _ = ctx.set_line_dash(&dash);
        ctx.set_line_dash_offset(-t * 30.0);
        ctx.set_stroke_style_str("rgba(242, 193, 78, 0.55)");
        cercle(&ctx, cx + decale.0, cy + decale.1, unite * 0.36);
        ctx.stroke();
        ctx.set_line_dash_offset(t * 22.0);
        ctx.set_stroke_style_str("rgba(22, 199, 132, 0.5)");
        cercle(&ctx, cx + decale.0, cy + decale.1, unite * 0.46);
        ctx.stroke();
        ctx.restore();

        // Les objets qui flottent (les plus proches par-dessus).
        for f in &mut flotteurs {
            f.x = (f.x + f.vx * dt * calme).rem_euclid(1.0);
            f.y = (f.y + f.vy * dt * calme * f.plan).rem_euclid(1.0);
            f.rot += f.spin * dt * calme;
            let bord = 0.12;
            let x = -bord * w + f.x * w * (1.0 + 2.0 * bord) + lisse.0 * 40.0 * f.plan;
            let y = -bord * h
                + f.y * h * (1.0 + 2.0 * bord)
                + lisse.1 * 28.0 * f.plan
                + (t * 0.8 + f.phase).sin() * 8.0
                - defilement_lisse * 0.4 * f.plan;
            let y = y.rem_euclid(h * (1.0 + 2.0 * bord)) - bord * h;
            // Sur un écran large, ceux qui passent derrière le titre s'effacent.
            let voile = if large {
                (x / (w * 0.45)).clamp(0.25, 1.0)
            } else {
                0.7
            };
            let alpha = (0.35 + 0.65 * f.plan) * voile;
            let taille = unite * (0.03 + 0.04 * f.plan);
            match f.genre {
                Genre::Jeton(c) => jeton(&ctx, x, y, taille, f.rot, c, alpha),
                Genre::Carte(face, dos) => {
                    carte(&ctx, x, y, taille * 1.6, f.rot * 0.4, 1.0, face, dos, alpha);
                }
            }
        }

        // Au centre : trois piles de jetons et un éventail de cartes qui respire.
        let (ox, oy) = (cx + decale.0 * 1.4, cy + decale.1 * 1.2);
        let lj = unite * 0.1;
        for (k, (dx, n)) in [(-1.1f64, 9usize), (0.0, 12), (1.1, 6)]
            .into_iter()
            .enumerate()
        {
            pile(&ctx, ox + dx * lj * 1.15, oy + unite * 0.2, lj, n, k * 2);
        }
        let ouverture = 0.8 + (t * 0.9).sin() * 0.2;
        let wc = unite * 0.11;
        for (i, face) in FACES.iter().take(5).enumerate() {
            let k = i as f64 - 2.0;
            let a = k * 0.2 * ouverture;
            let (sx, sy) = (
                ox + a.sin() * wc * 1.6,
                oy - wc * 0.4 - a.cos() * wc * 0.9 + (t * 1.3).sin() * 4.0,
            );
            carte(&ctx, sx, sy, wc, a, 1.0, Some(*face), EMERAUDE, 1.0);
        }
    });
    Running {
        _boucle: Some(boucle),
        _ecouteurs: ecouteurs,
    }
}

// ————————————————————————————————————————————————————————————————————————————
// La table de la démo
// ————————————————————————————————————————————————————————————————————————————

/// Les positions, du bouton vers sa gauche, selon le nombre de joueurs.
fn positions(n: usize) -> &'static [&'static str] {
    match n {
        3 => &["BTN", "SB", "BB"],
        6 => &["BTN", "SB", "BB", "UTG", "HJ", "CO"],
        _ => &["BTN", "SB", "BB", "UTG", "UTG+1", "MP", "LJ", "HJ", "CO"],
    }
}

/// L'angle du siège `i` sur `n` (le siège 0, le vôtre, est en bas).
fn angle(i: f64, n: f64) -> f64 {
    FRAC_PI_2 + i / n * TAU
}

fn table(canvas: HtmlCanvasElement, ctx: Ctx, info: State<u32>, l: Liens) -> Running {
    let calme = reduced_motion();
    let mut hasard = Hasard::new();
    let tapis: Vec<usize> = (0..9).map(|_| 3 + (hasard.next() * 8.0) as usize).collect();

    // Le siège survolé par le pointeur.
    let survol = Rc::new(Cell::new((-1.0f64, -1.0f64)));
    let cible: EventTarget = canvas.clone().into();
    let mut ecouteurs = Vec::new();
    {
        let survol = survol.clone();
        let c = canvas.clone();
        ecouteurs.push(ecouter(&cible, "pointermove", move |e| {
            survol.set(dans_canvas(&c, &e))
        }));
    }
    {
        let survol = survol.clone();
        ecouteurs.push(ecouter(&cible, "pointerleave", move |_| {
            survol.set((-1.0, -1.0))
        }));
    }

    let mut fps = Fps::new();
    let mut horloge = Horloge(0.0);
    let mut n_lisse = FORMATS[l.format.get().min(FORMATS.len() - 1)].1 as f64;
    let mut presence = [0.0f64; 9];
    let mut progres = [0.0f64; 5];
    let mut main = 0.0f64;
    let mut donne_vue = l.donne.get();
    let boucle = boucle(move |time| {
        let dt = horloge.dt(time, 0.1);
        if !visible(&canvas) {
            return;
        }
        fps.tick(time, info);
        let (w, h) = ajuster(&canvas, &ctx);
        let vite = |k: f64| if calme { 1.0 } else { (dt * k).min(1.0) };
        let n = FORMATS[l.format.get().min(FORMATS.len() - 1)].1;
        n_lisse += (n as f64 - n_lisse) * vite(5.0);

        // Une nouvelle main : le board retourne au paquet, les cartes sont redistribuées.
        let donne = l.donne.get();
        if donne != donne_vue {
            donne_vue = donne;
            progres = [0.0; 5];
            main = 0.0;
        }
        main = (main + if calme { 1.0 } else { dt * 2.5 }).min(1.0);

        ctx.clear_rect(0.0, 0.0, w, h);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let rx = (w * 0.38).min(h * 0.62);
        let ry = (h * 0.3).min(rx * 0.62);
        let unite = rx.min(ry * 1.6);

        // La table : rail, tapis, ligne dorée.
        ctx.set_fill_style_str("rgba(0, 0, 0, 0.35)");
        ctx.begin_path();
        let _ = ctx.ellipse(cx, cy + unite * 0.06, rx * 1.12, ry * 1.2, 0.0, 0.0, TAU);
        ctx.fill();
        ctx.set_fill_style_str("#1a1410");
        ctx.begin_path();
        let _ = ctx.ellipse(cx, cy, rx * 1.1, ry * 1.17, 0.0, 0.0, TAU);
        ctx.fill();
        if let Ok(g) = ctx.create_radial_gradient(cx, cy - ry * 0.2, 0.0, cx, cy, rx) {
            let _ = g.add_color_stop(0.0, "#139158");
            let _ = g.add_color_stop(1.0, "#0b5236");
            ctx.set_fill_style_canvas_gradient(&g);
        }
        ctx.begin_path();
        let _ = ctx.ellipse(cx, cy, rx, ry, 0.0, 0.0, TAU);
        ctx.fill();
        ctx.set_stroke_style_str("rgba(242, 193, 78, 0.55)");
        ctx.set_line_width(1.2);
        ctx.begin_path();
        let _ = ctx.ellipse(cx, cy, rx * 0.8, ry * 0.72, 0.0, 0.0, TAU);
        ctx.stroke();
        ctx.set_fill_style_str("rgba(255, 255, 255, 0.08)");
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.set_font(&format!(
            "700 {:.0}px 'Space Grotesk', system-ui, sans-serif",
            unite * 0.13
        ));
        let _ = ctx.fill_text("STRATYX", cx, cy + ry * 0.45);

        // Le pot.
        let pot_y = cy - ry * 0.42;
        for i in 0..3 {
            pile(
                &ctx,
                cx + (i as f64 - 1.0) * unite * 0.11,
                pot_y,
                unite * 0.09,
                3 + i * 2,
                i,
            );
        }

        // Les sièges glissent à leur place quand le format change.
        let (sx, sy) = survol.get();
        let noms = positions(n);
        let bouton = n - 1;
        for (i, presence) in presence.iter_mut().enumerate() {
            let present = i < n;
            *presence += ((if present { 1.0 } else { 0.0 }) - *presence) * vite(6.0);
            if *presence < 0.02 {
                continue;
            }
            let a = angle(i as f64, n_lisse.max(2.0));
            let (ex, ey) = (cx + a.cos() * rx * 1.18, cy + a.sin() * ry * 1.3);
            let r = unite * 0.1;
            let pointe = ((sx - ex).powi(2) + (sy - ey).powi(2)).sqrt() < r * 1.4;
            ctx.save();
            ctx.set_global_alpha(*presence);
            // Les jetons et les cartes du siège, vers le centre.
            let (ix, iy) = (cx + a.cos() * rx * 0.78, cy + a.sin() * ry * 0.7);
            pile(
                &ctx,
                ix + unite * 0.09,
                iy + unite * 0.04,
                unite * 0.07,
                tapis[i],
                i,
            );
            if i == 0 {
                let cartes = l.cartes.get();
                for j in 0..2 {
                    let e = douce(main * 2.0 - j as f64 * 0.5);
                    let face = cartes.get(j).and_then(|id| face_de(*id));
                    carte(
                        &ctx,
                        ix - unite * 0.1 + j as f64 * unite * 0.13,
                        iy - unite * 0.06 + (1.0 - e) * unite * 0.3,
                        unite * 0.15,
                        (j as f64 - 0.5) * 0.12,
                        1.0,
                        face,
                        EMERAUDE,
                        e,
                    );
                }
            } else {
                for j in 0..2 {
                    carte(
                        &ctx,
                        ix - unite * 0.06 + j as f64 * unite * 0.05,
                        iy - unite * 0.02,
                        unite * 0.09,
                        a - FRAC_PI_2 + (j as f64 - 0.5) * 0.2,
                        1.0,
                        None,
                        "#b3263a",
                        douce(main * 1.5),
                    );
                }
            }
            // L'avatar : position et tapis.
            if pointe || i == 0 {
                ctx.set_fill_style_str(if i == 0 {
                    "rgba(22, 199, 132, 0.3)"
                } else {
                    "rgba(242, 193, 78, 0.25)"
                });
                cercle(&ctx, ex, ey, r * 1.35);
                ctx.fill();
            }
            ctx.set_fill_style_str("#0c1813");
            cercle(&ctx, ex, ey, r);
            ctx.fill();
            ctx.set_line_width(2.0);
            ctx.set_stroke_style_str(if i == 0 {
                EMERAUDE
            } else if pointe {
                OR
            } else {
                "rgba(255, 255, 255, 0.25)"
            });
            ctx.stroke();
            let nom = if present {
                noms[(i + n - bouton) % n]
            } else {
                ""
            };
            ctx.set_fill_style_str("#f1f7f3");
            ctx.set_font(&format!(
                "600 {:.0}px 'Space Grotesk', system-ui, sans-serif",
                r * 0.55
            ));
            let _ = ctx.fill_text(if i == 0 { "Vous" } else { nom }, ex, ey - r * 0.18);
            ctx.set_fill_style_str("#a3b8ad");
            ctx.set_font(&format!(
                "500 {:.0}px Inter, system-ui, sans-serif",
                r * 0.4
            ));
            let _ = ctx.fill_text(
                &if i == 0 {
                    nom.to_owned()
                } else {
                    format!("{} BB", tapis[i] * 4)
                },
                ex,
                ey + r * 0.38,
            );
            ctx.restore();
        }

        // Le bouton du donneur.
        let ab = angle(bouton as f64, n_lisse.max(2.0)) + 0.32 * 6.0 / n_lisse.max(2.0);
        let (bx, by) = (cx + ab.cos() * rx * 0.86, cy + ab.sin() * ry * 0.82);
        ctx.set_fill_style_str(PAPIER);
        cercle(&ctx, bx, by, unite * 0.045);
        ctx.fill();
        ctx.set_fill_style_str(ENCRE);
        ctx.set_font(&format!(
            "700 {:.0}px 'Space Grotesk', system-ui, sans-serif",
            unite * 0.045
        ));
        let _ = ctx.fill_text("D", bx, by + 0.5);

        // Le board : cinq cartes, qui partent du paquet l'une après l'autre.
        let cartes = l.cartes.get();
        let visibles = [0usize, 3, 4, 5][l.etape.get().min(3) as usize];
        let wc = unite * 0.17;
        // Le paquet, à côté du pot.
        let paquet = (cx + unite * 0.36, pot_y - unite * 0.02);
        carte(
            &ctx,
            paquet.0,
            paquet.1,
            wc * 0.65,
            0.2,
            1.0,
            None,
            EMERAUDE,
            0.9,
        );
        let mut precedent = 1.0;
        for (i, p) in progres.iter_mut().enumerate() {
            let cible = if i < visibles { 1.0 } else { 0.0 };
            if cible > *p && precedent < 0.55 {
                // Attend que la carte précédente soit à mi-chemin.
            } else if calme {
                *p = cible;
            } else {
                let v = if cible > *p { 2.4 } else { 5.0 };
                *p = (*p + (cible - *p).signum() * dt * v).clamp(0.0, 1.0);
            }
            precedent = *p;
            if *p < 0.01 {
                continue;
            }
            let e = douce(*p);
            let fin = (cx + (i as f64 - 2.0) * wc * 1.12, cy + ry * 0.05);
            let x = paquet.0 + (fin.0 - paquet.0) * e;
            let y = paquet.1 + (fin.1 - paquet.1) * e;
            // La carte se retourne à plat : elle rétrécit, puis montre sa face.
            let retour = ((e - 0.5) * 2.0).abs();
            let face = if e > 0.5 {
                cartes.get(2 + i).and_then(|id| face_de(*id))
            } else {
                None
            };
            carte(&ctx, x, y, wc, (1.0 - e) * 0.3, retour, face, EMERAUDE, 1.0);
        }
    });
    Running {
        _boucle: Some(boucle),
        _ecouteurs: ecouteurs,
    }
}

// ————————————————————————————————————————————————————————————————————————————
// La pluie de jetons : une physique 2D écrite ici
// ————————————————————————————————————————————————————————————————————————————

const MAX_JETONS: usize = 160;

/// Un jeton : un disque avec sa position, sa vitesse et sa rotation.
struct Disque {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    rot: f64,
    w: f64,
    couleur: &'static str,
}

/// Un pas de la physique : gravité, rebonds sur les bords, collisions entre disques
/// (correction de position puis impulsion), frottements.
fn physique(disques: &mut [Disque], r: f64, w: f64, sol: f64, dt: f64) {
    const GRAVITE: f64 = 2200.0;
    const RESTITUTION: f64 = 0.35;
    let sous_pas = 4;
    let h = dt / sous_pas as f64;
    for _ in 0..sous_pas {
        for d in disques.iter_mut() {
            d.vy += GRAVITE * h;
            d.x += d.vx * h;
            d.y += d.vy * h;
            d.rot += d.w * h;
            d.w *= 0.995;
        }
        for _ in 0..3 {
            // Les bords.
            for d in disques.iter_mut() {
                if d.y > sol - r {
                    d.y = sol - r;
                    if d.vy > 0.0 {
                        d.vy = -d.vy * RESTITUTION;
                        if d.vy.abs() < 40.0 {
                            d.vy = 0.0;
                        }
                    }
                    // Le jeton roule : la rotation suit la vitesse, le tapis freine.
                    d.vx *= 0.96;
                    d.w = d.vx / r;
                }
                if d.x < r {
                    d.x = r;
                    d.vx = d.vx.abs() * RESTITUTION;
                }
                if d.x > w - r {
                    d.x = w - r;
                    d.vx = -d.vx.abs() * RESTITUTION;
                }
            }
            // Les collisions entre disques.
            let n = disques.len();
            for i in 0..n {
                for j in (i + 1)..n {
                    let (a, b) = disques.split_at_mut(j);
                    let (a, b) = (&mut a[i], &mut b[0]);
                    let (dx, dy) = (b.x - a.x, b.y - a.y);
                    let d2 = dx * dx + dy * dy;
                    let min = 2.0 * r;
                    if d2 >= min * min || d2 < 1e-9 {
                        continue;
                    }
                    let d = d2.sqrt();
                    let (nx, ny) = (dx / d, dy / d);
                    let recouvrement = (min - d) * 0.5;
                    a.x -= nx * recouvrement;
                    a.y -= ny * recouvrement;
                    b.x += nx * recouvrement;
                    b.y += ny * recouvrement;
                    let vn = (b.vx - a.vx) * nx + (b.vy - a.vy) * ny;
                    if vn < 0.0 {
                        let j = -(1.0 + RESTITUTION) * vn * 0.5;
                        a.vx -= j * nx;
                        a.vy -= j * ny;
                        b.vx += j * nx;
                        b.vy += j * ny;
                        // Le frottement tangent fait tourner les deux jetons.
                        let vt = (b.vx - a.vx) * -ny + (b.vy - a.vy) * nx;
                        a.w += vt / r * 0.1;
                        b.w -= vt / r * 0.1;
                        a.vx *= 0.995;
                        b.vx *= 0.995;
                    }
                }
            }
        }
    }
}

fn jetons(canvas: HtmlCanvasElement, ctx: Ctx, info: State<u32>, l: Liens) -> Running {
    let mut hasard = Hasard::new();
    let mut disques: Vec<Disque> = Vec::new();
    let mut a_lacher = 24u32;
    let mut lache_vu = l.lacher.get();
    let mut secousse_vue = l.secousse.get();
    let mut minuterie = 0.0f64;
    let mut horloge = Horloge(0.0);

    // Un appui sur le tapis lâche un jeton à cet endroit.
    let appuis: Rc<Cell<Option<(f64, f64)>>> = Rc::default();
    let mut ecouteurs = Vec::new();
    {
        let appuis = appuis.clone();
        let c = canvas.clone();
        let cible: EventTarget = canvas.clone().into();
        ecouteurs.push(ecouter(&cible, "pointerdown", move |e| {
            appuis.set(Some(dans_canvas(&c, &e)))
        }));
    }

    let boucle = boucle(move |time| {
        let dt = horloge.dt(time, 0.033);
        if !visible(&canvas) {
            return;
        }
        let (w, h) = ajuster(&canvas, &ctx);
        let r = (w.min(h) * 0.045).clamp(12.0, 26.0);
        let sol = h - h * 0.08;

        let demande = l.lacher.get();
        if demande != lache_vu {
            a_lacher += demande.saturating_sub(lache_vu);
            lache_vu = demande;
        }
        let mut nouveau = |x: f64, y: f64, hasard: &mut Hasard| {
            disques.push(Disque {
                x: x.clamp(r, w - r),
                y,
                vx: hasard.range(-120.0, 120.0),
                vy: hasard.range(0.0, 200.0),
                rot: hasard.range(0.0, TAU),
                w: hasard.range(-4.0, 4.0),
                couleur: hasard.pick(&JETONS),
            });
        };
        // Un jeton toutes les 50 ms, en pluie.
        minuterie -= dt;
        if a_lacher > 0 && minuterie <= 0.0 {
            minuterie = 0.05;
            a_lacher -= 1;
            let x = hasard.range(w * 0.3, w * 0.7);
            nouveau(x, -r * 2.0, &mut hasard);
        }
        if let Some((x, y)) = appuis.take() {
            nouveau(x, y.min(sol - r * 3.0), &mut hasard);
        }
        if disques.len() > MAX_JETONS {
            let trop = disques.len() - MAX_JETONS;
            disques.drain(0..trop);
        }
        let secousse = l.secousse.get();
        if secousse != secousse_vue {
            secousse_vue = secousse;
            for d in &mut disques {
                d.vx += hasard.range(-500.0, 500.0);
                d.vy -= hasard.range(700.0, 1300.0);
                d.w += hasard.range(-12.0, 12.0);
            }
        }
        physique(&mut disques, r, w, sol, dt);
        if info.get() != disques.len() as u32 {
            info.set(disques.len() as u32);
        }

        // Le décor : le tapis vu de face, son bord et sa lumière.
        ctx.clear_rect(0.0, 0.0, w, h);
        if let Ok(g) =
            ctx.create_radial_gradient(w / 2.0, h * 0.35, 0.0, w / 2.0, h * 0.35, w.max(h) * 0.7)
        {
            let _ = g.add_color_stop(0.0, "#123224");
            let _ = g.add_color_stop(1.0, "#07110c");
            ctx.set_fill_style_canvas_gradient(&g);
        }
        ctx.fill_rect(0.0, 0.0, w, h);
        ctx.set_fill_style_str("#0d5c3b");
        ctx.fill_rect(0.0, sol, w, h - sol);
        ctx.set_fill_style_str("rgba(242, 193, 78, 0.6)");
        ctx.fill_rect(0.0, sol, w, 2.0);
        for d in &disques {
            jeton(&ctx, d.x, d.y, r, d.rot, d.couleur, 1.0);
        }
    });
    Running {
        _boucle: Some(boucle),
        _ecouteurs: ecouteurs,
    }
}
