//! Les scènes 3D, dans le navigateur : `paquets::three` (l'API de Three.js, en Rust)
//! dessine avec WebGL 2, `paquets::three::Physics` fait tomber les jetons.
//!
//! Chaque scène tourne dans une `animation_loop` qui s'arrête quand l'îlot disparaît, et
//! ne dessine rien tant que son canvas est hors de l'écran.

use std::cell::Cell;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::rc::Rc;

use paquets::prelude::State;
use paquets::three::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::{FORMATS, Liens, Scene3d};

/// Une scène qui tourne : la lâcher l'arrête.
pub struct Running {
    _loop: Option<AnimationLoop>,
    _pointer: Option<Listener>,
}

/// Un écouteur d'événement sur la fenêtre, retiré quand on le lâche.
struct Listener {
    event: &'static str,
    closure: Closure<dyn FnMut(web_sys::PointerEvent)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback(
                self.event,
                self.closure.as_ref().unchecked_ref(),
            );
        }
    }
}

/// Lance `scene` dans `canvas` ; `info` reçoit les images par seconde (ou le nombre de
/// jetons pour la pluie).
pub fn start(
    canvas: &paquets::active::Dom,
    scene: Scene3d,
    info: State<u32>,
    liens: Liens,
) -> Running {
    let mut renderer = match WebGLRenderer::new(canvas) {
        Ok(renderer) => renderer,
        Err(_) => {
            // Pas de WebGL 2 : la page garde son fond animé en CSS.
            let _ = canvas.class_list().add_1("no-webgl");
            return Running {
                _loop: None,
                _pointer: None,
            };
        }
    };
    renderer.set_pixel_ratio(device_pixel_ratio().min(2.0));
    let _ = canvas.class_list().add_1("live");
    let canvas = canvas.clone();
    match scene {
        Scene3d::Hero => hero(renderer, canvas, info),
        Scene3d::Table => table(renderer, canvas, info, liens),
        Scene3d::Jetons => jetons(renderer, canvas, info, liens),
    }
}

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

/// Si le canvas est au moins en partie à l'écran.
fn visible(canvas: &web_sys::Element) -> bool {
    let rect = canvas.get_bounding_client_rect();
    let height = web_sys::window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|h| h.as_f64())
        .unwrap_or(1000.0);
    rect.bottom() > 0.0 && rect.top() < height && rect.width() > 0.0
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

/// Des nombres au hasard pour placer et colorer les objets (xorshift).
struct Hasard(u64);

impl Hasard {
    fn new() -> Hasard {
        Hasard(((js_sys::Math::random() * u32::MAX as f64) as u64) << 16 | 1)
    }

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next() * items.len() as f32) as usize).min(items.len() - 1)]
    }
}

/// Les couleurs des jetons.
const JETONS: [u32; 6] = [0xe63946, 0x2b6fff, 0x16c784, 0x1d1f24, 0xf1efe8, 0xf2c14e];
const EMERAUDE: u32 = 0x16c784;
const OR: u32 = 0xf2c14e;

fn mat(couleur: u32, metal: f32, rugosite: f32) -> MeshStandardMaterial {
    MeshStandardMaterial::new()
        .color(couleur)
        .metalness(metal)
        .roughness(rugosite)
}

/// Un jeton : un corps coloré et un médaillon plus clair sur chaque face.
fn jeton(scene: &mut Scene, parent: ObjectId, couleur: u32) -> ObjectId {
    let g = scene.add_to(parent, Group::new());
    scene.add_to(
        g,
        Mesh::new(
            CylinderGeometry::new(0.5, 0.5, 0.11, 40),
            mat(couleur, 0.15, 0.45),
        ),
    );
    let blanc = if couleur == 0xf1efe8 {
        0x2b6fff
    } else {
        0xf1efe8
    };
    for i in 0..8 {
        let a = i as f32 / 8.0 * TAU;
        let m = scene.add_to(
            g,
            Mesh::new(BoxGeometry::new(0.12, 0.115, 0.07), mat(blanc, 0.1, 0.5)),
        );
        scene[m].position.set(a.cos() * 0.47, 0.0, a.sin() * 0.47);
        scene[m].rotation.y = -a;
    }
    let medaillon = scene.add_to(
        g,
        Mesh::new(
            CylinderGeometry::new(0.3, 0.3, 0.118, 32),
            mat(couleur, 0.35, 0.3).emissive(Color::from(couleur).scale(0.15)),
        ),
    );
    let _ = medaillon;
    g
}

/// Une carte : une face claire et un dos coloré ; rend le groupe et la pastille de couleur.
fn carte(scene: &mut Scene, parent: ObjectId, dos: u32) -> (ObjectId, ObjectId) {
    let g = scene.add_to(parent, Group::new());
    scene.add_to(
        g,
        Mesh::new(BoxGeometry::new(0.63, 0.012, 0.88), mat(0xf7f5ee, 0.0, 0.6)),
    );
    let d = scene.add_to(
        g,
        Mesh::new(BoxGeometry::new(0.57, 0.004, 0.82), mat(dos, 0.3, 0.35)),
    );
    scene[d].position.y = -0.008;
    // La couleur de la carte, au centre de la face.
    let pip = scene.add_to(
        g,
        Mesh::new(BoxGeometry::new(0.2, 0.004, 0.2), mat(0x1d1f24, 0.0, 0.5)),
    );
    scene[pip].position.y = 0.008;
    scene[pip].rotation.y = PI / 4.0;
    (g, pip)
}

// ————————————————————————————————————————————————————————————————————————————
// Le héros
// ————————————————————————————————————————————————————————————————————————————

struct Satellite {
    id: ObjectId,
    rayon: f32,
    angle: f32,
    vitesse: f32,
    hauteur: f32,
    phase: f32,
    spin: Vector3,
}

fn hero(mut renderer: WebGLRenderer, canvas: web_sys::Element, info: State<u32>) -> Running {
    let calme = if reduced_motion() { 0.15 } else { 1.0 };
    let mut hasard = Hasard::new();
    let mut scene = Scene::new();
    let monde = scene.add(Group::new());

    // Au centre : trois piles de jetons.
    let centre = scene.add_to(monde, Group::new());
    for (k, (x, z, n)) in [(-0.6f32, 0.2f32, 9usize), (0.55, 0.35, 6), (0.0, -0.55, 12)]
        .into_iter()
        .enumerate()
    {
        for i in 0..n {
            let c = jeton(&mut scene, centre, JETONS[(k * 2 + i / 3) % JETONS.len()]);
            scene[c]
                .position
                .set(x + hasard.range(-0.02, 0.02), -0.7 + i as f32 * 0.115, z);
            scene[c].rotation.y = hasard.range(0.0, TAU);
        }
    }
    // Un éventail de cartes au-dessus.
    let eventail = scene.add_to(monde, Group::new());
    scene[eventail].position.y = 1.2;
    let mut cartes_eventail = Vec::new();
    for i in 0..5 {
        let (c, pip) = carte(
            &mut scene,
            eventail,
            if i % 2 == 0 { EMERAUDE } else { 0xe63946 },
        );
        if let Some(m) = scene[pip].mesh_mut() {
            m.material.surface_mut().color =
                Color::from(if i % 2 == 0 { 0xe63946 } else { 0x1d1f24 });
        }
        scene[c].scale.set(1.5, 1.5, 1.5);
        cartes_eventail.push(c);
    }
    // Deux anneaux lumineux.
    let anneau_a = scene.add_to(
        monde,
        Mesh::new(
            TorusGeometry::new(2.9, 0.025, 8, 128),
            MeshBasicMaterial::new().color(OR),
        ),
    );
    scene[anneau_a].rotation.x = FRAC_PI_2 * 0.82;
    let anneau_b = scene.add_to(
        monde,
        Mesh::new(
            TorusGeometry::new(3.5, 0.018, 8, 128),
            MeshBasicMaterial::new().color(EMERAUDE),
        ),
    );
    scene[anneau_b].rotation.x = FRAC_PI_2 * 1.2;
    scene[anneau_b].rotation.y = 0.5;

    // Des jetons et des cartes en orbite.
    let mut satellites = Vec::new();
    for i in 0..34 {
        let id = if i % 3 == 0 {
            let (c, _) = carte(
                &mut scene,
                monde,
                hasard.pick(&[EMERAUDE, 0xe63946, 0x2b6fff]),
            );
            c
        } else {
            jeton(&mut scene, monde, hasard.pick(&JETONS))
        };
        let taille = hasard.range(0.45, 0.8);
        scene[id].scale.set(taille, taille, taille);
        satellites.push(Satellite {
            id,
            rayon: hasard.range(3.2, 6.6),
            angle: hasard.range(0.0, TAU),
            vitesse: hasard.range(0.07, 0.26) * if hasard.next() > 0.5 { 1.0 } else { -1.0 },
            hauteur: hasard.range(-1.9, 1.9),
            phase: hasard.range(0.0, TAU),
            spin: Vector3::new(
                hasard.range(-1.2, 1.2),
                hasard.range(-1.2, 1.2),
                hasard.range(-1.2, 1.2),
            ),
        });
    }

    // Un champ d'étoiles.
    let etoile = BoxGeometry::new(0.035, 0.035, 0.035);
    let ciel = scene.add(Group::new());
    for _ in 0..170 {
        let id = scene.add_to(
            ciel,
            Mesh::new(
                etoile.clone(),
                MeshBasicMaterial::new().color(hasard.pick(&[0xffffff, 0xd8f5e8, 0xffe7a8])),
            ),
        );
        let r = hasard.range(9.0, 22.0);
        let theta = hasard.range(0.0, TAU);
        let phi = hasard.range(0.2, PI - 0.2);
        scene[id].position.set(
            r * phi.sin() * theta.cos(),
            r * phi.cos(),
            r * phi.sin() * theta.sin() - 6.0,
        );
    }

    scene.add(AmbientLight::new(0xd8f5e8, 0.55));
    let soleil = scene.add(DirectionalLight::new(0xffffff, 2.8));
    scene[soleil].position.set(3.0, 5.0, 5.0);
    let or = scene.add(PointLight::new(OR, 40.0));
    scene[or].position.set(-4.0, 1.5, 2.5);
    let vert = scene.add(PointLight::new(EMERAUDE, 35.0));
    scene[vert].position.set(4.0, -2.0, 2.0);

    let mut camera = PerspectiveCamera::new(48.0, 0.0, 0.1, 100.0);
    camera.position.set(0.0, 0.0, 10.0);

    // Le pointeur, de -1 à 1 sur chaque axe.
    let pointeur = Rc::new(Cell::new((0.0f32, 0.0f32)));
    let listener = {
        let pointeur = pointeur.clone();
        let closure = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(
            move |event: web_sys::PointerEvent| {
                if let Some(window) = web_sys::window() {
                    let w = window
                        .inner_width()
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0);
                    let h = window
                        .inner_height()
                        .ok()
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0);
                    pointeur.set((
                        (event.client_x() as f64 / w * 2.0 - 1.0) as f32,
                        (event.client_y() as f64 / h * 2.0 - 1.0) as f32,
                    ));
                }
            },
        );
        web_sys::window().map(|window| {
            let _ = window
                .add_event_listener_with_callback("pointermove", closure.as_ref().unchecked_ref());
            Listener {
                event: "pointermove",
                closure,
            }
        })
    };

    let mut fps = Fps::new();
    let mut lisse = (0.0f32, 0.0f32);
    let mut defilement_lisse = 0.0f32;
    let mut decalage = 0.0f32;
    let mut avant = 0.0f64;
    let boucle = animation_loop(move |time| {
        let dt = if avant == 0.0 {
            0.016
        } else {
            ((time - avant) / 1000.0) as f32
        }
        .min(0.1);
        avant = time;
        if !visible(&canvas) {
            return;
        }
        fps.tick(time, info);
        let t = time as f32 / 1000.0 * calme;

        let (px, py) = pointeur.get();
        lisse.0 += (px - lisse.0) * (dt * 3.0).min(1.0);
        lisse.1 += (py - lisse.1) * (dt * 3.0).min(1.0);
        let defilement = web_sys::window()
            .and_then(|w| w.scroll_y().ok())
            .unwrap_or(0.0) as f32;
        defilement_lisse += (defilement - defilement_lisse) * (dt * 6.0).min(1.0);
        let s = (defilement_lisse / 900.0).min(1.5);

        camera.position.x = lisse.0 * 1.6;
        camera.position.y = -lisse.1 * 1.1 + s * 1.5;
        camera.position.z = 10.0 + s * 4.0;
        camera.look_at(Vector3::new(0.0, s * 0.8, 0.0));

        // Sur un écran large, la scène se range à droite du titre.
        let aspect = renderer.aspect();
        let cible = if aspect > 1.15 {
            (aspect - 1.0).min(0.9) * 4.2
        } else {
            0.0
        };
        decalage += (cible - decalage) * (dt * 4.0).min(1.0);
        scene[monde].position.x = decalage;
        scene[monde].position.y = s * 1.2;
        scene[monde].rotation.y = t * 0.12 + lisse.0 * 0.35 + s * 1.2;
        scene[monde].rotation.x = lisse.1 * 0.2 + 0.25;
        scene[centre].rotation.y = t * 0.4;
        scene[eventail].rotation.y = -t * 0.5;
        scene[eventail].position.y = 1.3 + (t * 1.3).sin() * 0.12;
        let ouverture = 0.8 + (t * 0.9).sin() * 0.25;
        for (i, c) in cartes_eventail.iter().enumerate() {
            let k = i as f32 - 2.0;
            let o = &mut scene[*c];
            o.rotation.set(-1.2, 0.0, k * 0.28 * ouverture);
            o.position
                .set(k * 0.32 * ouverture, (2.0 - k.abs()) * 0.05, k.abs() * 0.02);
        }
        scene[anneau_a].rotation.z = t * 0.4;
        scene[anneau_b].rotation.z = -t * 0.3;
        scene[ciel].rotation.y = t * 0.02;
        for sat in &satellites {
            let a = sat.angle + t * sat.vitesse;
            let o = &mut scene[sat.id];
            o.position.set(
                a.cos() * sat.rayon,
                sat.hauteur + (t * 0.8 + sat.phase).sin() * 0.35,
                a.sin() * sat.rayon,
            );
            o.rotation.x = t * sat.spin.x;
            o.rotation.y = t * sat.spin.y;
            o.rotation.z = t * sat.spin.z;
        }
        scene[or].position.x = (t * 0.7).cos() * 4.5;
        scene[or].position.z = (t * 0.7).sin() * 4.5;

        renderer.render(&scene, &camera);
    });
    Running {
        _loop: Some(boucle),
        _pointer: listener,
    }
}

// ————————————————————————————————————————————————————————————————————————————
// La table de la démo
// ————————————————————————————————————————————————————————————————————————————

/// Un siège : sa pile de jetons et ses deux cartes.
struct Siege {
    groupe: ObjectId,
    cartes: [ObjectId; 2],
}

/// Une carte du board et son animation.
struct CarteBoard {
    groupe: ObjectId,
    pip: ObjectId,
    progres: f32,
}

/// Où se trouve le siège `i` d'une table de `n` joueurs (le siège 0 est face à la caméra).
fn place(i: usize, n: usize) -> (f32, f32, f32) {
    let a = FRAC_PI_2 + i as f32 / n as f32 * TAU;
    (a.cos() * 3.55, a.sin() * 2.05, a)
}

fn douce(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

fn table(
    mut renderer: WebGLRenderer,
    canvas: web_sys::Element,
    info: State<u32>,
    l: Liens,
) -> Running {
    let mut hasard = Hasard::new();
    let mut scene = Scene::new();

    // La table : un tapis ovale, un rail, un pied.
    let tapis = scene.add(Mesh::new(
        CylinderGeometry::new(1.0, 1.0, 0.2, 72),
        mat(0x0d6b45, 0.0, 0.95),
    ));
    scene[tapis].scale.set(4.3, 1.0, 2.6);
    let ligne = scene.add(Mesh::new(
        TorusGeometry::new(1.0, 0.012, 6, 96),
        MeshBasicMaterial::new()
            .color(OR)
            .transparent(true)
            .opacity(0.6),
    ));
    scene[ligne].scale.set(3.2, 1.75, 1.0);
    scene[ligne].rotation.x = FRAC_PI_2;
    scene[ligne].position.y = 0.105;
    let rail = scene.add(Mesh::new(
        TorusGeometry::new(1.0, 0.07, 16, 120),
        mat(0x1a1410, 0.4, 0.35),
    ));
    scene[rail].scale.set(4.38, 2.68, 2.2);
    scene[rail].rotation.x = FRAC_PI_2;
    scene[rail].position.y = 0.1;
    let pied = scene.add(Mesh::new(
        CylinderGeometry::new(1.2, 1.7, 1.6, 48),
        mat(0x111312, 0.6, 0.3),
    ));
    scene[pied].position.y = -0.9;

    // Le pot au centre.
    let pot = scene.add(Group::new());
    scene[pot].position.set(0.0, 0.1, -0.85);
    for i in 0..10 {
        let c = jeton(&mut scene, pot, JETONS[i % JETONS.len()]);
        scene[c].scale.set(0.42, 0.42, 0.42);
        let pile = i / 4;
        scene[c].position.set(
            pile as f32 * 0.45 - 0.45,
            0.03 + (i % 4) as f32 * 0.05,
            (pile as f32 * 1.7).sin() * 0.1,
        );
    }

    // Neuf sièges : seuls ceux du format choisi se montrent.
    let mut sieges = Vec::new();
    for i in 0..9 {
        let groupe = scene.add(Group::new());
        let hauteur = 3 + (hasard.next() * 8.0) as usize;
        for k in 0..hauteur {
            let c = jeton(&mut scene, groupe, JETONS[(i + k / 3) % JETONS.len()]);
            scene[c].scale.set(0.4, 0.4, 0.4);
            scene[c]
                .position
                .set(hasard.range(-0.01, 0.01), 0.13 + k as f32 * 0.046, 0.0);
        }
        let mut cs = Vec::with_capacity(2);
        for j in 0..2 {
            let (g, _) = carte(&mut scene, groupe, if i == 0 { EMERAUDE } else { 0xb3263a });
            scene[g].position.set(-0.55 + j as f32 * 0.38, 0.13, 0.0);
            scene[g].rotation.y = (j as f32 - 0.5) * 0.25;
            scene[g].scale.set(0.8, 0.8, 0.8);
            cs.push(g);
        }
        let cs = [cs[0], cs[1]];
        let (x, z, _) = place(i, 9);
        scene[groupe].position.set(x, 0.0, z);
        sieges.push(Siege { groupe, cartes: cs });
    }
    // Le bouton du donneur.
    let bouton = scene.add(Mesh::new(
        CylinderGeometry::new(0.18, 0.18, 0.06, 32),
        mat(0xf7f5ee, 0.1, 0.4),
    ));

    // Le board : cinq cartes, qui partent du paquet.
    let paquet = Vector3::new(1.9, 0.14, -1.0);
    let mut board = Vec::new();
    let tas = scene.add(Group::new());
    for _ in 0..5 {
        let (groupe, pip) = carte(&mut scene, tas, EMERAUDE);
        scene[groupe].scale.set(0.95, 0.95, 0.95);
        board.push(CarteBoard {
            groupe,
            pip,
            progres: 0.0,
        });
    }

    scene.add(HemisphereLight::new(0xeafff4, 0x0b1a12, 1.3));
    let cle = scene.add(DirectionalLight::new(0xffffff, 2.6));
    scene[cle].position.set(3.0, 8.0, 5.0);
    let lampe = scene.add(PointLight::new(0xffe7a8, 60.0));
    scene[lampe].position.set(0.0, 4.5, 0.0);

    let mut camera = PerspectiveCamera::new(40.0, 0.0, 0.1, 100.0);
    camera.position.set(0.0, 6.2, 8.4);
    camera.look_at(Vector3::new(0.0, 0.0, 0.3));
    let mut controls = OrbitControls::new(&canvas);
    controls.target = Vector3::new(0.0, 0.0, 0.3);
    controls.enable_damping = true;
    controls.enable_pan = false;
    controls.min_distance = 6.0;
    controls.max_distance = 16.0;
    controls.max_polar_angle = FRAC_PI_2 * 0.88;
    controls.auto_rotate = !reduced_motion();
    controls.auto_rotate_speed = 0.35;

    let mut fps = Fps::new();
    let mut n_lisse = 6.0f32;
    let mut donne_vue = l.donne.get();
    let mut avant = 0.0f64;
    let boucle = animation_loop(move |time| {
        let dt = if avant == 0.0 {
            0.016
        } else {
            ((time - avant) / 1000.0) as f32
        }
        .min(0.1);
        avant = time;
        if !visible(&canvas) {
            return;
        }
        fps.tick(time, info);
        let t = time as f32 / 1000.0;
        let n = FORMATS[l.format.get().min(FORMATS.len() - 1)].1;
        n_lisse += (n as f32 - n_lisse) * (dt * 5.0).min(1.0);

        // Les sièges glissent à leur place quand le format change.
        for (i, s) in sieges.iter().enumerate() {
            let present = i < n;
            let (x, z, a) = place(i, n_lisse.max(2.0).round() as usize);
            let (cx, cz) = if present { (x, z) } else { (0.0, 0.0) };
            let o = &mut scene[s.groupe];
            o.visible = present || o.scale.x > 0.05;
            o.position.x += (cx - o.position.x) * (dt * 6.0).min(1.0);
            o.position.z += (cz - o.position.z) * (dt * 6.0).min(1.0);
            let e = if present { 1.0 } else { 0.0 };
            let sc = o.scale.x + (e - o.scale.x) * (dt * 6.0).min(1.0);
            o.scale.set(sc, sc, sc);
            o.rotation.y = -a + FRAC_PI_2;
            // Les cartes du joueur (siège 0) sont face visible ; les autres de dos.
            for c in s.cartes {
                scene[c].rotation.z = if i == 0 { 0.0 } else { PI };
            }
        }
        let (bx, bz, _) = place(n.saturating_sub(1), n);
        scene[bouton].position.set(bx * 0.72, 0.13, bz * 0.72);

        // Une nouvelle main : les cartes du board retournent au paquet.
        let donne = l.donne.get();
        if donne != donne_vue {
            donne_vue = donne;
            for c in &mut board {
                c.progres = 0.0;
            }
        }
        let cartes = l.cartes.get();
        let visibles = [0usize, 3, 4, 5][l.etape.get().min(3) as usize];
        for (i, c) in board.iter_mut().enumerate() {
            let cible = if i < visibles { 1.0 } else { 0.0 };
            // Les cartes partent l'une après l'autre.
            let vitesse = if cible > c.progres { 2.2 } else { 4.0 };
            c.progres += (cible - c.progres).signum() * dt * vitesse;
            c.progres = c.progres.clamp(0.0, 1.0);
            let e = douce(c.progres);
            let fin = Vector3::new(-1.36 + i as f32 * 0.68, 0.13, 0.25);
            let o = &mut scene[c.groupe];
            o.visible = c.progres > 0.01;
            o.position = paquet.lerp(fin, e);
            o.position.y += (e * PI).sin() * 0.6;
            o.rotation.set(0.0, (1.0 - e) * 0.8, (1.0 - e) * PI);
            let rouge = cartes
                .get(2 + i)
                .and_then(|id| stratyx_poker::Card::from_id(*id))
                .is_some_and(|c| matches!(c.to_string().chars().nth(1), Some('h' | 'd')));
            if let Some(m) = scene[c.pip].mesh_mut() {
                m.material.surface_mut().color =
                    Color::from(if rouge { 0xe63946 } else { 0x1d1f24 });
            }
        }
        // Le joueur : ses deux cartes portent leur couleur.
        for (j, c) in sieges[0].cartes.iter().enumerate() {
            let rouge = cartes
                .get(j)
                .and_then(|id| stratyx_poker::Card::from_id(*id))
                .is_some_and(|c| matches!(c.to_string().chars().nth(1), Some('h' | 'd')));
            let enfants: Vec<ObjectId> = scene[*c].children().to_vec();
            if let Some(pip) = enfants.get(2)
                && let Some(m) = scene[*pip].mesh_mut()
            {
                m.material.surface_mut().color =
                    Color::from(if rouge { 0xe63946 } else { 0x1d1f24 });
            }
        }
        scene[pot].rotation.y = t * 0.2;
        controls.update(&mut camera);
        renderer.render(&scene, &camera);
    });
    Running {
        _loop: Some(boucle),
        _pointer: None,
    }
}

// ————————————————————————————————————————————————————————————————————————————
// La pluie de jetons
// ————————————————————————————————————————————————————————————————————————————

const MAX_JETONS: usize = 160;

fn jetons(
    mut renderer: WebGLRenderer,
    canvas: web_sys::Element,
    info: State<u32>,
    l: Liens,
) -> Running {
    let mut hasard = Hasard::new();
    let mut scene = Scene::new();
    let sol = scene.add(Mesh::new(
        PlaneGeometry::new(14.0, 14.0),
        mat(0x0d5c3b, 0.0, 0.95),
    ));
    scene[sol].rotation.x = -FRAC_PI_2;

    scene.add(HemisphereLight::new(0xeafff4, 0x0b1a12, 1.4));
    let soleil = scene.add(DirectionalLight::new(0xffffff, 2.8));
    scene[soleil].position.set(5.0, 10.0, 6.0);
    let or = scene.add(PointLight::new(OR, 45.0));
    scene[or].position.set(-5.0, 4.0, 3.0);

    let mut camera = PerspectiveCamera::new(45.0, 0.0, 0.1, 100.0);
    camera.position.set(9.0, 7.5, 11.0);
    camera.look_at(Vector3::new(0.0, 1.0, 0.0));
    let mut controls = OrbitControls::new(&canvas);
    controls.target = Vector3::new(0.0, 1.0, 0.0);
    controls.enable_damping = true;
    controls.enable_pan = false;
    controls.min_distance = 8.0;
    controls.max_distance = 26.0;
    controls.max_polar_angle = FRAC_PI_2 * 0.92;
    controls.auto_rotate = !reduced_motion();
    controls.auto_rotate_speed = 0.8;

    let forme = CylinderGeometry::new(0.5, 0.5, 0.14, 32);
    let mut physics = Physics::new();
    let mut liste: Vec<ObjectId> = Vec::new();
    let mut a_lacher = 24u32;
    let mut lache_vu = l.lacher.get();
    let mut secousse_vue = l.secousse.get();
    let mut minuterie = 0.0f32;
    let mut avant = 0.0f64;
    let boucle = animation_loop(move |time| {
        let dt = if avant == 0.0 {
            0.016
        } else {
            ((time - avant) / 1000.0) as f32
        }
        .min(0.05);
        avant = time;
        if !visible(&canvas) {
            return;
        }
        let demande = l.lacher.get();
        if demande != lache_vu {
            a_lacher += demande.saturating_sub(lache_vu);
            lache_vu = demande;
        }
        // Un jeton toutes les 50 ms, en pluie.
        minuterie -= dt;
        if a_lacher > 0 && minuterie <= 0.0 {
            minuterie = 0.05;
            a_lacher -= 1;
            let couleur = hasard.pick(&JETONS);
            let id = scene.add(Mesh::new(
                forme.clone(),
                mat(couleur, 0.2, 0.4).emissive(Color::from(couleur).scale(0.1)),
            ));
            let o = &mut scene[id];
            o.position.set(
                hasard.range(-2.0, 2.0),
                hasard.range(7.0, 10.0),
                hasard.range(-2.0, 2.0),
            );
            o.rotation.set(
                hasard.range(0.0, TAU),
                hasard.range(0.0, TAU),
                hasard.range(0.0, TAU),
            );
            liste.push(id);
            if liste.len() > MAX_JETONS {
                let vieux = liste.remove(0);
                scene.remove(vieux);
            }
        }
        physics.step(&mut scene, dt);
        let secousse = l.secousse.get();
        if secousse != secousse_vue {
            secousse_vue = secousse;
            for id in &liste {
                if let Some(body) = physics.body_mut(*id) {
                    body.wake_up();
                    let m = body.mass();
                    body.apply_impulse(
                        Vector3::new(
                            hasard.range(-2.0, 2.0),
                            hasard.range(5.0, 8.0),
                            hasard.range(-2.0, 2.0),
                        ) * m,
                    );
                }
            }
        }
        // Ceux qui tombent de la table disparaissent.
        liste.retain(|id| {
            let tombe = scene.get(*id).is_some_and(|o| o.position.y < -12.0);
            if tombe {
                scene.remove(*id);
            }
            !tombe
        });
        if info.get() != liste.len() as u32 {
            info.set(liste.len() as u32);
        }
        controls.update(&mut camera);
        renderer.render(&scene, &camera);
    });
    Running {
        _loop: Some(boucle),
        _pointer: None,
    }
}
