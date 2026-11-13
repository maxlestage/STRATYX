//! L'îlot `analyse` de la page `/app` : on colle (ou partage, ou importe) un historique de
//! main, et le navigateur l'analyse avec le vrai moteur et les modèles entraînés des deux
//! IA (crate `stratyx-analytics`), compilés en WebAssembly. Rien n'est envoyé : l'analyse
//! marche aussi hors ligne, dans l'application installée.

use paquets::prelude::*;
use stratyx_analytics::{
    AnalysisError, DecisionReview, HandAnalysis, ImportedHand, Options, Severity, bb, import_text,
    pct,
};
use stratyx_poker::Street;
use stratyx_poker::import::{SAMPLE_POKERSTARS, SAMPLE_WINAMAX};
use stratyx_strategy::{Basis, Decision};

use crate::{carte_html, plus_tard};

/// Un exemple pédagogique : un call de tapis trop léger en tête-à-tête (gagné à la river,
/// mais perdant à long terme).
pub const EXEMPLE_CALL: &str = "PokerStars Hand #250000000042: Tournament #3500000042, $1.00+$0.00 USD Hold'em No Limit - Level III (20/40) - 2026/10/04 20:02:11 ET
Table '3500000042 1' 3-max Seat #1 is the button
Seat 1: Vilain (1000 in chips)
Seat 2: Hero (1000 in chips)
Vilain: posts small blind 20
Hero: posts big blind 40
*** HOLE CARDS ***
Dealt to Hero [Jd 4c]
Vilain: raises 960 to 1000 and is all-in
Hero: calls 960 and is all-in
*** FLOP *** [Js 8h 3d]
*** TURN *** [Js 8h 3d] [Kc]
*** RIVER *** [Js 8h 3d Kc] [2s]
*** SHOW DOWN ***
Vilain: shows [Ah Qh]
Hero: shows [Jd 4c]
Hero collected 2000 from pot
*** SUMMARY ***
";

const EXEMPLES: [(&str, &str); 3] = [
    ("Expresso Winamax", SAMPLE_WINAMAX),
    ("Tournoi 6-max PokerStars", SAMPLE_POKERSTARS),
    ("Call de tapis léger", EXEMPLE_CALL),
];

/// Le résultat affiché.
#[derive(Clone, Debug, PartialEq)]
enum Resultat {
    Rien,
    Erreur(String),
    Pret(Box<HandAnalysis>),
}

fn options() -> Options {
    Options {
        samples: 2500,
        ..Options::default()
    }
}

fn analyser_main(main: &ImportedHand, joueur: &str) -> Resultat {
    let joueur = joueur.trim();
    let hero = if joueur.is_empty() {
        main.hero.as_deref()
    } else {
        Some(joueur)
    };
    match stratyx_analytics::analyse(&main.record, hero, &options()) {
        Ok(a) => Resultat::Pret(Box::new(a)),
        Err(AnalysisError::NoHero) => Resultat::Erreur(
            "Les cartes du joueur analysé sont inconnues dans cette main : indiquez son pseudo, ou choisissez une main où ses cartes sont données (« Dealt to »).".into(),
        ),
        Err(e) => Resultat::Erreur(e.to_string()),
    }
}

fn classe_gravite(s: Severity) -> &'static str {
    match s {
        Severity::Good => "sev sev-ok",
        Severity::Inaccuracy => "sev sev-imp",
        Severity::Mistake => "sev sev-err",
        Severity::Blunder => "sev sev-grave",
    }
}

fn rue(s: Street) -> &'static str {
    match s {
        Street::Preflop => "Préflop",
        Street::Flop => "Flop",
        Street::Turn => "Turn",
        Street::River => "River",
    }
}

fn cartes(liste: &[String]) -> Element {
    div().class("hand-mini").children(
        liste
            .iter()
            .map(|c| carte_html(c.parse().ok()).class("small")),
    )
}

/// Une jauge de note, en SVG.
fn jauge(score: u8) -> Element {
    let r = 34.0f32;
    let c = 2.0 * std::f32::consts::PI * r;
    let plein = c * f32::from(score) / 100.0;
    let teinte = match score {
        85.. => "var(--accent, #16c784)",
        60..=84 => "#f5b942",
        _ => "#ff4d5e",
    };
    div().class("score").child(span().class("sr-only").text(format!("Note {score} sur 100"))).html(format!(
        r#"<svg viewBox="0 0 80 80" aria-hidden="true"><circle cx="40" cy="40" r="{r}" class="score-track"/><circle cx="40" cy="40" r="{r}" class="score-fill" style="stroke:{teinte};stroke-dasharray:{plein:.1} {c:.1}"/><text x="40" y="46" text-anchor="middle" class="score-num">{score}</text></svg>"#
    ))
}

/// Les EV de chaque option, en barres.
fn barres_ev(d: &DecisionReview) -> Element {
    let options: Vec<(Decision, f32)> = Decision::ALL
        .iter()
        // Sans mise à suivre, se coucher n'a pas de sens : la parole est gratuite.
        .filter(|o| **o != Decision::Fold || d.to_call_bb > 0.0)
        .filter_map(|o| d.ev[o.index()].filter(|v| v.is_finite()).map(|v| (*o, v)))
        .collect();
    let max = options.iter().map(|o| o.1.abs()).fold(0.5f32, f32::max);
    let (best, played, facing) = (d.best, d.played, d.to_call_bb > 0.0);
    div()
        .class("ev-bars")
        .children(options.into_iter().map(move |(o, v)| {
            let largeur = (v.abs() / max * 50.0).min(50.0);
            let style = if v >= 0.0 {
                format!("left:50%;width:{largeur:.1}%")
            } else {
                format!("left:{:.1}%;width:{largeur:.1}%", 50.0 - largeur)
            };
            div()
                .class("ev-row")
                .class(if o == best { "best" } else { "" })
                .class(if o == played { "played" } else { "" })
                .child(span().class("ev-name").text(o.label(facing)))
                .child(
                    div().class("ev-track").child(
                        div()
                            .class(if v >= 0.0 {
                                "ev-fill pos"
                            } else {
                                "ev-fill neg"
                            })
                            .attr("style", style),
                    ),
                )
                .child(
                    span()
                        .class("ev-val")
                        .text(format!("{:+.1}", v).replace('.', ",") + " BB"),
                )
        }))
}

fn carte_decision(i: usize, d: &DecisionReview, ia: bool) -> Element {
    let mut carte = article()
        .class("card decision")
        .id(format!("decision-{}", i + 1))
        .child(
            div()
                .class("decision-head")
                .child(
                    p().class("decision-title")
                        .child(strong().text(format!("Décision {}", i + 1)))
                        .text(format!(" · {} · {}", rue(d.street), d.position)),
                )
                .child(
                    span()
                        .class(classe_gravite(d.severity))
                        .text(d.severity.label()),
                ),
        );
    if !d.board.is_empty() {
        carte = carte.child(cartes(&d.board));
    }
    carte = carte
        .child(
            div()
                .class("decision-grid")
                .child(
                    div()
                        .class("kv")
                        .child(span().text("Joué"))
                        .child(strong().text(d.played_text.clone())),
                )
                .child(
                    div()
                        .class("kv")
                        .child(span().text(match d.basis {
                            Basis::PushFold => "Référence (push/fold)",
                            Basis::Ev => "Référence (EV)",
                        }))
                        .child(strong().text(d.best_text.clone())),
                )
                .child(
                    div()
                        .class("kv")
                        .child(span().text("Tapis effectif"))
                        .child(strong().text(bb(d.effective_bb))),
                )
                .child(
                    div()
                        .class("kv")
                        .child(span().text("Pot"))
                        .child(strong().text(bb(d.pot_bb))),
                ),
        )
        .child(
            div()
                .class("meter")
                .child(
                    p().class("meter-label")
                        .text(format!("Équité {}", pct(d.equity))),
                )
                .child(
                    div()
                        .class("meter-track")
                        .child(
                            div()
                                .class("meter-fill")
                                .attr("style", format!("width:{:.1}%", d.equity * 100.0)),
                        )
                        .child(if d.facing_bet {
                            div()
                                .class("meter-mark")
                                .attr("style", format!("left:{:.1}%", d.pot_odds * 100.0))
                                .attr("title", format!("Cote du pot : {}", pct(d.pot_odds)))
                        } else {
                            div().class("alert-none")
                        }),
                )
                .child(p().class("fine").text(if d.facing_bet {
                    format!(
                        "Trait : équité nécessaire pour suivre ({}).",
                        pct(d.pot_odds)
                    )
                } else {
                    "Équité contre les ranges estimées des adversaires encore en jeu.".to_owned()
                })),
        )
        .child(barres_ev(d))
        .child(p().class("decision-comment").text(d.comment.clone()));
    if ia {
        let mut extra = div().class("ia-row");
        if let Some((m, conf)) = d.model {
            extra = extra.child(
                p().class("chip-info")
                    .child(span().text("Modèle de décision"))
                    .child(strong().text(format!(
                        "{} · {}",
                        m.label(d.to_call_bb > 0.0),
                        pct(conf)
                    ))),
            );
        }
        if let Some(pop) = d.population {
            extra = extra.child(
                p().class("chip-info")
                    .child(span().text("La population ici"))
                    .child(strong().text(format!(
                        "couché {} · suit {} · relance {} · tapis {}",
                        pct(pop[0]),
                        pct(pop[1]),
                        pct(pop[2]),
                        pct(pop[3])
                    ))),
            );
        }
        carte = carte.child(extra);
    }
    if !d.villains.is_empty() {
        carte = carte.child(p().class("fine").text(format!(
                "Ranges estimées : {}.",
                d.villains
                    .iter()
                    .map(|(n, f)| format!("{n} {}", pct(*f)))
                    .collect::<Vec<_>>()
                    .join(" · ")
            )));
    }
    carte
}

fn rendu(a: &HandAnalysis) -> Vec<Node> {
    let ia = a.engine.starts_with("IA");
    let entete = div()
        .class("card analyse-head")
        .child(jauge(a.summary.score))
        .child(
            div()
                .class("analyse-meta")
                .child(
                    p().class("eyebrow")
                        .text(format!("{} · {}", a.format, a.engine)),
                )
                .child(h3().text(a.summary.verdict.clone()))
                .child(p().class("muted").text(format!(
                    "{} · {} · {} joueurs · tapis {} · EV perdue {}",
                    a.hero,
                    a.position,
                    a.players,
                    bb(a.stack_bb),
                    bb(a.summary.ev_lost_bb)
                )))
                .child(
                    div()
                        .class("analyse-cards")
                        .child(
                            div().class("hand").children(
                                [&a.hero_cards[..2], &a.hero_cards[2..]]
                                    .map(|c| carte_html(c.parse().ok())),
                            ),
                        )
                        .child(
                            div()
                                .class("board")
                                .children(a.board.iter().map(|c| carte_html(c.parse().ok()))),
                        ),
                )
                .child(p().class("fine").text(a.outcome.clone())),
        );

    // Le déroulé de la main, tour par tour.
    let mut deroule = div()
        .class("card timeline-hand")
        .child(h3().text("Déroulé"));
    let mut liste = ol().class("hh-lines");
    for l in &a.lines {
        let mut li_el = li().class(if l.player.is_empty() {
            "hh-street"
        } else if l.hero {
            "hh-line moi"
        } else {
            "hh-line"
        });
        if l.player.is_empty() {
            li_el = li_el.child(span().text(l.text.clone()));
        } else {
            li_el = li_el
                .child(span().class("hh-pos").text(l.position.clone()))
                .child(span().class("hh-name").text(l.player.clone()))
                .child(span().class("hh-act").text(l.text.clone()));
            if let Some(i) = l.decision
                && let Some(d) = a.decisions.get(i)
            {
                li_el = li_el.child(
                    a_link(format!("#decision-{}", i + 1))
                        .class(classe_gravite(d.severity))
                        .text(d.severity.label()),
                );
            }
        }
        li_el = li_el.child(span().class("hh-pot").text(bb(l.pot_bb)));
        liste = liste.child(li_el);
    }
    deroule = deroule.child(liste);

    let conseils = div()
        .class("card tips")
        .child(h3().text("À travailler"))
        .child(
            ul().class("checks")
                .children(a.summary.tips.iter().map(|t| li().text(t.clone()))),
        );

    let mut out: Vec<Node> = vec![entete.into(), deroule.into()];
    out.extend(
        a.decisions
            .iter()
            .enumerate()
            .map(|(i, d)| Node::from(carte_decision(i, d, ia))),
    );
    out.push(conseils.into());
    out
}

fn a_link(href: String) -> Element {
    paquets::prelude::a().href(href)
}

/// Le texte partagé vers l'application (cible de partage du manifeste, `/app?texte=…`) ou
/// passé dans un lien (`/app#texte=…`), lu dans le navigateur seulement : le service worker
/// sert la page `/app` sans son adresse complète, le texte ne quitte pas l'appareil.
fn texte_partage() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let location = web_sys::window()?.location();
        let search = location.search().ok().unwrap_or_default();
        let hash = location.hash().ok().unwrap_or_default();
        let source = if hash.contains("texte=") {
            hash.trim_start_matches('#').to_owned()
        } else {
            search
        };
        let params = web_sys::UrlSearchParams::new_with_str(&source).ok()?;
        let texte: Vec<String> = ["titre", "texte", "lien"]
            .iter()
            .filter_map(|k| params.get(k))
            .filter(|v| !v.trim().is_empty())
            .collect();
        (!texte.is_empty()).then(|| texte.join("\n"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Lit le fichier choisi dans un `<input type="file">` et le donne à `cible`.
fn lire_fichier(e: &paquets::active::Event, cible: State<String>, apres: impl Fn() + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        let Some(input) = e
            .raw()
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        let Some(fichier) = input.files().and_then(|f| f.get(0)) else {
            return;
        };
        let Ok(lecteur) = web_sys::FileReader::new() else {
            return;
        };
        let l = lecteur.clone();
        let fin = Closure::once_into_js(move || {
            if let Some(t) = l.result().ok().and_then(|v| v.as_string()) {
                cible.set(t);
                apres();
            }
        });
        lecteur.set_onload(Some(fin.unchecked_ref()));
        let _ = lecteur.read_as_text(&fichier);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (e, cible, apres);
    }
}

/// L'îlot : formulaire, liste des mains, analyse.
pub fn analyse(props: &str) -> Node {
    let texte = use_state(props.to_owned());
    let joueur = use_state(String::new());
    let mains: State<Vec<ImportedHand>> = use_state(Vec::new());
    let choisie = use_state(0usize);
    let resultat = use_state(Resultat::Rien);
    let message = use_state(String::from(
        "Collez un ou plusieurs historiques (PokerStars, Winamax…), importez un fichier .txt, ou essayez un exemple.",
    ));
    let calcul = use_state(false);

    let analyser_index = move |i: usize| {
        calcul.set(true);
        choisie.set(i);
        plus_tard(move || {
            let r = mains.with(|m| m.get(i).map(|h| analyser_main(h, &joueur.get())));
            resultat.set(r.unwrap_or(Resultat::Rien));
            calcul.set(false);
        });
    };
    let lancer = move || {
        let rapport = import_text(&texte.get());
        let n = rapport.hands.len();
        let erreurs = rapport.errors.len();
        mains.set(rapport.hands);
        if n == 0 {
            resultat.set(Resultat::Erreur(rapport.errors.first().map_or_else(
                || "Aucune main reconnue.".to_owned(),
                ToString::to_string,
            )));
            message.set("Aucune main lisible dans ce texte.".into());
            return;
        }
        message.set(match (n, erreurs) {
            (1, 0) => "1 main lue.".to_owned(),
            (n, 0) => format!("{n} mains lues : choisissez celle à analyser."),
            (n, e) => format!("{n} mains lues, {e} illisible(s) (variante non prise en charge ou texte incomplet)."),
        });
        analyser_index(0);
    };

    div()
        .class("analyse")
        .on_mount(move |_| {
            if let Some(t) = texte_partage() {
                texte.set(t);
            }
            if !texte.get().trim().is_empty() {
                lancer();
            }
        })
        .child(
            form()
                .class("tool card analyse-form")
                .on_submit(move |_| lancer())
                .child(
                    div()
                        .class("field")
                        .child(label().attr("for", "hh").text("Historique de main"))
                        .child(
                            textarea()
                                .id("hh")
                                .attr("rows", "9")
                                .attr("spellcheck", "false")
                                .attr("autocomplete", "off")
                                .attr("placeholder", "Winamax Poker - Tournament \"Expresso\" … ou PokerStars Hand #…")
                                .bind_value(texte),
                        ),
                )
                .child(
                    div()
                        .class("row-2")
                        .child(
                            div()
                                .class("field")
                                .child(label().attr("for", "hh-joueur").text("Joueur analysé (facultatif)"))
                                .child(
                                    input()
                                        .id("hh-joueur")
                                        .attr("type", "text")
                                        .attr("autocomplete", "off")
                                        .attr("placeholder", "par défaut : celui dont les cartes sont données")
                                        .bind_value(joueur),
                                ),
                        )
                        .child(
                            div()
                                .class("field")
                                .child(label().attr("for", "hh-fichier").text("Ou un fichier d'historiques"))
                                .child(
                                    input()
                                        .id("hh-fichier")
                                        .attr("type", "file")
                                        .attr("accept", ".txt,text/plain")
                                        .on("change", move |e| lire_fichier(&e, texte, lancer)),
                                ),
                        ),
                )
                .child(
                    div()
                        .class("presets")
                        .child(span().class("muted").text("Exemples :"))
                        .children(EXEMPLES.map(|(nom, t)| {
                            button()
                                .class("chip")
                                .attr("type", "button")
                                .on_click(move |_| {
                                    texte.set(t.to_owned());
                                    lancer();
                                })
                                .text(nom)
                        })),
                )
                .child(
                    button()
                        .class("btn btn-primary magnetic")
                        .attr("type", "submit")
                        .bool_attr("disabled", move || calcul.get())
                        .text_dyn(move || {
                            if calcul.get() { "Analyse en cours…" } else { "Analyser la partie" }.into()
                        }),
                )
                .child(p().class("tool-info").attr("aria-live", "polite").text_dyn(move || message.get())),
        )
        .child(div().class("chips hands-list").children_dyn(move || {
            let liste = mains.get();
            if liste.len() < 2 {
                return Vec::new();
            }
            liste
                .iter()
                .enumerate()
                .take(60)
                .map(|(i, h)| {
                    let cartes = h
                        .record
                        .seats
                        .iter()
                        .find_map(|s| s.hole_cards.as_ref().map(|c| c.value.to_string()))
                        .unwrap_or_else(|| "?".into());
                    Node::from(
                        button()
                            .class("chip")
                            .attr("type", "button")
                            .attr_dyn("aria-pressed", move || (choisie.get() == i).to_string())
                            .on_click(move |_| analyser_index(i))
                            .text(format!("Main {} · {cartes}", i + 1)),
                    )
                })
                .collect()
        }))
        .child(div().class("analyse-result").attr("aria-live", "polite").children_dyn(move || {
            if calcul.get() {
                return vec![Node::from(p().class("tool-info").text("Le moteur rejoue la main et calcule chaque décision…"))];
            }
            match resultat.get() {
                Resultat::Rien => Vec::new(),
                Resultat::Erreur(e) => vec![Node::from(p().class("alert").attr("role", "alert").text(format!("⚠️ {e}")))],
                Resultat::Pret(a) => rendu(&a),
            }
        }))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_examples_are_analysed() {
        for (nom, t) in EXEMPLES {
            let r = import_text(t);
            assert_eq!(r.hands.len(), 1, "{nom}");
            let Resultat::Pret(a) = analyser_main(&r.hands[0], "") else {
                panic!("{nom}")
            };
            assert!(!a.decisions.is_empty(), "{nom}");
            let html = rendu(&a)
                .into_iter()
                .map(|n| n.render())
                .collect::<String>();
            assert!(html.contains("decision-1"), "{nom}");
        }
        let r = import_text(EXEMPLE_CALL);
        let Resultat::Pret(a) = analyser_main(&r.hands[0], "") else {
            panic!()
        };
        assert!(a.decisions[0].severity >= Severity::Mistake);
    }
}
