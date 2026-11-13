//! L'îlot `capture` de la page `/app` : filmer sa session (écran ou caméra), importer une
//! vidéo ou des captures d'écran, puis — une fois la session close — retrouver les cartes,
//! les corriger au besoin (la reconnaissance apprend le jeu de cartes de la salle) et
//! analyser chaque spot avec l'IA du format.
//!
//! **Garde-fou (décision D6)** : pendant un enregistrement, rien n'est reconnu à l'écran et
//! aucun conseil n'est affiché ; l'analyse ne commence qu'à l'arrêt. Tout reste sur
//! l'appareil.

use paquets::prelude::*;
use stratyx_analytics::pct;
use stratyx_analytics::spot::{Situation, SpotAdvice, SpotInput, analyse_spot};
use stratyx_poker::position::Position;
use stratyx_poker::{Card, HoleCards, Rank, Specialist, Suit};
use stratyx_strategy::{Basis, Decision};
use stratyx_vision::{
    DetectedCard, Frame, Recognized, TableRead, Templates, arrange, detect_cards, recognize,
};

use crate::carte_html;

/// Les actions des boutons (démarrer, arrêter, importer une vidéo, des images).
type Actions = (
    std::rc::Rc<dyn Fn(bool)>,
    std::rc::Rc<dyn Fn()>,
    std::rc::Rc<dyn Fn(paquets::active::Event)>,
    std::rc::Rc<dyn Fn(paquets::active::Event)>,
);

/// Une image gardée de la session.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageCle {
    pub t_ms: u64,
    pub vignette: String,
    pub largeur: u32,
    pub hauteur: u32,
    pub cartes: Vec<DetectedCard>,
    pub lecture: TableRead,
}

// Certains états et réglages ne servent que dans le navigateur.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Etat {
    Pret,
    Enregistrement,
    Lecture,
    Revue,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const CLE_MODELES: &str = "stratyx.vision.modeles";
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const MAX_IMAGES: usize = 400;
/// Sous ce seuil, une carte reconnue est signalée comme douteuse.
const DOUTE: f32 = 0.6;

/// Analyse une image (pixels RGBA) : cartes et disposition.
#[must_use]
pub fn analyser_image(
    largeur: u32,
    hauteur: u32,
    rgba: Vec<u8>,
    t_ms: u64,
    vignette: String,
) -> Option<ImageCle> {
    let frame = Frame::new(largeur, hauteur, rgba)?;
    let cartes = detect_cards(&frame);
    let lecture = arrange(&frame, &cartes);
    Some(ImageCle {
        t_ms,
        vignette,
        largeur,
        hauteur,
        cartes,
        lecture,
    })
}

fn lire(modeles: &Templates, c: &DetectedCard) -> Option<Recognized> {
    recognize(modeles, &c.patch)
}

/// Les cartes reconnues d'une liste d'index (main ou board).
fn reconnues(modeles: &Templates, img: &ImageCle, index: &[usize]) -> Vec<Option<Recognized>> {
    index
        .iter()
        .map(|i| img.cartes.get(*i).and_then(|c| lire(modeles, c)))
        .collect()
}

fn texte_temps(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Une carte reconnue (ou douteuse), cliquable pour la corriger.
fn carte_lue(
    r: Option<Recognized>,
    img: usize,
    carte: usize,
    choix: State<Option<(usize, usize)>>,
) -> Element {
    let doute = r.is_none_or(|r| r.confidence < DOUTE);
    let titre = match r {
        Some(r) => format!(
            "{} · confiance {} · toucher pour corriger",
            r.card,
            pct(r.confidence)
        ),
        None => "Carte inconnue · toucher pour l'indiquer".to_owned(),
    };
    button()
        .class(if doute {
            "carte-lue doute"
        } else {
            "carte-lue"
        })
        .attr("type", "button")
        .attr("title", titre.clone())
        .attr("aria-label", titre)
        .on_click(move |_| choix.set(Some((img, carte))))
        .child(carte_html(r.map(|r| r.card)).class("small"))
}

/// Le sélecteur de carte (13 rangs × 4 couleurs) pour corriger une lecture.
fn selecteur(
    choix: State<Option<(usize, usize)>>,
    images: State<Vec<ImageCle>>,
    modeles: State<Templates>,
) -> Vec<Node> {
    let Some((i, c)) = choix.get() else {
        return Vec::new();
    };
    let apercu = images.with(|v| {
        v.get(i).and_then(|img| {
            img.cartes.get(c).map(|carte| {
                let r = carte.rect;
                // La carte découpée dans la vignette, en arrière-plan CSS.
                let sx = f64::from(img.largeur) / f64::from(r.w.max(1));
                let sy = f64::from(img.hauteur) / f64::from(r.h.max(1));
                let px = f64::from(r.x) / f64::from((img.largeur - r.w).max(1)) * 100.0;
                let py = f64::from(r.y) / f64::from((img.hauteur - r.h).max(1)) * 100.0;
                format!(
                    "background-image:url({});background-size:{:.1}% {:.1}%;background-position:{:.2}% {:.2}%",
                    img.vignette,
                    sx * 100.0,
                    sy * 100.0,
                    px.clamp(0.0, 100.0),
                    py.clamp(0.0, 100.0)
                )
            })
        })
    });
    let couleurs = [
        (Suit::Spades, "♠", false),
        (Suit::Hearts, "♥", true),
        (Suit::Diamonds, "♦", true),
        (Suit::Clubs, "♣", false),
    ];
    let rangs: Vec<Rank> = Rank::ALL.into_iter().rev().collect();
    vec![Node::from(
        div()
            .class("picker card")
            .attr("role", "dialog")
            .attr("aria-label", "Indiquer la carte")
            .child(
                div()
                    .class("picker-head")
                    .child(div().class("picker-crop").attr("style", apercu.unwrap_or_default()))
                    .child(
                        div()
                            .child(h3().text("Quelle est cette carte ?"))
                            .child(p().class("fine").text(
                                "La reconnaissance apprend le jeu de cartes de votre salle : chaque carte indiquée lui apprend son rang et sa couleur, pour toute la session et les suivantes.",
                            )),
                    ),
            )
            .child(div().class("picker-grid").children(couleurs.iter().flat_map(|(su, sym, rouge)| {
                let rangs = rangs.clone();
                rangs.into_iter().map(move |r| {
                    let carte = Card::new(r, *su);
                    let rang = match r.to_char() {
                        'T' => "10".to_owned(),
                        x => x.to_string(),
                    };
                    button()
                        .class(if *rouge { "pick red" } else { "pick" })
                        .attr("type", "button")
                        .attr("aria-label", carte.to_string())
                        .on_click(move |_| {
                            let patch = images.with(|v| v.get(i).and_then(|img| img.cartes.get(c)).map(|k| k.patch.clone()));
                            if let Some(patch) = patch {
                                modeles.update(|m| m.learn(&patch, carte));
                                #[cfg(target_arch = "wasm32")]
                                if let Ok(json) = paquets::serde_json::to_string(&modeles.get()) {
                                    crate::media::garder(CLE_MODELES, &json);
                                }
                            }
                            choix.set(None);
                        })
                        .text(format!("{rang}{sym}"))
                })
            })))
            .child(
                div()
                    .class("actions")
                    .child(button().class("btn btn-ghost btn-sm").attr("type", "button").on_click(move |_| choix.set(None)).text("Fermer")),
            ),
    )]
}

fn position_de(nom: &str) -> Position {
    match nom {
        "UTG" => Position::UnderTheGun,
        "HJ" => Position::Hijack,
        "CO" => Position::Cutoff,
        "SB" => Position::SmallBlind,
        "BB" => Position::BigBlind,
        _ => Position::Button,
    }
}

fn nombre(s: &str) -> f32 {
    s.trim().replace(',', ".").parse().unwrap_or(0.0)
}

/// Le conseil d'un spot, affiché.
fn rendu_conseil(a: &SpotAdvice) -> Element {
    let facing = a.facing_bet;
    let options: Vec<(Decision, f32)> = Decision::ALL
        .iter()
        .filter(|o| **o != Decision::Fold || facing)
        .filter_map(|o| a.ev[o.index()].filter(|v| v.is_finite()).map(|v| (*o, v)))
        .collect();
    div()
        .class("card conseil")
        .child(p().class("eyebrow").text(format!(
            "{} · {}",
            a.engine,
            match a.basis {
                Basis::PushFold => "équilibre push/fold",
                Basis::Ev => "référence EV",
            }
        )))
        .child(h3().text(format!("À jouer : {}", a.best_text)))
        .child(p().text(a.summary.clone()))
        .child(div().class("ev-bars").children(options.into_iter().map(|(o, v)| {
            p().class(if o == a.best { "ev-line best" } else { "ev-line" })
                .child(span().text(o.label(facing)))
                .child(strong().text(format!("{:+.1}", v).replace('.', ",") + " BB"))
        })))
        .child(p().class("fine").text(format!(
            "Équité {}{}. Après la partie seulement : STRATYX ne conseille jamais pendant une partie réelle.",
            pct(a.equity),
            if facing { format!(" · cote {}", pct(a.pot_odds)) } else { String::new() }
        )))
}

/// L'îlot.
pub fn capture(_props: &str) -> Node {
    let etat = use_state(Etat::Pret);
    let images: State<Vec<ImageCle>> = use_state(Vec::new());
    let modeles: State<Templates> = use_state(Templates::default());
    let choix: State<Option<(usize, usize)>> = use_state(None);
    let message = use_state(String::from(
        "Filmez votre écran pendant la session, ou importez une vidéo ou des captures. L'analyse commence quand la session est terminée.",
    ));
    let debut = use_state(0f64);
    let duree = use_state(0u64);
    let image_choisie: State<Option<usize>> = use_state(None);
    let format = use_state(String::from("expresso"));
    let position = use_state(String::from("BTN"));
    let tapis = use_state(String::from("15"));
    let pot = use_state(String::from("3"));
    let a_suivre = use_state(String::from("0"));
    let situation = use_state(String::from("ouvert"));
    let relance = use_state(String::from("2"));
    let adversaires = use_state(String::from("1"));
    let conseil: State<Option<Result<SpotAdvice, String>>> = use_state(None);

    #[cfg(target_arch = "wasm32")]
    let moteur = std::rc::Rc::new(std::cell::RefCell::new(navigateur::Moteur::default()));

    let analyser_spot = move || {
        let Some(i) = image_choisie.get() else { return };
        let m = modeles.get();
        let Some(img) = images.with(|v| v.get(i).cloned()) else {
            return;
        };
        let main: Vec<Card> = reconnues(&m, &img, &img.lecture.hero)
            .into_iter()
            .flatten()
            .map(|r| r.card)
            .collect();
        let board: Vec<Card> = reconnues(&m, &img, &img.lecture.board)
            .into_iter()
            .flatten()
            .map(|r| r.card)
            .collect();
        let Some(hole) = (main.len() == 2)
            .then(|| HoleCards::new(main[0], main[1]))
            .flatten()
        else {
            conseil.set(Some(Err(
                "Les deux cartes du joueur ne sont pas lues : touchez-les pour les indiquer."
                    .into(),
            )));
            return;
        };
        let specialist = match format.get().as_str() {
            "expresso" => Some(Specialist::Expresso3Max),
            "6max" => Some(Specialist::Tournament6Max),
            _ => None,
        };
        let players = if specialist == Some(Specialist::Expresso3Max) {
            3
        } else {
            6
        };
        let situation = if board.len() >= 3 {
            Situation::Postflop {
                opponents: nombre(&adversaires.get()).max(1.0) as u8,
            }
        } else {
            match situation.get().as_str() {
                "relance" => Situation::FacingRaise {
                    raise_to_bb: nombre(&relance.get()).max(2.0),
                },
                "tapis" => Situation::FacingAllIn,
                _ => Situation::Unopened,
            }
        };
        let input = SpotInput {
            specialist,
            players,
            position: position_de(&position.get()),
            hole,
            board,
            effective_bb: nombre(&tapis.get()).max(1.0),
            pot_bb: nombre(&pot.get()),
            to_call_bb: nombre(&a_suivre.get()),
            situation,
        };
        conseil.set(Some(analyse_spot(&input, 3000).map_err(|e| e.to_string())));
    };

    // Les boutons de démarrage (navigateur seulement).
    #[cfg(target_arch = "wasm32")]
    let (demarrer, arreter, importer_video, importer_images) =
        navigateur::actions(moteur.clone(), etat, images, message, debut, duree);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = debut;
    #[cfg(not(target_arch = "wasm32"))]
    let (demarrer, arreter, importer_video, importer_images): Actions = (
        std::rc::Rc::new(|_s: bool| {}),
        std::rc::Rc::new(|| {}),
        std::rc::Rc::new(|_e: paquets::active::Event| {}),
        std::rc::Rc::new(|_e: paquets::active::Event| {}),
    );

    div()
        .class("capture")
        .on_mount(move |_| {
            #[cfg(target_arch = "wasm32")]
            if let Some(m) = crate::media::relire(CLE_MODELES).and_then(|j| paquets::serde_json::from_str::<Templates>(&j).ok()) {
                modeles.set(m);
            }
        })
        .child(
            video()
                .class("capture-video")
                .attr("muted", "")
                .attr("playsinline", "")
                .attr("aria-hidden", "true")
                .use_action(move |dom| {
                    #[cfg(target_arch = "wasm32")]
                    navigateur::brancher(&moteur, dom);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = dom;
                    || {}
                }),
        )
        .child(
            div()
                .class("tool card capture-panel")
                .child(div().class("capture-actions").children_dyn(move || match etat.get() {
                    Etat::Enregistrement | Etat::Lecture => vec![
                        Node::from(
                            p().class("rec")
                                .child(span().class("rec-dot").attr("aria-hidden", "true"))
                                .text_dyn(move || {
                                    format!(
                                        "{} · {} · {} images gardées",
                                        if etat.get() == Etat::Lecture { "Lecture de la vidéo" } else { "Enregistrement" },
                                        texte_temps(duree.get()),
                                        images.with(Vec::len)
                                    )
                                }),
                        ),
                        Node::from(
                            p().class("fine").text("Aucune analyse ni aucun conseil pendant la partie : tout commence à l'arrêt."),
                        ),
                        Node::from(
                            button()
                                .class("btn btn-primary magnetic")
                                .attr("type", "button")
                                .on_click({
                                    let f = arreter.clone();
                                    move |_| f()
                                })
                                .text("Arrêter et analyser"),
                        ),
                    ],
                    _ => vec![
                        Node::from(
                            button()
                                .class("btn btn-primary magnetic")
                                .attr("type", "button")
                                .on_click({
                                    let f = demarrer.clone();
                                    move |_| f(true)
                                })
                                .text("Filmer l'écran"),
                        ),
                        Node::from(
                            button()
                                .class("btn btn-ghost")
                                .attr("type", "button")
                                .on_click({
                                    let f = demarrer.clone();
                                    move |_| f(false)
                                })
                                .text("Filmer avec la caméra"),
                        ),
                        Node::from(
                            label()
                                .class("btn btn-ghost file-btn")
                                .text("Importer une vidéo")
                                .child(input().attr("type", "file").attr("accept", "video/*").on("change", {
                                    let f = importer_video.clone();
                                    move |e| f(e)
                                })),
                        ),
                        Node::from(
                            label()
                                .class("btn btn-ghost file-btn")
                                .text("Importer des captures")
                                .child(
                                    input()
                                        .attr("type", "file")
                                        .attr("accept", "image/*")
                                        .attr("multiple", "")
                                        .on("change", {
                                            let f = importer_images.clone();
                                            move |e| f(e)
                                        }),
                                ),
                        ),
                    ],
                }))
                .child(p().class("tool-info").attr("aria-live", "polite").text_dyn(move || message.get())),
        )
        .child(div().class("picker-zone").children_dyn(move || selecteur(choix, images, modeles)))
        .child(div().class("revue").children_dyn(move || {
            if etat.get() != Etat::Revue {
                return Vec::new();
            }
            let m = modeles.get();
            let n = images.with(Vec::len);
            let mut out = vec![Node::from(
                div()
                    .class("revue-head")
                    .child(h3().text(format!("{n} image(s) de la session")))
                    .child(p().class("fine").text(format!(
                        "Reconnaissance : {} rang(s) et {} couleur(s) appris sur votre jeu de cartes. Les cartes douteuses sont entourées : touchez-les pour les corriger. Choisissez ensuite une image pour analyser le spot.",
                        m.ranks_known(),
                        m.suits_known()
                    ))),
            )];
            out.extend(images.with(|v| {
                v.iter()
                    .enumerate()
                    .map(|(i, img)| {
                        let main = reconnues(&m, img, &img.lecture.hero);
                        let board = reconnues(&m, img, &img.lecture.board);
                        let boites = img.cartes.iter().enumerate().map(|(k, c)| {
                            let r = c.rect;
                            let role = if img.lecture.hero.contains(&k) {
                                "boite main"
                            } else if img.lecture.board.contains(&k) {
                                "boite board"
                            } else {
                                "boite"
                            };
                            div().class(role).attr(
                                "style",
                                format!(
                                    "left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%",
                                    f64::from(r.x) / f64::from(img.largeur) * 100.0,
                                    f64::from(r.y) / f64::from(img.hauteur) * 100.0,
                                    f64::from(r.w) / f64::from(img.largeur) * 100.0,
                                    f64::from(r.h) / f64::from(img.hauteur) * 100.0
                                ),
                            )
                        });
                        Node::from(
                            article()
                                .class("image-cle card")
                                .class_if("choisie", move || image_choisie.get() == Some(i))
                                .child(
                                    div()
                                        .class("vignette")
                                        .child(img_el(&img.vignette))
                                        .children(boites),
                                )
                                .child(
                                    div()
                                        .class("lu")
                                        .child(span().class("muted").text(texte_temps(img.t_ms)))
                                        .child(div().class("hand-mini").children(
                                            img.lecture.hero.iter().zip(main).map(|(k, r)| carte_lue(r, i, *k, choix)),
                                        ))
                                        .child(div().class("hand-mini board-mini").children(
                                            img.lecture.board.iter().zip(board).map(|(k, r)| carte_lue(r, i, *k, choix)),
                                        )),
                                )
                                .child(
                                    button()
                                        .class("btn btn-ghost btn-sm")
                                        .attr("type", "button")
                                        .on_click(move |_| {
                                            image_choisie.set(Some(i));
                                            conseil.set(None);
                                        })
                                        .text("Analyser ce spot"),
                                ),
                        )
                    })
                    .collect::<Vec<_>>()
            }));
            out
        }))
        .child(div().class("spot-zone").children_dyn(move || {
            if image_choisie.get().is_none() || etat.get() != Etat::Revue {
                return Vec::new();
            }
            let champ = |id: &'static str, libelle: &'static str, s: State<String>| {
                div()
                    .class("field")
                    .child(label().attr("for", id).text(libelle))
                    .child(input().id(id).attr("type", "text").attr("inputmode", "decimal").bind_value(s))
            };
            let choix_liste = |id: &'static str, libelle: &'static str, s: State<String>, options: &'static [(&'static str, &'static str)]| {
                div().class("field").child(label().attr("for", id).text(libelle)).child(
                    select()
                        .id(id)
                        .bind_value(s)
                        .children(options.iter().map(|(v, t)| option().attr("value", *v).text(*t))),
                )
            };
            vec![Node::from(
                form()
                    .class("tool card spot-form")
                    .on_submit(move |_| analyser_spot())
                    .child(h3().text(format!("Spot de l'image {}", image_choisie.get().map_or(0, |i| i + 1))))
                    .child(p().class("fine").text(
                        "Les cartes viennent de l'image ; indiquez ce que la vision ne lit pas encore : format, position, tapis effectif, pot et montant à suivre (en grosses blindes).",
                    ))
                    .child(
                        div()
                            .class("row-3")
                            .child(choix_liste("sp-format", "Format", format, &[("expresso", "Expresso 3-Max"), ("6max", "Tournoi 6-Max"), ("autre", "Autre (moteur commun)")]))
                            .child(choix_liste("sp-pos", "Position", position, &[("BTN", "Bouton"), ("SB", "Petite blinde"), ("BB", "Grosse blinde"), ("UTG", "UTG (6-max)"), ("HJ", "HJ (6-max)"), ("CO", "CO (6-max)")]))
                            .child(champ("sp-tapis", "Tapis effectif (BB)", tapis)),
                    )
                    .child(
                        div()
                            .class("row-3")
                            .child(choix_liste("sp-sit", "Avant vous (préflop)", situation, &[("ouvert", "Personne n'est entré"), ("relance", "Une relance"), ("tapis", "Un tapis")]))
                            .child(champ("sp-relance", "Relance à (BB)", relance))
                            .child(champ("sp-adv", "Adversaires en jeu (après le flop)", adversaires)),
                    )
                    .child(
                        div()
                            .class("row-2")
                            .child(champ("sp-pot", "Pot affiché (BB, après le flop)", pot))
                            .child(champ("sp-suivre", "À suivre (BB, après le flop)", a_suivre)),
                    )
                    .child(button().class("btn btn-primary magnetic").attr("type", "submit").text("Analyser ce spot"))
                    .child(div().class("conseil-zone").children_dyn(move || match conseil.get() {
                        Some(Ok(a)) => vec![Node::from(rendu_conseil(&a))],
                        Some(Err(e)) => vec![Node::from(p().class("alert").attr("role", "alert").text(format!("⚠️ {e}")))],
                        None => Vec::new(),
                    })),
            )]
        }))
        .into()
}

fn img_el(src: &str) -> Element {
    paquets::prelude::img()
        .attr("src", src.to_owned())
        .attr("alt", "Image de la session")
        .attr("loading", "lazy")
}

/// Le côté navigateur : capture, échantillonnage, import.
#[cfg(target_arch = "wasm32")]
mod navigateur {
    use std::cell::RefCell;
    use std::rc::Rc;

    use paquets::prelude::State;
    use stratyx_vision::Keyframer;
    use wasm_bindgen::JsCast;

    use super::{Etat, ImageCle, MAX_IMAGES, analyser_image};
    use crate::media;

    #[derive(Default)]
    pub struct Moteur {
        pub video: Option<web_sys::HtmlVideoElement>,
        pub minuteur: Option<media::Minuteur>,
        pub images_cles: Option<Keyframer>,
    }

    pub fn brancher(moteur: &Rc<RefCell<Moteur>>, dom: &paquets::active::Dom) {
        moteur.borrow_mut().video = dom.clone().dyn_into::<web_sys::HtmlVideoElement>().ok();
    }

    fn maintenant() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now())
    }

    /// Prend une image de la vidéo et la garde si la table a changé.
    fn echantillon(moteur: &Rc<RefCell<Moteur>>, images: State<Vec<ImageCle>>, t_ms: u64) {
        let mut m = moteur.borrow_mut();
        let Some(video) = m.video.clone() else { return };
        let Some(cap) = media::image_video(&video, 1280) else {
            return;
        };
        let Some(frame) = stratyx_vision::Frame::new(cap.largeur, cap.hauteur, cap.rgba.clone())
        else {
            return;
        };
        let garder = m
            .images_cles
            .get_or_insert_with(Keyframer::new)
            .push(&frame);
        drop(m);
        if garder && images.with(Vec::len) < MAX_IMAGES {
            let vignette = media::vignette(&cap, 480).unwrap_or_default();
            if let Some(img) = analyser_image(cap.largeur, cap.hauteur, cap.rgba, t_ms, vignette) {
                images.update(|v| v.push(img));
            }
        }
    }

    use super::Actions;

    pub fn actions(
        moteur: Rc<RefCell<Moteur>>,
        etat: State<Etat>,
        images: State<Vec<ImageCle>>,
        message: State<String>,
        debut: State<f64>,
        duree: State<u64>,
    ) -> Actions {
        let a: Actions = {
            let m = moteur.clone();
            let arreter: Rc<dyn Fn()> = Rc::new(move || {
                let mut mo = m.borrow_mut();
                mo.minuteur = None;
                mo.images_cles = None;
                if let Some(v) = &mo.video {
                    media::arreter(v);
                }
                drop(mo);
                let n = images.with(Vec::len);
                etat.set(Etat::Revue);
                message.set(format!("Session close : {n} image(s) gardée(s). Corrigez les cartes douteuses, puis analysez chaque spot."));
            });
            let m2 = moteur.clone();
            let a2 = arreter.clone();
            let demarrer: Rc<dyn Fn(bool)> = Rc::new(move |ecran: bool| {
                let Some(video) = m2.borrow().video.clone() else {
                    return;
                };
                let m3 = m2.clone();
                let a3 = a2.clone();
                message.set("Autorisez la capture dans la fenêtre du navigateur…".into());
                media::demarrer(
                    if ecran {
                        media::Source::Ecran
                    } else {
                        media::Source::Camera
                    },
                    &video,
                    move |r| match r {
                        Ok(()) => {
                            images.set(Vec::new());
                            debut.set(maintenant());
                            duree.set(0);
                            etat.set(Etat::Enregistrement);
                            message.set("Enregistrement en cours. Jouez normalement ; arrêtez à la fin de la session.".into());
                            let m4 = m3.clone();
                            let a4 = a3.clone();
                            let minuteur = media::Minuteur::new(600, move || {
                                let t = (maintenant() - debut.get()).max(0.0) as u64;
                                duree.set(t);
                                // La capture s'arrête si l'utilisateur coupe le partage d'écran.
                                let fini = m4
                                    .borrow()
                                    .video
                                    .as_ref()
                                    .is_none_or(|v| v.src_object().is_none() || v.ended());
                                if fini {
                                    a4();
                                    return;
                                }
                                echantillon(&m4, images, t);
                            });
                            m3.borrow_mut().minuteur = minuteur;
                        }
                        Err(e) => message.set(format!("⚠️ {e}")),
                    },
                );
            });
            let m5 = moteur.clone();
            let a5 = arreter.clone();
            let importer_video: Rc<dyn Fn(paquets::active::Event)> = Rc::new(move |e| {
                let Some(fichier) = media::fichiers(&e).into_iter().next() else {
                    return;
                };
                let Some(video) = m5.borrow().video.clone() else {
                    return;
                };
                if let Err(err) = media::lire_video(&fichier, &video, 4.0) {
                    message.set(format!("⚠️ {err}"));
                    return;
                }
                images.set(Vec::new());
                etat.set(Etat::Lecture);
                message.set("Lecture accélérée de la vidéo…".into());
                let m6 = m5.clone();
                let a6 = a5.clone();
                let minuteur = media::Minuteur::new(150, move || {
                    let v = m6.borrow().video.clone();
                    let Some(v) = v else { return };
                    if media::finie(&v) {
                        a6();
                        return;
                    }
                    let t = media::temps_ms(&v);
                    duree.set(t);
                    echantillon(&m6, images, t);
                });
                m5.borrow_mut().minuteur = minuteur;
            });
            let importer_images: Rc<dyn Fn(paquets::active::Event)> = Rc::new(move |e| {
                let fichiers = media::fichiers(&e);
                if fichiers.is_empty() {
                    return;
                }
                let n = fichiers.len();
                images.set(Vec::new());
                message.set(format!("Lecture de {n} capture(s)…"));
                media::lire_images(
                    fichiers,
                    1600,
                    move |i, cap| {
                        let vignette = media::vignette(&cap, 480).unwrap_or_default();
                        if let Some(img) = analyser_image(
                            cap.largeur,
                            cap.hauteur,
                            cap.rgba,
                            i as u64 * 1000,
                            vignette,
                        ) {
                            images.update(|v| v.push(img));
                        }
                    },
                    move || {
                        etat.set(Etat::Revue);
                        message.set(format!("{} capture(s) lue(s). Corrigez les cartes douteuses, puis analysez chaque spot.", images.with(Vec::len)));
                    },
                );
            });
            (demarrer, arreter, importer_video, importer_images)
        };
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_ilot_se_rend_sans_rien_lire() {
        let html = capture("").render();
        assert!(html.contains("Filmer avec la caméra"), "{html}");
        assert!(html.contains("Importer des captures"));
        assert!(
            !html.contains("carte-lue"),
            "rien n'est lu avant la fin de la session"
        );
    }

    #[test]
    fn une_image_vide_ne_donne_aucune_carte() {
        let img = analyser_image(64, 48, vec![30u8; 64 * 48 * 4], 0, String::new()).unwrap();
        assert!(img.cartes.is_empty());
        assert!(analyser_image(64, 48, vec![0u8; 10], 0, String::new()).is_none());
    }
}
