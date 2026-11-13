//! Les mouvements de la page, en Rust (navigateur seulement) : apparitions au défilement,
//! compteurs, barre de progression, en-tête, frise, menu mobile, filtres, retour en haut,
//! PWA (service worker, mise à jour, hors ligne, installation), cartes qui se soulèvent,
//! boutons magnétiques et curseur. Tout est en 2D.
//!
//! Lancé une fois par page par l'îlot `mouvement` (src/lib.rs), présent sur chaque page :
//! une seule boucle d'animation (`crate::anim::boucle`) fait tout ce qui bouge image par
//! image.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    AddEventListenerOptions, Document, Element, Event, EventTarget, HtmlElement,
    IntersectionObserver, IntersectionObserverEntry, IntersectionObserverInit, KeyboardEvent,
    MouseEvent, MutationObserver, MutationObserverInit, ScrollBehavior, ScrollToOptions,
    ServiceWorkerRegistration, ServiceWorkerState, Window,
};

thread_local! {
    static LANCE: Cell<bool> = const { Cell::new(false) };
}

/// Ce que l'on observe pour le faire apparaître.
const A_REVELER: &str = "[data-reveal]:not(.is-in), .split:not(.is-in), .step:not(.is-in)";

fn fenetre() -> Window {
    web_sys::window().expect("une fenêtre")
}

fn document() -> Document {
    fenetre().document().expect("un document")
}

/// Les éléments qui répondent à `selecteur`, dans `racine` ou dans tout le document.
fn tous(selecteur: &str, racine: Option<&Element>) -> Vec<Element> {
    let liste = match racine {
        Some(r) => r.query_selector_all(selecteur),
        None => document().query_selector_all(selecteur),
    };
    let mut out = Vec::new();
    if let Ok(liste) = liste {
        for i in 0..liste.length() {
            if let Some(e) = liste.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                out.push(e);
            }
        }
    }
    out
}

fn un(selecteur: &str) -> Option<Element> {
    document().query_selector(selecteur).ok().flatten()
}

fn media(requete: &str) -> bool {
    fenetre()
        .match_media(requete)
        .ok()
        .flatten()
        .is_some_and(|m| m.matches())
}

fn prop(el: &Element, nom: &str, valeur: &str) {
    if let Some(h) = el.dyn_ref::<HtmlElement>() {
        let _ = h.style().set_property(nom, valeur);
    }
}

fn classe(el: &Element, nom: &str, oui: bool) {
    let _ = el.class_list().toggle_with_force(nom, oui);
}

/// Écoute `evenement` sur `cible` pour toute la vie de la page.
fn ecouter(cible: &EventTarget, evenement: &str, passif: bool, f: impl FnMut(Event) + 'static) {
    let closure = Closure::<dyn FnMut(Event)>::new(f);
    let options = AddEventListenerOptions::new();
    options.set_passive(passif);
    let _ = cible.add_event_listener_with_callback_and_add_event_listener_options(
        evenement,
        closure.as_ref().unchecked_ref(),
        &options,
    );
    closure.forget();
}

fn pointeur(e: &Event) -> (f64, f64) {
    e.dyn_ref::<MouseEvent>()
        .map(|m| (m.client_x() as f64, m.client_y() as f64))
        .unwrap_or((0.0, 0.0))
}

fn taille_fenetre() -> (f64, f64) {
    let w = fenetre();
    (
        w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1.0),
        w.inner_height()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0),
    )
}

/// Un compteur qui monte jusqu'à sa valeur.
struct Compteur {
    el: Element,
    fin: f64,
    debut: Option<f64>,
}

/// Ce que la boucle d'animation partage avec les écouteurs.
#[derive(Default)]
struct Etat {
    defiler: Cell<bool>,
    rescanner: Cell<bool>,
    compteurs: RefCell<Vec<Compteur>>,
    souris: Cell<(f64, f64)>,
}

/// Lance les mouvements de la page (une seule fois).
pub fn demarrer() {
    if LANCE.with(|l| l.replace(true)) {
        return;
    }
    let html = document().document_element().expect("<html>");
    let _ = html.class_list().add_1("motion");
    let reduit = media("(prefers-reduced-motion: reduce)");
    let precis = media("(pointer: fine)");
    let etat = Rc::new(Etat {
        souris: Cell::new((-100.0, -100.0)),
        ..Etat::default()
    });

    let observateur = apparitions(etat.clone(), reduit);
    scanner(&observateur, precis && !reduit);
    surveiller_le_dom(etat.clone());

    let fenetre_cible: EventTarget = fenetre().into();
    {
        let etat = etat.clone();
        ecouter(&fenetre_cible, "scroll", true, move |_| {
            etat.defiler.set(true)
        });
    }
    {
        let etat = etat.clone();
        ecouter(&fenetre_cible, "resize", true, move |_| {
            etat.defiler.set(true)
        });
    }
    etat.defiler.set(true);

    menu(&html);
    retour_en_haut(reduit);
    filtres();
    installation();
    service_worker();
    hors_ligne();
    if precis && !reduit {
        curseur(etat.clone());
    }

    // La boucle : défilement, compteurs, nouveaux éléments, anneau du curseur.
    let anneau = un(".cursor-ring");
    let mut anneau_pos = (-100.0f64, -100.0f64);
    let mut dernier_y = 0.0f64;
    let boucle = crate::anim::boucle(move |temps| {
        if etat.rescanner.replace(false) {
            scanner(&observateur, precis && !reduit);
        }
        if etat.defiler.replace(false) {
            dernier_y = defilement(&html, dernier_y);
        }
        etat.compteurs.borrow_mut().retain_mut(|c| {
            let debut = *c.debut.get_or_insert(temps);
            let x = ((temps - debut) / 1800.0).clamp(0.0, 1.0);
            let x = if reduit { 1.0 } else { x };
            let v = (c.fin * (1.0 - (1.0 - x).powi(4))).round();
            c.el.set_text_content(Some(&v.to_string()));
            x < 1.0
        });
        if let Some(anneau) = &anneau {
            let (mx, my) = etat.souris.get();
            anneau_pos.0 += (mx - anneau_pos.0) * 0.18;
            anneau_pos.1 += (my - anneau_pos.1) * 0.18;
            prop(
                anneau,
                "transform",
                &format!("translate({:.1}px, {:.1}px)", anneau_pos.0, anneau_pos.1),
            );
        }
    });
    // La page entière vit avec elle.
    std::mem::forget(boucle);
}

/// L'observateur qui fait apparaître les éléments et lance les compteurs.
fn apparitions(etat: Rc<Etat>, reduit: bool) -> IntersectionObserver {
    let rappel = Closure::<dyn FnMut(js_sys::Array, IntersectionObserver)>::new(
        move |entrees: js_sys::Array, obs: IntersectionObserver| {
            for entree in entrees.iter() {
                let Ok(entree) = entree.dyn_into::<IntersectionObserverEntry>() else {
                    continue;
                };
                if !entree.is_intersecting() {
                    continue;
                }
                let el = entree.target();
                let _ = el.class_list().add_1("is-in");
                obs.unobserve(&el);
                let mut compteurs = tous("[data-count]", Some(&el));
                if el.has_attribute("data-count") {
                    compteurs.push(el.clone());
                }
                for c in compteurs {
                    let fin = c
                        .get_attribute("data-count")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    if reduit {
                        c.set_text_content(Some(&fin.to_string()));
                    } else {
                        etat.compteurs.borrow_mut().push(Compteur {
                            el: c,
                            fin,
                            debut: None,
                        });
                    }
                }
            }
        },
    );
    let options = IntersectionObserverInit::new();
    options.set_root_margin("0px 0px -8% 0px");
    options.set_threshold(&JsValue::from_f64(0.12));
    let obs = IntersectionObserver::new_with_options(rappel.as_ref().unchecked_ref(), &options)
        .expect("IntersectionObserver");
    rappel.forget();
    obs
}

/// Observe ce qui doit apparaître et branche les effets de pointeur sur les nouveaux
/// éléments (les îlots en ajoutent en s'hydratant).
fn scanner(obs: &IntersectionObserver, pointeur_precis: bool) {
    for el in tous(A_REVELER, None) {
        obs.observe(&el);
    }
    if pointeur_precis {
        for el in tous(".tilt:not([data-bound]), .glow:not([data-bound])", None) {
            incliner(el);
        }
        for el in tous(".magnetic:not([data-mag])", None) {
            aimanter(el);
        }
    }
}

/// Rescanne quand des éléments arrivent (au plus une fois par image).
fn surveiller_le_dom(etat: Rc<Etat>) {
    let rappel = Closure::<dyn FnMut(js_sys::Array, MutationObserver)>::new(
        move |_: js_sys::Array, _: MutationObserver| etat.rescanner.set(true),
    );
    if let (Ok(obs), Some(body)) = (
        MutationObserver::new(rappel.as_ref().unchecked_ref()),
        document().body(),
    ) {
        let options = MutationObserverInit::new();
        options.set_child_list(true);
        options.set_subtree(true);
        let _ = obs.observe_with_options(&body, &options);
        std::mem::forget(obs);
    }
    rappel.forget();
}

/// Barre de progression, en-tête, frise et grand nom du pied de page ; rend le nouveau `y`.
fn defilement(html: &Element, dernier: f64) -> f64 {
    let w = fenetre();
    let y = w.scroll_y().unwrap_or(0.0);
    let (_, hauteur) = taille_fenetre();
    let total = html.scroll_height() as f64 - hauteur;
    if let Some(p) = un(".progress") {
        let v = if total > 0.0 { y / total } else { 0.0 };
        prop(&p, "--p", &format!("{v:.4}"));
    }
    if let Some(header) = un(".site-header") {
        classe(&header, "scrolled", y > 20.0);
        let menu = html.class_list().contains("menu-open");
        if !menu && y > 400.0 && y > dernier + 4.0 {
            classe(&header, "hide", true);
        } else if y < dernier - 4.0 || y <= 400.0 || menu {
            classe(&header, "hide", false);
        }
    }
    if let (Some(ligne), Some(frise)) = (un(".timeline-line"), un(".timeline")) {
        let r = frise.get_bounding_client_rect();
        let t = ((hauteur * 0.6 - r.top()) / r.height().max(1.0)).clamp(0.0, 1.0);
        prop(&ligne, "--t", &format!("{t:.3}"));
    }
    if let Some(mot) = un(".footer-word") {
        let r = mot.get_bounding_client_rect();
        let f = (1.0 - (r.top() - hauteur * 0.55) / (hauteur * 0.45)).clamp(0.0, 1.0);
        prop(&mot, "--fw", &format!("{f:.3}"));
    }
    y
}

/// Le menu mobile.
fn menu(html: &Element) {
    let Some(burger) = un(".burger") else {
        return;
    };
    let fermer = {
        let html = html.clone();
        let burger = burger.clone();
        move || {
            classe(&html, "menu-open", false);
            let _ = burger.set_attribute("aria-expanded", "false");
            let _ = burger.set_attribute("aria-label", "Ouvrir le menu");
        }
    };
    {
        let html = html.clone();
        let b = burger.clone();
        ecouter(&burger, "click", true, move |_| {
            let ouvert = html.class_list().toggle("menu-open").unwrap_or(false);
            let _ = b.set_attribute("aria-expanded", &ouvert.to_string());
            let _ = b.set_attribute(
                "aria-label",
                if ouvert {
                    "Fermer le menu"
                } else {
                    "Ouvrir le menu"
                },
            );
        });
    }
    for lien in tous(".nav a", None) {
        let fermer = fermer.clone();
        ecouter(&lien, "click", true, move |_| fermer());
    }
    ecouter(&fenetre().into(), "keydown", true, move |e| {
        if e.dyn_ref::<KeyboardEvent>()
            .is_some_and(|k| k.key() == "Escape")
        {
            fermer();
        }
    });
}

fn retour_en_haut(reduit: bool) {
    for bouton in tous(".to-top", None) {
        ecouter(&bouton, "click", true, move |_| {
            let options = ScrollToOptions::new();
            options.set_top(0.0);
            options.set_behavior(if reduit {
                ScrollBehavior::Auto
            } else {
                ScrollBehavior::Smooth
            });
            fenetre().scroll_to_with_scroll_to_options(&options);
        });
    }
}

/// Les filtres des réalisations.
fn filtres() {
    let boutons = tous("[data-filter]", None);
    for bouton in &boutons {
        let boutons = boutons.clone();
        let ce = bouton.clone();
        ecouter(bouton, "click", true, move |_| {
            let filtre = ce.get_attribute("data-filter").unwrap_or_default();
            for b in &boutons {
                let _ = b.set_attribute("aria-pressed", &(b == &ce).to_string());
            }
            for (i, projet) in tous(".project[data-cat]", None).into_iter().enumerate() {
                let montre = filtre == "*"
                    || projet.get_attribute("data-cat").as_deref() == Some(filtre.as_str());
                if montre {
                    classe(&projet, "hidden", false);
                    classe(&projet, "is-in", false);
                    prop(&projet, "--d", &format!("{}ms", (i % 3) * 90));
                    // Lire la taille force le navigateur à repartir de l'état caché.
                    let _ = projet.get_bounding_client_rect();
                    classe(&projet, "is-in", true);
                } else {
                    classe(&projet, "hidden", true);
                }
            }
        });
    }
}

/// Montre ou cache un message de la PWA (`.toast-maj`, `.toast-hors-ligne`…).
fn toast(selecteur: &str, oui: bool) {
    for t in tous(selecteur, None) {
        if let Some(h) = t.dyn_ref::<HtmlElement>() {
            h.set_hidden(!oui);
        }
    }
}

/// Une propriété JavaScript booléenne (`navigator.standalone`…).
fn vrai(objet: &JsValue, nom: &str) -> bool {
    js_sys::Reflect::get(objet, &JsValue::from_str(nom))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Le bouton « Installer l'application », montré quand le navigateur le propose. Sur
/// iPhone et iPad (pas d'invite d'installation), il explique comment faire.
fn installation() {
    let invite: Rc<RefCell<Option<Event>>> = Rc::default();
    let montrer = |oui: bool| {
        for b in tous(".install-app", None) {
            if let Some(h) = b.dyn_ref::<HtmlElement>() {
                h.set_hidden(!oui);
            }
        }
    };
    let installee = media("(display-mode: standalone)")
        || media("(display-mode: window-controls-overlay)")
        || vrai(&fenetre().navigator(), "standalone");
    if installee {
        if let Some(html) = document().document_element() {
            classe(&html, "standalone", true);
        }
        return;
    }
    let agent = fenetre().navigator().user_agent().unwrap_or_default();
    let ios = agent.contains("iPhone") || agent.contains("iPad") || agent.contains("iPod");
    if ios {
        montrer(true);
    }
    let fenetre_cible: EventTarget = fenetre().into();
    {
        let invite = invite.clone();
        ecouter(&fenetre_cible, "beforeinstallprompt", false, move |e| {
            e.prevent_default();
            *invite.borrow_mut() = Some(e);
            montrer(true);
        });
    }
    ecouter(&fenetre_cible, "appinstalled", true, move |_| {
        montrer(false);
        toast(".toast-ios", false);
    });
    for bouton in tous(".install-app", None) {
        let invite = invite.clone();
        ecouter(&bouton, "click", true, move |_| {
            if let Some(e) = invite.borrow_mut().take()
                && let Ok(prompt) = js_sys::Reflect::get(&e, &JsValue::from_str("prompt"))
                && let Some(prompt) = prompt.dyn_ref::<js_sys::Function>()
            {
                let _ = prompt.call0(&e);
                montrer(false);
            } else if ios {
                toast(".toast-ios", true);
            }
        });
    }
    for fermer in tous("[data-toast-fermer]", None) {
        let ce = fermer.clone();
        ecouter(&fermer, "click", true, move |_| {
            if let Some(t) = ce.closest(".toast").ok().flatten()
                && let Some(h) = t.dyn_ref::<HtmlElement>()
            {
                h.set_hidden(true);
            }
        });
    }
}

/// Enregistre le service worker (/sw.js) et propose la mise à jour quand une nouvelle
/// version attend : le bouton lui demande de prendre la main, puis la page se recharge.
fn service_worker() {
    let navigateur = fenetre().navigator();
    if !js_sys::Reflect::has(&navigateur, &JsValue::from_str("serviceWorker")).unwrap_or(false) {
        return;
    }
    let conteneur = navigateur.service_worker();
    let demandee = Rc::new(Cell::new(false));
    {
        let demandee = demandee.clone();
        ecouter(
            &conteneur.clone().into(),
            "controllerchange",
            true,
            move |_| {
                if demandee.get() {
                    let _ = fenetre().location().reload();
                }
            },
        );
    }
    let enregistrement: Rc<RefCell<Option<ServiceWorkerRegistration>>> = Rc::default();
    for bouton in tous("[data-pwa-maj]", None) {
        let enregistrement = enregistrement.clone();
        let demandee = demandee.clone();
        ecouter(&bouton, "click", true, move |_| {
            let attente = enregistrement.borrow().as_ref().and_then(|r| r.waiting());
            match attente {
                Some(sw) => {
                    demandee.set(true);
                    let message = js_sys::Object::new();
                    let _ = js_sys::Reflect::set(
                        &message,
                        &JsValue::from_str("type"),
                        &JsValue::from_str("SKIP_WAITING"),
                    );
                    let _ = sw.post_message(&message);
                }
                None => {
                    let _ = fenetre().location().reload();
                }
            }
            toast(".toast-maj", false);
        });
    }
    let suivre = Closure::<dyn FnMut(JsValue)>::new(move |valeur: JsValue| {
        let Ok(reg) = valeur.dyn_into::<ServiceWorkerRegistration>() else {
            return;
        };
        let controle = fenetre()
            .navigator()
            .service_worker()
            .controller()
            .is_some();
        if reg.waiting().is_some() && controle {
            toast(".toast-maj", true);
        }
        let r = reg.clone();
        ecouter(&reg.clone().into(), "updatefound", true, move |_| {
            let Some(nouveau) = r.installing() else {
                return;
            };
            let n = nouveau.clone();
            ecouter(&nouveau.into(), "statechange", true, move |_| {
                if n.state() == ServiceWorkerState::Installed
                    && fenetre()
                        .navigator()
                        .service_worker()
                        .controller()
                        .is_some()
                {
                    toast(".toast-maj", true);
                }
            });
        });
        // Une version plus récente ? On vérifie quand la page redevient visible.
        let r = reg.clone();
        ecouter(&document().into(), "visibilitychange", true, move |_| {
            if document().visibility_state() == web_sys::VisibilityState::Visible {
                let _ = r.update();
            }
        });
        *enregistrement.borrow_mut() = Some(reg);
    });
    let _ = conteneur.register("/sw.js").then(&suivre);
    suivre.forget();
}

/// Un message quand la connexion tombe (les outils continuent de marcher), et quand elle
/// revient.
fn hors_ligne() {
    let fenetre_cible: EventTarget = fenetre().into();
    if !fenetre().navigator().on_line() {
        toast(".toast-hors-ligne", true);
    }
    ecouter(&fenetre_cible, "offline", true, move |_| {
        toast(".toast-en-ligne", false);
        toast(".toast-hors-ligne", true);
    });
    ecouter(&fenetre_cible, "online", true, move |_| {
        toast(".toast-hors-ligne", false);
        toast(".toast-en-ligne", true);
        let rappel = Closure::once_into_js(|| toast(".toast-en-ligne", false));
        let _ = fenetre()
            .set_timeout_with_callback_and_timeout_and_arguments_0(rappel.unchecked_ref(), 3000);
    });
}

/// Une carte qui se soulève au survol (en 2D), avec une lueur qui suit le pointeur.
fn incliner(el: Element) {
    let _ = el.set_attribute("data-bound", "1");
    {
        let ce = el.clone();
        ecouter(&el, "pointermove", true, move |e| {
            let (px, py) = pointeur(&e);
            let r = ce.get_bounding_client_rect();
            let x = (px - r.left()) / r.width().max(1.0);
            let y = (py - r.top()) / r.height().max(1.0);
            prop(&ce, "--mx", &format!("{:.1}%", x * 100.0));
            prop(&ce, "--my", &format!("{:.1}%", y * 100.0));
        });
    }
}

/// Un bouton attiré par le pointeur.
fn aimanter(el: Element) {
    let _ = el.set_attribute("data-mag", "1");
    {
        let ce = el.clone();
        ecouter(&el, "pointermove", true, move |e| {
            let (px, py) = pointeur(&e);
            let r = ce.get_bounding_client_rect();
            let x = px - r.left() - r.width() / 2.0;
            let y = py - r.top() - r.height() / 2.0;
            prop(
                &ce,
                "transform",
                &format!("translate({:.1}px, {:.1}px)", x * 0.25, y * 0.3),
            );
        });
    }
    let ce = el.clone();
    ecouter(&el, "pointerleave", true, move |_| {
        if let Some(h) = ce.dyn_ref::<HtmlElement>() {
            let _ = h.style().remove_property("transform");
        }
    });
}

/// Le curseur : un point sous la souris, et un anneau qui le suit (dans la boucle).
fn curseur(etat: Rc<Etat>) {
    let (Some(curseur), Some(point)) = (un(".cursor"), un(".cursor-dot")) else {
        return;
    };
    let fenetre_cible: EventTarget = fenetre().into();
    {
        let curseur = curseur.clone();
        ecouter(&fenetre_cible, "pointermove", true, move |e| {
            let (x, y) = pointeur(&e);
            etat.souris.set((x, y));
            classe(&curseur, "on", true);
            prop(&point, "transform", &format!("translate({x}px, {y}px)"));
            let survol = e
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .and_then(|t| {
                    t.closest("a, button, summary, label, input, select, textarea, canvas")
                        .ok()
                        .flatten()
                })
                .is_some();
            classe(&curseur, "hover", survol);
        });
    }
    {
        let c = curseur.clone();
        ecouter(&fenetre_cible, "pointerdown", true, move |_| {
            classe(&c, "down", true)
        });
    }
    {
        let c = curseur.clone();
        ecouter(&fenetre_cible, "pointerup", true, move |_| {
            classe(&c, "down", false)
        });
    }
    ecouter(&document().into(), "pointerleave", true, move |_| {
        classe(&curseur, "on", false)
    });
}
